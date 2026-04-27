// 对标 Mole bin/status.sh + cmd/status/main.go：
//
// - Mole TUI：进程内复用同一个 `Collector`，每 `refreshInterval = 1s` 调一次；
//   使用三级采集节奏：Fast(2s) / Process(2s) / Full(30s)。
// - Mole `--watch`：持续 NDJSON，同样三层节奏。
//
// MoleStudio 端降低 Fast/Process 频率至 2s，减少 CPU/电池开销：
// - **持久 Collector**（全局单例）：跨命令调用复用，保留网络/磁盘 I/O 上一次计数。
// - **`mole_status_start_watch` / `mole_status_stop_watch`**（Tauri 命令）：
//   前端可通过 invoke 启停后台采集循环。
// - **`start_status_watch` / `stop_status_watch`**（pub(crate) 内部函数）：
//   供 tray.rs 等 Rust 模块直接调用，与 Tauri 命令共享同一套消费者引用计数。
//
// 消费者生命周期：
//   - 支持多个消费者并行（Status 页面 + Dashboard 弹窗）。
//   - 首个消费者接入时启动后台线程；最后一个消费者退出时才停止。
//   - CONSUMER_COUNT 原子计数保证安全，无需额外的互斥锁。
//   - WATCH_RUNNING 使用 compare_exchange 原子操作，防止多个线程同时运行。
//
// 防抖机制（避免频繁开关托盘时的线程重建开销）：
//   - stop_status_watch 不立即停止线程，而是延迟 2 秒
//   - 如果在 2 秒内调用 start_status_watch，则取消停止操作
//   - 使用 CANCEL_PENDING_STOP 原子标志协调两个函数
//   - 这样用户频繁开关托盘时，采集线程保持运行，避免重复启动/停止的开销

use std::panic::AssertUnwindSafe;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Runtime};

use crate::cmd::status::metrics::{Collector, instant_snapshot_lockfree};
use crate::cmd::status::process_watch::ProcessWatchOptions;

/// 与前端 `EVT_STATUS_SNAPSHOT` 字符串保持一致。
const EVT_STATUS_SNAPSHOT: &str = "status::snapshot";

/// 快速/进程采集间隔：2s（降低 CPU/电池开销，影响所有 Fast 路径指标）。
const FAST_REFRESH_INTERVAL: f64 = 2.0;

/// 对齐 Go: slowRefreshInterval = 30s
const SLOW_REFRESH_INTERVAL: f64 = 30.0;

/// 停止防抖延迟：2s（避免用户频繁开关托盘时的线程重建开销）。
const STOP_DEBOUNCE_SECS: u64 = 2;

fn collector() -> &'static Mutex<Collector> {
    static INSTANCE: OnceLock<Mutex<Collector>> = OnceLock::new();
    INSTANCE.get_or_init(|| Mutex::new(Collector::new(ProcessWatchOptions::default())))
}

/// watch 后台循环线程内部控制开关。
static WATCH_RUNNING: AtomicBool = AtomicBool::new(false);

/// watch 线程世代号：stop 时自增。防抖线程置 WATCH_RUNNING=false 后，
/// 若新 start 立即重新置位并 spawn 新线程，旧线程通过世代号比对退出，
/// 避免旧线程看到被新 start 重新置位的 WATCH_RUNNING=true 而永久续命（僵尸双线程）。
static WATCH_GEN: AtomicU64 = AtomicU64::new(0);

/// 活跃消费者数量（Status 页面 + Dashboard 弹窗等）。
static CONSUMER_COUNT: AtomicUsize = AtomicUsize::new(0);

/// 是否有待取消的停止操作（防抖机制）。
static CANCEL_PENDING_STOP: AtomicBool = AtomicBool::new(false);

fn consumer_enter() -> bool {
    let prev = CONSUMER_COUNT.fetch_add(1, Ordering::SeqCst);
    if prev == usize::MAX {
        // 防御：计数曾因重复 stop 下溢回绕（修复前历史版本可能遗留），
        // 恢复为 1 并视为首个消费者，保证 watch 线程能重新启动。
        CONSUMER_COUNT.store(1, Ordering::SeqCst);
        log::error!("[status] CONSUMER_COUNT overflowed, reset to 1");
        return true;
    }
    prev == 0
}

