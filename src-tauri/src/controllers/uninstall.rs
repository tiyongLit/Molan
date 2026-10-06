use serde::Serialize;
use serde_json::Value;
use tauri::Manager;

use crate::uninstall::app_ops::{run_dry_run, run_execute};
use crate::uninstall::app_scan::{AppListEntry, list_apps_blocking};

// CLI scan_applications 对齐翻译
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_list_apps() -> Result<Vec<AppListEntry>, String> {
    // 扫描含目录遍历 + Spotlight 查询 + plist 解析（重 I/O）：spawn_blocking 隔离，
    // 避免占用 tokio worker 线程导致其他 IPC 排队。
    tauri::async_runtime::spawn_blocking(list_apps_blocking)
        .await
        .map_err(|e| format!("应用列表扫描任务失败: {e}"))?
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mole_uninstall(
    app: tauri::AppHandle,
    app_path: String,
    dry_run: bool,
    data_only: Option<bool>,
) -> Result<Value, String> {
    let data_only = data_only.unwrap_or(false);
    if dry_run {
        run_dry_run(&app_path, data_only)
    } else {
        run_execute(&app, &app_path, data_only)
    }
}

#[derive(Serialize, Clone)]
pub struct BatchUninstallOutcome {
    pub app_name: String,
    pub app_path: String,
    pub success: bool,
    pub freed_bytes: u64,
    pub freed_human: String,
    pub reason: String,
    pub suggestion: String,
}

/// 批量卸载多个应用。
/// 后端调用 `batch_uninstall_applications`，返回每个应用的卸载结果。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_uninstall_batch(
    app: tauri::AppHandle,
    app_paths: Vec<String>,
    data_only: Option<bool>,
) -> Result<Value, String> {
    let data_only = data_only.unwrap_or(false);
    // Clear Data 模式：保留 app 本体，只清残留
    if data_only {
        unsafe { std::env::set_var("MOLE_UNINSTALL_DATA_ONLY", "1") };
    }
    // 卸载是耗时操作（多轮 du/trash/进程检查）：spawn_blocking 避免阻塞主线程
    // （Tauri 同步命令跑在主线程，几十个 app 的批量卸载会冻结整个事件循环）。
    let app_clone = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _busy = crate::core::busy_state::enter_busy();
        let _awake = crate::core::keep_awake::KeepAwakeGuard::acquire("Uninstalling apps");
        crate::uninstall::batch::batch_uninstall_applications(&app_paths, Some(&app_clone))
    })
    .await
    .map_err(|e| format!("卸载任务失败: {e}"))?;
    if data_only {
        unsafe { std::env::remove_var("MOLE_UNINSTALL_DATA_ONLY") };
    }

    let total_cleaned = result.total_size_freed_kb.saturating_mul(1024);
    let outcomes: Vec<Value> = result
        .outcomes
        .iter()
        .map(|o| {
            serde_json::json!({
                "app_name": o.app_name,
                "app_path": o.app_path,
                "success": o.success,
                "freed_bytes": o.freed_kb.saturating_mul(1024),
                "freed_human": crate::core::base::bytes_to_human(o.freed_kb.saturating_mul(1024)),
                "reason": o.reason,
                "suggestion": o.suggestion
            })
        })
        .collect();

    Ok(serde_json::json!({
        "mode": "batch",
        "success_count": result.success_count,
        "failed_count": result.failed_count,
        "total_cleaned_size": total_cleaned,
        "total_cleaned_size_human": crate::core::base::bytes_to_human(total_cleaned),
        "outcomes": outcomes,
        "running_apps": result.running_apps,
        "running_at_uninstall_apps": result.running_at_uninstall_apps,
        "sudo_apps": result.sudo_apps,
        "brew_cask_apps": result.brew_cask_apps,
        "blocked_apps": result.blocked_apps,
        "manual_removal_apps": result.manual_removal_apps,
        "background_item_leftovers": result.background_item_leftovers,
        "local_network_warning_apps": result.local_network_warning_apps,
        "system_extension_warning_apps": result.system_extension_warning_apps,
        "status_title": result.title,
        "status": result.status
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_open_uninstall_window(app: tauri::AppHandle) -> Result<(), String> {
    log::info!("[mole_open_uninstall_window] called");
    if let Some(window) = app.get_webview_window("uninstall") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    use tauri::WebviewUrl;
    use tauri::WebviewWindowBuilder;
    use tauri::webview::PageLoadEvent;

    let window = WebviewWindowBuilder::new(&app, "uninstall", WebviewUrl::App("/uninstall".into()))
        .title("应用卸载")
        .inner_size(1056.0, 640.0)
        .resizable(true)
        .visible(false)
        // Reveal 门控：等前端渲染完成后再显示窗口，避免白屏闪烁
        .on_page_load(|webview, payload| {
            if let PageLoadEvent::Finished = payload.event() {
                let _ = webview.show();
                let _ = webview.set_focus();
                log::info!("[uninstall] window revealed on page load");
            }
        })
        .build()
        .map_err(|e| e.to_string())?;

    let w = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = w.hide();
        }
    });

    // 3s fallback：防止 page load 事件延迟时窗口一直不可见
    let fallback_win = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(3));
        if !fallback_win.is_visible().unwrap_or(false) {
            let _ = fallback_win.show();
            let _ = fallback_win.set_focus();
            log::info!("[uninstall] window revealed by 3s fallback");
        }
    });

    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_get_uninstall_history() -> Result<Value, String> {
    let records = crate::uninstall::history::get_history();

    Ok(serde_json::json!({
        "records": records
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_clear_uninstall_history() -> Result<(), String> {
    crate::uninstall::history::clear_history()
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_reveal_in_trash() -> Result<(), String> {
    let home = crate::core::base::home_dir_opt().ok_or("无法获取用户目录")?;
    let trash_path = home.join(".Trash");

    // 使用 open 命令打开废纸篓
    std::process::Command::new("open")
        .arg(&trash_path)
        .output()
        .map_err(|e| format!("打开废纸篓失败: {}", e))?;

    Ok(())
}

// ============================================================================
// 孤儿残留扫描（对齐 PureMac AppState.findOrphans + ReversePathsFetch）
// ============================================================================

use crate::uninstall::orphan_safety::{
    OrphanEntry, is_safe_orphan_candidate, scan_orphans, scan_orphans_for,
};

/// 孤儿残留扫描：反向扫描已安装 app 列表之外的残留文件。
///
/// 用户在用 Molan 之前通过拖到废纸篓等方式卸载的 app，
/// 其残留数据靠正向扫描（find_app_files）抓不到。本命令实现反向扫描：
/// 遍历一组固定路径，过滤掉属于已安装 app 的条目，剩下的就是孤儿候选。
///
/// 安全策略（对齐 PureMac OrphanSafetyPolicy）：
/// - 白名单 root：只有 Caches/Logs/HTTPStorages 等易失数据目录下的孤儿可删除
/// - 黑名单 fragment：Preferences/Containers 等持久状态目录只展示不删
/// - 高风险 dotpath：~/.ssh、~/.claude 等一律拦截
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_orphan_scan(_app: tauri::AppHandle) -> Result<Vec<OrphanEntry>, String> {
    // 1. 复用 mole_list_apps 拿到已安装 app 列表
    let apps = mole_list_apps().await?;
    let installed: Vec<(String, String)> = apps
        .iter()
        .map(|a| (a.bundle_id.clone(), a.name.clone()))
        .collect();

    let home = crate::core::base::home_dir();

    // 2. spawn_blocking 执行扫描（重 I/O）
    let orphans = tauri::async_runtime::spawn_blocking(move || scan_orphans(&installed, &home))
        .await
        .map_err(|e| format!("孤儿扫描任务失败: {}", e))?;

    Ok(orphans)
}

/// 孤儿删除请求项：path + 扫描时测得体积（字节）。
///
/// 体积来自扫描结果（与列表展示同口径），随请求回传用于：
/// - 删除日志 / 操作记录的体积字段（替代删除前 `du -skP` 全树遍历）
/// - `total_freed_bytes` 汇总（原实现用 `metadata().len()` 取目录 inode 大小，严重偏小）
#[derive(serde::Deserialize)]
pub struct OrphanDeleteItem {
    pub path: String,
    pub size_bytes: u64,
}

/// 孤儿残留删除：走 trash crate + 用户确认。
///
/// 对每条路径再次校验 `is_safe_orphan_candidate`，
/// 然后调 `file_ops::mole_delete_with_size`（内部已有 validate_path_for_deletion + TOCTOU 防护）。
///
/// 删除口径对齐扫描口径：启用 `MOLE_UNINSTALL_MODE`，让 `should_protect_path`
/// 放宽 DATA_PROTECTED_BUNDLES / 宽口径名单（com.jetbrains.、com.macpaw. 等
/// 已卸载 app 的易失数据），SYSTEM_CRITICAL 仍拦截；与 uninstall::batch 同款写法。
///
/// 逐条 trash 是阻塞 IO：整体搬进 spawn_blocking，避免占用 tokio worker。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_orphan_delete(
    _app: tauri::AppHandle,
    items: Vec<OrphanDeleteItem>,
) -> Result<Value, String> {
    if items.is_empty() {
        return Ok(serde_json::json!({
            "success_count": 0,
            "failed_count": 0,
            "total_freed_bytes": 0,
            "deleted_paths": []
        }));
    }

    let (success_count, failed_count, total_freed, deleted_paths) =
        tauri::async_runtime::spawn_blocking(move || {
            // 孤儿删除 = 用户显式确认后的真实删除：清掉可能泄漏的 dry-run 标记，
            // 避免 mole_delete 提前返回 MOLE_OK 造成"假成功"。
            std::env::remove_var("MOLE_DRY_RUN");

            // 启用 uninstall 模式（设置/恢复沿用 uninstall::batch 的既有模式）
            let prev_mode = std::env::var("MOLE_UNINSTALL_MODE").ok();
            std::env::set_var("MOLE_UNINSTALL_MODE", "1");

            let home = crate::core::base::home_dir();
            let mut success_count = 0usize;
            let mut failed_count = 0usize;
            let mut total_freed: u64 = 0;
            // 逐条成功结果：前端据此精确移除列表项（不能按"前 N 个成功"推断）
            let mut deleted_paths: Vec<String> = Vec::new();

            for item in &items {
                let path = item.path.as_str();
                // 二次安全校验（防前端传入非法路径）
                if !is_safe_orphan_candidate(path, &home) {
                    log::warn!("[orphan_delete] rejected unsafe path: {}", path);
                    failed_count += 1;
                    continue;
                }

                // 复用扫描已测体积（KB 字符串透传给删除日志），跳过 du 全树
                let size_kb = (item.size_bytes / 1024).to_string();

                // 走统一删除管线（trash crate + validate_path_for_deletion）
                let exit_code =
                    crate::core::file_ops::mole_delete_with_size(path, false, None, Some(&size_kb));
                if exit_code == crate::core::file_ops::MOLE_OK {
                    success_count += 1;
                    total_freed += item.size_bytes;
                    deleted_paths.push(item.path.clone());
                    log::info!("[orphan_delete] trashed: {}", path);
                } else {
                    failed_count += 1;
                    log::warn!("[orphan_delete] failed (exit={}): {}", exit_code, path);
                }
            }

            // 恢复原值（可能被外层流程设置为其他值）
            match prev_mode {
                Some(v) => std::env::set_var("MOLE_UNINSTALL_MODE", v),
                None => std::env::remove_var("MOLE_UNINSTALL_MODE"),
            }

            (success_count, failed_count, total_freed, deleted_paths)
        })
        .await
        .map_err(|e| format!("孤儿删除任务失败: {}", e))?;

    Ok(serde_json::json!({
        "success_count": success_count,
        "failed_count": failed_count,
        "total_freed_bytes": total_freed,
        "deleted_paths": deleted_paths
    }))
}

/// 定向孤儿扫描：只返回指定 app 的残留（通知点击 → 卸载页定向清理链路）。
///
/// 与 `mole_orphan_scan` 共用同一安全策略与候选管线；命中口径按 bundleId 优先、
/// appName（≥3 字符）兜底，详见 `orphan_safety::scan_orphans_for`。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_orphan_scan_for(
    _app: tauri::AppHandle,
    bundle_id: Option<String>,
    app_name: String,
) -> Result<Vec<OrphanEntry>, String> {
    let home = crate::core::base::home_dir();

    let orphans = tauri::async_runtime::spawn_blocking(move || {
        scan_orphans_for(bundle_id.as_deref(), &app_name, &home)
    })
    .await
    .map_err(|e| format!("定向残留扫描任务失败: {}", e))?;

    Ok(orphans)
}

/// 消费"卸载残留"通知点击写入的 pending 快照。
///
/// 前端（layout）在收到 `uninstall::residual-open` 事件或冷启动时调用；
/// take 语义——消费即清空，避免重复跳转（对齐 trash_watch 快照事实源原则）。
#[tauri::command(rename_all = "snake_case")]
pub fn mole_residual_take_pending() -> Option<crate::runtime::residual_watch::ResidualTarget> {
    crate::runtime::residual_watch::take_pending()
}
