//! 磁盘分析扫描核心（V3 重写：getattrlistbulk + work-stealing 全并行遍历 + disjoint merge 聚合）。
//!
//! 与旧版（Mole Go `scanner.go` 的 1:1 翻译）的结构性差异：
//!
//! - **遍历**：`bulkwalk` getattrlistbulk + work-stealing 全并行（每目录一次 bulk 取全部
//!   条目元数据，免除旧 jwalk 管线的逐条目 lstat/namei 与单线程串行瓶颈）；
//! - **聚合**：每 worker 持局部聚合表（meta/contrib 两组 key 跨 worker 天然不相交，
//!   merge = 纯 union），硬链接以 (dev, ino) 全局集合去重，替代旧版单消费者
//!   线程串行聚合；
//! - **外部命令**：不再调用 `du` / `mdfind`（纯 Rust，对齐红线 1「不调用外部二进制」）；
//! - **展示口径对齐 lemon-cleaner 磁盘分析**（`LemonSpaceAnalyse/LMFileScanTask`）：
//!   每一级目录的全部条目（含隐藏文件、0 大小条目、symlink）都枚举并展示，
//!   不按目录名跳过/折叠；递归排除仅 `/System/Volumes`、`/Volumes`、
//!   `/private/tmp/msu-*`（条目仍展示，大小为 0）；symlink 一律按文件展示、不跟随。
//!
//! 快照（单根目录一个 bincode 文件）由 `cache.rs` 负责读写；本模块只产出内存聚合结果。
//! 取消语义：全局扫描代际 token —— 新扫描自动取消旧扫描（walker/consumer 每批检查），
//! `mole_analyze_cancel` 命令显式取消当前扫描。

use super::bulkwalk::{self, EntryInfo, VisitOutcome};
use super::constants::*;
use super::heap::{DirEntry, FileEntry, LargeFileHeap};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{BuildHasherDefault, Hasher};
use std::io;
use std::path::Path;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::core::base::current_analyze_app_handle;
use crate::events::{ScanProgressPayload, emit_analyze_scan_progress};

// ── 扫描代际与取消（全局） ──────────────────────────────────────────────────
//
// 同一时刻只允许一个活跃扫描：`begin_scan` 递增代际并清除取消标记，旧扫描的
// walker/consumer 每批调用 `is_scan_stale(generation)` 发现代际变化后自行退出。
// 注意：analyze 子窗口与 shell 主窗口共享同一进程，代际为全局（多窗口并发扫描
// 会互相取消，属已知限制——UI 层同一用户同一时刻只会操作一个分析会话）。

static SCAN_GENERATION: AtomicI64 = AtomicI64::new(0);
static SCAN_CANCEL: AtomicBool = AtomicBool::new(false);

/// 取消标记文本：扫描被取消时构造错误的消息片段，controller 据此区分「用户取消」与真失败。
pub const SCAN_CANCELLED_MARK: &str = "scan cancelled";

/// 取消错误码：`mole_analyze` 对外返回的稳定标识，前端精确匹配后静默收尾（不弹「扫描失败」）。
pub const SCAN_CANCELLED_CODE: &str = "SCAN_CANCELLED";

/// 开启一次新扫描：递增代际、清除取消标记，返回本次代际号。
pub fn begin_scan() -> i64 {
    SCAN_CANCEL.store(false, Ordering::SeqCst);
    SCAN_GENERATION.fetch_add(1, Ordering::SeqCst) + 1
}

/// 请求取消当前活跃扫描（由 `mole_analyze_cancel` 命令调用）。
pub fn cancel_active_scan() {
    SCAN_CANCEL.store(true, Ordering::SeqCst);
    SCAN_GENERATION.fetch_add(1, Ordering::SeqCst);
}

pub(crate) fn is_scan_stale(generation: i64) -> bool {
    SCAN_CANCEL.load(Ordering::SeqCst) || SCAN_GENERATION.load(Ordering::SeqCst) != generation
}

/// 扫描字节目标估算（进度百分比分母）：优先既往快照子树大小（重扫/钻取），
/// 其次根卷已用空间（首次全盘扫描，对齐柠檬字节进度口径）；0 表示未设置。
static SCAN_BYTES_TARGET: AtomicI64 = AtomicI64::new(0);

/// 卷总容量（展示用，不参与百分比计算）。
static SCAN_DISK_TOTAL: AtomicI64 = AtomicI64::new(0);

/// 设置本次扫描的字节目标与卷总容量。controller 在发起扫描前调用。
pub fn set_scan_bytes_estimate(bytes_target: i64, disk_total: i64) {
    SCAN_BYTES_TARGET.store(bytes_target, Ordering::Release);
    SCAN_DISK_TOTAL.store(disk_total, Ordering::Release);
}

/// 上一次推送时间（全局，所有扫描共享节流）。
static LAST_EMIT_PROGRESS: OnceLock<Mutex<Instant>> = OnceLock::new();

