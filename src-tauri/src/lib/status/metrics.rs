use serde::Serialize;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Instant;

use super::metrics_battery::{ThermalStatus, collect_thermal};
use super::metrics_cpu::{CPUStatus, collect_cpu, collect_cpu_fast};
use super::metrics_disk::{DiskStatus, collect_disks, collect_disks_fast, collect_disks_instant};
use super::metrics_hardware::{HardwareInfo, collect_hardware};
use super::metrics_health::calculate_health_score;
use super::metrics_memory::{MemoryStatus, collect_memory, collect_memory_fast};
use super::metrics_network::{
    NETWORK_HISTORY_SIZE, NetworkHistory, NetworkStatus, ProxyStatus, collect_network,
    collect_proxy,
};
use super::metrics_process::{ProcessInfo, collect_processes, top_processes};
use super::process_watch::{ProcessAlert, ProcessWatchConfig, ProcessWatchOptions, ProcessWatcher};

#[derive(Debug, Clone, Serialize)]
pub struct DiskIOStatus {
    pub read_rate: f64,
    pub write_rate: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SensorReading {
    pub label: String,
    pub value: f64,
    pub unit: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub collected_at: String,
    pub host: String,
    pub platform: String,
    pub procs: u64,
    pub hardware: HardwareInfo,
    pub health_score: i32,
    pub health_score_msg: String,
    pub cpu: CPUStatus,
    pub memory: MemoryStatus,
    pub disks: Vec<DiskStatus>,
    pub trash_size: u64,
    pub trash_approx: bool,
    pub disk_io: DiskIOStatus,
    pub network: Vec<NetworkStatus>,
    pub network_history: NetworkHistory,
    pub proxy: ProxyStatus,
    pub thermal: ThermalStatus,
    pub sensors: Option<Vec<SensorReading>>,
    pub top_processes: Vec<ProcessInfo>,
    pub process_watch: ProcessWatchConfig,
    pub process_alerts: Vec<ProcessAlert>,
}

// ── 三级采集：中间结果 ──────────────────────────────────────────

struct CollectedMetrics {
    cpu: CPUStatus,
    memory: MemoryStatus,
    disks: Vec<DiskStatus>,
    trash: (u64, bool),
    proxy: ProxyStatus,
    thermal: ThermalStatus,
    processes: Vec<ProcessInfo>,
    has_processes: bool,
}

// ── Snapshot 缓存层 对齐 Go snapshotEnrichment ──────────────────

#[derive(Clone)]
struct SnapshotEnrichment {
    hardware: HardwareInfo,
    cpu_p_cores: i32,
    cpu_e_cores: i32,
    disks: Vec<DiskStatus>,
    has_disks: bool,
    trash_size: u64,
    trash_approx: bool,
    proxy: ProxyStatus,
    top_processes: Vec<ProcessInfo>,
    process_alerts: Vec<ProcessAlert>,
}

// ── Collector ───────────────────────────────────────────────────

pub struct Collector {
    pub cached_hw: Option<HardwareInfo>,
    pub last_hw_at: Option<Instant>,

    pub prev_net: HashMap<String, (u64, u64)>,
    pub last_net_at: Option<Instant>,
    pub rx_history_buf: Vec<f64>,
    pub tx_history_buf: Vec<f64>,

    pub prev_disk_io: Option<(f64, f64)>,
    pub last_disk_at: Option<Instant>,

    pub process_watch_config: ProcessWatchConfig,
    pub process_watcher: ProcessWatcher,

    /// 全量采集后缓存的慢变字段；快速采集时注入。
    enrichment: Option<SnapshotEnrichment>,
    has_enrichment: bool,
}

impl Collector {
    pub fn new(options: ProcessWatchOptions) -> Self {
        let process_watch_config = options.snapshot_config();
        Self {
            cached_hw: None,
            last_hw_at: None,
            prev_net: HashMap::new(),
            last_net_at: None,
            rx_history_buf: Vec::with_capacity(NETWORK_HISTORY_SIZE),
            tx_history_buf: Vec::with_capacity(NETWORK_HISTORY_SIZE),
            prev_disk_io: None,
            last_disk_at: None,
            process_watch_config,
            process_watcher: ProcessWatcher::new(options),
            enrichment: None,
            has_enrichment: false,
        }
    }

    // ── 三层采集 API ────────────────────────────────────────────
    // 对齐 Go: CollectFast / CollectProcesses / Collect

