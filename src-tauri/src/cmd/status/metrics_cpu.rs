use serde::Serialize;
use std::process::Command;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

const CPU_SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Serialize)]
pub struct CPUStatus {
    pub usage: f64,
    pub per_core: Vec<f64>,
    pub per_core_estimated: bool,
    pub core_count: i32,
    pub logical_cpu: i32,
    pub p_core_count: i32,
    pub e_core_count: i32,
    /// CPU 时间片分解（对齐活动监视器：用户/系统，单位 %）。
    /// 基于两次采样的 ticks 增量，首次采集无前值时为 null，前端据此隐藏分解行。
    /// 闲置不单独下发：usage = 100 - idle，与闲置互为反面，属冗余信息。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_pct: Option<f64>,
}

/// 完整采集：带 100ms warmup 延时以保证 per-core 准确。
pub fn collect_cpu() -> CPUStatus {
    collect_cpu_with_options(true)
}

/// 快速采集：跳过 100ms warmup，对齐 Go collectCPUFast()。
/// per-core 可能为零值估算（per_core_estimated = true），
/// 由全量采集的 snapshotEnrichment 缓存 P/E 核数覆盖。
pub fn collect_cpu_fast() -> CPUStatus {
    collect_cpu_with_options(false)
}

fn collect_cpu_with_options(include_slow: bool) -> CPUStatus {
    // 只刷 CPU：`System::new_all()` = `RefreshKind::everything()`，会连带刷新
    // processes/disks/networks/components/users —— 464 进程时 sysinfo 对每个 pid
    // 走 proc_pidinfo + proc_pidpath，是秒级开销，而本函数只用到核数与每核占用
    // （usage/user/system 由原生 cpu_breakdown::sample() 提供）。
    let mut sys = sysinfo::System::new();

    // `refresh_cpu_usage()` = `CpuRefreshKind::nothing().with_cpu_usage()`：不取 frequency。
    // CPUStatus 不序列化 frequency，而 `refresh_cpu_all()` 在 Apple Silicon 上
    // （`hw.cpufrequency` 不存在）会每次走 IOKit `AppleARMIODevice` 匹配；
    // 本函数每次都新建 System（got_cpu_frequency 无法复用），故必须显式排除。
    if include_slow {
        sys.refresh_cpu_usage();
        thread::sleep(CPU_SAMPLE_INTERVAL);
    }
    sys.refresh_cpu_usage();

    let (core_count, logical_cpu) = core_counts(&sys);
    let per_core: Vec<f64> = sys.cpus().iter().map(|c| c.cpu_usage() as f64).collect();

    // CPU 时间片分解（系统/用户），区间增量取自上次采集。
    // usage 改用 100 - idle 计算，与 cpu_breakdown 共享同一数据源，
    // 避免 Fast 路径无采样间隔导致 global_cpu_usage() 不准。
    // idle 仅内部参与 usage 计算，不对外序列化。
    let (usage, user_pct, system_pct) = match cpu_breakdown::sample() {
        Some((u, s, i)) => (100.0 - i, Some(u), Some(s)),
        None => {
            // 首次采集无前值，回退到 sysinfo（可能不准）
            (sys.global_cpu_usage() as f64, None, None)
        }
    };

    // P/E core topology 只在完整采集中获取并缓存到 enrichment；
    // 快速路径用零值占位（F0 轻帧要求零子进程，topology 的 sysctl 留给 Full 帧）。
    // get_core_topology 内部 OnceLock 缓存，Full tick 不再重复 fork sysctl。
    let (p_cores, e_cores) = if include_slow {
        get_core_topology()
    } else {
        (0, 0)
    };

    // 跳过 warmup 时 per-core 数值不可靠，标记为估算。
    let per_core_estimated = !include_slow;

    CPUStatus {
        usage,
        per_core,
        per_core_estimated,
        core_count,
        logical_cpu,
        p_core_count: p_cores,
        e_core_count: e_cores,
        user_pct,
        system_pct,
    }
}

// ── CPU 时间片分解（对齐活动监视器 / 柠檬 CmcGetCpuTicks 语义） ──

#[cfg(target_os = "macos")]
mod cpu_breakdown {
    use std::sync::Mutex;

    /// 上次采样的整机累计 ticks [user, system, idle, nice]。
    /// 全局保留前值，使 Fast/Full 两条采集路径共享同一增量基准。
    static PREV_TICKS: Mutex<Option<[u64; 4]>> = Mutex::new(None);

