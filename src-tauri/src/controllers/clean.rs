// 对标 Mole bin/clean.sh — 薄壳：调 lib/clean/* → 返回 JSON
// dry_run 通过 MOLE_DRY_RUN 环境变量控制（lib/clean/ 各函数内部读取）

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::clean::model::CleanStatusInfo;
use crate::clean::orchestrator::{boot_volume_free_bytes, bytes_to_human, run_clean_core};
use crate::clean::scan_registry::{take_scan_snapshot, LAST_SCAN_AT};
use crate::clean::{
    app_caches, apps, caches, dev, job_state, launch_services, system, user,
};
use crate::core::debug_trace;
use crate::core::log::{log_operation_session_end, log_operation_session_start};
use crate::core::sudo;
use crate::events::{CleanApplyProgressPayload, emit_clean_apply_progress};
use crate::manage::whitelist;

static MOLE_CLEAN_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

struct CleanGuard;

impl Drop for CleanGuard {
    fn drop(&mut self) {
        MOLE_CLEAN_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mole_clean(app: tauri::AppHandle, dry_run: bool) -> Result<Value, String> {
    if !dry_run {
        crate::clean::ensure_execution_allowed()?;
    }
    let t0 = std::time::Instant::now();
    log::info!("[diagnose] mole_clean ENTRY dry_run={dry_run}");

    if MOLE_CLEAN_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return Err("A scan is already in progress. Please wait for it to complete.".into());
    }
    // 取消标志在「准入后、提交 worker 前」立即复位；worker 内不再 reset，
    // 消除「取消先于 worker 启动、随后被 worker 的 reset 覆盖」的竞态。
    crate::core::base::reset_clean_cancelled();

    if dry_run {
        std::env::set_var("MOLE_DRY_RUN", "1");
    } else {
        std::env::remove_var("MOLE_DRY_RUN");
    }

    let app_clean = app.clone();
    log::info!("[diagnose] mole_clean about to spawn_blocking...");
    let output = tauri::async_runtime::spawn_blocking(move || {
        // guard 移入 worker：由真正执行工作的闭包持有，
        // 确保 MOLE_CLEAN_IN_PROGRESS 在 worker（及其并行子任务）退出后才释放。
        let _guard = CleanGuard;
        run_clean_core(app_clean, dry_run)
    })
    .await
    .map_err(|e| format!("clean task panicked: {}", e))?;

    log::info!(
        "[diagnose] mole_clean spawn_blocking AWAITED, total_elapsed={:.1}s",
        t0.elapsed().as_secs_f64()
    );

    serde_json::to_value(&output).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
pub struct MoleCleanPathItem {
    pub item_id: String,
    pub category_id: String,
    pub path: String,
    pub size: u64,
}

#[derive(Deserialize)]
pub struct MoleCleanPathsArgs {
    pub items: Vec<MoleCleanPathItem>,
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_clean_paths(args: MoleCleanPathsArgs) -> Result<Value, String> {
    crate::clean::ensure_execution_allowed()?;
    log::info!("[mole_clean_paths] {} items", args.items.len());
    let mut results = Vec::new();
    let mut total_cleaned: u64 = 0;
    let mut success_count: u64 = 0;
    let mut failed_count: u64 = 0;

    for item in &args.items {
        let path = std::path::Path::new(&item.path);
        let meta = std::fs::metadata(path).ok();
        let file_size = meta.map(|m| m.len()).unwrap_or(0);

        match trash::delete(path) {
            Ok(()) => {
                total_cleaned += file_size;
                success_count += 1;
                results.push(serde_json::json!({
                    "category_id": item.category_id,
                    "item_id": item.item_id,
                    "path": item.path,
                    "size_cleaned": file_size,
                    "status": "cleaned",
                }));
            }
            Err(e) => {
                failed_count += 1;
                results.push(serde_json::json!({
                    "category_id": item.category_id,
                    "item_id": item.item_id,
                    "path": item.path,
                    "size_cleaned": 0,
                    "status": "failed",
                    "error": e.to_string(),
                }));
            }
        }
    }

    Ok(serde_json::json!({
        "results": results,
        "summary": {
            "total_cleaned_size": total_cleaned,
            "success_count": success_count,
            "failed_count": failed_count,
        }
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mole_clean_execute(category_ids: Vec<String>) -> Result<Value, String> {
    crate::clean::ensure_execution_allowed()?;
    let output = tauri::async_runtime::spawn_blocking(move || {
        let _busy = crate::core::busy_state::enter_busy();
        let _awake = crate::core::keep_awake::KeepAwakeGuard::acquire("Deep cleaning");
        if MOLE_CLEAN_IN_PROGRESS.swap(true, Ordering::SeqCst) {
            return Err("A cleanup is already in progress. Please wait.".into());
        }
        let _guard = CleanGuard;

        log::info!(
            "[mole_clean_execute] executing {} categories: {:?}",
            category_ids.len(),
            category_ids
        );

        let whitelist_result = whitelist::load_whitelist("clean");
        crate::core::set_whitelist(whitelist_result.patterns);

        std::env::remove_var("MOLE_DRY_RUN");

        crate::core::base::reset_clean_cancelled();
        log_operation_session_start("clean");

        let initial_free_bytes = boot_volume_free_bytes();

        let has_sudo = sudo::is_admin_authorized();

        let mut results: Vec<serde_json::Value> = Vec::new();
        let mut total_cleaned: u64 = 0;
        let mut success_count: u64 = 0;
        let mut failed_count: u64 = 0;

        for cat_id in &category_ids {
            // 对齐 Mole `_run_cleanup_step`：每个 category 执行前检查取消状态
            if crate::core::base::is_clean_cancelled() {
                log::info!("[mole_clean_execute] cancelled by user, stopping");
                break;
            }
            let cleaned_kb: u64 = match cat_id.as_str() {
                "system_caches" => {
                    if has_sudo {
                        let r = system::clean_deep_system();
                        system::clean_local_snapshots();
                        r.total_kb()
                    } else {
                        log::warn!("[mole_clean_execute] system_caches skipped: no sudo");
                        results.push(serde_json::json!({
                            "category_id": cat_id,
                            "item_id": "",
                            "path": "System caches",
                            "size_cleaned": 0,
                            "status": "skipped",
                            "error": "Requires admin session"
                        }));
                        failed_count += 1;
                        continue;
                    }
                }
                "user_cache" => {
                    let r = user::clean_user_essentials();
                    let (kb, _) = user::clean_finder_metadata();
                    r.total_kb().saturating_add(kb)
                }
                "app_caches" => user::clean_app_caches().total_kb(),
                "browser_cache" => user::clean_browsers().0,
                "cloud_storage" => user::clean_cloud_storage().0,
                "office_cache" => user::clean_office_applications().0,
                "dev_tools" => dev::clean_developer_tools().total_kb(),
                "applications" => app_caches::clean_user_gui_applications().0,
                "virtualization" => user::clean_virtualization_tools().0,
                "app_support_logs" => user::clean_application_support_logs().0,
                "orphaned_data" => {
                    let (k1, _) = apps::clean_orphaned_app_data();
                    let (k2, _) = apps::clean_orphaned_system_services();
                    let (k3, _) = apps::clean_orphaned_container_stubs();
                    let (k4, _) = launch_services::clean_stale_launch_services_registrations();
                    k1.saturating_add(k2).saturating_add(k3).saturating_add(k4)
                }
                "apple_silicon" => {
                    if has_sudo {
                        user::clean_apple_silicon_caches().0
                    } else {
                        log::warn!("[mole_clean_execute] apple_silicon skipped: no sudo");
                        results.push(serde_json::json!({
                            "category_id": cat_id,
                            "item_id": "",
                            "path": "Apple Silicon caches",
                            "size_cleaned": 0,
                            "status": "skipped",
                            "error": "Requires admin session"
                        }));
                        failed_count += 1;
                        continue;
                    }
                }
                "device_firmware" => user::clean_cached_device_firmware().0,
                "time_machine" => system::clean_time_machine_failed_backups().0,
                _ => {
                    log::info!(
                        "[mole_clean_execute] skipping non-cleanable category: {}",
                        cat_id
                    );
                    continue;
                }
            };

            total_cleaned = total_cleaned.saturating_add(cleaned_kb.saturating_mul(1024));
            success_count += 1;
            results.push(serde_json::json!({
                "category_id": cat_id,
                "item_id": "",
                "path": "",
                "size_cleaned": cleaned_kb.saturating_mul(1024),
                "status": "cleaned",
            }));
        }

        log_operation_session_end("clean", success_count, total_cleaned / 1024);

        let final_free = boot_volume_free_bytes();
        let free_space_change = (final_free as i64) - (initial_free_bytes as i64);
        let free_space_change_human = {
            let abs_delta = free_space_change.unsigned_abs();
            let human = bytes_to_human(abs_delta);
            if free_space_change >= 0 {
                format!("+{human}")
            } else {
                format!("-{human}")
            }
        };

        Ok(serde_json::json!({
            "results": results,
            "summary": {
                "total_cleaned_size": total_cleaned,
                "success_count": success_count,
                "failed_count": failed_count,
                "free_space_change": free_space_change,
                "free_space_change_human": free_space_change_human,
            }
        }))
    })
    .await
    .map_err(|e| format!("clean execute panicked: {e}"))?;

    output
}

// ============================================================
// v2 命令：clean_status / clean_scan / clean_apply / 取消
// ------------------------------------------------------------
// 设计要点（对齐 .trae/rules/04_后端工作流.md）：
// - 扫描与执行分离：`clean_scan` 仅 dry-run 预览，无副作用；`clean_apply` 才真删。
// - 参数显式化：不再用 `MOLE_DRY_RUN` 环境变量对外传模式（lib 内部仍读它，controller
//   据显式语义在内部设置）；新增 `size_metric` 参数贯通前后端。
// - 执行粒度：`clean_apply` 接收 `item_ids`（"categoryId::itemId"），后端按 category
//   去重执行。当前 lib 函数是「整类清理」，多项分类的部分勾选会整类清理——前端对
//   多项分类的部分勾选会提示用户。真 per-item 删除需后续重构 lib clean_* 函数。
// - 旧命令（mole_clean / mole_clean_execute / mole_clean_paths）保留向后兼容。
// ============================================================

/// 查询清理页进入时的状态：管理员会话是否有效、上次扫描时间。
/// 火绒式流程：进入页面先调此命令，再决定是否提示授权 / 数据过期。
#[tauri::command(rename_all = "snake_case")]
pub fn clean_status() -> Result<Value, String> {
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

/// 启动扫描（dry-run 预览，无副作用）+ 等待完成（兼容旧契约）。
/// 扫描执行统一走 `clean_job_start` 的任务 worker（后端唯一事实来源），
/// 本命令仅为旧调用方保留「同步取回整份结果」的语义。
#[tauri::command(rename_all = "snake_case")]
pub async fn clean_scan(
    app: tauri::AppHandle,
    size_metric: Option<String>,
) -> Result<Value, String> {
    let metric = size_metric.unwrap_or_else(|| "logical".into());
    let snap = start_clean_scan_job(&app, &metric)?;
    let Some(job_id) = snap.job_id.clone() else {
        return Err("scan job did not start".into());
    };
    wait_for_job(&job_id, Duration::from_secs(30 * 60)).await;
    job_state::result_for(&job_id).ok_or_else(|| "scan did not produce a result".into())
}

/// 取消正在进行的扫描（兼容旧契约；新前端请用 `clean_job_cancel` 携带 job_id）。
#[tauri::command(rename_all = "snake_case")]
pub fn clean_scan_cancel(app: tauri::AppHandle) -> Result<(), String> {
    request_clean_scan_cancel(&app, None);
    Ok(())
}

// ============================================================
// Clean Job — 任务状态机命令（后端唯一事实来源）
// ------------------------------------------------------------
// 与前端协议：
// 1. `clean_job_start` 受理任务（幂等：活动任务直接挂接，不重复启动）；
// 2. 每次状态转换 emit `clean::job-state` 全量快照（seq 单调，乱序可丢弃）；
// 3. 完成后 `clean_job_result(job_id)` 取回结果（取消的任务同样入槽，含 cancelled=true）；
// 4. 前端挂载/回到前台先 `clean_job_state` 对账，再订阅事件。
// ============================================================

/// 启动一次扫描任务：受理后立即返回快照，授权+扫描在 worker 线程内推进。
#[tauri::command(rename_all = "snake_case")]
pub async fn clean_job_start(
    app: tauri::AppHandle,
    size_metric: Option<String>,
) -> Result<Value, String> {
    let metric = size_metric.unwrap_or_else(|| "logical".into());
    let snap = start_clean_scan_job(&app, &metric)?;
    serde_json::to_value(&snap).map_err(|e| e.to_string())
}

pub fn legacy_clean_busy() -> bool {
    MOLE_CLEAN_IN_PROGRESS.load(Ordering::SeqCst)
}

/// 查询当前任务快照（挂载/可见性对账）。
#[tauri::command(rename_all = "snake_case")]
pub fn clean_job_state() -> Result<Value, String> {
    serde_json::to_value(job_state::snapshot()).map_err(|e| e.to_string())
}

/// 取回任务结果（须在完成后调用；job_id 归属校验）。
#[tauri::command(rename_all = "snake_case")]
pub fn clean_job_result(job_id: String) -> Result<Value, String> {
    job_state::result_for(&job_id).ok_or_else(|| "No result available for this job".into())
}

/// 取消任务：只对 job_id 匹配的活动任务生效（旧任务 id 被忽略，避免误伤新任务）。
#[tauri::command(rename_all = "snake_case")]
pub fn clean_job_cancel(app: tauri::AppHandle, job_id: Option<String>) -> Result<Value, String> {
    let snap = request_clean_scan_cancel(&app, job_id.as_deref());
    serde_json::to_value(&snap).map_err(|e| e.to_string())
}

/// 受理扫描任务：幂等（活动任务直接返回当前快照），受理成功后提交 worker。
fn start_clean_scan_job(
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
fn request_clean_scan_cancel(
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
async fn wait_for_job(job_id: &str, timeout: Duration) {
    let want = job_id.to_string();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + timeout;
        while job_state::is_job_running(&want) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
    })
    .await;
}

#[derive(Deserialize)]
pub struct CleanApplyArgs {
    /// 选中项的 id 数组，格式 `"categoryId::itemId"`。
    /// 后端按 category 去重后整类执行（当前 lib 限制，P4 阶段改造为 item 级执行）。
    pub item_ids: Vec<String>,
    /// 扫描唯一标识，必须与最近一次 clean_scan 返回的 scan_id 一致。
    /// 后端据此从 SCAN_REGISTRY 取快照验证，防重放、防篡改。
    pub scan_id: String,
    /// 删除模式：true=直接永久删除（默认），false=移到废纸篓。
    /// 双模式设计（对齐 Trashly `to_trash` 语义）。
    #[serde(default = "default_permanent_delete")]
    pub permanent_delete: bool,
}

fn default_permanent_delete() -> bool {
    true
}

/// 执行清理。接收前端勾选的 item_ids + scan_id + permanent_delete，验证快照后按 category 去重逐类执行。
/// 执行进度通过 `clean::apply-progress` 事件推送；失败/跳过的分类计入 `failed_count`，
/// 不中断后续分类（对齐「清理失败继续删其他」的产品决策）。
#[tauri::command(rename_all = "snake_case")]
pub async fn clean_apply(app: tauri::AppHandle, args: CleanApplyArgs) -> Result<Value, String> {
    crate::clean::ensure_execution_allowed()?;
    log::info!(
        "[clean_apply] scan_id={}, {} items",
        args.scan_id,
        args.item_ids.len(),
    );

    // ── 验证快照：scan_id 匹配 + 未过期 ──
    let snapshot = take_scan_snapshot(&args.scan_id)?;
    if snapshot.is_expired() {
        return Err("Scan data expired (>30 min). Please rescan before cleaning.".into());
    }

    // ── 验证每个 item_id 都在快照中，且未被白名单拦截 ──
    let mut valid_item_keys: Vec<String> = Vec::new();
    let mut skipped_items: Vec<String> = Vec::new();
    for item_key in &args.item_ids {
        match snapshot.items.get(item_key.as_str()) {
            None => {
                log::warn!("[clean_apply] item not in scan snapshot: {}", item_key);
                return Err(format!(
                    "Item '{}' not found in scan snapshot. Please rescan.",
                    item_key
                ));
            }
            Some(item) => {
                if item.whitelist_matched {
                    log::info!("[clean_apply] item skipped (whitelist): {}", item_key);
                    skipped_items.push(item_key.clone());
                } else if item.status != "cleanable" {
                    log::info!(
                        "[clean_apply] item skipped (not cleanable, status={}): {}",
                        item.status,
                        item_key
                    );
                    skipped_items.push(item_key.clone());
                } else if item.requires_sudo && !sudo::is_admin_authorized() {
                    log::info!("[clean_apply] item skipped (requires sudo): {}", item_key);
                    skipped_items.push(item_key.clone());
                } else {
                    valid_item_keys.push(item_key.clone());
                }
            }
        }
    }

    // ── 将有效 item_keys 按 category 分组（保留 item 维度，用于精确派发） ──
    let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for key in &valid_item_keys {
        let parts: Vec<&str> = key.splitn(2, "::").collect();
        if parts.len() != 2 {
            continue;
        }
        let cat = parts[0].to_string();
        let item_id = parts[1].to_string();
        if seen.insert(cat.clone()) {
            grouped.push((cat, vec![item_id]));
        } else if let Some(last) = grouped.last_mut() {
            last.1.push(item_id);
        }
    }
    let category_ids: Vec<String> = grouped.iter().map(|(c, _)| c.clone()).collect();
    log::info!(
        "[clean_apply] {} valid items → {} categories: {:?}, {} skipped",
        valid_item_keys.len(),
        grouped.len(),
        category_ids,
        skipped_items.len(),
    );

    // 调试埋点：预构建勾选快照数据（全量 item 的 selected 状态），供闭包内写盘
    let dbg_selected_count: u64 = args.item_ids.len() as u64;
    let dbg_selected_size_bytes: u64 = args
        .item_ids
        .iter()
        .filter_map(|k| snapshot.items.get(k))
        .map(|it| it.size)
        .sum();
    let dbg_skipped_count: u64 = skipped_items.len() as u64;
    let dbg_skipped_size_bytes: u64 = skipped_items
        .iter()
        .filter_map(|k| snapshot.items.get(k))
        .map(|it| it.size)
        .sum();
    let dbg_snapshot_total_items: u64 = snapshot.items.len() as u64;
    let dbg_snapshot_total_size_bytes: u64 = snapshot.items.values().map(|it| it.size).sum();
    let dbg_selected_items: Vec<(String, String, u64, bool, String)> = {
        let selected_keys: std::collections::HashSet<&str> =
            args.item_ids.iter().map(|s| s.as_str()).collect();
        snapshot
            .items
            .iter()
            .map(|(key, item)| {
                (
                    key.clone(),
                    item.category_id.clone(),
                    item.size,
                    selected_keys.contains(key.as_str()),
                    item.status.clone(),
                )
            })
            .collect()
    };

    let permanent_delete = args.permanent_delete;
    let app_for_task = app.clone();
    let output = tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        if MOLE_CLEAN_IN_PROGRESS.swap(true, Ordering::SeqCst) {
            return Err("A cleanup is already in progress. Please wait.".into());
        }
        let _guard = CleanGuard;

        let whitelist_result = whitelist::load_whitelist("clean");
        crate::core::set_whitelist(whitelist_result.patterns);
        std::env::remove_var("MOLE_DRY_RUN");
        // 删除模式：permanent_delete=true 直接 rm，false 走废纸篓
        if permanent_delete {
            std::env::set_var("MOLE_PERMANENT_DELETE", "1");
        } else {
            std::env::remove_var("MOLE_PERMANENT_DELETE");
        }
        crate::core::base::reset_clean_cancelled();
        log_operation_session_start("clean");

        let initial_free_bytes = boot_volume_free_bytes();
        let has_sudo = sudo::is_admin_authorized();

        // 调试埋点：clean 阶段（清理前磁盘 → 勾选快照 → 逐分类实时结果 → 汇总 → verify）
        let dbg_clean_id = debug_trace::new_session_id("clean");
        debug_trace::clean_disk_before(
            &dbg_clean_id,
            initial_free_bytes,
            &bytes_to_human(initial_free_bytes.max(0) as u64),
        );
        for (dbg_key, dbg_cat, dbg_size, dbg_selected, dbg_status) in &dbg_selected_items {
            debug_trace::clean_selection_item(
                &dbg_clean_id,
                dbg_key,
                dbg_cat,
                *dbg_size,
                *dbg_selected,
                dbg_status,
            );
        }
        debug_trace::clean_selection_summary(
            &dbg_clean_id,
            dbg_selected_count,
            dbg_selected_size_bytes,
            dbg_skipped_count,
            dbg_skipped_size_bytes,
            dbg_snapshot_total_items,
            dbg_snapshot_total_size_bytes,
        );

        let total_categories = category_ids.len() as u64;
        emit_clean_apply_progress(
            &app_for_task,
            &CleanApplyProgressPayload {
                phase: "start".into(),
                current_category: None,
                current_path: None,
                cleaned_bytes: 0,
                done_categories: 0,
                total_categories,
                failed_count: 0,
            },
        );

        let mut results: Vec<serde_json::Value> = Vec::new();
        let mut total_cleaned: u64 = 0;
        let mut success_count: u64 = 0;
        let mut failed_count: u64 = 0;
        let mut done_categories: u64 = 0;

        for (cat_id, item_ids) in &grouped {
            if crate::core::base::is_clean_cancelled() {
                log::info!("[clean_apply] cancelled by user, stopping");
                break;
            }
            emit_clean_apply_progress(
                &app_for_task,
                &CleanApplyProgressPayload {
                    phase: "category_start".into(),
                    current_category: Some(cat_id.clone()),
                    current_path: Some(cat_id.clone()),
                    cleaned_bytes: total_cleaned,
                    done_categories,
                    total_categories,
                    failed_count,
                },
            );

            log::info!(
                "[clean_apply] executing category '{}', {} items: {:?}",
                cat_id,
                item_ids.len(),
                item_ids,
            );
            let (cleaned_bytes, status, error) = execute_category_items(cat_id, item_ids, has_sudo);
            done_categories += 1;

            if status == "cleaned" {
                total_cleaned = total_cleaned.saturating_add(cleaned_bytes);
                success_count += 1;
            } else {
                failed_count += 1;
            }

            // 调试埋点：逐分类执行结果实时落盘
            debug_trace::clean_category_result(
                &dbg_clean_id,
                cat_id,
                item_ids,
                cleaned_bytes,
                &status,
                error.as_deref(),
            );

            results.push(serde_json::json!({
                "category_id": cat_id,
                "item_id": item_ids.join(","),
                "path": "",
                "size_cleaned": cleaned_bytes,
                "status": status,
                "error": error,
            }));

            emit_clean_apply_progress(
                &app_for_task,
                &CleanApplyProgressPayload {
                    phase: "category_done".into(),
                    current_category: Some(cat_id.clone()),
                    current_path: None,
                    cleaned_bytes: total_cleaned,
                    done_categories,
                    total_categories,
                    failed_count,
                },
            );
        }

        log_operation_session_end("clean", success_count, total_cleaned / 1024);

        let final_free = boot_volume_free_bytes();
        let free_space_change = (final_free as i64) - (initial_free_bytes as i64);
        let free_space_change_human = {
            let abs_delta = free_space_change.unsigned_abs();
            let human = bytes_to_human(abs_delta);
            if free_space_change >= 0 {
                format!("+{human}")
            } else {
                format!("-{human}")
            }
        };

        // 调试埋点：clean 汇总 + verify（报告清理量 vs 磁盘实际变化）
        debug_trace::clean_summary(&dbg_clean_id, success_count, failed_count, total_cleaned);
        debug_trace::verify_disk_after(
            &dbg_clean_id,
            final_free,
            &bytes_to_human(final_free.max(0) as u64),
        );
        debug_trace::verify_verdict(
            &dbg_clean_id,
            total_cleaned,
            free_space_change,
            &free_space_change_human,
            free_space_change - total_cleaned as i64,
        );

        emit_clean_apply_progress(
            &app_for_task,
            &CleanApplyProgressPayload {
                phase: "complete".into(),
                current_category: None,
                current_path: None,
                cleaned_bytes: total_cleaned,
                done_categories,
                total_categories,
                failed_count,
            },
        );

        Ok(serde_json::json!({
            "results": results,
            "summary": {
                "total_cleaned_size": total_cleaned,
                "success_count": success_count,
                "failed_count": failed_count,
                "free_space_change": free_space_change,
                "free_space_change_human": free_space_change_human,
            }
        }))
    })
    .await
    .map_err(|e| format!("clean apply panicked: {e}"))?;

    output
}

/// 取消正在进行的清理。
#[tauri::command(rename_all = "snake_case")]
pub fn clean_apply_cancel() -> Result<(), String> {
    crate::core::base::set_clean_cancelled();
    Ok(())
}

/// 在 Finder 中定位指定路径（对齐 Lemon 的「Show in Finder」）。
/// 仅用于有真实路径（`real_path` 非空）的清理子项，前端据此显示并触发。
#[tauri::command(rename_all = "snake_case")]
pub fn clean_reveal_in_finder(path: String) -> Result<(), String> {
    crate::platform::macos_reveal::reveal_in_finder(&path)
}

/// 按 item 精确执行分类清理（P4 改造：解决"勾 1 清全"问题）。
/// 对 `dev_tools` / `system_caches` / `user_cache` / `app_caches` 4 个多 item 分类，
/// 根据用户实际勾选的 item_ids 逐条派发对应的 lib 子函数；
/// 其余分类 fallback 到原 `execute_category`（它们只有 1 个聚合 item）。
fn execute_category_items(
    cat_id: &str,
    item_ids: &[String],
    has_sudo: bool,
) -> (u64, String, Option<String>) {
    // ── 4 个多 item 分类的 dispatch table ──
    let dispatch: Option<&[(&str, fn() -> (u64, u64))]> = match cat_id {
        "dev_tools" => Some(&[
            ("dev_sqlite", dev::clean_sqlite_temp_files),
            ("dev_npm", dev::clean_dev_npm),
            ("dev_python", dev::clean_dev_python),
            ("dev_go", dev::clean_dev_go),
            ("dev_mise", dev::clean_dev_mise),
            ("dev_rust", dev::clean_dev_rust),
            ("dev_ruby", dev::clean_dev_ruby),
            ("dev_perl", dev::clean_dev_perl),
            ("dev_docker", dev::clean_dev_docker),
            ("dev_cloud", dev::clean_dev_cloud),
            ("dev_nix", dev::clean_dev_nix),
            ("dev_shell", dev::clean_dev_shell),
            ("dev_frontend", dev::clean_dev_frontend),
            ("dev_project_caches", caches::clean_project_caches),
            ("dev_mobile", dev::clean_dev_mobile),
            ("dev_jvm", dev::clean_dev_jvm),
            ("dev_jetbrains_toolbox", dev::clean_dev_jetbrains_toolbox),
            ("dev_jetbrains_logs", dev::clean_dev_jetbrains_logs),
            ("dev_ai_agents", dev::clean_dev_ai_agents_extended),
            ("dev_xctest", dev::clean_xcode_xctest_devices),
            (
                "dev_coresimulator",
                dev::clean_xcode_system_coresimulator_caches,
            ),
            ("dev_codex_runtimes", dev::clean_codex_runtimes),
            ("dev_codex_cli", dev::clean_codex_cli),
            ("dev_antigravity", dev::clean_antigravity_caches),
            ("dev_chrome_mcp", dev::clean_chrome_devtools_mcp_caches),
            ("dev_agent_wt", dev::clean_dev_agent_worktrees),
            ("dev_other_langs", dev::clean_dev_other_langs),
            ("dev_cicd", dev::clean_dev_cicd),
            ("dev_database", dev::clean_dev_database),
            ("dev_api_tools", dev::clean_dev_api_tools),
            ("dev_network", dev::clean_dev_network),
            ("dev_misc", dev::clean_dev_misc),
            ("dev_elixir", dev::clean_dev_elixir),
            ("dev_haskell", dev::clean_dev_haskell),
            ("dev_ocaml", dev::clean_dev_ocaml),
            ("dev_xcode", app_caches::clean_xcode_tools),
            ("dev_code_editors", app_caches::clean_code_editors),
            (
                "dev_homebrew_cache",
                dev::clean_dev_homebrew_cache_with_locks,
            ),
            (
                "dev_homebrew_locks",
                dev::clean_dev_homebrew_cache_with_locks,
            ),
            ("dev_homebrew_cleanup", dev::clean_dev_homebrew_cleanup_item),
            (
                "dev_homebrew_autoremove",
                dev::clean_dev_homebrew_autoremove_item,
            ),
        ]),
        "system_caches" => {
            if !has_sudo {
                return (0, "skipped".into(), Some("Requires admin session".into()));
            }
            Some(&[
                ("system_caches", system::clean_system_caches),
                ("system_temp_files", system::clean_system_temp_files),
                ("system_crash_reports", system::clean_system_crash_reports),
                ("system_logs", system::clean_system_logs),
                (
                    "system_third_party_logs",
                    system::clean_third_party_system_logs,
                ),
                (
                    "system_library_updates",
                    system::clean_system_library_updates,
                ),
                (
                    "system_macos_installers",
                    system::clean_macos_installer_files,
                ),
                (
                    "system_browser_code_sign",
                    system::clean_browser_code_sign_caches,
                ),
                (
                    "system_rebuildable_service",
                    system::clean_rebuildable_system_service_caches,
                ),
                (
                    "system_rebuildable_gpu",
                    system::clean_accessible_rebuildable_gpu_caches,
                ),
                (
                    "system_diagnostic_logs",
                    system::clean_system_diagnostic_logs,
                ),
                ("system_power_logs", system::clean_power_logs),
                (
                    "system_memory_exception",
                    system::clean_memory_exception_reports,
                ),
            ])
        }
        "user_cache" => Some(&[
            ("user_cache", user::clean_user_app_cache),
            ("user_logs", user::clean_user_app_logs),
            ("darwin_runtime", user::clean_darwin_user_runtime_dirs),
            ("trash", user::clean_user_trash),
            ("recent_items", user::clean_recent_items),
            ("finder_metadata", user::clean_finder_metadata),
        ]),
        "app_caches" => Some(&[
            ("app_caches_system", user::clean_app_caches_system),
            ("app_caches_downloads", user::clean_app_caches_downloads),
            ("app_caches_identity", user::clean_app_caches_identity),
            ("app_caches_support", user::clean_app_caches_support),
            ("app_caches_sandbox", user::clean_app_caches_sandbox),
        ]),
        _ => None,
    };

    if let Some(table) = dispatch {
        let mut total_kb: u64 = 0;
        for item_id in item_ids {
            if let Some((_, func)) = table.iter().find(|(id, _)| *id == item_id.as_str()) {
                let (kb, _cnt) = func();
                total_kb = total_kb.saturating_add(kb);
            } else {
                log::info!(
                    "[clean_apply] item '{}' not in {} dispatch table, skipping",
                    item_id,
                    cat_id
                );
            }
        }
        // system_caches 清理完后额外执行 local snapshots 清理（与扫描流程对齐）
        if cat_id == "system_caches" {
            system::clean_local_snapshots();
        }
        (total_kb.saturating_mul(1024), "cleaned".into(), None)
    } else {
        // 其余分类（单 item 或 info 类）：fallback 到原整类执行
        execute_category(cat_id, has_sudo)
    }
}

/// 执行单个分类的清理，返回 `(cleaned_bytes, status, error)`。
/// `status` ∈ `"cleaned" | "skipped"`；`skipped` 表示需 sudo 但未授权或非可清理分类。
fn execute_category(cat_id: &str, has_sudo: bool) -> (u64, String, Option<String>) {
    let cleaned_bytes: u64 = match cat_id {
        "system_caches" => {
            if has_sudo {
                let r = system::clean_deep_system();
                system::clean_local_snapshots();
                r.total_kb().saturating_mul(1024)
            } else {
                return (0, "skipped".into(), Some("Requires admin session".into()));
            }
        }
        "user_cache" => {
            let r = user::clean_user_essentials();
            let (kb, _) = user::clean_finder_metadata();
            r.total_kb().saturating_add(kb).saturating_mul(1024)
        }
        "app_caches" => user::clean_app_caches().total_kb().saturating_mul(1024),
        "browser_cache" => user::clean_browsers().0.saturating_mul(1024),
        "cloud_storage" => user::clean_cloud_storage().0.saturating_mul(1024),
        "office_cache" => user::clean_office_applications().0.saturating_mul(1024),
        "dev_tools" => dev::clean_developer_tools().total_kb().saturating_mul(1024),
        "applications" => app_caches::clean_user_gui_applications()
            .0
            .saturating_mul(1024),
        "virtualization" => user::clean_virtualization_tools().0.saturating_mul(1024),
        "app_support_logs" => user::clean_application_support_logs()
            .0
            .saturating_mul(1024),
        "orphaned_data" => {
            let (k1, _) = apps::clean_orphaned_app_data();
            let (k2, _) = apps::clean_orphaned_system_services();
            let (k3, _) = apps::clean_orphaned_container_stubs();
            let (k4, _) = launch_services::clean_stale_launch_services_registrations();
            k1.saturating_add(k2)
                .saturating_add(k3)
                .saturating_add(k4)
                .saturating_mul(1024)
        }
        "apple_silicon" => {
            if has_sudo {
                user::clean_apple_silicon_caches().0.saturating_mul(1024)
            } else {
                return (0, "skipped".into(), Some("Requires admin session".into()));
            }
        }
        "device_firmware" => user::clean_cached_device_firmware().0.saturating_mul(1024),
        "time_machine" => system::clean_time_machine_failed_backups()
            .0
            .saturating_mul(1024),
        // info / 提示类分类（large_files / system_data_clues / project_artifacts
        // 等）不在此执行，返回 skipped。
        _ => {
            log::info!("[clean_apply] skipping non-cleanable category: {}", cat_id);
            return (0, "skipped".into(), Some("Not a cleanable category".into()));
        }
    };
    (cleaned_bytes, "cleaned".into(), None)
}

// ============================================================
// Purge — 产物清理（Mole clean 的子功能）
// ============================================================

#[derive(Deserialize)]
pub struct MolePurgeArgs {
    pub dry_run: bool,
}

#[derive(Deserialize)]
pub struct MolePurgePathsWriteArgs {
    pub paths: Vec<String>,
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_purge(args: MolePurgeArgs) -> Result<Value, String> {
    log::info!("[mole_purge] called with dry_run={}", args.dry_run);
    Ok(serde_json::json!({
        "mode": if args.dry_run { "dry_run" } else { "execute" },
        "search_paths": [],
        "projects": [],
        "summary": {
            "total_artifact_count": 0,
            "total_artifact_size": 0
        }
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_purge_paths_read() -> Result<Value, String> {
    Ok(serde_json::json!({ "paths": [] }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_purge_paths_write(args: MolePurgePathsWriteArgs) -> Result<(), String> {
    log::info!("[mole_purge_paths_write] {} paths", args.paths.len());
    Ok(())
}

// ============================================================
// Installer — 安装器清理（Mole clean 的子功能）
// ============================================================

#[tauri::command(rename_all = "snake_case")]
pub fn mole_installer_scan() -> Result<Value, String> {
    Ok(serde_json::json!({
        "files": [],
        "total_size": 0,
        "total_files": 0
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_installer_trash(trash_root: String) -> Result<(), String> {
    log::info!("[mole_installer_trash] trash_root={}", trash_root);
    Ok(())
}

// ============================================================
// Whitelist — 白名单管理（Mole clean/optimize 的共用功能）
// ============================================================

#[derive(Deserialize)]
pub struct MoleWhitelistArgs {
    pub mode: String,
    pub patterns: Vec<String>,
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_whitelist_read(args: MoleWhitelistArgs) -> Result<Value, String> {
    let result = crate::manage::whitelist::load_whitelist(&args.mode);
    Ok(serde_json::json!({
        "patterns": result.patterns,
        "warnings": result.warnings,
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_whitelist_write(args: MoleWhitelistArgs) -> Result<(), String> {
    crate::manage::whitelist::save_whitelist_patterns(&args.mode, &args.patterns);
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_whitelist_predefined(args: MoleWhitelistArgs) -> Result<Value, String> {
    use std::path::Path;
    let home = crate::core::base::home_dir_opt().unwrap_or_else(|| Path::new("/").to_path_buf());
    let items = if args.mode == "optimize" {
        crate::whitelist_optimize::load_optimize_whitelist_patterns(&home)
            .into_iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.replace(['/', '.'], "_"),
                    "title": &p,
                    "pattern": &p,
                    "category": "optimize"
                })
            })
            .collect::<Vec<_>>()
    } else {
        let cache_items = crate::manage::whitelist::get_all_cache_items();
        if cache_items.is_empty() {
            let default_patterns = crate::manage::whitelist::default_whitelist_patterns();
            default_patterns
                .into_iter()
                .enumerate()
                .map(|(i, p)| {
                    serde_json::json!({
                        "id": format!("whitelist_{}", i),
                        "title": &p,
                        "pattern": &p,
                        "category": "system_cache"
                    })
                })
                .collect::<Vec<_>>()
        } else {
            cache_items
                .into_iter()
                .map(|(id, title, pattern)| {
                    serde_json::json!({
                        "id": id,
                        "title": title,
                        "pattern": pattern,
                        "category": "system_cache"
                    })
                })
                .collect::<Vec<_>>()
        }
    };
    Ok(serde_json::json!(items))
}

#[cfg(test)]
mod execution_gate_tests {
    use super::*;

    #[test]
    fn app_entries_keep_gate_before_snapshot_and_worker() {
        // 静态顺序回归；不能替代带真实 AppHandle 的 IPC 集成测试。
        // 闸门当前放行（产品决策：扫描完成 → 用户确认 → 执行清理），
        // 但检查点结构必须保留且先于快照/worker 等副作用——
        // 将来若重新关闭执行链路，拦截仍先于一切副作用生效。
        let source = include_str!("clean.rs");
        let apply = source.split("pub async fn clean_apply(").nth(1).unwrap();
        let body = apply.split("-> Result<Value, String> {").nth(1).unwrap();
        assert!(
            body.trim_start()
                .starts_with("crate::clean::ensure_execution_allowed()?;")
        );
        assert!(
            body.find("ensure_execution_allowed").unwrap()
                < body.find("take_scan_snapshot").unwrap()
        );
        let legacy = source.split("pub async fn mole_clean(").nth(1).unwrap();
        let body = legacy.split("-> Result<Value, String> {").nth(1).unwrap();
        assert!(body.trim_start().starts_with("if !dry_run {"));
        assert!(
            body.find("ensure_execution_allowed").unwrap()
                < body.find("MOLE_CLEAN_IN_PROGRESS.swap").unwrap()
        );
        // 当前闸门放行；两种删除偏好都不参与闸门判定。
        assert!(crate::clean::ensure_execution_allowed().is_ok());
        for permanent_delete in [true, false] {
            let args: CleanApplyArgs = serde_json::from_value(serde_json::json!({
                "item_ids": [], "scan_id": "test", "permanent_delete": permanent_delete,
            }))
            .unwrap();
            assert_eq!(args.permanent_delete, permanent_delete);
        }
    }
}