fn consumer_leave() -> bool {
    // compare_exchange 防下溢：计数已为 0 时忽略 stop（幂等），
    // 重复 stop 不再使计数回绕到 usize::MAX 导致 watch 永久无法停止。
    let mut cur = CONSUMER_COUNT.load(Ordering::SeqCst);
    loop {
        if cur == 0 {
            return false;
        }
        match CONSUMER_COUNT.compare_exchange_weak(
            cur,
            cur - 1,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => return cur == 1,
            Err(actual) => cur = actual,
        }
    }
}

// ── 三层采集模式 ──────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum CollectionMode {
    Fast,
    Process,
    Full,
}

struct WatchState {
    ready: bool,
    last_full_at: Option<Instant>,
    last_process_at: Option<Instant>,
}

impl WatchState {
    fn new() -> Self {
        Self {
            ready: false,
            last_full_at: None,
            last_process_at: None,
        }
    }

    /// 对齐 Go nextCollectionMode()
    fn next_mode(&self, now: Instant) -> CollectionMode {
        if !self.ready {
            return CollectionMode::Fast;
        }
        match self.last_full_at {
            None => CollectionMode::Full,
            Some(t) if now.duration_since(t).as_secs_f64() >= SLOW_REFRESH_INTERVAL => {
                CollectionMode::Full
            }
            _ => match self.last_process_at {
                None => CollectionMode::Process,
                Some(t) if now.duration_since(t).as_secs_f64() >= FAST_REFRESH_INTERVAL => {
                    CollectionMode::Process
                }
                _ => CollectionMode::Fast,
            },
        }
    }

    /// 对齐 Go recordCollectionFreshness()
    fn record(&mut self, mode: CollectionMode, now: Instant) {
        if mode == CollectionMode::Full {
            self.last_full_at = Some(now);
        }
        if mode == CollectionMode::Process || mode == CollectionMode::Full {
            self.last_process_at = Some(now);
        }
    }
}

// ──────────────────────────────────────────────
//  pub(crate) 内部 API：供 tray.rs 等 Rust 模块调用
// ──────────────────────────────────────────────