    /// 全量采集（对齐 Go Collect()）：所有指标，含慢指标。
    /// 采集完后 cache_enrichment() 供后续快速采集注入。
    pub fn collect(&mut self) -> MetricsSnapshot {
        let (snap, _err) = self.collect_full();
        snap
    }

    /// 快速采集（对齐 Go CollectFast()）：只采 CPU/内存/磁盘/网络/IO。
    /// 慢指标从 snapshotEnrichment 缓存注入。
    pub fn collect_fast(&mut self) -> MetricsSnapshot {
        self.collect_fast_inner(false)
    }

    /// 进程采集（对齐 Go CollectProcesses()）：Fast + 进程列表。
    pub fn collect_processes(&mut self) -> MetricsSnapshot {
        self.collect_fast_inner(true)
    }

    /// F0 轻帧采集（**watch 首帧专用**）：只取 GUI 首屏真正消费的字段
    /// （disks/memory/thermal/network），全程原生调用零子进程。
    ///
    /// 含 thermal：托盘 StatusBar 需要温度/风扇；SMC 首次连接 + 温度键全扫描
    /// 实测≈600ms（每进程仅一次，后续命中 SENSOR_TTL/TEMP_KEYS 缓存）。
    /// 需 `&mut self`（网络历史缓冲）→ 由 watch 线程持锁调用。
    /// Home/Analyze 的 `mole_status_once` 请用无锁的 [`instant_snapshot_lockfree`]。
    ///
    /// 与全量采集的差异：
    /// - `disks` 只含根卷（`collect_disks_instant`：statfs + NSURL 容量，SSOT 口径不变），
    ///   完整 <=3 卷列表由 F1/Full 帧经事件替换；
    /// - `hardware` 用 `cached_hw` 或空结构，**不**触发 system_profiler；
    /// - `processes`/`top_processes` 为空，`disk_io`/`proxy`/`trash_size` 为零值；
    /// - **不**调用 `cache_enrichment()`（enrichment 必须由全量帧建立，
    ///   否则 Fast tick 会注入轻帧的空 hardware/进程列表）。
    ///
    /// 上述留空字段经 grep 证实前端零消费（`hardware`/`cpu.per_core`/`disk_io`/
    /// `proxy`/`health_score`/`process_alerts`），仅作 MetricsSnapshot JSON 契约保留。
    pub fn collect_instant(&mut self) -> MetricsSnapshot {
        let now = Instant::now();
        let collected = collect_instant_metrics(true);

        // disk_io 走 iostat 子进程且前端零消费：F0 直接零值，留给 F1/Full
        let disk_io = DiskIOStatus {
            read_rate: 0.0,
            write_rate: 0.0,
        };
        let net_stats = self.collect_network_inner(now).unwrap_or_default();

        let now_str = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, false);
        let host_info = (hostname(), platform_string());
        self.snapshot_from_collected(now_str, &host_info, &collected, disk_io, &net_stats, false)
    }

    // ── 向后兼容：legacy API ─────────────────────────────────────
    // 用于 status.rs controller / main.rs CLI，首个 tick 全量，后续用 Fast 节奏。

    pub fn collect_first(&mut self) -> MetricsSnapshot {
        self.collect()
    }

    pub fn collect_second(&mut self) -> MetricsSnapshot {
        self.collect_fast()
    }

    // ── 内部实现 ─────────────────────────────────────────────────

    fn collect_full(&mut self) -> (MetricsSnapshot, Option<String>) {
        let now = Instant::now();
        let now_str = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, false);
        let host_info = (hostname(), platform_string());

        let pure = collect_pure_concurrent(); // 全量：含 trash/proxy/thermal/processes

        let collected = CollectedMetrics {
            cpu: pure.cpu,
            memory: pure.memory,
            disks: pure.disks,
            trash: pure.trash,
            proxy: pure.proxy,
            thermal: pure.thermal,
            processes: pure.processes,
            has_processes: true,
        };

        let disk_io = self.collect_disk_io_inner(now);
        let net_stats = self.collect_network_inner(now).unwrap_or_default();

        let snap = self
            .snapshot_from_collected(now_str, &host_info, &collected, disk_io, &net_stats, true);