/// 每 ~200ms 最多推送一次 scan-progress 事件。
fn maybe_emit_scan_progress(
    files: &AtomicI64,
    dirs: &AtomicI64,
    bytes: &AtomicI64,
    current: Option<&Mutex<String>>,
) {
    let app = match current_analyze_app_handle() {
        Some(a) => a,
        None => {
            log::debug!("[scan::emit_progress] no app handle, skip");
            return;
        }
    };

    let last = LAST_EMIT_PROGRESS.get_or_init(|| Mutex::new(Instant::now()));
    let mut last_lock = match last.lock() {
        Ok(l) => l,
        Err(_) => {
            log::warn!("[scan::emit_progress] lock poisoned, skip");
            return;
        }
    };

    if last_lock.elapsed().as_millis() < 200 {
        return;
    }

    let files_val = files.load(Ordering::Relaxed);
    let dirs_val = dirs.load(Ordering::Relaxed);
    let bytes_val = bytes.load(Ordering::Relaxed);
    let cp = current
        .and_then(|m| m.lock().ok())
        .map(|s| s.clone())
        .unwrap_or_default();

    log::info!(
        "[scan::emit_progress] files={files_val}, dirs={dirs_val}, bytes={bytes_val}, path={cp}"
    );

    let bytes_target = SCAN_BYTES_TARGET.load(Ordering::Acquire);
    let percent = if bytes_target > 0 {
        let pct = (bytes_val as f64 / bytes_target as f64 * 100.0) as i64;
        if pct > 100 {
            100
        } else if pct < 0 {
            0
        } else {
            pct
        }
    } else {
        -1
    };

    emit_analyze_scan_progress(
        &app,
        &ScanProgressPayload {
            files_scanned: files_val,
            dirs_scanned: dirs_val,
            bytes_scanned: bytes_val,
            current_path: cp,
            percent,
            bytes_target,
            disk_total: SCAN_DISK_TOTAL.load(Ordering::Acquire),
        },
    );

    *last_lock = Instant::now();
}

// ── 结果与快照结构 ──────────────────────────────────────────────────────────

/// 扫描对外结果（与旧版同名同构，前端契约不变）。
#[derive(Debug, Default, Clone)]
pub struct ScanResult {
    pub entries: Vec<DirEntry>,
    pub large_files: Vec<FileEntry>,
    pub total_size: i64,
    pub total_files: i64,
    /// 扫描中是否发生过硬链接去重。
    /// 快照保存策略与此解耦（单根快照内部自洽，可安全缓存），保留字段仅作口径记录。
    pub deduped_hardlink: bool,
}

/// GUI 扫描产物：对外结果 + 聚合节点（供快照落盘）。
#[derive(Debug)]
pub struct ScanOutcome {
    pub result: ScanResult,
    pub nodes: HashMap<String, DirNode>,
}

/// 快照内的文件明细记录（仅存于非折叠目录的直接子文件）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRec {
    pub name: String,
    pub size: i64,
    /// 恒为 false：symlink 一律按文件展示（对齐 Lemon VLNK 口径），
    /// 真目录走独立的 DirNode 聚合行，不进 files 明细。
    pub is_dir: bool,
    pub is_symlink: bool,
}

/// 快照内的目录聚合节点：单消费者聚合产物，key 为目录绝对路径。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DirNode {
    pub name: String,
    /// 目录自身直接文件大小合计（含 symlink 链接自身大小；传播前）。
    pub own_size: i64,
    /// 传播后的子树总大小（own_size + 子目录递归）。
    pub size: i64,
    /// 子树文件总数（不含 symlink，与旧版 total_files 口径一致）。
    pub total_files: i64,
    pub child_files: i64,
    pub child_dirs: i64,
    pub child_links: i64,
    pub depth: usize,
    pub files: Vec<FileRec>,
    pub children: Vec<String>,
    /// Bundle 叶子捷径（对齐 Lemon specialFileExtensions）：首扫时经 Spotlight
    /// 聚合大小叶子化，内部未递归；own_size 已含 physical_size。
    pub bundle_leaf: bool,
    pub bundle_id: Option<String>,
    pub bundle_display_name: Option<String>,
    pub bundle_content_types: Vec<String>,
}

/// Go `shouldSkipFileForLargeTracking`：按扩展名过滤大文件追踪。
/// 直接取 basename 与静态表不区分大小写直比（旧实现每文件 `format! + to_lowercase`
/// 两次分配，是热循环里不必要的序列化税）。
pub fn should_skip_file_for_large_tracking(name: &str) -> bool {
    // 对齐 Path::extension 语义：前置点且无其他点不算扩展名（如 .hidden）
    let ext = match name.rsplit_once('.') {
        Some((stem, e)) if !stem.is_empty() => e,
        _ => return false,
    };
    skip_extensions
        .iter()
        .any(|s| s[1..].eq_ignore_ascii_case(ext))
}

// ── FxHash（自写 ~30 行，不引新 crate） ────────────────────────────────────
// nodes/contrib/seen 三张表以长路径字符串或 (dev,ino) 为 key，std SipHash 的
// 逐字节成本在百万级记录下不可忽略。

#[derive(Default)]
struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut buf = [0u8; 8];
            buf[..chunk.len()].copy_from_slice(chunk);
            self.add(u64::from_le_bytes(buf));
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

type FxBuild = BuildHasherDefault<FxHasher>;

/// 路径拼接（旧版 join_path 同语义：root 为 "/" 时直接拼 name）。
pub fn join_path(root: &str, name: &str) -> String {
    if root.ends_with('/') {
        format!("{root}{name}")
    } else {
        format!("{root}/{name}")
    }
}

/// 取路径的父目录：`/a/b` → `/a`；`/a` → `/`。（仅单测使用）
#[cfg(test)]
fn parent_key(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) => "/",
        Some(i) => &path[..i],
        None => "",
    }
}

