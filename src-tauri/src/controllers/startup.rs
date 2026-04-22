//! 启动项管理 Tauri 命令：扫描 + 操作。
//! 对齐 Launchdeck 的 discovery/actions 架构，前端消费 Lemon 式 App 分组输出。
//! 所有重 I/O 在 spawn_blocking 中执行。

use tauri::AppHandle;

use crate::core::sudo;
use crate::core::timeout::run_with_timeout_capture_lossy;
use crate::startup::actions::{self, ActionKind};
use crate::startup::app_assoc::AppIndex;
use crate::startup::discovery;
use crate::startup::model::{ActionResult, StartupInventory};

/// 扫描启动项完整清单（App 分组 + 散装服务）。
///
/// - `include_login_items`：是否跑 sfltool dumpbtm 合并 BTM 登录项（需管理员授权）。
/// - `show_system`：是否显示 Apple 系统服务（默认隐藏）。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_startup_scan(
    _app: AppHandle,
    include_login_items: bool,
    show_system: bool,
) -> Result<StartupInventory, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // 构建 App 索引（复用 mole_list_apps 的扫描逻辑）
        let app_index = build_app_index();

        // BTM dump（需要时）
        let btm_dump = if include_login_items {
            Some(read_btm_dump())
        } else {
            None
        };

        Ok(discovery::load_inventory(
            include_login_items,
            show_system,
            &app_index,
            btm_dump.as_deref(),
        ))
    })
    .await
    .map_err(|e| format!("spawn_blocking 失败: {e}"))?
}

/// 对指定服务执行操作（start/stop/restart/enable/disable/delete）。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_startup_action(
    _app: AppHandle,
    service_id: String,
    action: String,
) -> Result<ActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let kind = ActionKind::from_str(&action)
            .ok_or_else(|| format!("未知操作: {action}，支持 start/stop/restart/enable/disable/delete"))?;

        // 重新扫描找到目标服务（保证状态最新）
        let app_index = build_app_index();
        let inventory = discovery::load_inventory(false, true, &app_index, None);

        // 在 app_groups + standalone 中查找
        let service = inventory
            .app_groups
            .iter()
            .flat_map(|g| g.services.iter())
            .chain(inventory.standalone_services.iter())
            .find(|s| s.id == service_id)
            .ok_or_else(|| format!("服务不存在: {service_id}"))?;

        // 规划 + 执行
        let plan = actions::plan(service, kind);
        if plan.is_blocked() {
            return Ok(ActionResult {
                success: false,
                message: plan.blocked_reason.unwrap_or_default(),
            });
        }

        let result = actions::execute(&plan);

        // enable/disable 后 re-read 验证（launchctl 退出码不可靠）
        if matches!(kind, ActionKind::Enable | ActionKind::Disable) && result.success {
            let uid = unsafe { libc::getuid() };
            let disabled_now = actions::verify_disabled_state(uid);
            let is_disabled = disabled_now.contains(&service.label);
            let expected_disabled = kind == ActionKind::Disable;
            if is_disabled != expected_disabled {
                return Ok(ActionResult {
                    success: false,
                    message: format!("操作已执行但状态未生效（期望 disabled={expected_disabled}，实际 disabled={is_disabled}）"),
                });
            }
        }

        Ok(result)
    })
    .await
    .map_err(|e| format!("spawn_blocking 失败: {e}"))?
}

// ── 内部辅助 ──

/// 构建已安装 App 索引（对齐 Lemon QMLocalAppHelper：多目录 + 递归 + mdfind 补充）。
fn build_app_index() -> AppIndex {
    let home = std::env::var("HOME").unwrap_or_default();

    // 1. 直接扫描的目录（对齐 Lemon appScanPathArray）
    let scan_dirs = vec![
        "/Applications".to_string(),
        format!("{home}/Applications"),
        "/Library/Input Methods".to_string(),
        format!("{home}/Library/Input Methods"),
    ];

    let mut entries: Vec<(String, String, String)> = Vec::new();
    let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 直接子目录扫描
    for dir in &scan_dirs {
        scan_apps_in_dir(dir, &mut entries, &mut seen_paths, 0, 3);
    }

    // 2. mdfind 补充：查找系统 LaunchServices 数据库中所有已注册的 .app
    // 对齐 Lemon 的 _LSCopyAllApplicationURLs，但我们用 mdfind 代替
    if let Some(mdfind_apps) = mdfind_apps() {
        for app_path in mdfind_apps {
            if !seen_paths.insert(app_path.clone()) {
                continue;
            }
            let name = std::path::Path::new(&app_path)
                .file_stem()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let info_plist = format!("{app_path}/Contents/Info.plist");
            let bundle_id = plist::Value::from_file(&info_plist)
                .ok()
                .and_then(|v| v.into_dictionary())
                .and_then(|d| {
                    d.get("CFBundleIdentifier")
                        .and_then(|v| v.as_string())
                        .map(String::from)
                })
                .unwrap_or_default();
            if !bundle_id.is_empty() {
                entries.push((bundle_id, name, app_path));
            }
        }
    }

    AppIndex::new(entries)
}

/// 递归扫描目录中的 .app 包，最多 max_depth 层。
fn scan_apps_in_dir(
    dir: &str,
    entries: &mut Vec<(String, String, String)>,
    seen: &mut std::collections::HashSet<String>,
    depth: usize,
    max_depth: usize,
) {
    if depth > max_depth {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("app") {
            // 递归进入子目录（非 .app 的目录）
            if path.is_dir() {
                scan_apps_in_dir(&path.to_string_lossy(), entries, seen, depth + 1, max_depth);
            }
            continue;
        }
        let path_str = path.to_string_lossy().to_string();
        if !seen.insert(path_str.clone()) {
            continue;
        }
        let name = path
            .file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let info_plist = path.join("Contents/Info.plist");
        let bundle_id = plist::Value::from_file(&info_plist)
            .ok()
            .and_then(|v| v.into_dictionary())
            .and_then(|d| {
                d.get("CFBundleIdentifier")
                    .and_then(|v| v.as_string())
                    .map(String::from)
            })
            .unwrap_or_default();
        if !bundle_id.is_empty() {
            entries.push((bundle_id, name, path_str));
        }
    }
}

/// 用 mdfind 查找所有已安装的 .app（对齐 LaunchServices 数据库）。
fn mdfind_apps() -> Option<Vec<String>> {
    let out = crate::core::timeout::run_with_timeout_capture_lossy(
        10.0,
        "/usr/bin/mdfind",
        &["kMDItemContentType == 'com.apple.application-bundle'"],
    )?;
    let apps: Vec<String> = out
        .lines()
        .filter(|l| l.ends_with(".app"))
        .map(|l| l.to_string())
        .collect();
    if apps.is_empty() { None } else { Some(apps) }
}

/// 读取 BTM dump（sfltool dumpbtm）。
/// 已授权 → root 执行（无 sfltool 自弹框）；未授权 → 直接跑（可能弹系统框）。
fn read_btm_dump() -> String {
    if sudo::is_admin_authorized() {
        String::from_utf8_lossy(&sudo::sudo_output(&["/usr/bin/sfltool", "dumpbtm"]).stdout)
            .to_string()
    } else {
        run_with_timeout_capture_lossy(10.0, "/usr/bin/sfltool", &["dumpbtm"]).unwrap_or_default()
    }
}
