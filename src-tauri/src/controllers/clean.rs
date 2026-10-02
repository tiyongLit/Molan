// 对标 Mole bin/clean.sh — 薄壳：调 lib/clean/* → 返回 JSON
// dry_run 通过 MOLE_DRY_RUN 环境变量控制（lib/clean/ 各函数内部读取）

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::clean::model::MoleCleanPathItem;
use crate::clean::orchestrator::run_clean_core;
use crate::clean::scan_job::{
    CleanGuard, MOLE_CLEAN_IN_PROGRESS, request_clean_scan_cancel, start_clean_scan_job,
    wait_for_job,
};
use crate::clean::scan_registry::take_scan_snapshot;
use crate::clean::job_state;
use crate::core::sudo;

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
pub struct MoleCleanPathsArgs {
    pub items: Vec<MoleCleanPathItem>,
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_clean_paths(args: MoleCleanPathsArgs) -> Result<Value, String> {
    crate::clean::ensure_execution_allowed()?;
    crate::clean::apply::clean_paths_items(&args.items)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mole_clean_execute(category_ids: Vec<String>) -> Result<Value, String> {
    crate::clean::ensure_execution_allowed()?;
    let output = tauri::async_runtime::spawn_blocking(move || {
        crate::clean::apply::execute_categories_legacy(category_ids)
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
    crate::clean::scan_job::status()
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
    crate::clean::scan_job::is_legacy_busy()
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

    let permanent_delete = args.permanent_delete;
    let output = tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        crate::clean::apply::run_apply(
            app,
            args.item_ids,
            snapshot,
            grouped,
            category_ids,
            skipped_items,
            permanent_delete,
        )
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