/// 取路径 basename。
fn base_name(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

// ── walker ──────────────────────────────────────────────────────────────────

/// 递归排除判定（对齐 Lemon `LMFileScanTask.m:255`）：命中仍展示条目，只是不下钻。
fn is_no_recurse(path: &Path) -> bool {
    let path_s = path.to_string_lossy();
    no_recurse_exact.iter().any(|p| path == Path::new(p))
        || no_recurse_prefix.iter().any(|p| path_s.starts_with(*p))
}

// ── worker 局部聚合 ────────────────────────────────────────────────────────

/// 目录自身元数据（由扫其父目录的 worker 写；跨 worker key 不相交）。
#[derive(Default)]
struct DirSelfMeta {
    name: String,
    depth: usize,
}

/// 目录内贡献（由扫该目录的 worker 写；跨 worker key 不相交）。
#[derive(Default)]
struct DirContrib {
    own_size: i64,
    child_files: i64,
    child_dirs: i64,
    child_links: i64,
    files: Vec<FileRec>,
    /// 与 files 平行：非 symlink 文件的硬链接去重键 (dev, ino)。
    /// 公共 attr.h 无 ATTR_CMN_LINKCOUNT，bulk 拿不到 nlink；改用全局 (dev, ino)
    /// 集合去重（唯一文件的 ino 不会碰撞，口径与旧版 nlink>1 一致）。
    /// 仅 merge 阶段瞬态使用，不落快照（FileRec schema 不变）。
    keys: Vec<Option<(u64, u64)>>,
    children: Vec<String>,
}

impl DirContrib {
    fn absorb(&mut self, other: DirContrib) {
        self.own_size = self.own_size.saturating_add(other.own_size);
        self.child_files += other.child_files;
        self.child_dirs += other.child_dirs;
        self.child_links += other.child_links;
        self.files.extend(other.files);
        self.keys.extend(other.keys);
        self.children.extend(other.children);
    }
}

/// 扩展名是否命中 bundle 叶子捷径集合（柠檬 specialFileExtensions 同集）。
fn is_bundle_leaf_name(name: &str) -> bool {
    let ext = match name.rsplit_once('.') {
        Some((stem, e)) if !stem.is_empty() => e,
        _ => return false,
    };
    bundle_leaf_extensions
        .iter()
        .any(|s| s.eq_ignore_ascii_case(ext))
}

/// 每 worker 局部聚合状态（stage = 当前目录的进行中贡献，懒 flush）。
struct WorkerAgg {
    meta: HashMap<String, DirSelfMeta, FxBuild>,
    contrib: HashMap<String, DirContrib, FxBuild>,
    /// bundle 叶子捷径（key = bundle 目录路径，与 meta 同 worker 写，disjoint）。
    bundle_meta: HashMap<String, crate::platform::macos_mditem::BundleShortcut, FxBuild>,
    stage: DirContrib,
    stage_key: String,
    large: LargeFileHeap,
    large_min: i64,
}

impl WorkerAgg {
    fn new() -> Self {
        Self {
            meta: HashMap::default(),
            contrib: HashMap::default(),
            bundle_meta: HashMap::default(),
            stage: DirContrib::default(),
            stage_key: String::new(),
            large: LargeFileHeap::default(),
            large_min: large_file_warmup_min_size as i64,
        }
    }

    /// 目录切换时 flush：免除每条目一次 HashMap 查找 + key 分配。
    fn ensure_stage(&mut self, parent: &str) {
        if self.stage_key != parent {
            self.flush_stage();
            self.stage_key = parent.to_string();
        }
    }

    fn flush_stage(&mut self) {
        if self.stage_key.is_empty() {
            return;
        }
        let key = std::mem::take(&mut self.stage_key);
        let stage = std::mem::take(&mut self.stage);
        match self.contrib.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut e) => e.get_mut().absorb(stage),
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(stage);
            }
        }
    }
}