pub(crate) fn start_status_watch<R: Runtime>(app: AppHandle<R>) {
    // 先增加消费者计数
    let should_spawn = consumer_enter();
    
    // 防抖机制：仅当本调用是首个消费者接入时才设置取消标志，阻止待执行的停止操作。
    // 非首个消费者（should_spawn=false）时不得设置：计数下溢等异常状态下，
    // 盲目设置会把合法的待停止操作取消，导致 WATCH_RUNNING 永远为 true。
    if should_spawn && !CANCEL_PENDING_STOP.swap(true, Ordering::SeqCst) {
        log::info!("[start_status_watch] set cancel flag to prevent pending stop (debounce)");
    }
    
    // 如果已经有线程在运行，不需要重新启动
    if !should_spawn {
        return;
    }
    
    // 使用 compare_exchange 原子地检查并设置 WATCH_RUNNING
    // 如果已经有线程在运行，跳过启动
    if WATCH_RUNNING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        log::warn!("[start_status_watch] thread already running, skipping spawn");
        return;
    }
    
    log::info!("[start_status_watch] spawning watch thread ({}s cadence, 3-tier)", FAST_REFRESH_INTERVAL);

    // 捕获本线程世代号（compare_exchange 成功后读取，保证晚于 stop 线程的世代自增）
    let my_gen = WATCH_GEN.load(Ordering::SeqCst);

    std::thread::spawn(move || {
        let mut state = WatchState::new();

        // ── 首帧分两段：F0 轻帧立即出图，全量帧随后补齐 ──
        // F0（collect_instant）全原生零子进程，毫秒级：气泡的 StatusBar（温度/风扇/磁盘）
        // 与网络卡片只依赖 F0 字段，故点开即有数据；内存占用列表（top_processes）
        // 由紧随其后的全量帧填充。F0 不写 enrichment（enrichment 必须由全量帧建立）。
        {
            let f0_started = Instant::now();
            let f0 = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let mut c = match collector().lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                c.collect_instant()
            }));

            match f0 {
                Ok(snap) => {
                    log::debug!(
                        "[status:f0] watch first frame in {:.1}ms",
                        f0_started.elapsed().as_secs_f64() * 1000.0
                    );
                    emit_snapshot(&app, &snap);
                }
                Err(_) => {
                    log::error!("[status:f0] instant frame panicked, waiting for full frame");
                }
            }
        }

        // 首个 tick 立即全量采集，保证后续快速 tick 有 enrichment 可注入。
        // catch_unwind + into_inner() 双重防护：采集 panic 不会杀死 watch 线程，
        // 即使 Mutex 被毒化也可恢复并继续循环（而非永久刷 ERROR 日志）。
        {
            let f1_started = Instant::now();
            let first = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let mut c = match collector().lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        log::warn!("[watch] recovering poisoned lock (first frame)");
                        poisoned.into_inner()
                    }
                };
                c.collect_first()
            }));

            match first {
                Ok(snap) => {
                    state.record(CollectionMode::Full, Instant::now());
                    state.ready = true;
                    log::debug!(
                        "[status:f1] watch full frame in {:.1}ms",
                        f1_started.elapsed().as_secs_f64() * 1000.0
                    );
                    emit_snapshot(&app, &snap);
                }
                Err(_) => {
                    log::error!("[watch] first frame collection panicked, will retry in loop");
                }
            }
        }

        while WATCH_RUNNING.load(Ordering::Relaxed) && WATCH_GEN.load(Ordering::Relaxed) == my_gen {
            std::thread::sleep(std::time::Duration::from_secs(FAST_REFRESH_INTERVAL as u64));

            if !WATCH_RUNNING.load(Ordering::Relaxed) || WATCH_GEN.load(Ordering::Relaxed) != my_gen {
                break;
            }

            let now = Instant::now();
            let mode = state.next_mode(now);

            // catch_unwind 包裹采集调用：
            // 1. panic 不会传播到 thread 边界（线程不退出）
            // 2. MutexGuard 在 unwind 中被 drop → 锁可能被毒化
            // 3. 下一 tick 若 lock() 返回 PoisonError，用 into_inner() 恢复
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let mut c = match collector().lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        log::warn!("[watch] recovering poisoned lock, proceeding with caution");
                        poisoned.into_inner()
                    }
                };
                match mode {
                    CollectionMode::Full => c.collect(),
                    CollectionMode::Process => c.collect_processes(),
                    CollectionMode::Fast => c.collect_fast(),
                }
            }));

            match result {
                Ok(snap) => {
                    state.record(mode, now);
                    emit_snapshot(&app, &snap);
                }
                Err(_) => {
                    log::error!(
                        "[watch] collection panicked (mode={:?}), will retry next tick",
                        mode
                    );
                }
            }
        }

        log::info!("[watch] watch thread exiting (gen={})", my_gen);
    });
}

fn emit_snapshot<R: Runtime>(
    app: &AppHandle<R>,
    snap: &crate::cmd::status::metrics::MetricsSnapshot,
) {
    match serde_json::to_value(snap) {
        Ok(v) => {
            let _ = app.emit(EVT_STATUS_SNAPSHOT, v);
        }
        Err(e) => log::error!("[watch] serialization failed: {}", e),
    }
}

pub(crate) fn stop_status_watch() {
    if consumer_leave() {
        // 防抖机制：延迟 2 秒再实际停止，避免频繁开关时的线程重建开销
        // 流程：
        // 1. 设置 CANCEL_PENDING_STOP = false（允许停止）
        // 2. 启动定时器线程，sleep 2s
        // 3. 2s 后检查 CANCEL_PENDING_STOP：
        //    - 如果为 true（被 start 取消）：不执行停止
        //    - 如果为 false：执行停止
        CANCEL_PENDING_STOP.store(false, Ordering::SeqCst);
        
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(STOP_DEBOUNCE_SECS));
            
            // 如果在等待期间被取消，则不执行停止
            if CANCEL_PENDING_STOP.load(Ordering::SeqCst) {
                log::info!("[stop_status_watch] stop cancelled during debounce");
                return;
            }
            
            // 世代号自增：即使新 start 在旧线程退出前重新置位 WATCH_RUNNING，
            // 旧线程也会因世代号不匹配而退出，避免僵尸双线程重复采集。
            WATCH_GEN.fetch_add(1, Ordering::SeqCst);
            WATCH_RUNNING.store(false, Ordering::Relaxed);
            log::info!("[stop_status_watch] last consumer left, stopping watch thread (after {}s debounce)", STOP_DEBOUNCE_SECS);
        });
    }
}