    /// 读取整机累计 CPU ticks（host_statistics + HOST_CPU_LOAD_INFO）。
    fn read_ticks() -> Option<[u64; 4]> {
        unsafe {
            let mut info: libc::host_cpu_load_info = std::mem::zeroed();
            let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
            let kr = libc::host_statistics(
                libc::mach_host_self(),
                libc::HOST_CPU_LOAD_INFO,
                &mut info as *mut libc::host_cpu_load_info as libc::host_info_t,
                &mut count as *mut _,
            );
            if kr != libc::KERN_SUCCESS {
                return None;
            }
            Some(info.cpu_ticks.map(u64::from))
        }
    }

    /// 基于两次采样的 ticks 增量，计算（用户%, 系统%, 闲置%）。
    /// 首次调用无前值返回 None；nice 时间计入用户态。
    pub fn sample() -> Option<(f64, f64, f64)> {
        let now = read_ticks()?;
        let prev = {
            // 对齐 status.rs 的毒锁恢复策略：采集 panic 毒化锁后仍可取回增量基准，
            // 避免一次异常导致前端分解行永久消失。
            let mut guard = match PREV_TICKS.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.replace(now)
        };
        let p = prev?;
        let du = now[0].saturating_sub(p[0]);
        let ds = now[1].saturating_sub(p[1]);
        let di = now[2].saturating_sub(p[2]);
        let dn = now[3].saturating_sub(p[3]);
        let total = du + ds + di + dn;
        if total == 0 {
            return None;
        }
        let user = (du + dn) as f64 / total as f64 * 100.0;
        let system = ds as f64 / total as f64 * 100.0;
        let idle = di as f64 / total as f64 * 100.0;
        Some((user, system, idle))
    }
}

#[cfg(not(target_os = "macos"))]
mod cpu_breakdown {
    pub fn sample() -> Option<(f64, f64, f64)> {
        None
    }
}

// ── 核数（进程内恒定，零子进程） ──────────────────────────────

/// 物理核数 / 逻辑核数：进程内不变，OnceLock 缓存一次探测。
/// macOS 走 `sysctlbyname`（零子进程）；失败或非 macOS 回退 sysinfo。
fn core_counts(sys: &sysinfo::System) -> (i32, i32) {
    static COUNTS: OnceLock<(i32, i32)> = OnceLock::new();
    *COUNTS.get_or_init(|| {
        sysctl_core_counts()
            .filter(|(physical, logical)| *physical > 0 && *logical > 0)
            .unwrap_or_else(|| {
                (
                    sys.physical_core_count().unwrap_or(1) as i32,
                    (sys.cpus().len() as i32).max(1),
                )
            })
    })
}

#[cfg(target_os = "macos")]
fn sysctl_core_counts() -> Option<(i32, i32)> {
    Some((sysctl_i32("hw.physicalcpu")?, sysctl_i32("hw.logicalcpu")?))
}

#[cfg(not(target_os = "macos"))]
fn sysctl_core_counts() -> Option<(i32, i32)> {
    None
}

#[cfg(target_os = "macos")]
fn sysctl_i32(name: &str) -> Option<i32> {
    use std::ffi::CString;

    let key = CString::new(name).ok()?;
    let mut value: i32 = 0;
    let mut size = std::mem::size_of::<i32>();
    let rc = unsafe {
        libc::sysctlbyname(
            key.as_ptr(),
            &mut value as *mut i32 as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc == 0 { Some(value) } else { None }
}

fn get_core_topology() -> (i32, i32) {
    // P/E 拓扑进程内恒定：缓存首次探测结果，后续 Full tick 直接复用，
    // 不再每 30s fork 一次 sysctl。
    static TOPOLOGY: OnceLock<(i32, i32)> = OnceLock::new();
    *TOPOLOGY.get_or_init(read_core_topology)
}

fn read_core_topology() -> (i32, i32) {
    let output = match Command::new("sysctl")
        .args([
            "-n",
            "hw.perflevel0.logicalcpu",
            "hw.perflevel0.name",
            "hw.perflevel1.logicalcpu",
            "hw.perflevel1.name",
        ])
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return (0, 0),
    };

    let lines: Vec<&str> = output.trim().lines().collect();
    if lines.len() < 4 {
        return (0, 0);
    }

    let l0_count: i32 = lines[0].trim().parse().unwrap_or(0);
    let l0_name = lines[1].trim().to_lowercase();
    let l1_count: i32 = lines[2].trim().parse().unwrap_or(0);
    let l1_name = lines[3].trim().to_lowercase();

    let mut p = 0;
    let mut e = 0;
    if l0_name.contains("performance") {
        p = l0_count;
    } else if l0_name.contains("efficiency") {
        e = l0_count;
    }
    if l1_name.contains("performance") {
        p = l1_count;
    } else if l1_name.contains("efficiency") {
        e = l1_count;
    }

    (p, e)
}