/// 每条目聚合回调（跑在 bulkwalk 各 worker 线程上）。
fn visit_entry(
    w: &mut WorkerAgg,
    parent: &str,
    parent_depth: usize,
    info: &EntryInfo<'_>,
) -> VisitOutcome {
    if info.is_dir {
        let full = join_path(parent, &info.name);
        // Bundle 叶子捷径：Spotlight 聚合大小命中 → 叶子化不下钻（对齐柠檬）；
        // 未索引（size<=0）→ 照常递归兜底。
        if is_bundle_leaf_name(&info.name) {
            if let Some(sc) = crate::platform::macos_mditem::bundle_shortcut(&full) {
                let physical_size = sc.physical_size;
                w.bundle_meta.insert(full.clone(), sc);
                w.meta.insert(
                    full.clone(),
                    DirSelfMeta {
                        name: info.name.to_string(),
                        depth: parent_depth + 1,
                    },
                );
                w.ensure_stage(parent);
                w.stage.child_dirs += 1;
                w.stage.children.push(full);
                // 进度字节计入 Spotlight 聚合大小（对齐柠檬把叶子 bundle 计入已扫字节）
                return VisitOutcome::KeepBytes(physical_size);
            }
        }
        let recurse = !is_no_recurse(Path::new(&full));
        w.meta.insert(
            full.clone(),
            DirSelfMeta {
                name: info.name.to_string(),
                depth: parent_depth + 1,
            },
        );
        w.ensure_stage(parent);
        w.stage.child_dirs += 1;
        w.stage.children.push(full);
        return if recurse {
            VisitOutcome::Recurse
        } else {
            VisitOutcome::Keep
        };
    }

    let size = info.size;
    w.ensure_stage(parent);
    w.stage.own_size = w.stage.own_size.saturating_add(size);
    if info.is_symlink {
        w.stage.child_links += 1;
        w.stage.keys.push(None);
    } else {
        w.stage.child_files += 1;
        w.stage.keys.push(Some((info.dev, info.ino)));
        // 大文件 Top-N 追踪：先阈值判定，仅为候选构造全路径
        if size > 0 && size >= w.large_min && !should_skip_file_for_large_tracking(&info.name) {
            let path = join_path(parent, &info.name);
            if w.large.len() < max_large_files {
                w.large.push(FileEntry {
                    name: info.name.to_string(),
                    path,
                    size,
                });
                if w.large.len() == max_large_files {
                    w.large_min = w.large.peek().map(|t| t.size).unwrap_or(w.large_min);
                }
            } else if let Some(top) = w.large.peek() {
                if size > top.size {
                    w.large.pop();
                    w.large.push(FileEntry {
                        name: info.name.to_string(),
                        path,
                        size,
                    });
                    w.large_min = w.large.peek().map(|t| t.size).unwrap_or(w.large_min);
                }
            }
        }
    }
    // 所有文件（含 symlink）全量落快照明细（对齐 Lemon：每级展示全部条目）。
    w.stage.files.push(FileRec {
        name: info.name.to_string(),
        size,
        is_dir: false,
        is_symlink: info.is_symlink,
    });
    VisitOutcome::Keep
}

fn mtime_secs_of(meta: &fs::Metadata) -> i64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.mtime()
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0
    }
}

// ── 扫描主流程 ──────────────────────────────────────────────────────────────

/// Go `scanPathConcurrentAllEntries` 的等价入口（CLI 与 GUI 共用）。
pub fn scan_path_concurrent_all_entries(
    root: &str,
    files_scanned: &AtomicI64,
    dirs_scanned: &AtomicI64,
    bytes_scanned: &AtomicI64,
    current_path: Option<&Mutex<String>>,
) -> io::Result<ScanResult> {
    let outcome = scan_subtree(
        root,
        files_scanned,
        dirs_scanned,
        bytes_scanned,
        current_path,
    )?;
    Ok(outcome.result)
}