// ──────────────────────────────────────────────
//  Tauri 命令：供前端 invoke 调用
// ──────────────────────────────────────────────

#[tauri::command(rename_all = "snake_case")]
pub fn mole_status_start_watch(app: AppHandle) -> Result<(), String> {
    start_status_watch(app);
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_status_stop_watch() -> Result<(), String> {
    stop_status_watch();
    Ok(())
}

/// once 快照缓存 TTL：命中直接返回 F0 轻帧，避免每次进入 Home/Analyze 都重新采集。
/// 数据最多滞后 5s，可接受；watch 运行期间事件流会顺风车覆盖，滞后无感。
const ONCE_CACHE_TTL: Duration = Duration::from_secs(5);

/// 最近一次 F0 轻帧缓存（stale-while-revalidate）。
/// 只缓存 F0：F1 全量帧体积大（含全量进程列表）且经事件下发，
/// 写入缓存会让 once 的返回值重新变重。
static LAST_SNAPSHOT: Mutex<Option<(Instant, serde_json::Value)>> = Mutex::new(None);

/// F1 补帧单飞标志：防 Home 反复进出 / 多窗口叠加派发导致多次全量采集。
static F1_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// 单次状态采集：F0 无锁轻帧同步返回 + F1 全量帧后台补发。
/// 不启动 watch 线程、不增减消费者计数、不抢 Collector 锁。
///
/// 静默设计：应用启动后默认不持续采集。Home/Analyze 页面挂载时通过本命令
/// 取一次快照展示磁盘（F0，全原生零子进程，毫秒级）；完整字段（top_processes/
/// hardware/多卷磁盘列表）由 F1 后台采集后经 EVT_STATUS_SNAPSHOT 事件补发，
/// 页面顺风车监听。持续刷新仅由托盘气泡的 watch 事件流驱动。
///
/// F0 与 F1 均在 spawn_blocking 中执行并共享同一个 Collector，锁竞争安全；
/// 两者都是一次性任务，不会形成周期采集（静默语义不变）。
///
/// 日志约定（供静默验证）：F0 采集成功打印 `[mole_status_once] collected F0 instant
/// snapshot in ..ms`，命中缓存打印 `[mole_status_once] cache hit (age=..s)`。
/// 启动后应只看到一条 F0 日志 + 一条 `[status:f1]` debug 日志，此后无持续采集输出。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_status_once(app: AppHandle) -> Result<serde_json::Value, String> {
    // stale-while-revalidate：TTL 内直接返回缓存（也不再派发 F1）
    {
        let guard = LAST_SNAPSHOT.lock().map_err(|e| e.to_string())?;
        if let Some((at, v)) = guard.as_ref() {
            let age = at.elapsed();
            if age < ONCE_CACHE_TTL {
                log::info!("[mole_status_once] cache hit (age={:.1}s)", age.as_secs_f64());
                return Ok(v.clone());
            }
        }
    }

    let started = Instant::now();
    // 无锁轻帧：不抢 Collector 互斥锁。全量采集（F1/watch Full）实测可达 10s+ 且全程持锁，
    // 若此处排队等锁，Home 在缓存过期后重新挂载就又会等几秒。
    // 磁盘口径不变：仍为 NSURLVolumeAvailableCapacityForImportantUsageKey（SSOT）。
    let snap = tauri::async_runtime::spawn_blocking(instant_snapshot_lockfree)
        .await
        .map_err(|e| format!("单次状态采集失败: {e}"))?;

    let value = serde_json::to_value(&snap).map_err(|e| format!("快照序列化失败: {e}"))?;
    log::info!(
        "[mole_status_once] collected F0 instant snapshot in {:.1}ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    *LAST_SNAPSHOT.lock().map_err(|e| e.to_string())? = Some((Instant::now(), value.clone()));

    // F1 补帧：不阻塞本命令返回，完成后经事件下发
    spawn_f1(app);

    Ok(value)
}

/// 派发 F1 补帧：后台跑一次全量采集（processes / hardware / disk_io / proxy /
/// 完整磁盘列表），完成后经 `EVT_STATUS_SNAPSHOT` 下发并建立 enrichment。
///
/// 单飞：已有 F1 在跑时直接返回（compare_exchange），避免反复进出页面叠加派发。
/// 与 watch 线程共享 Collector 互斥锁，两者串行执行，无死锁风险。
fn spawn_f1<R: Runtime>(app: AppHandle<R>) {
    if F1_IN_FLIGHT
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        log::debug!("[status:f1] already in flight, skip dispatch");
        return;
    }

    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();

        // catch_unwind + into_inner()：与 watch 线程同款防护，panic 不会遗留
        // F1_IN_FLIGHT=true（否则后续补帧永久被拒）。
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let mut c = match collector().lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            c.collect()
        }));

        match result {
            Ok(snap) => {
                log::debug!(
                    "[status:f1] full frame ready in {:.1}ms",
                    started.elapsed().as_secs_f64() * 1000.0
                );
                emit_snapshot(&app, &snap);
            }
            Err(_) => log::error!("[status:f1] full frame collection panicked"),
        }

        F1_IN_FLIGHT.store(false, Ordering::SeqCst);
    });
}

