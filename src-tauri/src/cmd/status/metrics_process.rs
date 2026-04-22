//! 进程采集与内存列表数据（对齐腾讯柠檬 LemonMonitor 口径）。
//!
//! 数据链路（柠檬 memoryTopRepeater / CmcProcInfo 同款）：
//!   ① proc_listpids(PROC_ALL_PIDS) 枚举全部 pid + proc_pidinfo(PROC_PIDTBSDINFO)
//!      取 ppid（纯系统调用，无 ps shell 开销；libc 0.2 已移除 Apple kinfo_proc，
//!      故不走 sysctl(KERN_PROC_ALL)，与柠檬守护进程 CmcProcInfo 完全同源）
//!   ② proc_pidinfo(PROC_PIDTASKINFO) → pti_resident_size（与活动监视器「实际内存」同口径）
//!   ③ 父子进程树后序遍历聚合：aggRSS = 自身 RSS + Σ 子孙聚合 RSS
//!      ——Chrome 主进程单独 RSS 仅百 MB 级，内存大头在 Renderer/GPU 等子孙进程，
//!      聚合后才能对齐柠檬/活动监视器「按进程分组」的 GB 级展示
//!   ④ top_processes：Regular 过滤 → 聚合 RSS 降序 → pid 匹配图标 → proc_pidpath 去截断名
//!
//! CPU 增量：pti_total_user+system 两次采样差值 / 采样间隔（柠檬 calcCpuUsage 同款）。
//! libproc 链路失败时退化 ps 旧链路保底（无聚合）。

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;

#[cfg(target_os = "macos")]
extern "C" {
    /// libproc：按 pid 取完整可执行路径（无 ps comm 的 16 字节截断），
    /// 返回路径长度，失败返回 -1。属 libSystem，无需额外链接配置。
    fn proc_pidpath(pid: libc::c_int, buffer: *mut libc::c_void, buffersize: u32) -> libc::c_int;
    /// libproc：按 pid 取任务级信息（内存/CPU 时间），失败返回 <= 0。
    fn proc_pidinfo(
        pid: libc::c_int,
        flavor: libc::c_int,
        arg: u64,
        buffer: *mut libc::c_void,
        buffersize: libc::c_int,
    ) -> libc::c_int;
    /// libproc：枚举全部 pid（柠檬 CmcProcInfo 同款入口）。
    fn proc_listpids(
        typ: libc::c_int,
        typeinfo: u32,
        buffer: *mut libc::c_void,
        buffersize: libc::c_int,
    ) -> libc::c_int;
}

#[cfg(target_os = "macos")]
const PROC_PIDTASKINFO: libc::c_int = 4;
#[cfg(target_os = "macos")]
const PROC_PIDTBSDINFO: libc::c_int = 3;
#[cfg(target_os = "macos")]
const PROC_ALL_PIDS: libc::c_int = 1;
#[cfg(target_os = "macos")]
const MAXCOMLEN: usize = 16;

/// sys/proc_info.h struct proc_bsdinfo 的 64 位布局（字段顺序/类型严格对齐，
/// 只取所需：pbi_ppid 与 pbi_comm）。
#[cfg(target_os = "macos")]
#[repr(C)]
struct ProcBsdInfo {
    pbi_flags: u32,
    pbi_status: u32,
    pbi_xstatus: u32,
    pbi_pid: u32,
    pbi_ppid: u32,
    pbi_uid: u32,
    pbi_gid: u32,
    pbi_ruid: u32,
    pbi_rgid: u32,
    pbi_svuid: u32,
    pbi_svgid: u32,
    rfu_1: u32,
    pbi_comm: [u8; MAXCOMLEN],
    pbi_name: [u8; 2 * MAXCOMLEN + 1],
    pbi_nfiles: u32,
    pbi_pgid: u32,
    pbi_pjobc: u32,
    e_tdev: u32,
    e_tpgid: u32,
    pbi_nice: i32,
    pbi_start_tvsec: u64,
    pbi_start_tvusec: u64,
}

