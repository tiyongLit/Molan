//! getattrlistbulk 并行遍历（work-stealing）—— 扫描引擎的 syscall 层。
//!
//! 对齐 lemon-cleaner `LMFileScanTask.m` 的 syscall 模型（每目录一次 bulk 取全部
//! 条目元数据，免除逐条目 lstat/namei），但修正其三个缺陷：
//! - 缓冲 512KB（柠檬 10KB，大目录需多次 bulk）；
//! - crossbeam work-stealing deque（柠檬为全局 mutex 任务栈 + usleep 自旋）；
//! - 遍历与聚合同步并行（柠檬扫描期零聚合、展示期单线程递归）。
//!
//! 缓冲布局依据 `getattrlist(2)` man：属性按 4 字节边界打包（含 64 位类型），
//! 组首 u32 length 8 字节对齐；变长 name 数据位于条目尾部，由 attrreference 偏移定位。
//! ATTR_CMN_ERROR 由内核特例放在 returned 集合之后、其余 common 属性之前
//!（man 示例代码与柠檬生产代码同序）。

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use crossbeam_deque::{Stealer, Worker};

/// 每 worker 的 bulk 缓冲（柠檬为 10KB 栈缓冲；此处放大到 512KB 减少 syscall 次数）。
const BULK_BUF_BYTES: usize = 512 * 1024;

// ── attr 位与 vnode 类型（sys/attr.h / sys/vnode.h；libc 未导出的本地补齐） ──

const ATTR_CMN_ERROR: u32 = 0x20000000;

const VDIR: u32 = 2;
const VLNK: u32 = 5;

/// 单条目元数据（name 借用 bulk 缓冲，零拷贝传给 visit 回调）。
#[derive(Debug, Clone)]
pub struct EntryInfo<'a> {
    /// 借用 bulk 缓冲零拷贝传递；非 UTF-8 名称走 lossy（对齐旧 to_string_lossy 口径）。
    pub name: Cow<'a, str>,
    pub is_dir: bool,
    pub is_symlink: bool,
    /// 文件/symlink 自身大小（min(allocsize, datalength)，allocsize=0 取 datalength）；目录为 0。
    pub size: i64,
    pub dev: u64,
    pub ino: u64,
}

/// visit 回调返回值。
pub enum VisitOutcome {
    /// 完全丢弃（不计进度、不下钻）。
    Ignore,
    /// 计入进度，不下钻。
    Keep,
    /// 计入进度；目录则 push 子任务。
    Recurse,
    /// 计入进度，并为进度字节计数额外追加指定值（bundle 叶子的 Spotlight 聚合大小）。
    KeepBytes(i64),
}

/// 进度上报钩子（计数器每条目累加，current_path 每目录更新，节流回调每目录触发）。
pub struct Progress<'a> {
    pub files: &'a AtomicI64,
    pub dirs: &'a AtomicI64,
    pub bytes: &'a AtomicI64,
    pub current: Option<&'a Mutex<String>>,
    /// 每目录结束调用一次（扫描侧在此做 200ms 节流事件推送）。
    pub per_dir: Option<&'a (dyn Fn() + Sync + 'a)>,
}

/// 诊断开关：MOLE_SCAN_TIMING=1 时输出 worker/阶段细分计时。
fn timing_enabled() -> bool {
    static T: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *T.get_or_init(|| std::env::var_os("MOLE_SCAN_TIMING").is_some())
}

/// scan_dir 内部阶段累计（仅诊断用）。
#[derive(Default, Clone, Copy)]
pub struct Diag {
    pub open: std::time::Duration,
    pub bulk: std::time::Duration,
    pub visit: std::time::Duration,
    pub dirs: u64,
    pub entries: u64,
    pub bulk_calls: u64,
    pub open_fail: u64,
}

struct Task {
    path: String,
    depth: usize,
}

// ── 非对齐安全读取（内核按 4 字节边界打包，8 字节值可能落在 4-mod-8 偏移） ──

fn rd_u32(b: &[u8], off: usize) -> Option<u32> {
    let s: &[u8; 4] = b.get(off..off + 4)?.try_into().ok()?;
    Some(u32::from_ne_bytes(*s))
}
fn rd_u64(b: &[u8], off: usize) -> Option<u64> {
    let s: &[u8; 8] = b.get(off..off + 8)?.try_into().ok()?;
    Some(u64::from_ne_bytes(*s))
}
fn rd_i64(b: &[u8], off: usize) -> Option<i64> {
    let s: &[u8; 8] = b.get(off..off + 8)?.try_into().ok()?;
    Some(i64::from_ne_bytes(*s))
}