// ─────────────────────────────────────────────
//  内存释放（对标腾讯柠檬 QMPurgeRAM.m 语义）
// ─────────────────────────────────────────────

/// 柠檬频率控制：两次释放间隔 < 10s 直接返回，防止高频空耗。
const PURGE_MIN_INTERVAL_SECS: f64 = 10.0;

static LAST_PURGE_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// vm_stat 页数采样：严格对齐柠檬 GetPhysMemoryInfo/GetPhysMemoryInfo2——
/// 32 位版 vm_statistics（全 natural_t/u32 字段）+ host_statistics(HOST_VM_INFO=2)。
///
/// 历史崩溃修复记录：旧实现手搓 VmStatistics64（全 u64 字段），与内核真实
/// 布局不符——vm_statistics64 的页计数字段实为 natural_t(u32)，仅计数类
/// 字段是 u64。内核按真实布局回填后，Rust 侧把相邻两个 u32 拼读成一个
/// u64（free_count 实际 = free | active<<32），active 页数百万级时乘
/// page_size 直接越过 u64::MAX，debug 构建 panic: attempt to multiply
/// with overflow。改用柠檬同款 32 位结构后布局零歧义。
#[cfg(target_os = "macos")]
#[repr(C)]
struct VmStatistics {
    free_count: u32,
    active_count: u32,
    inactive_count: u32,
    wire_count: u32,
    zero_fill_count: u64,
    reactivations: u64,
    pageins: u64,
    pageouts: u64,
    faults: u64,
    cow_faults: u64,
    lookups: u64,
    hits: u64,
    purgeable_count: u32,
    purges: u32,
    speculative_count: u32,
}

#[cfg(target_os = "macos")]
extern "C" {
    fn mach_host_self() -> u32;
    fn host_page_size(host: u32, out: *mut usize) -> i32;
    fn host_statistics(host: u32, flavor: u32, info: *mut VmStatistics, count: *mut u32) -> i32;
}

/// 采样页数四元组 (free, inactive, purgeable, page_size)。
/// count 按内核口径传 sizeof/sizeof(integer_t)（柠檬：sizeof(vm_stat)/sizeof(natural_t)）。
/// 采样失败返回 None（各策略自行兜底），不 panic。
#[cfg(target_os = "macos")]
fn vm_page_stats() -> Option<(u64, u64, u64, u64)> {
    const HOST_VM_INFO: u32 = 2;
    const KERN_SUCCESS: i32 = 0;
    unsafe {
        let host = mach_host_self();
        let mut page_size: usize = 4096;
        if host_page_size(host, &mut page_size) != KERN_SUCCESS {
            return None;
        }
        let mut stat = std::mem::zeroed::<VmStatistics>();
        let mut count = (std::mem::size_of::<VmStatistics>() / 4) as u32;
        if host_statistics(host, HOST_VM_INFO, &mut stat, &mut count) != KERN_SUCCESS {
            return None;
        }
        Some((
            stat.free_count as u64,
            stat.inactive_count as u64,
            stat.purgeable_count as u64,
            page_size as u64,
        ))
    }
}

