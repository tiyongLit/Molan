//! Clean 执行引擎 — 路径清理、旧/新执行入口与分类派发。
//! 自 `controllers/clean.rs` 纯搬迁（行为不变）；命令壳负责校验与 spawn_blocking。

use std::sync::atomic::Ordering;

use serde_json::Value;

use crate::clean::model::MoleCleanPathItem;
use crate::clean::orchestrator::{boot_volume_free_bytes, bytes_to_human};
use crate::clean::scan_job::{CleanGuard, MOLE_CLEAN_IN_PROGRESS};
use crate::clean::scan_registry::ScanSnapshot;
use crate::clean::{app_caches, apps, caches, dev, launch_services, system, user};
use crate::core::debug_trace;
use crate::core::log::{log_operation_session_end, log_operation_session_start};
use crate::core::sudo;
use crate::events::{CleanApplyProgressPayload, emit_clean_apply_progress};
use crate::manage::whitelist;

/// 按显式路径清单清理（自 mole_clean_paths 命令体逐字搬迁，去掉准入闸门调用）。
pub fn clean_paths_items(items: &[MoleCleanPathItem]) -> Result<Value, String> {
    log::info!("[mole_clean_paths] {} items", items.len());
    let mut results = Vec::new();
    let mut total_cleaned: u64 = 0;
    let mut success_count: u64 = 0;
    let mut failed_count: u64 = 0;

    for item in items {
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

/// 旧命令 mole_clean_execute 的执行主体（自闭包体逐字搬迁，去掉准入与 spawn 包装）。
pub fn execute_categories_legacy(category_ids: Vec<String>) -> Result<Value, String> {
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
}

/// clean_apply 执行主体（自闭包体逐字搬迁，含 busy 守卫、进度事件与调试埋点）。
pub fn run_apply(
    app: tauri::AppHandle,
    item_ids: Vec<String>,
    snapshot: ScanSnapshot,
    grouped: Vec<(String, Vec<String>)>,
    category_ids: Vec<String>,
    skipped_items: Vec<String>,
    permanent_delete: bool,
) -> Result<Value, String> {
    // 调试埋点：预构建勾选快照数据（全量 item 的 selected 状态），供闭包内写盘
    let dbg_selected_count: u64 = item_ids.len() as u64;
    let dbg_selected_size_bytes: u64 = item_ids
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
            item_ids.iter().map(|s| s.as_str()).collect();
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

    let app_for_task = app.clone();
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