/// 对齐 sys/libproc.h struct proc_taskinfo（布局稳定）。
#[cfg(target_os = "macos")]
#[repr(C)]
struct ProcTaskInfo {
    pti_virtual_size: u64,
    pti_resident_size: u64,
    pti_total_user: u64,
    pti_total_system: u64,
    pti_threads_user: u64,
    pti_threads_system: u64,
    pti_policy: i32,
    pti_faults: i32,
    pti_pageins: i32,
    pti_cow_faults: i32,
    pti_messages_sent: i32,
    pti_messages_received: i32,
    pti_syscalls_mach: i32,
    pti_syscalls_unix: i32,
    pti_csw: i32,
    pti_threadnum: i32,
    pti_numrunning: i32,
    pti_priority: i32,
}

/// 取进程真实可执行路径（去 ps comm 16 字节截断，
/// 对齐柠檬 McProcessInfoData.pExecutePath 的完整路径语义）。
#[cfg(target_os = "macos")]
fn real_executable_path(pid: i32) -> Option<String> {
    const PROC_PIDPATHINFO_MAXSIZE: u32 = 4096;
    let mut buf = vec![0u8; PROC_PIDPATHINFO_MAXSIZE as usize];
    let ret = unsafe {
        proc_pidpath(
            pid,
            buf.as_mut_ptr() as *mut libc::c_void,
            PROC_PIDPATHINFO_MAXSIZE,
        )
    };
    if ret <= 0 {
        return None;
    }
    buf.truncate(ret as usize);
    String::from_utf8(buf).ok()
}

#[cfg(not(target_os = "macos"))]
fn real_executable_path(_pid: i32) -> Option<String> {
    None
}