/// vm_stat free 页数采样（对齐柠檬 GetPhysMemoryInfo 的 mem_info[0] = free_count * pagesize）。
/// 与 reclaimable_bytes 的「可回收估算」不同口径：purge 前后差值只数纯 free 页，
/// 这是柠檬 QMPurgeRAM purge 的量化反馈口径（不含 inactive/purgeable）。
/// 边界安全：u32 页数上限 2^32，×16K 页上限 2^46，乘法恒不溢出。
#[cfg(target_os = "macos")]
fn free_page_bytes() -> Option<u64> {
    let (free, _, _, ps) = vm_page_stats()?;
    Some(free.saturating_mul(ps))
}

// ── 柠檬 QMPurgeRAM 同款：用户态免 root 释放 ──
//
// 关键洞察：系统 `purge` 命令需要 root；柠檬的首选策略是用户态内存压力法
// ——分配 ≈ 可回收量（free+inactive+purgeable）的匿名内存并逐页写入，
// 逼内核回收其他进程的 inactive/purgeable 页，随后立即释放。

/// 可回收内存估算（对齐柠檬 GetPhysMemoryInfo2 的 before[0]+[1]+[4]：
/// free + inactive + purgeable，即 mmap/malloc 策略的 alloc_size）。
/// 边界安全：三项 u32 页数之和上限 3×2^32，×16K 页上限约 2×10^14，恒不溢出；
/// 另用 saturating 双保险（防未来内核扩展字段语义变化）。
#[cfg(target_os = "macos")]
fn reclaimable_bytes() -> Option<u64> {
    let (free, inactive, purgeable, ps) = vm_page_stats()?;
    Some(
        free.saturating_add(inactive)
            .saturating_add(purgeable)
            .saturating_mul(ps),
    )
}

/// 逐页写入首字节，强制分配物理页（mmap/malloc 两策略共用）。
unsafe fn touch_pages(mem: *mut u8, size: usize) {
    let page_size = libc::sysconf(libc::_SC_PAGESIZE).max(4096) as usize;
    let bytes = std::slice::from_raw_parts_mut(mem, size);
    let mut offset = 0;
    while offset < size {
        bytes[offset] = 0xAA;
        offset += page_size;
    }
}

/// 柠檬第一层 purgeRamByHugeMMAPFree：mmap 巨块逐页写入逼出可回收页，
/// munmap 后追加 malloc_zone_pressure_relief(0)（柠檬原版收尾动作）。
/// 返回 true = 已执行（含「无可回收内存」的提前成功，对齐柠檬 return 0），
/// 后续策略不再执行；false = 实际失败，继续下一层。
#[cfg(target_os = "macos")]
fn purge_by_mmap() -> bool {
    let Some(size) = reclaimable_bytes() else {
        return false;
    };
    if size == 0 {
        // 柠檬：无可回收内存时直接视为成功，不再走后续策略
        return true;
    }
    if size > usize::MAX as u64 {
        return false;
    }
    let size = size as usize;
    unsafe {
        let mem = libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_ANON | libc::MAP_PRIVATE,
            -1,
            0,
        );
        if mem == libc::MAP_FAILED {
            return false;
        }
        touch_pages(mem as *mut u8, size);
        libc::munmap(mem, size);
        // 柠檬原版收尾：向默认 zone 施压回收
        malloc_zone_pressure_relief(malloc_default_zone(), 0);
    }
    true
}

/// 柠檬第二层 purgeRamByHugeMallocFree：malloc 巨块稀疏写入后 free（堆压力路径，与 mmap 互补）。
#[cfg(target_os = "macos")]
fn purge_by_malloc() -> bool {
    let Some(size) = reclaimable_bytes() else {
        return false;
    };
    if size == 0 {
        return true;
    }
    if size > isize::MAX as u64 {
        return false;
    }
    unsafe {
        let mem = libc::malloc(size as usize);
        if mem.is_null() {
            return false;
        }
        touch_pages(mem as *mut u8, size as usize);
        libc::free(mem);
        malloc_zone_pressure_relief(malloc_default_zone(), 0);
    }
    true
}

// ── 柠檬第三层 purgeRamByZoneAlloc：malloc zone + 多核并行 calloc ──