/// GUI 扫描入口：返回结果 + 聚合节点（供快照落盘）。
///
/// 管线（五段计时）：bulkwalk 并行遍历（含局部聚合）→ disjoint merge →
/// 全局硬链接去重 → 自底向上传播 → 组装结果。
pub fn scan_subtree(
    root: &str,
    files_scanned: &AtomicI64,
    dirs_scanned: &AtomicI64,
    bytes_scanned: &AtomicI64,
    current_path: Option<&Mutex<String>>,
) -> io::Result<ScanOutcome> {
    let t0 = Instant::now();
    let generation = begin_scan();

    let root_meta = fs::symlink_metadata(root)?;
    if !root_meta.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{root} is not a directory"),
        ));
    }
    let root_mtime = mtime_secs_of(&root_meta);
    let _ = root_mtime; // mtime 不再存入 DirNode，保留 root_meta 读取以验证路径存在性

    let progress = bulkwalk::Progress {
        files: files_scanned,
        dirs: dirs_scanned,
        bytes: bytes_scanned,
        current: current_path,
        per_dir: Some(&|| {
            maybe_emit_scan_progress(files_scanned, dirs_scanned, bytes_scanned, current_path)
        }),
    };

    let mut workers = bulkwalk::parallel_walk(
        root,
        generation,
        Some(progress),
        || WorkerAgg::new(),
        |w: &mut WorkerAgg, parent: &str, parent_depth: usize, info: &EntryInfo<'_>| {
            visit_entry(w, parent, parent_depth, info)
        },
        |w: &mut WorkerAgg| w.flush_stage(),
    );
    let t_walk = t0.elapsed();

    if is_scan_stale(generation) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            SCAN_CANCELLED_MARK,
        ));
    }

    // ── Top-N 大文件：各 worker 局部堆并集，全局再取前 20（降序） ──
    let mut large_files: Vec<FileEntry> = Vec::new();
    for w in &mut workers {
        while let Some(f) = w.large.pop() {
            large_files.push(f);
        }
    }
    large_files.sort_unstable_by(|a, b| b.size.cmp(&a.size));
    large_files.truncate(max_large_files);

    // ── disjoint merge：meta[D] 只由扫 D 父目录的 worker 写、contrib[D] 只由扫 D 的
    //    worker 写，两组 key 跨 worker 天然不相交，merge = 纯 union ──
    let mut nodes_fx: HashMap<String, DirNode, FxBuild> = HashMap::default();
    nodes_fx.insert(
        root.to_string(),
        DirNode {
            name: if root == "/" {
                "/".to_string()
            } else {
                base_name(root).to_string()
            },
            depth: 0,
            ..DirNode::default()
        },
    );
    for w in &workers {
        for (path, m) in &w.meta {
            debug_assert!(!nodes_fx.contains_key(path), "meta key overlap: {path}");
            nodes_fx.insert(
                path.clone(),
                DirNode {
                    name: m.name.clone(),
                    depth: m.depth,
                    ..DirNode::default()
                },
            );
        }
    }
    // bundle 叶子捷径并入节点：own_size 记 Spotlight 聚合大小（无子节点，传播后 size 即该值）
    for w in &workers {
        for (path, sc) in &w.bundle_meta {
            if let Some(n) = nodes_fx.get_mut(path) {
                n.bundle_leaf = true;
                n.own_size = n.own_size.saturating_add(sc.physical_size);
                n.bundle_id = sc.bundle_id.clone();
                n.bundle_display_name = sc.display_name.clone();
                n.bundle_content_types = sc.content_types.clone();
            }
        }
    }
    // 全局硬链接去重：重复 (dev, ino) 置零并修正 own_size
    let mut deduped = false;
    let mut seen: HashSet<(u64, u64), FxBuild> = HashSet::default();
    for w in workers {
        for (path, mut c) in w.contrib {
            let mut dup_bytes: i64 = 0;
            for (f, key) in c.files.iter_mut().zip(c.keys.iter()) {
                if let Some(k) = key {
                    if !seen.insert(*k) {
                        dup_bytes = dup_bytes.saturating_add(f.size);
                        f.size = 0;
                        deduped = true;
                    }
                }
            }
            let n = nodes_fx.entry(path).or_default();
            n.own_size = n
                .own_size
                .saturating_add(c.own_size.saturating_sub(dup_bytes));
            n.child_files += c.child_files;
            n.child_dirs += c.child_dirs;
            n.child_links += c.child_links;
            n.files.extend(c.files);
            n.children.extend(c.children);
        }
    }
    let t_merge = t0.elapsed();

    // ── 自底向上传播子树大小 ──
    let mut dirs: Vec<(String, usize)> =
        nodes_fx.iter().map(|(k, v)| (k.clone(), v.depth)).collect();
    dirs.sort_by_key(|(_, d)| std::cmp::Reverse(*d));
    for (path, _) in &dirs {
        let (children, own_size, child_files) = {
            let n = &nodes_fx[path];
            (n.children.clone(), n.own_size, n.child_files)
        };
        let mut size = own_size;
        let mut total_files = child_files;
        for c in &children {
            if let Some(cn) = nodes_fx.get(c) {
                size = size.saturating_add(cn.size);
                total_files += cn.total_files;
            }
        }
        let n = nodes_fx.get_mut(path).expect("node must exist");
        n.size = size;
        n.total_files = total_files;
    }
    let t_prop = t0.elapsed();

    // ── 组装结果（root 的直接子项 + Top-N 大文件） ──
    let nodes: HashMap<String, DirNode> = nodes_fx.into_iter().collect();
    let entries = entries_for_dir(&nodes, root);
    let root_node = nodes.get(root).expect("root node must exist");
    let result = ScanResult {
        entries,
        large_files,
        total_size: root_node.size,
        total_files: root_node.total_files,
        deduped_hardlink: deduped,
    };

    let scanned = files_scanned.load(Ordering::Relaxed);
    let rate = if t_walk.as_secs_f64() > 0.0 {
        scanned as f64 / t_walk.as_secs_f64()
    } else {
        0.0
    };
    log::info!(
        "[scan::timing] walk={t_walk:?} merge={:?} propagate={:?} total={:?} files={scanned} rate={rate:.0}/s",
        t_merge - t_walk,
        t_prop - t_merge,
        t_prop,
    );
    if std::env::var_os("MOLE_SCAN_TIMING").is_some() {
        eprintln!(
            "[scan::phase] walk={t_walk:?} merge={:?} propagate={:?} total={:?}",
            t_merge - t_walk,
            t_prop - t_merge,
            t_prop,
        );
    }

    Ok(ScanOutcome { result, nodes })
}

/// 从聚合节点构建某目录的直接子项列表（文件明细 + 子目录聚合行），按 size 降序。
///
/// 供扫描结果组装与快照查询共用；`cleanable`/`protected`/`insight` 标记由
/// `json::json_entries_from_dir_entries` 在转换时补齐。
pub fn entries_for_dir(nodes: &HashMap<String, DirNode>, dir: &str) -> Vec<DirEntry> {
    let mut entries: Vec<DirEntry> = Vec::new();
    let Some(node) = nodes.get(dir) else {
        return entries;
    };

    for f in &node.files {
        entries.push(DirEntry {
            name: f.name.clone(),
            path: join_path(dir, &f.name),
            size: f.size,
            is_dir: f.is_dir,
            last_access: None,
            is_symlink: f.is_symlink,
            child_files: 0,
            child_dirs: 0,
            child_links: 0,
            is_bundle_leaf: false,
            bundle_id: None,
            bundle_display_name: None,
        });
    }
    for c in &node.children {
        if let Some(cn) = nodes.get(c) {
            entries.push(DirEntry {
                name: cn.name.clone(),
                path: c.clone(),
                size: cn.size,
                is_dir: true,
                last_access: None,
                is_symlink: false,
                child_files: cn.child_files,
                child_dirs: cn.child_dirs,
                child_links: cn.child_links,
                is_bundle_leaf: cn.bundle_leaf,
                bundle_id: cn.bundle_id.clone(),
                bundle_display_name: cn.bundle_display_name.clone(),
            });
        }
    }
    entries.sort_by(|a, b| b.size.cmp(&a.size));
    entries
}