/// 取进程任务信息：(resident_size, cpu_time_ms)。cpu_time 对齐柠檬
/// CmcProcInfo：(pti_total_user + pti_total_system) / 1000（ns → ms）。
#[cfg(target_os = "macos")]
fn task_info(pid: i32) -> Option<(u64, u64)> {
    let mut ti = std::mem::MaybeUninit::<ProcTaskInfo>::uninit();
    let ret = unsafe {
        proc_pidinfo(
            pid,
            PROC_PIDTASKINFO,
            0,
            ti.as_mut_ptr() as *mut libc::c_void,
            std::mem::size_of::<ProcTaskInfo>() as libc::c_int,
        )
    };
    if ret <= 0 {
        return None;
    }
    let ti = unsafe { ti.assume_init() };
    let cpu_ms = (ti.pti_total_user + ti.pti_total_system) / 1000;
    Some((ti.pti_resident_size, cpu_ms))
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    pub pid: i32,
    pub ppid: i32,
    pub name: String,
    pub command: String,
    pub cpu: f64,
    pub memory: f64,
    /// RSS 物理内存 (bytes)。sysctl 链路下为**进程家族聚合值**
    /// （自身 + 全部子孙进程 RSS，柠檬 topMemoryArray 同口径）；
    /// ps 保底链路下为单进程值。
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub memory_bytes: u64,
    /// 原生应用图标 SVG data URI（`data:image/svg+xml;base64,…`）：按 pid/ppid 匹配
    /// NSWorkspace runningApplications 索引得到 bundle 路径后，走 `native_icon_registry`
    /// （内容寻址 + 128px + SVG 信封，与文件域共享 contentStore）取得；daemon/helper 无
    /// bundle 则缺省，前端显示骨架屏。仅 top_processes 截断后的展示项填充。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 应用 bundle 路径（`.app`）：前端 iconService 的取图键，与 Uninstall 列表的
    /// `path` 同源，因而能直接命中文件域的磁盘图标缓存（跨窗口共享：托盘与
    /// 主窗口各自一份前端内存 LRU，但磁盘缓存是同一份）。
    /// 前端据此自行预取时，与 `icon` 走的是同一个编码器，data URI 逐字节相同，
    /// 即使两个源先后到位也不会产生图标跳变。仅 top_processes 填充。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_path: Option<String>,
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

// ── libproc 采集链路（macOS，柠檬 CmcProcInfo 同款） ────────────────────

/// sysctl 枚举的轻量进程记录。
#[cfg(target_os = "macos")]
struct RawProc {
    pid: i32,
    ppid: i32,
    rss: u64,
    cpu_time_ms: u64,
}

/// CPU 增量采样缓存（柠檬 calcCpuUsage 同款）：pid → (采样时刻, 累计 cpu 时间 ms)。
static CPU_SAMPLE: Mutex<Option<(std::time::Instant, HashMap<i32, u64>)>> = Mutex::new(None);

/// ① + ②：proc_listpids 枚举全部 pid，逐 pid 采 ppid / RSS / cpu_time。
#[cfg(target_os = "macos")]
fn libproc_collect_processes() -> Result<Vec<RawProc>, String> {
    // 第一次调用拿所需缓冲区大小（柠檬同款，预留 2 倍容忍枚举期间进程增减）
    let need = unsafe { proc_listpids(PROC_ALL_PIDS, 0, std::ptr::null_mut(), 0) };
    if need <= 0 {
        return Err(format!(
            "proc_listpids 探测失败: {}",
            std::io::Error::last_os_error()
        ));
    }
    let buf_size = need * 2;
    let mut pid_buf = vec![0i32; buf_size as usize];
    let got = unsafe {
        proc_listpids(
            PROC_ALL_PIDS,
            0,
            pid_buf.as_mut_ptr() as *mut libc::c_void,
            (buf_size * 4) as libc::c_int,
        )
    };
    if got <= 0 {
        return Err(format!(
            "proc_listpids 枚举失败: {}",
            std::io::Error::last_os_error()
        ));
    }

    let mut procs = Vec::with_capacity(got as usize / 4);
    for &pid in pid_buf.iter().take(got as usize / 4) {
        if pid <= 0 {
            continue;
        }
        // bsd info：ppid（顺带可取 pbi_comm，本处不用于展示名）
        let mut bsd = std::mem::MaybeUninit::<ProcBsdInfo>::zeroed();
        let ret = unsafe {
            proc_pidinfo(
                pid,
                PROC_PIDTBSDINFO,
                0,
                bsd.as_mut_ptr() as *mut libc::c_void,
                std::mem::size_of::<ProcBsdInfo>() as libc::c_int,
            )
        };
        if ret <= 0 {
            continue; // root 进程/已退出（柠檬同款跳过）
        }
        let bsd = unsafe { bsd.assume_init() };
        // task info：RSS + cpu_time
        let Some((rss, cpu_time_ms)) = task_info(pid) else {
            continue;
        };
        procs.push(RawProc {
            pid,
            ppid: bsd.pbi_ppid as i32,
            rss,
            cpu_time_ms,
        });
    }
    Ok(procs)
}

/// CPU 增量计算（两次采样差值 / 间隔，柠檬 calcCpuUsage 同款），并更新采样缓存。
fn compute_cpu_percents(raw: &[(i32, u64)]) -> HashMap<i32, f64> {
    let now = std::time::Instant::now();
    let current: HashMap<i32, u64> = raw.iter().cloned().collect();
    let mut percents = HashMap::new();
    let prev = CPU_SAMPLE.lock().ok().and_then(|mut g| g.take());
    if let Some((prev_time, prev_map)) = prev {
        let interval_ms = now.duration_since(prev_time).as_millis() as u64;
        if interval_ms >= 500 {
            for (pid, t) in &current {
                if let Some(t0) = prev_map.get(pid) {
                    if *t > *t0 {
                        // ms/ms 即单核百分比（多核进程可 >100，与 ps pcpu 同行为）
                        percents.insert(*pid, (*t - *t0) as f64 / interval_ms as f64 * 100.0);
                    }
                }
            }
        }
    }
    if let Ok(mut g) = CPU_SAMPLE.lock() {
        *g = Some((now, current));
    }
    percents
}

// ── ③ 进程家族聚合（柠檬 memoryTopRepeater L106-135 同款） ─────

/// 后序遍历聚合：aggRSS(pid) = rss(pid) + Σ aggRSS(children)。
/// 迭代实现（防深进程树栈溢出）；visit 集合防环（ppid 环异常兜底）。
fn aggregate_rss(raw: &[(i32, i32, u64)]) -> HashMap<i32, u64> {
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    let mut rss: HashMap<i32, u64> = HashMap::new();
    for (pid, ppid, r) in raw {
        rss.insert(*pid, *r);
        children.entry(*ppid).or_default().push(*pid);
    }

    let mut agg: HashMap<i32, u64> = HashMap::new();
    let mut visited = std::collections::HashSet::new();
    for root in rss.keys().copied().collect::<Vec<_>>() {
        if visited.contains(&root) {
            continue;
        }
        let mut stack = vec![(root, false)];
        while let Some((pid, processed)) = stack.pop() {
            if processed {
                let mut total = rss.get(&pid).copied().unwrap_or(0);
                if let Some(kids) = children.get(&pid) {
                    for c in kids {
                        total += agg.get(c).copied().unwrap_or(0);
                    }
                }
                agg.insert(pid, total);
                visited.insert(pid);
            } else {
                if visited.contains(&pid) {
                    continue;
                }
                stack.push((pid, true));
                if let Some(kids) = children.get(&pid) {
                    for c in kids.iter().rev() {
                        if !visited.contains(c) {
                            stack.push((*c, false));
                        }
                    }
                }
            }
        }
    }
    agg
}

// ── 采集入口 ──────────────────────────────────────────────────

pub fn collect_processes() -> Result<Vec<ProcessInfo>, String> {
    #[cfg(target_os = "macos")]
    {
        match sysctl_build() {
            Ok(procs) => return Ok(procs),
            Err(e) => {
                log::warn!("[metrics_process] libproc 链路失败，退化 ps 保底: {}", e);
            }
        }
    }
    ps_collect_fallback()
}

/// libproc 链路组装 ProcessInfo（轻量版：名字留待 top_processes 按需
/// proc_pidpath 解析，避免每帧对全量进程取路径）。
#[cfg(target_os = "macos")]
fn sysctl_build() -> Result<Vec<ProcessInfo>, String> {
    let raw = libproc_collect_processes()?;
    let cpu_map = compute_cpu_percents(
        &raw.iter()
            .map(|p| (p.pid, p.cpu_time_ms))
            .collect::<Vec<_>>(),
    );
    // 家族聚合：memory_bytes = 自身 + 全部子孙 RSS（柠檬 topMemoryArray 口径）
    let triples: Vec<(i32, i32, u64)> = raw.iter().map(|p| (p.pid, p.ppid, p.rss)).collect();
    let agg = aggregate_rss(&triples);
    let total_mem = {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        sys.total_memory()
    };

    Ok(raw
        .iter()
        .map(|p| {
            let mem_bytes = agg.get(&p.pid).copied().unwrap_or(p.rss);
            ProcessInfo {
                pid: p.pid,
                ppid: p.ppid,
                name: String::new(), // top_processes 阶段 proc_pidpath 解析
                command: String::new(),
                cpu: cpu_map.get(&p.pid).copied().unwrap_or(0.0),
                memory: if total_mem > 0 {
                    mem_bytes as f64 / total_mem as f64 * 100.0
                } else {
                    0.0
                },
                memory_bytes: mem_bytes,
                icon: None,
                bundle_path: None,
            }
        })
        .collect())
}

/// ps 保底链路（libproc 失败时）：对齐 Go: ps -Aceo pid=,ppid=,pcpu=,pmem=,rss=,comm= -r。
/// 注意：此链路无家族聚合，memory_bytes 为单进程值。
fn ps_collect_fallback() -> Result<Vec<ProcessInfo>, String> {
    let out = std::process::Command::new("ps")
        .args(["-Aceo", "pid=,ppid=,pcpu=,pmem=,rss=,comm=", "-r"])
        .output()
        .map_err(|e| e.to_string())?;
    let raw = String::from_utf8_lossy(&out.stdout);
    let mut procs = Vec::new();
    for line in raw.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 5 {
            continue;
        }
        let pid: i32 = match fields[0].parse() {
            Ok(p) if p > 0 => p,
            _ => continue,
        };
        let ppid: i32 = fields[1].parse().unwrap_or(0);
        let cpu_val: f64 = match fields[2].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let mem_val: f64 = match fields[3].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let (rss_bytes, command_start) = if fields.len() >= 6 {
            let rss: u64 = fields[4].parse().unwrap_or(0);
            (rss * 1024, 5)
        } else {
            (0, 4)
        };
        let command = fields[command_start..].join(" ");
        let name = command.rsplit('/').next().unwrap_or(&command).to_string();
        procs.push(ProcessInfo {
            pid,
            ppid,
            name,
            command,
            cpu: cpu_val,
            memory: mem_val,
            memory_bytes: rss_bytes,
            icon: None,
            bundle_path: None,
        });
    }
    Ok(procs)
}

