// 对标 Mole bin/status.sh + cmd/status/main.go：
//
// - Mole TUI：进程内复用同一个 `Collector`，每 `refreshInterval = 1s` 调一次；
//   使用三级采集节奏：Fast(2s) / Process(2s) / Full(30s)。
// - Mole `--watch`：持续 NDJSON，同样三层节奏。
//
// Molan 端降低 Fast/Process 频率至 2s，减少 CPU/电池开销：
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

use crate::status::metrics::{Collector, instant_snapshot_lockfree};
use crate::status::process_watch::ProcessWatchOptions;
use crate::platform::native_icon_registry;

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
        match CONSUMER_COUNT.compare_exchange_weak(cur, cur - 1, Ordering::SeqCst, Ordering::SeqCst)
        {
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
    if WATCH_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        log::warn!("[start_status_watch] thread already running, skipping spawn");
        return;
    }

    log::info!(
        "[start_status_watch] spawning watch thread ({}s cadence, 3-tier)",
        FAST_REFRESH_INTERVAL
    );

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

            if !WATCH_RUNNING.load(Ordering::Relaxed) || WATCH_GEN.load(Ordering::Relaxed) != my_gen
            {
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
    snap: &crate::status::metrics::MetricsSnapshot,
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
            log::info!(
                "[stop_status_watch] last consumer left, stopping watch thread (after {}s debounce)",
                STOP_DEBOUNCE_SECS
            );
        });
    }
}

// ──────────────────────────────────────────────
//  Tauri 命令：供前端 invoke 调用
// ──────────────────────────────────────────────

#[tauri::command(rename_all = "snake_case")]
pub fn mole_status_start_watch(app: AppHandle) -> Result<(), String> {
    // 进程级单例：确保 metrics_process::top_processes 在 rayon scoped thread 里
    // 调 app_icon_svg 时能拿到 AppHandle（idempotent）。
    native_icon_registry::set_app_handle(app.clone());
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
    // 首次进入 Home/Analyze 时也要注册 AppHandle（应用可能从未开过 watch）
    native_icon_registry::set_app_handle(app.clone());
    // stale-while-revalidate：TTL 内直接返回缓存（也不再派发 F1）
    {
        let guard = LAST_SNAPSHOT.lock().map_err(|e| e.to_string())?;
        if let Some((at, v)) = guard.as_ref() {
            let age = at.elapsed();
            if age < ONCE_CACHE_TTL {
                return Ok(v.clone());
            }
        }
    }

    // 无锁轻帧：不抢 Collector 互斥锁。全量采集（F1/Watch Full）实测可达 10s+ 且全程持锁，
    // 若此处排队等锁，Home 在缓存过期后重新挂载就又会等几秒。
    // 磁盘口径不变：仍为 NSURLVolumeAvailableCapacityForImportantUsageKey（SSOT）。
    let snap = tauri::async_runtime::spawn_blocking(instant_snapshot_lockfree)
        .await
        .map_err(|e| format!("单次状态采集失败: {e}"))?;

    let value = serde_json::to_value(&snap).map_err(|e| format!("快照序列化失败: {e}"))?;
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
        return;
    }

    tauri::async_runtime::spawn_blocking(move || {
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
                emit_snapshot(&app, &snap);
            }
            Err(_) => log::error!("[status:f1] full frame collection panicked"),
        }

        F1_IN_FLIGHT.store(false, Ordering::SeqCst);
    });
}

// ─────────────────────────────────────────────
//  进程关闭（对齐腾讯柠檬 killProcessByID，用户态 SIGTERM）
// ─────────────────────────────────────────────

/// 关闭进程：向指定 pid 发送 SIGTERM（对齐柠檬 killProcessByID，
/// 无 root helper 故走用户态信号；内核会校验同 UID 权限，非本用户进程静默失败）。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_kill_process(pid: i32) -> Result<serde_json::Value, String> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        let ret = unsafe { libc::kill(pid, libc::SIGTERM) };
        if ret == 0 {
            log::info!("[mole_kill_process] SIGTERM sent to pid={}", pid);
            Ok(serde_json::json!({ "success": true, "pid": pid }))
        } else {
            let errno = std::io::Error::last_os_error();
            log::warn!("[mole_kill_process] kill({}) failed: {}", pid, errno);
            Err(format!("kill({}) 失败: {}", pid, errno))
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    result
}