fn secs_to_opt(secs: i64) -> Option<SystemTime> {
    if secs > 0 {
        Some(UNIX_EPOCH + Duration::from_secs(secs as u64))
    } else {
        None
    }
}

// ── 目录大小测量（overview / insights 共用，纯 Rust 替代 du） ────────────────

/// Go `overviewIgnoreNamesForPath`：扫描目录中存在的 `overviewDuIgnoreNames` 子目录。
fn overview_ignore_names_for_path(path: &str) -> Vec<String> {
    let read = match fs::read_dir(path) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut names = Vec::new();
    for entry in read.filter_map(Result::ok) {
        let name = entry.file_name();
        let name_s = name.to_string_lossy().to_string();
        if overview_du_ignore_names.contains(&name_s.as_str())
            && entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
        {
            names.push(name_s);
        }
    }
    names
}

/// 目录大小（纯 Rust size-only 并行遍历，替代旧版 `du -skPxA`）。
///
/// 口径与旧版 du 一致（`blocks*512` 与 `len` 取小，bulk 以 allocsize/datalength 等价变换）；
/// `exclude_path` 剪枝该子树，`ignore_names` 按 basename 精确剪枝（对齐旧版 `du -I name` 全局忽略语义）。
/// 受全局扫描代际控制：检测到新扫描 / 显式取消时返回 `scan cancelled`。
pub fn measure_dir_size_native(
    path: &str,
    exclude_path: &str,
    ignore_names: &[String],
) -> Result<i64, String> {
    let generation = SCAN_GENERATION.load(Ordering::SeqCst);
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(format!("path must be absolute: {path}"));
    }
    if !p.exists() {
        return Err(format!("cannot access path: {path}"));
    }

    let exclude = exclude_path.to_string();
    let ignore = ignore_names.to_vec();
    let workers = bulkwalk::parallel_walk(
        path,
        generation,
        None,
        || 0i64,
        |sum, parent, _depth, info| {
            if ignore.iter().any(|n| &*info.name == n.as_str()) {
                return VisitOutcome::Ignore;
            }
            if !exclude.is_empty() && path_eq_join(&exclude, parent, &info.name) {
                return VisitOutcome::Ignore;
            }
            if info.is_dir {
                VisitOutcome::Recurse
            } else {
                *sum = sum.saturating_add(info.size);
                VisitOutcome::Keep
            }
        },
        |_| {},
    );
    if is_scan_stale(generation) {
        return Err(SCAN_CANCELLED_MARK.into());
    }
    Ok(workers.iter().sum())
}

/// `parent/name == full` 的零分配等价比较：measure 热循环免每条目构造全路径。
fn path_eq_join(full: &str, parent: &str, name: &str) -> bool {
    full.len() == parent.len() + 1 + name.len()
        && full.starts_with(parent)
        && full[parent.len() + 1..] == *name
}

