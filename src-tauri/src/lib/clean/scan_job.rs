//! Clean 任务状态机接线 — busy 标志/守卫、任务受理/取消/worker、状态查询。
//! 自 `controllers/clean.rs` 纯搬迁（行为不变）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::clean::job_state;
use crate::clean::model::CleanStatusInfo;
use crate::clean::orchestrator::run_clean_core;
use crate::clean::scan_registry::LAST_SCAN_AT;
use crate::core::sudo;

pub static MOLE_CLEAN_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

pub struct CleanGuard;

impl Drop for CleanGuard {
    fn drop(&mut self) {
        MOLE_CLEAN_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}

/// 兼容旧命令的 busy 查询（trash_watch 与旧前端契约使用）。
pub fn is_legacy_busy() -> bool {
    MOLE_CLEAN_IN_PROGRESS.load(Ordering::SeqCst)
}

/// 查询清理页进入时的状态（自 clean_status 命令体逐字搬迁）。
pub fn status() -> Result<Value, String> {
    let sudo_active = sudo::is_admin_authorized();
    let last_iso = LAST_SCAN_AT
        .lock()
        .ok()
        .and_then(|g| *g)
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, 0))
        .map(|dt| dt.to_rfc3339());
    // 闸门状态为单一事实来源：execution_allowed 直接由其派生。
    let execution_allowed = crate::clean::ensure_execution_allowed().is_ok();
    serde_json::to_value(CleanStatusInfo {
        execution_allowed,
        execution_blocked_reason: if execution_allowed {
            None
        } else {
            Some(crate::clean::EXECUTION_BLOCKED)
        },
        sudo_session_active: sudo_active,
        last_scan_at: last_iso,
    })
    .map_err(|e| e.to_string())
}

/// 受理扫描任务：幂等（活动任务直接返回当前快照），受理成功后提交 worker。
pub fn start_clean_scan_job(
    app: &tauri::AppHandle,
    metric: &str,
) -> Result<job_state::CleanJobSnapshot, String> {
    // 与旧 apply/execute 路径互斥（它们仍占用全局 busy 标志；任务系统统一后移除该检查）
    if !job_state::is_active() && MOLE_CLEAN_IN_PROGRESS.load(Ordering::SeqCst) {
        return Err("A cleanup is already in progress. Please wait.".into());
    }
    match job_state::begin_scan_job(metric) {
        Some(snap) => {
            job_state::emit(app, &snap);
            let app_worker = app.clone();
            let job_id = snap.job_id.clone().unwrap_or_default();
            let metric_worker = metric.to_string();
            tauri::async_runtime::spawn_blocking(move || {
                run_scan_job_worker(app_worker, job_id, metric_worker)
            });
            Ok(snap)
        }
        // 已有活动任务：幂等挂接，返回现有快照（不重复启动）
        None => Ok(job_state::snapshot()),
    }
}

/// 受理取消请求：置 cancelling + 落全局取消标志（section guard 据此让扫描提前收尾）。
pub fn request_clean_scan_cancel(
    app: &tauri::AppHandle,
    job_id: Option<&str>,
) -> job_state::CleanJobSnapshot {
    let (snap, applied) = job_state::request_cancel(job_id);
    if applied {
        crate::core::base::set_clean_cancelled();
        job_state::emit(app, &snap);
    }
    snap
}

/// 扫描任务 worker（阻塞线程）：
/// 授权（阻塞式系统面板，不再占用主线程）→ 扫描内核 → 结果入槽 → 租约收尾置 idle。
fn run_scan_job_worker(app: tauri::AppHandle, job_id: String, metric: String) {
    // 租约 Drop 必达收尾：任何退出路径（含 panic）都会把状态放回 idle 并广播
    let _lease = job_state::JobLease::new(app.clone(), job_id.clone());

    // 取消标志在「准入后、授权前」复位；此后到达的取消请求会保留（不再被覆盖）
    crate::core::base::reset_clean_cancelled();
    let t_worker = std::time::Instant::now();
    log::info!("[clean-job] worker started job={job_id} metric={metric}");

    // 1) 管理员授权：三态结果记录到快照（前端可据 auth 展示提示，但不阻断受限扫描）
    // 时序埋点（卡顿分析）：应用进程内未见授权（冷启动首次扫描）时会弹系统认证面板，
    // 面板存在期间整个 app 不可交互——单独计时，区分「等用户输入」与「扫描耗时」。
    log::info!("[clean-job] auth begin job={job_id}");
    let t_auth = std::time::Instant::now();
    let auth = sudo::ensure_admin_session_detailed();
    let auth_tag = match auth {
        sudo::AdminAuthResult::Authorized => "authorized",
        sudo::AdminAuthResult::UserCanceled => "canceled",
        sudo::AdminAuthResult::Failed => "failed",
    };
    log::info!(
        "[clean-job] auth end job={job_id} result={auth_tag} took={:.0}ms",
        t_auth.elapsed().as_secs_f64() * 1000.0
    );
    // 授权期间被取消 → 不进入扫描，直接收尾（lease Drop 置 idle 并广播）
    if job_state::is_cancelling(&job_id) {
        log::info!(
            "[clean-job] job {job_id} cancelled during authorization, skip scan total={:.1}s",
            t_worker.elapsed().as_secs_f64()
        );
        return;
    }
    let snap = job_state::mark_scanning(&job_id, auth_tag);
    job_state::emit(&app, &snap);

    // 2) 与旧 apply/execute 路径互斥（沿用全局 busy 标志；busy 由 CleanGuard 在退出时释放）
    if MOLE_CLEAN_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        log::warn!("[clean-job] busy flag held by another cleanup, job {job_id} aborted");
        return;
    }
    let _busy = CleanGuard;

    // 3) dry-run 扫描内核（与旧 mole_clean(dry_run=true) 完全同一实现）
    std::env::set_var("MOLE_DRY_RUN", "1");
    let output = run_clean_core(app.clone(), true);
    let mut value = match serde_json::to_value(&output) {
        Ok(v) => v,
        Err(e) => {
            log::error!("[clean-job] serialize scan output failed: {e}");
            return;
        }
    };
    if let Some(obj) = value.as_object_mut() {
        obj.insert("size_metric".into(), Value::String(metric));
    }

    // 结果先入槽、再置 idle：前端看到 idle 时结果必然可取；取消/超时任务不刷新成功时间，
    // 避免 clean_status 把一次被取消的扫描当作最新有效数据。
    let cancelled = value
        .get("cancelled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !cancelled {
        if let Ok(mut g) = LAST_SCAN_AT.lock() {
            *g = Some(std::time::SystemTime::now());
        }
    }
    job_state::store_result(&job_id, value);
    log::info!(
        "[clean-job] job {job_id} completed, cancelled={cancelled} total={:.1}s",
        t_worker.elapsed().as_secs_f64()
    );
    // lease Drop：登记 idle + 广播（前端据此取结果并切视图）
}

/// 等待任务结束（兼容旧 `clean_scan` 的同步契约）；轮询在阻塞线程内，不占 async 执行器。
pub async fn wait_for_job(job_id: &str, timeout: Duration) {
    let want = job_id.to_string();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + timeout;
        while job_state::is_job_running(&want) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
    })
    .await;
}