        self.cache_enrichment(&snap);
        (snap, None)
    }

    fn collect_fast_inner(&mut self, include_processes: bool) -> MetricsSnapshot {
        let now = Instant::now();
        let now_str = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, false);
        let host_info = (hostname(), platform_string());

        let collected = collect_fast_concurrent(include_processes);

        let disk_io = self.collect_disk_io_inner(now);
        let net_stats = self.collect_network_inner(now).unwrap_or_default();

        let mut snap = self
            .snapshot_from_collected(now_str, &host_info, &collected, disk_io, &net_stats, false);

        self.apply_enrichment(&mut snap, collected.has_processes);
        snap
    }

    fn snapshot_from_collected(
        &mut self,
        now_str: String,
        host_info: &(String, String),
        collected: &CollectedMetrics,
        disk_io: DiskIOStatus,
        net_stats: &[NetworkStatus],
        refresh_hardware: bool,
    ) -> MetricsSnapshot {
        let (ref host, ref platform) = *host_info;

        // 硬件缓存：全量时刷新
        if refresh_hardware
            && (!self.cached_hw.is_some()
                || self
                    .last_hw_at
                    .map_or(true, |t| t.elapsed().as_secs() > 600))
        {
            let hw = timed_branch("full.hardware", || {
                collect_hardware(collected.memory.total, &collected.disks)
            });
            self.cached_hw = Some(hw);
            self.last_hw_at = Some(Instant::now());
        }
        let hw = self.cached_hw.clone().unwrap_or_else(|| HardwareInfo {
            model: String::new(),
            cpu_model: String::new(),
            total_ram: String::new(),
            disk_size: String::new(),
            os_version: String::new(),
            refresh_rate: String::new(),
        });

        let (score, score_msg) = calculate_health_score(
            &collected.cpu,
            &collected.memory,
            &collected.disks,
            &disk_io,
            &collected.thermal,
        );

        let top_procs = if collected.has_processes {
            // 图标索引首次构建（NSWorkspace runningApplications + PNG base64）开销可观，计时定位
            timed_branch("full.top_processes", || top_processes(&collected.processes))
        } else {
            Vec::new()
        };

        let process_alerts = if collected.has_processes {
            self.process_watcher
                .update(Instant::now(), &collected.processes)
        } else {
            self.process_watcher.snapshot()
        };

        MetricsSnapshot {
            collected_at: now_str,
            host: host.clone(),
            platform: platform.clone(),
            procs: collected.processes.len() as u64,
            hardware: hw,
            health_score: score,
            health_score_msg: score_msg,
            cpu: collected.cpu.clone(),
            memory: collected.memory.clone(),
            disks: collected.disks.clone(),
            trash_size: collected.trash.0,
            trash_approx: collected.trash.1,
            disk_io,
            network: net_stats.to_vec(),
            network_history: NetworkHistory {
                rx_latest: self.rx_history_buf.last().copied().unwrap_or(0.0),
                tx_latest: self.tx_history_buf.last().copied().unwrap_or(0.0),
                rx_history: self.rx_history_buf.clone(),
                tx_history: self.tx_history_buf.clone(),
            },
            proxy: collected.proxy.clone(),
            thermal: collected.thermal.clone(),
            sensors: None,
            top_processes: top_procs,
            process_watch: self.process_watch_config.clone(),
            process_alerts,
        }
    }

    // ── 缓存注入 对齐 Go cacheEnrichment / applyEnrichment ──────

    fn cache_enrichment(&mut self, snap: &MetricsSnapshot) {
        self.enrichment = Some(SnapshotEnrichment {
            hardware: snap.hardware.clone(),
            cpu_p_cores: snap.cpu.p_core_count,
            cpu_e_cores: snap.cpu.e_core_count,
            disks: snap.disks.clone(),
            has_disks: !snap.disks.is_empty(),
            trash_size: snap.trash_size,
            trash_approx: snap.trash_approx,
            proxy: snap.proxy.clone(),
            top_processes: snap.top_processes.clone(),
            process_alerts: snap.process_alerts.clone(),
        });
        self.has_enrichment = true;
    }

    fn apply_enrichment(&self, snap: &mut MetricsSnapshot, preserve_live_processes: bool) {
        let Some(ref e) = self.enrichment else { return };
        if !self.has_enrichment {
            return;
        }

        snap.hardware = e.hardware.clone();
        snap.cpu.p_core_count = e.cpu_p_cores;
        snap.cpu.e_core_count = e.cpu_e_cores;
        // memory.cached / memory.pressure / thermal 不从 enrichment 覆盖：
        // Fast 路径已每 2s 采集新鲜数据，覆盖会让这些指标退化为 30s 阶梯更新。
        // 磁盘矫正值（APFS purgeable 等）来自全量采集，覆盖快速路径的原始 statfs 值
        if e.has_disks && !e.disks.is_empty() {
            snap.disks = e.disks.clone();
        }
        snap.trash_size = e.trash_size;
        snap.trash_approx = e.trash_approx;
        snap.proxy = e.proxy.clone();

        if !preserve_live_processes {
            snap.top_processes = e.top_processes.clone();
            snap.process_alerts = e.process_alerts.clone();
        }

        // 基于注入值重新计算健康分
        let (score, score_msg) = calculate_health_score(
            &snap.cpu,
            &snap.memory,
            &snap.disks,
            &snap.disk_io,
            &snap.thermal,
        );
        snap.health_score = score;
        snap.health_score_msg = score_msg;
    }

    // ── 子采集器 ─────────────────────────────────────────────────

    fn collect_network_inner(&mut self, now: Instant) -> Result<Vec<NetworkStatus>, String> {
        collect_network(
            &mut self.prev_net,
            &mut self.last_net_at,
            &mut self.rx_history_buf,
            &mut self.tx_history_buf,
            now,
        )
    }

    fn collect_disk_io_inner(&mut self, now: Instant) -> DiskIOStatus {
        use std::process::Command;
        let out = Command::new("iostat")
            .arg("-d")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();
        let mut total_read: f64 = 0.0;
        let mut total_write: f64 = 0.0;
        for line in out.lines().skip(2) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 3 {
                continue;
            }
            if let (Ok(r), Ok(w)) = (fields[1].parse::<f64>(), fields[2].parse::<f64>()) {
                total_read += r;
                total_write += w;
            }
        }
        let r = match (&mut self.prev_disk_io, &self.last_disk_at) {
            (Some(pref), Some(lt)) => {
                let e = now.duration_since(*lt).as_secs_f64().max(1.0);
                let (pr, pw) = *pref;
                let rr = if total_read >= pr {
                    (total_read - pr) / 1024.0 / e
                } else {
                    0.0
                };
                let wr = if total_write >= pw {
                    (total_write - pw) / 1024.0 / e
                } else {
                    0.0
                };
                *pref = (total_read, total_write);
                DiskIOStatus {
                    read_rate: rr,
                    write_rate: wr,
                }
            }
            _ => {
                self.prev_disk_io = Some((total_read, total_write));
                DiskIOStatus {
                    read_rate: 0.0,
                    write_rate: 0.0,
                }
            }
        };
        self.last_disk_at = Some(now);
        r
    }
}