/// Go `measureOverviewSize`：原生并行遍历 → 快照兜底。
/// HOME 额外排除 `~/Library`（由 overview 缓存单独兜底），并忽略 iCloud 占位目录。
pub fn measure_overview_size(path: &str) -> Result<i64, String> {
    if path.is_empty() {
        return Err("empty path".into());
    }
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(format!("path must be absolute: {path}"));
    }
    if !p.exists() {
        return Err(format!("cannot access path: {path}"));
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let exclude_path = if !home.is_empty() && path == home {
        format!("{home}/Library")
    } else {
        String::new()
    };
    let ignore_names = overview_ignore_names_for_path(path);

    if let Ok(size) = measure_dir_size_native(path, &exclude_path, &ignore_names) {
        let _ = super::cache::store_overview_size(path, size);
        return Ok(size);
    }

    // 快照兜底：该目录曾作为扫描根被快照过 → 直接复用其 total_size。
    if let Ok(Some(hit)) = super::cache::find_fresh_snapshot(path) {
        if let Some(node) = hit.snapshot.nodes.get(path) {
            if node.size > 0 {
                let _ = super::cache::store_overview_size(path, node.size);
                return Ok(node.size);
            }
        }
    }

    Err("unable to measure directory size with fast methods".into())
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicI64;

    // 扫描代际/取消标志是进程级全局状态：并行测试会互相取消，用全局锁串行化。
    use std::sync::{Mutex, MutexGuard, OnceLock as TestOnceLock};
    fn scan_test_lock() -> MutexGuard<'static, ()> {
        static LOCK: TestOnceLock<Mutex<()>> = TestOnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    // ---- 基础助手 ----

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(label: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let pid = std::process::id();
            let p = std::env::temp_dir().join(format!("rmole2_{label}_{pid}_{nanos}"));
            fs::create_dir_all(&p).expect("create temp dir");
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_file(root: &Path, rel: &str, contents: &[u8]) {
        let full = root.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("mkdir parent");
        }
        fs::write(&full, contents).expect("write file");
    }

    fn zero_counters() -> (AtomicI64, AtomicI64, AtomicI64) {
        (AtomicI64::new(0), AtomicI64::new(0), AtomicI64::new(0))
    }

    // ---- 路径助手 ----

    #[test]
    fn parent_key_splits_correctly() {
        assert_eq!(parent_key("/a/b/c"), "/a/b");
        assert_eq!(parent_key("/a"), "/");
        // 根路径自身无父目录：rfind 命中首个 '/' 返回 "/"（生产路径不会以根为 parent_key 入参）
        assert_eq!(parent_key("/"), "/");
    }

    #[test]
    fn join_path_handles_root() {
        assert_eq!(join_path("/", "foo"), "/foo");
        assert_eq!(join_path("/a", "b"), "/a/b");
    }

    // ---- 递归排除判定（对齐 Lemon LMFileScanTask.m:255） ----

    #[test]
    fn is_no_recurse_matches_lemon_exclusions() {
        assert!(is_no_recurse(Path::new("/Volumes")));
        assert!(is_no_recurse(Path::new("/System/Volumes")));
        assert!(is_no_recurse(Path::new("/private/tmp/msu-abc123")));
        assert!(!is_no_recurse(Path::new("/usr")));
        assert!(!is_no_recurse(Path::new("/System")));
        assert!(!is_no_recurse(Path::new("/private")));
    }

    // ---- 端到端：小夹具扫描 ----

    #[test]
    fn scan_small_tree_aggregates_sizes_and_counts() {
        let _lock = scan_test_lock();
        let dir = TempDir::new("small_tree");
        let root = dir.path().to_string_lossy().into_owned();
        write_file(dir.path(), "a.txt", &[0u8; 100]);
        write_file(dir.path(), "sub/b.bin", &[0u8; 200]);
        write_file(dir.path(), "sub/c.bin", &[0u8; 50]);
        write_file(dir.path(), "empty", &[]);

        let (f, d, b) = zero_counters();
        let outcome = scan_subtree(&root, &f, &d, &b, None).expect("scan");

        assert_eq!(outcome.result.total_size, 350);
        assert_eq!(outcome.result.total_files, 4);
        assert_eq!(outcome.result.entries.len(), 3, "a.txt + sub + empty");
        // 降序：sub(250) > a.txt(100) > empty(0)
        let sub = &outcome.result.entries[0];
        assert_eq!(sub.name, "sub");
        assert!(sub.is_dir);
        assert_eq!(sub.size, 250);
        assert_eq!(sub.child_files, 2);
        assert_eq!(sub.child_dirs, 0);

        // 子目录节点：drill-in 数据齐备
        let sub_path = format!("{root}/sub");
        let sub_node = outcome.nodes.get(&sub_path).expect("sub node");
        assert_eq!(sub_node.files.len(), 2);
        assert_eq!(sub_node.child_files, 2);
    }

    #[test]
    fn scan_stores_full_details_under_node_modules() {
        let _lock = scan_test_lock();
        let dir = TempDir::new("fold_tree");
        let root = dir.path().to_string_lossy().into_owned();
        write_file(dir.path(), "proj/node_modules/pkg/a.js", &[0u8; 300]);
        write_file(dir.path(), "proj/node_modules/pkg/b.js", &[0u8; 200]);
        write_file(dir.path(), "proj/src/main.rs", &[0u8; 100]);

        let (f, d, b) = zero_counters();
        let outcome = scan_subtree(&root, &f, &d, &b, None).expect("scan");

        // proj = 300 + 200 + 100 = 600
        let proj_path = format!("{root}/proj");
        let proj = outcome.nodes.get(&proj_path).expect("proj node");
        assert_eq!(proj.size, 600);

        // node_modules 不再折叠：每级明细全量落快照（对齐 Lemon）
        let nm_path = format!("{root}/proj/node_modules");
        let nm = outcome.nodes.get(&nm_path).expect("nm node");
        assert_eq!(nm.size, 500);
        assert_eq!(nm.child_dirs, 1, "pkg 子目录计入");

        let pkg_path = format!("{root}/proj/node_modules/pkg");
        let pkg = outcome.nodes.get(&pkg_path).expect("pkg node");
        assert_eq!(pkg.size, 500);
        assert_eq!(pkg.files.len(), 2, "折叠语义已移除，文件明细全量落快照");
        assert_eq!(pkg.child_files, 2);

        // total_files 口径：3 个文件
        assert_eq!(outcome.result.total_files, 3);
    }

    #[test]
    fn scan_includes_hidden_and_zero_size_entries() {
        let _lock = scan_test_lock();
        let dir = TempDir::new("hidden_tree");
        let root = dir.path().to_string_lossy().into_owned();
        write_file(dir.path(), ".hidden", &[0u8; 5]);
        write_file(dir.path(), "empty", &[]);
        fs::create_dir_all(dir.path().join("empty_dir")).unwrap();

        let (f, d, b) = zero_counters();
        let outcome = scan_subtree(&root, &f, &d, &b, None).expect("scan");

        // 对齐 Lemon：隐藏文件、0 大小文件、空目录全部展示
        let names: Vec<&str> = outcome
            .result
            .entries
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert!(names.contains(&".hidden"), "隐藏文件应展示");
        assert!(names.contains(&"empty"), "0 大小文件应展示");
        assert!(names.contains(&"empty_dir"), "空目录应展示");
        assert_eq!(outcome.result.total_size, 5);
    }

    #[cfg(unix)]
    #[test]
    fn scan_records_symlinks_as_files() {
        let _lock = scan_test_lock();
        let dir = TempDir::new("symlink_tree");
        let root = dir.path().to_string_lossy().into_owned();
        write_file(dir.path(), "real.txt", &[0u8; 10]);
        write_file(dir.path(), "sub/target.bin", &[0u8; 20]);
        std::os::unix::fs::symlink(dir.path().join("real.txt"), dir.path().join("ln-file"))
            .unwrap();
        std::os::unix::fs::symlink(dir.path().join("sub"), dir.path().join("ln-dir")).unwrap();

        let (f, d, b) = zero_counters();
        let outcome = scan_subtree(&root, &f, &d, &b, None).expect("scan");

        let file_link = outcome
            .result
            .entries
            .iter()
            .find(|e| e.name == "ln-file")
            .expect("file symlink row");
        assert!(file_link.is_symlink);
        assert!(!file_link.is_dir);

        let dir_link = outcome
            .result
            .entries
            .iter()
            .find(|e| e.name == "ln-dir")
            .expect("dir symlink row");
        assert!(dir_link.is_symlink);
        assert!(
            !dir_link.is_dir,
            "对齐 Lemon VLNK：symlink 一律按文件展示，不跟随"
        );

        // symlink 不计入 total_files（与旧版口径一致）
        assert_eq!(outcome.result.total_files, 2);
    }

    #[cfg(unix)]
    #[test]
    fn scan_dedupes_hardlinks() {
        let _lock = scan_test_lock();
        let dir = TempDir::new("hardlink_tree");
        let root = dir.path().to_string_lossy().into_owned();
        let a = dir.path().join("a.bin");
        fs::write(&a, vec![0u8; 4096]).unwrap();
        fs::hard_link(&a, dir.path().join("b.bin")).unwrap();

        let (f, d, b) = zero_counters();
        let outcome = scan_subtree(&root, &f, &d, &b, None).expect("scan");
        assert!(outcome.result.deduped_hardlink);
        assert_eq!(outcome.result.total_size, 4096);
        assert_eq!(outcome.result.total_files, 2, "文件数仍计 2，大小去重");
    }

    #[test]
    fn bundle_leaf_falls_back_to_recursion_when_not_indexed() {
        let _lock = scan_test_lock();
        let dir = TempDir::new("bundle_leaf");
        let root = dir.path().to_string_lossy().into_owned();
        // 夹具 .app 不在 Spotlight 索引内 → bundle_shortcut 返回 None → 照常递归兜底
        write_file(dir.path(), "Fake.app/Contents/Info.plist", b"plist");
        write_file(dir.path(), "Fake.app/Contents/MacOS/Fake", b"bin");

        let (f, d, b) = zero_counters();
        let outcome = scan_subtree(&root, &f, &d, &b, None).expect("scan");

        let app_path = format!("{root}/Fake.app");
        let app = outcome.nodes.get(&app_path).expect("Fake.app node");
        assert!(!app.bundle_leaf, "未索引 .app 不应叶子化");
        assert!(!app.children.is_empty(), "应照常递归 .app 内部");
        let contents = outcome
            .nodes
            .get(&format!("{app_path}/Contents"))
            .expect("Contents node");
        assert_eq!(contents.files.len(), 1, "内部明细全量落快照");
    }

    #[test]
    fn entries_for_dir_sorts_by_size_desc() {
        let _lock = scan_test_lock();
        let dir = TempDir::new("entries_tree");
        let root = dir.path().to_string_lossy().into_owned();
        write_file(dir.path(), "small", &[0u8; 1]);
        write_file(dir.path(), "big", &[0u8; 9]);
        write_file(dir.path(), "mid", &[0u8; 5]);

        let (f, d, b) = zero_counters();
        let outcome = scan_subtree(&root, &f, &d, &b, None).expect("scan");
        let entries = entries_for_dir(&outcome.nodes, &root);
        let sizes: Vec<i64> = entries.iter().map(|e| e.size).collect();
        assert_eq!(sizes, vec![9, 5, 1]);
    }

    // ---- 目录大小测量 ----

    #[test]
    fn measure_dir_size_native_sums_and_excludes() {
        let _lock = scan_test_lock();
        let dir = TempDir::new("measure_tree");
        let root = dir.path().to_string_lossy().into_owned();
        write_file(dir.path(), "x.bin", &[0u8; 100]);
        write_file(dir.path(), "skip_me/y.bin", &[0u8; 400]);

        let size = measure_dir_size_native(&root, "", &[]).expect("measure");
        assert_eq!(size, 500);

        let exclude = format!("{root}/skip_me");
        let size2 = measure_dir_size_native(&root, &exclude, &[]).expect("measure excl");
        assert_eq!(size2, 100);
    }

    #[test]
    fn measure_dir_size_rejects_invalid() {
        let _lock = scan_test_lock();
        assert!(measure_dir_size_native("", "", &[]).is_err());
        assert!(measure_dir_size_native("/no/such/path__rmole2", "", &[]).is_err());
    }

    // ---- 取消 ----

    #[test]
    fn begin_scan_and_cancel_bump_generation() {
        let _lock = scan_test_lock();
        let g1 = begin_scan();
        assert!(!is_scan_stale(g1));
        cancel_active_scan();
        assert!(is_scan_stale(g1));
        let g2 = begin_scan();
        assert!(g2 > g1);
        assert!(!is_scan_stale(g2));
    }

    #[test]
    fn scan_error_on_non_directory() {
        let _lock = scan_test_lock();
        let dir = TempDir::new("nondir");
        let file = dir.path().join("f.txt");
        fs::write(&file, b"x").unwrap();
        let (f, d, b) = zero_counters();
        let err = scan_subtree(&file.to_string_lossy(), &f, &d, &b, None).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotADirectory);
    }
}