/// 解析单个 bulk 条目；布局错误或带 error 属性时返回 None（跳过该条目）。
fn parse_entry(e: &[u8]) -> Option<EntryInfo<'_>> {
    // [u32 length][attribute_set_t 20B][u32 error?][common...][file...]
    let returned_common = rd_u32(e, 4)?;
    let returned_file = rd_u32(e, 16)?;
    let mut cur = 24usize;

    if returned_common & ATTR_CMN_ERROR != 0 {
        let err = rd_u32(e, cur)?;
        cur += 4;
        if err != 0 {
            return None;
        }
    }

    let mut name: Cow<'_, str> = Cow::Borrowed("");
    if returned_common & libc::ATTR_CMN_NAME != 0 {
        let dataoff = rd_u32(e, cur)? as i32;
        let datalen = rd_u32(e, cur + 4)?;
        let base = cur as i64 + dataoff as i64;
        let start = usize::try_from(base).ok()?;
        let end = start.checked_add(datalen as usize)?;
        let raw = e.get(start..end)?;
        // attr_length 含结尾 NUL；截掉所有尾部 0 字节。
        // Borrowed 臂零拷贝借缓冲；Owned 臂（非 UTF-8 lossy）自持。
        name = match String::from_utf8_lossy(raw) {
            Cow::Borrowed(s) => Cow::Borrowed(s.trim_end_matches('\0')),
            Cow::Owned(s) => Cow::Owned(s.trim_end_matches('\0').to_string()),
        };
        cur += 8;
    }

    let mut dev = 0u64;
    if returned_common & libc::ATTR_CMN_DEVID != 0 {
        dev = rd_u32(e, cur)? as u64;
        cur += 4;
    }
    let mut obj_type = 0u32;
    if returned_common & libc::ATTR_CMN_OBJTYPE != 0 {
        obj_type = rd_u32(e, cur)?;
        cur += 4;
    }
    let mut ino = 0u64;
    if returned_common & libc::ATTR_CMN_FILEID != 0 {
        ino = rd_u64(e, cur)?;
        cur += 8;
    }

    let mut alloc: i64 = 0;
    if returned_file & libc::ATTR_FILE_ALLOCSIZE != 0 {
        alloc = rd_i64(e, cur)?;
        cur += 8;
    }
    let mut data: i64 = 0;
    if returned_file & libc::ATTR_FILE_DATALENGTH != 0 {
        data = rd_i64(e, cur)?;
    }

    let is_dir = obj_type == VDIR;
    let is_symlink = obj_type == VLNK;
    // 口径等价旧版 min(blocks*512, len)：allocsize≈blocks*512，datalength≈len；
    // symlink 的 allocsize 通常为 0 → 取 datalength（链接自身长度），与旧 lstat 分支一致。
    let size = if is_dir {
        0
    } else if alloc > 0 && alloc < data {
        alloc
    } else {
        data
    };

    Some(EntryInfo {
        name,
        is_dir,
        is_symlink,
        size,
        dev,
        ino,
    })
}