// ── ④ 内存列表组装 ────────────────────────────────────────────

/// 内存列表展示上限（对齐柠檬 LMCleanViewController MAX_COUNT_ITEM=20）。
/// 单一事实源：截取只在后端做，前端不再二次 sort/slice。
const MEMORY_LIST_LIMIT: usize = 20;

/// 组装内存列表（严格对齐柠檬 refreshProcMemUIWithInfo 语义）：
/// ① Regular 过滤（只留前台 GUI 应用，daemon/helper 已被家族聚合计入宿主）；
/// ② 聚合 RSS 降序；③ 截取 MEMORY_LIST_LIMIT；④ 富化 localizedName + 图标。
///
/// 索引为空（regular_pids 空）= 采集异常，返回空列表——绝不降级为全量展示，
/// 否则 daemon/广告弹窗会混入列表（柠檬同样无此降级）。
pub fn top_processes(processes: &[ProcessInfo]) -> Vec<ProcessInfo> {
    if processes.is_empty() {
        return Vec::new();
    }

    let index = crate::platform::macos_running_apps::running_apps_icon_index();

    // ① Regular 过滤（对齐柠檬 LMCleanViewController）：内存列表只展示前台应用
    // （activationPolicy == Regular），Helper/daemon 不进列表——它们的内存
    // 已通过家族聚合计入宿主应用。by_pid 由 NSWorkspace.runningApplications 全量构建，
    // 过滤后留下的前台应用 pid 必在 by_pid 中，故富化阶段只用 by_pid.get(pid)。
    // 索引为空（构建异常）时返回空列表，不降级为全量（避免 daemon/广告混入）。
    if index.regular_pids.is_empty() {
        log::warn!("[top_processes] regular_pids empty (index build failed) → 返回空列表");
        return Vec::new();
    }
    let mut sorted: Vec<ProcessInfo> = processes
        .iter()
        .filter(|p| index.regular_pids.contains(&p.pid))
        .cloned()
        .collect();

    // ② 排序口径对齐柠檬内存面板：聚合 RSS 降序（memory_bytes 在 sysctl 链路
    // 已是家族聚合值），内存大户（如 Chrome）自然排前；同值时 cpu 次键、pid 稳定序。
    sorted.sort_by(|a, b| {
        b.memory_bytes
            .cmp(&a.memory_bytes)
            .then_with(|| {
                b.cpu
                    .partial_cmp(&a.cpu)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.pid.cmp(&b.pid))
    });

    // ③ 截断（唯一截取点，对齐柠檬 MAX_COUNT_ITEM=20）
    sorted.truncate(MEMORY_LIST_LIMIT);

    // ④ 富化：仅对展示的进程取图标 + localizedName。
    for p in sorted.iter_mut() {
        // 只用 pid 精确命中：过滤后每个展示进程都是 Regular 应用，pid 必在索引中。
        // 不用 ppid/name/command 兜底——用父进程信息回填子进程逻辑上是错的，
        // 且 sysctl 链路此时 name/command 仍为空，那些兜底本就是死代码。
        if let Some(app) = index.by_pid.get(&p.pid) {
            // 显示名优先用 NSRunningApplication.localizedName（对齐柠檬：
            // 「访达」/「Google Chrome」本地化名），而非 proc_pidpath 的二进制名（「Finder」）。
            if !app.name.is_empty() {
                p.name = app.name.clone();
            }
            if !app.bundle_path.is_empty() {
                p.bundle_path = Some(app.bundle_path.clone());
                // 走 native_icon_registry（内容寻址 + 128px + SVG 信封）。
                // 与文件域共享 contentStore：同一应用的 bundle 路径在 Uninstall / Dashboard
                // / Analyze 等处命中同一份 SVG，前端 contentStore 只存一份。
                p.icon = crate::platform::macos_running_apps::app_icon_svg(&app.bundle_path);
            } else if !app.exec_path.is_empty() {
                // 对齐柠檬 fallback：`iconForFile:pExecutePath`。
                // bundleURL 为 nil（裸二进制 / dev 模式）时，用可执行文件路径取图。
                p.bundle_path = Some(app.exec_path.clone());
                p.icon = crate::platform::macos_running_apps::app_icon_svg(&app.exec_path);
            }
        }

        // proc_pidpath 补完整路径 command；name 仍为空（索引未命中，极少）时用末段兜底。
        if let Some(path) = real_executable_path(p.pid) {
            if p.command.is_empty() {
                p.command = path.clone();
            }
            if p.name.is_empty() {
                if let Some(last) = path.rsplit('/').next() {
                    if !last.is_empty() {
                        p.name = last.to_string();
                    }
                }
            }
        }
    }

    sorted
}