#[cfg(target_os = "macos")]
extern "C" {
    fn malloc_default_zone() -> *mut std::ffi::c_void;
    fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize);
    fn malloc_create_zone(start_size: usize, flags: u32) -> *mut std::ffi::c_void;
    fn malloc_zone_calloc(
        zone: *mut std::ffi::c_void,
        num_items: usize,
        size_per_item: usize,
    ) -> *mut std::ffi::c_void;
    fn malloc_destroy_zone(zone: *mut std::ffi::c_void);
}

/// zone 句柄包装：libmalloc zone 的分配接口本身线程安全（柠檬即多核并行
/// 同 zone calloc），允许跨 scoped 线程传递。
#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
struct ZoneHandle(*mut std::ffi::c_void);

#[cfg(target_os = "macos")]
unsafe impl Send for ZoneHandle {}

/// 第三层并行 calloc 辅助：以整个 ZoneHandle 为参数传入，避免 scoped 闭包
/// 因字段分离捕获（disjoint capture）直接捕获裸指针字段而破坏 Send。
#[cfg(target_os = "macos")]
fn zone_calloc_within(zone: ZoneHandle, num: usize, size: usize) -> *mut std::ffi::c_void {
    unsafe { malloc_zone_calloc(zone.0, num, size) }
}

/// 柠檬 purgeRamByZoneAlloc 的阶梯目标量（free/inactive 比例决定吃多少 inactive）。
#[cfg(target_os = "macos")]
fn zone_reclaim_target(free: u64, inactive: u64) -> u64 {
    if inactive <= free {
        free
    } else if inactive <= free * 2 {
        free + inactive / 4
    } else if inactive < free * 4 {
        free + inactive / 2
    } else {
        free + inactive
    }
}

/// 柠檬第三层 purgeRamByZoneAlloc：创建巨 zone，按 CPU 核数并行 calloc + 逐 4KB
/// 块触碰首字节，最后 destroy_zone 整体归还。柠檬自注该策略效率偏低且可能
/// 短暂抬升占用，故严格保持其链位：仅 mmap/malloc 均失败后兑底。
#[cfg(target_os = "macos")]
fn purge_by_zone() -> bool {
    let Some((free, inactive)) = reclaimable_split() else {
        return false;
    };
    let total = zone_reclaim_target(free, inactive);
    if total == 0 {
        return true;
    }
    let proc_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    const UNIT_SIZE: usize = 0x1000;
    let alloc_count = ((total / proc_count as u64) / UNIT_SIZE as u64) as usize;

    unsafe {
        let zone = malloc_create_zone(total as usize, 0);
        if zone.is_null() {
            return false;
        }
        std::thread::scope(|s| {
            let zone = ZoneHandle(zone);
            for _ in 0..proc_count {
                s.spawn(move || {
                    let p = zone_calloc_within(zone, alloc_count, UNIT_SIZE);
                    if p.is_null() {
                        return;
                    }
                    // 逐 unit 触碰首字节（对齐柠檬的 count*unitSize 步进写入）
                    let bytes = p as *mut u8;
                    let mut i = 0usize;
                    while i < alloc_count {
                        bytes.add(i * UNIT_SIZE).write_volatile(i as u8);
                        i += 1;
                    }
                });
            }
        });
        malloc_destroy_zone(zone);
    }
    true
}

/// free / inactive 分开采样（第三层阶梯目标量需要两个分量）。
#[cfg(target_os = "macos")]
fn reclaimable_split() -> Option<(u64, u64)> {
    let (free, inactive, _, ps) = vm_page_stats()?;
    Some((free.saturating_mul(ps), inactive.saturating_mul(ps)))
}

/// 柠檬 purgeByLocal 同款：三层策略链 + 15s 整体超时
/// （dispatch_group_wait 15s；超时即弃守本地策略走系统兑底）。
#[cfg(target_os = "macos")]
fn purge_by_local() -> bool {
    let (tx, rx) = std::sync::mpsc::channel::<bool>();
    std::thread::spawn(move || {
        let ok = purge_by_mmap() || purge_by_malloc() || purge_by_zone();
        let _ = tx.send(ok);
    });
    matches!(
        rx.recv_timeout(std::time::Duration::from_secs(15)),
        Ok(true)
    )
}