// ── 辅助函数 ────────────────────────────────────────────────────

fn hostname() -> String {
    // 进程内恒定：OnceLock 缓存，避免每帧 fork。
    static HOST: OnceLock<String> = OnceLock::new();
    HOST.get_or_init(read_hostname).clone()
}

/// `libc::gethostname` 零子进程（原 `hostname` fork ≈35ms/帧）；失败回退 "unknown"。
#[cfg(target_os = "macos")]
fn read_hostname() -> String {
    let mut buf = [0u8; 256];
    if unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) } != 0 {
        return "unknown".into();
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let name = String::from_utf8_lossy(&buf[..len]).trim().to_string();
    if name.is_empty() {
        "unknown".into()
    } else {
        name
    }
}

#[cfg(not(target_os = "macos"))]
fn read_hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn platform_string() -> String {
    format!("darwin {}", os_version())
}

/// 系统版本号（如 "12.3.1"）：进程内恒定，OnceLock 缓存。
/// `pub(super)` 供 metrics_hardware 复用，消除原先 collect_full 与 collect_hardware
/// 各 fork 一次 `sw_vers` 的重复开销。探测失败为 "unknown"。
pub(super) fn os_version() -> String {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(read_os_version).clone()
}

/// `sysctlbyname("kern.osproductversion")` 零子进程（原 `sw_vers` fork ≈64ms/帧）；
/// 失败回退 `sw_vers`，保持输出格式不变。
#[cfg(target_os = "macos")]
fn read_os_version() -> String {
    if let Some(v) = sysctl_string("kern.osproductversion") {
        if !v.is_empty() {
            return v;
        }
    }
    std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(not(target_os = "macos"))]
fn read_os_version() -> String {
    "unknown".into()
}