fn cpus() -> usize {
    // 诊断旋钮：MOLE_BULKWORKERS 覆盖 worker 数（用于测量并发扩展性）
    if let Ok(v) = std::env::var("MOLE_BULKWORKERS") {
        if let Ok(n) = v.parse::<usize>() {
            if n > 0 {
                return n;
            }
        }
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

/// N worker work-stealing 并行遍历 root 子树。
///
/// - 每 worker 一个 LIFO deque + 全局 stealer 列表；空闲时偷取他人任务 halves；
/// - `visit` 返回 Recurse 的目录才会 push 子任务（调用侧实现 no_recurse 语义）；
/// - 代际 token 每目录检查，取消时各 worker 提前退出（返回部分结果，调用侧判 stale）；
/// - `finish` 在 worker 退出前调用一次（调用侧 flush 最后一批 stage 聚合）。
pub fn parallel_walk<W, FM, FV, FF>(
    root: &str,
    gen: i64,
    progress: Option<Progress<'_>>,
    make_worker: FM,
    visit: FV,
    finish: FF,
) -> Vec<W>
where
    W: Send,
    FM: Fn() -> W + Sync,
    FV: Fn(&mut W, &str, usize, &EntryInfo<'_>) -> VisitOutcome + Sync,
    FF: Fn(&mut W) + Sync,
{
    let n = cpus();
    let pairs: Vec<(Worker<Task>, Stealer<Task>)> = (0..n)
        .map(|_| {
            let w = Worker::new_lifo();
            let s = w.stealer();
            (w, s)
        })
        .collect();
    let stealers: Arc<Vec<Stealer<Task>>> =
        Arc::new(pairs.iter().map(|(_, s)| s.clone()).collect());
    // 已 push 未完成的任务数（终止判定：无本地/可偷任务且 remaining==0）
    let remaining = Arc::new(AtomicUsize::new(1));
    pairs[0].0.push(Task {
        path: root.to_string(),
        depth: 0,
    });

    let results: Arc<Mutex<Vec<W>>> = Arc::new(Mutex::new(Vec::with_capacity(n)));
    let is_stale = move || super::scanner::is_scan_stale(gen);
    // Option<&Progress> 是 Copy，可被多个 spawn 闭包共享（Option<Progress> 本身非 Copy）
    let progress_ref = progress.as_ref();
    // 诊断开关：MOLE_SCAN_TIMING=1 时打印每 worker 任务数/窃取数/自旋数
    let timing = std::env::var_os("MOLE_SCAN_TIMING").is_some();
    let stats: Arc<Mutex<Vec<(usize, usize, usize)>>> = Arc::new(Mutex::new(vec![(0, 0, 0); n]));

    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(n);
        for (idx, (local, _)) in pairs.into_iter().enumerate() {
            let stealers = Arc::clone(&stealers);
            let remaining = Arc::clone(&remaining);
            let results = Arc::clone(&results);
            let stats = Arc::clone(&stats);
            let mk = &make_worker;
            let vs = &visit;
            let fn_ = &finish;
            let stale = &is_stale;
            handles.push(scope.spawn(move || {
                let mut w = mk();
                let mut buf = vec![0u8; BULK_BUF_BYTES];
                let mut local = local;
                let mut n_tasks = 0usize;
                let mut n_steals = 0usize;
                let mut n_spins = 0usize;
                let mut diag = if timing { Some(Diag::default()) } else { None };
                loop {
                    if stale() {
                        break;
                    }
                    let task = match local.pop() {
                        Some(t) => t,
                        None => {
                            let mut stolen = None;
                            for k in 1..stealers.len() {
                                let victim = (idx + k) % stealers.len();
                                if let crossbeam_deque::Steal::Success(t) =
                                    stealers[victim].steal_batch_and_pop(&local)
                                {
                                    stolen = Some(t);
                                    break;
                                }
                            }
                            match stolen {
                                Some(t) => {
                                    n_steals += 1;
                                    t
                                }
                                None => {
                                    if remaining.load(Ordering::Acquire) == 0 {
                                        break;
                                    }
                                    n_spins += 1;
                                    std::hint::spin_loop();
                                    continue;
                                }
                            }
                        }
                    };
                    n_tasks += 1;

                    scan_dir(
                        &task,
                        &mut local,
                        &remaining,
                        &mut buf,
                        &mut w,
                        vs,
                        progress_ref,
                        stale,
                        diag.as_mut(),
                    );
                    remaining.fetch_sub(1, Ordering::AcqRel);
                }
                fn_(&mut w);
                if let Some(d) = diag {
                    eprintln!(
                        "[bulkwalk::diag] worker {idx}: dirs={} entries={} bulk_calls={} open_fail={} open={:?} bulk={:?} visit={:?}",
                        d.dirs, d.entries, d.bulk_calls, d.open_fail, d.open, d.bulk, d.visit,
                    );
                }
                if timing {
                    if let Ok(mut g) = stats.lock() {
                        g[idx] = (n_tasks, n_steals, n_spins);
                    }
                }
                if let Ok(mut g) = results.lock() {
                    g.push(w);
                }
            }));
        }
        for h in handles {
            let _ = h.join();
        }
    });

    if timing {
        if let Ok(g) = stats.lock() {
            for (i, (t, s, sp)) in g.iter().enumerate() {
                eprintln!("[bulkwalk::stat] worker {i}: tasks={t} steals={s} spins={sp}");
            }
        }
    }

    Arc::try_unwrap(results)
        .map(|m| m.into_inner().unwrap_or_default())
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn scan_dir<W, FV, FS>(
    task: &Task,
    local: &Worker<Task>,
    remaining: &AtomicUsize,
    buf: &mut Vec<u8>,
    w: &mut W,
    visit: &FV,
    progress: Option<&Progress<'_>>,
    is_stale: &FS,
    mut diag: Option<&mut Diag>,
) where
    FV: Fn(&mut W, &str, usize, &EntryInfo<'_>) -> VisitOutcome,
    FS: Fn() -> bool,
{
    let cstr = match std::ffi::CString::new(task.path.as_str()) {
        Ok(c) => c,
        Err(_) => return,
    };
    let t_open = std::time::Instant::now();
    let fd = unsafe { libc::open(cstr.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if let Some(d) = diag.as_deref_mut() {
        d.open += t_open.elapsed();
        d.dirs += 1;
        if fd < 0 {
            d.open_fail += 1;
        }
    }
    if fd < 0 {
        // 权限不足 / 已消失：跳过该目录（条目元数据已由父目录 bulk 记录，对齐旧 jwalk Err→continue）
        return;
    }

    let mut attrlist = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS
            | ATTR_CMN_ERROR
            | libc::ATTR_CMN_NAME
            | libc::ATTR_CMN_DEVID
            | libc::ATTR_CMN_OBJTYPE
            | libc::ATTR_CMN_FILEID,
        volattr: 0,
        dirattr: 0,
        fileattr: libc::ATTR_FILE_ALLOCSIZE | libc::ATTR_FILE_DATALENGTH,
        forkattr: 0,
    };

    loop {
        if is_stale() {
            break;
        }
        let t_bulk = std::time::Instant::now();
        let count = unsafe {
            libc::getattrlistbulk(
                fd,
                &mut attrlist as *mut libc::attrlist as *mut libc::c_void,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                0,
            )
        };
        if let Some(d) = diag.as_deref_mut() {
            d.bulk_calls += 1;
            d.bulk += t_bulk.elapsed();
        }
        if count <= 0 {
            // 0 = 目录读完；-1 = 错误（含单条目超缓冲的 ENOSPC，直接放弃该目录剩余部分）
            break;
        }
        let mut off = 0usize;
        for _ in 0..count {
            let Some(len) = rd_u32(buf, off).map(|l| l as usize) else {
                break;
            };
            if len == 0 || off + len > buf.len() {
                break;
            }
            let entry = &buf[off..off + len];
            off += len;
            let Some(info) = parse_entry(entry) else {
                continue;
            };
            if let Some(d) = diag.as_deref_mut() {
                d.entries += 1;
            }
            let t_visit = std::time::Instant::now();
            let is_dir = info.is_dir;
            let is_symlink = info.is_symlink;
            let size = info.size;
            match visit(w, &task.path, task.depth, &info) {
                VisitOutcome::Ignore => {}
                VisitOutcome::Keep => {
                    bump_progress(progress, is_dir, is_symlink, size);
                }
                VisitOutcome::KeepBytes(extra) => {
                    bump_progress(progress, is_dir, is_symlink, size);
                    if let Some(p) = progress {
                        p.bytes.fetch_add(extra, Ordering::Relaxed);
                    }
                }
                VisitOutcome::Recurse => {
                    bump_progress(progress, is_dir, is_symlink, size);
                    local.push(Task {
                        path: super::scanner::join_path(&task.path, &info.name),
                        depth: task.depth + 1,
                    });
                    remaining.fetch_add(1, Ordering::AcqRel);
                }
            }
            if let Some(d) = diag.as_deref_mut() {
                d.visit += t_visit.elapsed();
            }
        }
    }
    unsafe { libc::close(fd) };

    if let Some(p) = progress {
        if let Some(cur) = p.current {
            if let Ok(mut s) = cur.lock() {
                *s = task.path.clone();
            }
        }
        if let Some(hook) = p.per_dir {
            hook();
        }
    }
}

fn bump_progress(p: Option<&Progress<'_>>, is_dir: bool, is_symlink: bool, size: i64) {
    let Some(p) = p else { return };
    if is_dir {
        p.dirs.fetch_add(1, Ordering::Relaxed);
    } else if !is_symlink {
        p.files.fetch_add(1, Ordering::Relaxed);
        p.bytes.fetch_add(size, Ordering::Relaxed);
    }
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::os::unix::fs::MetadataExt;
    use std::sync::atomic::AtomicI64;

    /// 遍历结果收集器：parent → 条目列表（与 lstat ground truth 对照用）。
    #[derive(Default, Debug)]
    struct Collector {
        rows: HashMap<String, Vec<(String, bool, bool, i64, u64, u64)>>,
    }

    fn collect_walk(root: &str) -> HashMap<String, Vec<(String, bool, bool, i64, u64, u64)>> {
        let out: Arc<Mutex<Collector>> = Arc::new(Mutex::new(Collector::default()));
        let workers = parallel_walk(
            root,
            0,
            None,
            || Arc::clone(&out),
            |w, parent, _depth, info| {
                w.lock()
                    .unwrap()
                    .rows
                    .entry(parent.to_string())
                    .or_default()
                    .push((
                        info.name.to_string(),
                        info.is_dir,
                        info.is_symlink,
                        info.size,
                        info.dev,
                        info.ino,
                    ));
                if info.is_dir {
                    VisitOutcome::Recurse
                } else {
                    VisitOutcome::Keep
                }
            },
            |_| {},
        );
        // 各 worker 持同一 Arc<Collector>：任取一个锁读即可
        let first = workers.into_iter().next().unwrap();
        let rows = first.lock().unwrap().rows.clone();
        rows
    }

    #[test]
    fn bulk_fields_match_lstat_ground_truth() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rmole2_bulk_{nanos}"));
        std::fs::create_dir_all(root.join("sub/deep")).unwrap();
        std::fs::write(root.join("a.txt"), vec![7u8; 1234]).unwrap();
        std::fs::write(root.join("sub/deep/b.bin"), vec![1u8; 99]).unwrap();
        std::fs::write(root.join(".hidden"), b"h").unwrap();
        std::os::unix::fs::symlink(root.join("a.txt"), root.join("ln")).unwrap();

        let rows = collect_walk(&root.to_string_lossy());
        let root_rows = rows
            .get(&root.to_string_lossy().to_string())
            .expect("root rows");

        for (name, is_dir, is_symlink, size, dev, ino) in root_rows {
            let full = root.join(&name);
            let meta = std::fs::symlink_metadata(&full).unwrap();
            assert_eq!(*is_dir, meta.file_type().is_dir(), "{name} is_dir");
            assert_eq!(
                *is_symlink,
                meta.file_type().is_symlink(),
                "{name} is_symlink"
            );
            assert_eq!(*dev, meta.dev() & 0xFFFF_FFFF, "{name} dev");
            assert_eq!(*ino, meta.ino(), "{name} ino");
            if !is_dir {
                // 口径：min(blocks*512, len)
                let len = meta.len() as i64;
                let actual = (meta.blocks() as i64).saturating_mul(512);
                let expect = if actual > 0 && actual < len {
                    actual
                } else {
                    len
                };
                assert_eq!(*size, expect, "{name} size");
            }
        }
        // 覆盖断言：根下 4 个条目全在（含隐藏与 symlink）
        assert_eq!(root_rows.len(), 4);
        // 子目录被递归（sub 的条目存在）
        let sub_key = root.join("sub").to_string_lossy().to_string();
        assert!(rows.contains_key(&sub_key), "sub dir recursed");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn progress_counters_and_cancellation_shape() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rmole2_bulkprog_{nanos}"));
        std::fs::create_dir_all(root.join("d1")).unwrap();
        std::fs::write(root.join("f1"), vec![0u8; 10]).unwrap();
        std::fs::write(root.join("d1/f2"), vec![0u8; 20]).unwrap();

        let (files, dirs, bytes) = (AtomicI64::new(0), AtomicI64::new(0), AtomicI64::new(0));
        let current = Mutex::new(String::new());
        let prog = Progress {
            files: &files,
            dirs: &dirs,
            bytes: &bytes,
            current: Some(&current),
            per_dir: None,
        };
        let workers = parallel_walk(
            &root.to_string_lossy(),
            0,
            Some(prog),
            || 0i64,
            |_w, _p, _d, info| {
                if info.is_dir {
                    VisitOutcome::Recurse
                } else {
                    VisitOutcome::Keep
                }
            },
            |_| {},
        );
        let _ = workers;
        assert_eq!(files.load(Ordering::Relaxed), 2);
        assert_eq!(dirs.load(Ordering::Relaxed), 1);
        assert_eq!(bytes.load(Ordering::Relaxed), 30);

        let _ = std::fs::remove_dir_all(&root);
    }
}