/// 执行柠檬策略：purgeByLocal（mmap → malloc → zone，15s 超时）→ purgeBySystem 兑底。
/// 本地策略成功则不调系统命令（对齐柠檬 if(!purgeByLocal) purgeBySystem 语义）。
/// 柠檬的 purgeBySystem 走 root 特权 helper（XPC MCCMD_PURGE_MEMORY）；本项目无
/// helper 进程，等价保留为 spawn `purge`（非 root 静默失败，不影响本地策略成果）。
fn run_purge_strategies() {
    #[cfg(target_os = "macos")]
    {
        if purge_by_local() {
            return;
        }
    }
    let _ = std::process::Command::new("purge").status();
}

/// 释放内存：严格对齐柠檬 QMPurgeRAM purge 全流程——
/// ① 10s 频控（冷却期内 sleep(1) 后返回 0）；② purgeByLocal 三层策略链，
/// 失败才走 purgeBySystem 兑底；③ sleep(1) 等内核刷新；④ 释放量 = 前后
/// vm_stat free 页差值（GetPhysMemoryInfo mem_info[0] 口径）。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_purge_memory() -> Result<serde_json::Value, String> {
    // 频控判定先行（柠檬：interval 先于执行更新，失败/无效果也占用冷却窗口）
    {
        let mut guard = LAST_PURGE_AT.lock().map_err(|e| e.to_string())?;
        if let Some(t) = *guard {
            let elapsed = t.elapsed().as_secs_f64();
            if elapsed < PURGE_MIN_INTERVAL_SECS {
                let retry = (PURGE_MIN_INTERVAL_SECS - elapsed).ceil() as u64;
                drop(guard);
                // 柠檬节流时 sleep(1) 再返回 0，防连点空耗
                std::thread::sleep(std::time::Duration::from_secs(1));
                return Ok(serde_json::json!({
                    "freed_bytes": 0u64,
                    "throttled": true,
                    "retry_after_secs": retry,
                }));
            }
        }
        *guard = Some(Instant::now());
    }

    let (before, after) = tauri::async_runtime::spawn_blocking(|| {
        let before = free_page_bytes();
        run_purge_strategies();
        // 柠檬同款：sleep(1) 等待系统刷新内存状态后再采样
        std::thread::sleep(std::time::Duration::from_secs(1));
        (before, free_page_bytes())
    })
    .await
    .map_err(|e| e.to_string())?;

    let freed = match (before, after) {
        (Some(b), Some(a)) => a.saturating_sub(b),
        _ => 0,
    };
    log::info!(
        "[mole_purge_memory] freed={} bytes (before={:?}, after={:?})",
        freed,
        before,
        after
    );
    Ok(serde_json::json!({
        "freed_bytes": freed,
        "throttled": false,
        "before_available": before.unwrap_or(0),
        "after_available": after.unwrap_or(0),
    }))
}

// ── 运行时探针：vm_stat 采样布局与边界安全 ──
//
// 旧 VmStatistics64 布局错位时，free_count 会读成 free|active<<32（10^15 量级）；
// 布局正确后各分量必落在物理内存页数之内。`cargo test purge_probe` 验证。
#[cfg(all(test, target_os = "macos"))]
mod purge_probe {
    use super::*;

    fn physical_bytes() -> u64 {
        let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
        let ps = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        (pages.max(0) as u64).saturating_mul(ps.max(0) as u64)
    }

    #[test]
    fn vm_page_stats_within_physical_bounds() {
        let phys = physical_bytes();
        let (free, inactive, purgeable, ps) =
            vm_page_stats().expect("vm_stat 采样失败（host_statistics 返回非 0）");
        assert!(ps == 4096 || ps == 16384, "异常页尺寸: {}", ps);
        let phys_pages = phys / ps;
        assert!(free <= phys_pages, "free 页数越界: {free} > {phys_pages}");
        assert!(inactive <= phys_pages, "inactive 页数越界: {inactive}");
        assert!(purgeable <= phys_pages, "purgeable 页数越界: {purgeable}");

        let free_bytes = free_page_bytes().unwrap();
        assert!(free_bytes <= phys, "free 字节越界: {free_bytes}");

        // purgeable 与 active/inactive 存在重叠，用 2 倍物理内存做宽松上界
        let reclaim = reclaimable_bytes().unwrap();
        assert!(reclaim <= phys * 2, "可回收估算越界: {reclaim}");

        let (f, i) = reclaimable_split().unwrap();
        assert!(f <= phys && i <= phys, "split 越界: {f} / {i}");
    }
}