/// 读字符串型 sysctl（两次调用：先取长度再取值）。
#[cfg(target_os = "macos")]
fn sysctl_string(name: &str) -> Option<String> {
    use std::ffi::CString;

    let key = CString::new(name).ok()?;
    let mut size: usize = 0;
    let rc = unsafe {
        libc::sysctlbyname(
            key.as_ptr(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || size == 0 {
        return None;
    }

    let mut buf = vec![0u8; size];
    let rc = unsafe {
        libc::sysctlbyname(
            key.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }

    buf.truncate(size);
    // sysctl 字符串带结尾 NUL
    if let Some(end) = buf.iter().position(|&b| b == 0) {
        buf.truncate(end);
    }
    Some(String::from_utf8_lossy(&buf).trim().to_string())
}

// ── 并发采集 ────────────────────────────────────────────────────

/// 全量采集（CPU/内存/磁盘/Trash/代理/散热/进程），并行 fork。
/// 各分支计时为 debug 级：全量帧耗时 = 最慢分支，据此定位慢点。
fn collect_pure_concurrent() -> PureCollect {
    std::thread::scope(|s| {
        let h_cpu = s.spawn(|| timed_branch("full.cpu", collect_cpu));
        let h_mem = s.spawn(|| timed_branch("full.memory", collect_memory));
        let h_disks = s.spawn(|| timed_branch("full.disks", collect_disks));
        let h_trash = s.spawn(|| timed_branch("full.trash", collect_trash_size));
        let h_proxy = s.spawn(|| timed_branch("full.proxy", collect_proxy));
        let h_therm = s.spawn(|| timed_branch("full.thermal", collect_thermal));
        let h_proc =
            s.spawn(|| timed_branch("full.processes", || collect_processes().unwrap_or_default()));

        PureCollect {
            cpu: h_cpu.join().unwrap_or_else(|_| collect_cpu()),
            memory: h_mem.join().unwrap_or_else(|_| collect_memory()),
            disks: h_disks.join().unwrap_or_default(),
            trash: h_trash.join().unwrap_or((0, false)),
            proxy: h_proxy.join().unwrap_or_default(),
            thermal: h_therm.join().unwrap_or_else(|_| collect_thermal()),
            processes: h_proc.join().unwrap_or_default(),
        }
    })
}

/// 快速采集（CPU_fast/内存_fast/磁盘_fast/thermal），可选进程列表。
/// 跳过 vm_stat / memory_pressure / system_profiler / ioreg 等外部命令。
/// thermal（SMC 风扇/温度）已加入 Fast 路径，确保趋势图每秒更新真实 RPM。
fn collect_fast_concurrent(include_processes: bool) -> CollectedMetrics {
    std::thread::scope(|s| {
        let h_cpu = s.spawn(collect_cpu_fast);
        let h_mem = s.spawn(collect_memory_fast);
        let h_disks = s.spawn(collect_disks_fast);
        let h_therm = s.spawn(collect_thermal);

        let processes = if include_processes {
            let h_proc = s.spawn(|| collect_processes().unwrap_or_default());
            Some(h_proc.join().unwrap_or_default())
        } else {
            None
        };

        let procs = processes.unwrap_or_default();
        let has_procs = include_processes && !procs.is_empty();

        CollectedMetrics {
            cpu: h_cpu.join().unwrap_or_else(|_| collect_cpu_fast()),
            memory: h_mem.join().unwrap_or_else(|_| collect_memory_fast()),
            disks: h_disks.join().unwrap_or_default(),
            trash: (0, false),
            proxy: ProxyStatus::default(),
            thermal: h_therm.join().unwrap_or_else(|_| collect_thermal()),
            processes: procs,
            has_processes: has_procs,
        }
    })
}

/// F0 轻帧采集：CPU_fast / 内存_fast / 磁盘_instant（+ 可选 thermal）并行。
/// 与 Fast 路径的唯一差异是磁盘：`collect_disks_instant` 走 statfs + NSURL 容量，
/// 不再 fork `df`/`diskutil`，使首帧磁盘分支从 ≈370ms 降到毫秒级。
/// 慢字段（hardware / processes / disk_io / proxy / trash）留空，由 F1 补帧填充。
///
/// `with_thermal`：托盘气泡首帧需要温度/风扇（StatusBar），故传 true；
/// Home/Analyze 不展示温度，传 false 可避开 SMC 首次连接与温度键全扫描
/// （实测冷启动 ≈630ms，且为每进程仅一次）。
fn collect_instant_metrics(with_thermal: bool) -> CollectedMetrics {
    std::thread::scope(|s| {
        let h_cpu = s.spawn(|| timed_branch("cpu", collect_cpu_fast));
        let h_mem = s.spawn(|| timed_branch("memory", collect_memory_fast));
        let h_disks = s.spawn(|| timed_branch("disks", collect_disks_instant));
        let h_therm = with_thermal.then(|| s.spawn(|| timed_branch("thermal", collect_thermal)));

        let thermal = match h_therm {
            Some(h) => h.join().unwrap_or_else(|_| collect_thermal()),
            // 不采 thermal：保持零值（无消费者会渲染它，前端按「无数据」处理）
            None => ThermalStatus {
                cpu_temp: 0.0,
                fan_speed: 0,
                fan_count: 0,
            },
        };

        CollectedMetrics {
            cpu: h_cpu.join().unwrap_or_else(|_| collect_cpu_fast()),
            memory: h_mem.join().unwrap_or_else(|_| collect_memory_fast()),
            disks: h_disks.join().unwrap_or_default(),
            trash: (0, false),
            proxy: ProxyStatus::default(),
            thermal,
            processes: Vec::new(),
            has_processes: false,
        }
    })
}

/// 无锁轻帧：不访问 `Collector`，因而**不需要 Collector 互斥锁**。
///
/// 为何必要：全量采集（F1/Full）实测可达 10s+ 且全程持锁，若 `mole_status_once`
/// 也去抢同一把锁，Home 在缓存过期后重新挂载就会排队等锁，磁盘卡片又被拖慢。
/// 本函数只组 GUI 首屏必需的零成本字段（disks/memory/cpu），永不阻塞。
///
/// 不采 thermal（Home/Analyze 不展示温度/风扇，避开 SMC 冷启动 ≈630ms）；
/// thermal / network / hardware / processes 均为零值或空，经 grep 证实
/// 这两页零消费，完整值由 F1 经事件补齐。
///
/// 实测（debug 构建）：memory 0.1ms + cpu 0.2ms + disks 107ms（首次 NSURL
/// important-usage 查询，120s 缓存后降至毫秒级）= 总计 ≈107ms；
/// 改造前 `mole_status_once` 走同步全量采集，实测 ≈12.5s。
pub fn instant_snapshot_lockfree() -> MetricsSnapshot {
    let collected = collect_instant_metrics(false);

    let disk_io = DiskIOStatus {
        read_rate: 0.0,
        write_rate: 0.0,
    };
    let (score, score_msg) = calculate_health_score(
        &collected.cpu,
        &collected.memory,
        &collected.disks,
        &disk_io,
        &collected.thermal,
    );

    MetricsSnapshot {
        collected_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, false),
        host: hostname(),
        platform: platform_string(),
        procs: 0,
        hardware: HardwareInfo {
            model: String::new(),
            cpu_model: String::new(),
            total_ram: String::new(),
            disk_size: String::new(),
            os_version: String::new(),
            refresh_rate: String::new(),
        },
        health_score: score,
        health_score_msg: score_msg,
        cpu: collected.cpu,
        memory: collected.memory,
        disks: collected.disks,
        trash_size: 0,
        trash_approx: false,
        disk_io,
        network: Vec::new(),
        network_history: NetworkHistory {
            rx_latest: 0.0,
            tx_latest: 0.0,
            rx_history: Vec::new(),
            tx_history: Vec::new(),
        },
        proxy: ProxyStatus::default(),
        thermal: collected.thermal,
        sensors: None,
        top_processes: Vec::new(),
        process_watch: ProcessWatchOptions::default().snapshot_config(),
        process_alerts: Vec::new(),
    }
}

/// 分支计时：一帧的耗时 = 最慢分支，任一分支超标可直接从日志定位。
/// F0 分支名为 cpu/memory/disks/thermal/network，全量帧分支名统一带 `full.` 前缀。
fn timed_branch<T>(branch: &str, f: impl FnOnce() -> T) -> T {
    let out = f();
    let _ = branch; // 仅用于标识，日志已移除
    out
}

struct PureCollect {
    cpu: CPUStatus,
    memory: MemoryStatus,
    disks: Vec<DiskStatus>,
    trash: (u64, bool),
    proxy: ProxyStatus,
    thermal: ThermalStatus,
    processes: Vec<ProcessInfo>,
}

fn collect_trash_size() -> (u64, bool) {
    let home = crate::core::base::home_dir_opt().unwrap_or_default();
    let trash = home.join(".Trash");
    match trash.read_dir() {
        Ok(rd) => {
            let mut sz: u64 = 0;
            for e in rd.flatten() {
                if let Ok(m) = e.metadata() {
                    sz += m.len();
                }
            }
            (sz, false)
        }
        Err(_) => (0, false),
    }
}
