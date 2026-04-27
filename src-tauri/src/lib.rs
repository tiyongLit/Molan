#[cfg(all(feature = "mas", feature = "full"))]
compile_error!(
    "Cargo features `mas` 与 `full` 不能同时启用；MAS 构建请使用: --no-default-features --features mas"
);

pub mod cmd;
pub mod constants;
pub mod controllers;
pub mod embedded_rules;
pub mod events;
pub mod tray;
pub mod vendor;
pub mod whitelist_optimize;

#[path = "lib/check/mod.rs"]
pub mod check;
#[path = "lib/clean/mod.rs"]
pub mod clean;
#[path = "lib/core/mod.rs"]
pub mod core;
#[path = "lib/manage/mod.rs"]
pub mod manage;
#[path = "lib/optimize/mod.rs"]
pub mod optimize;
#[path = "lib/platform/mod.rs"]
pub mod platform;
#[path = "lib/startup/mod.rs"]
pub mod startup;
#[path = "lib/uninstall/mod.rs"]
pub mod uninstall;
#[path = "lib/updates/mod.rs"]
pub mod updates;

/// macOS Quick Look 预览指定路径的文件/目录
#[tauri::command]
fn mole_quick_look(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("qlmanage")
            .args(["-p", &path])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("启动快速查看失败: {}", e))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err("快速查看仅支持 macOS".to_string())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Debug)
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .invoke_handler(tauri::generate_handler![
            // Status — 系统监控
            controllers::status::mole_status_start_watch,
            controllers::status::mole_status_stop_watch,
            controllers::status::mole_status_once,
            controllers::status::mole_purge_memory,
            controllers::platform::mole_get_icon_cached,
            controllers::platform::mole_get_icons_batch,
            // Analyze — 磁盘分析
            controllers::analyze::mole_analyze,
            controllers::analyze::mole_analyze_cancel,
            // Clean — 清理（含 purge/installer/whitelist）
            controllers::clean::mole_clean,
            controllers::clean::mole_clean_paths,
            controllers::clean::mole_clean_execute,
            // Clean v2 — 扫描/执行分离 + size_metric + 取消
            controllers::clean::clean_status,
            controllers::clean::clean_scan,
            controllers::clean::clean_scan_cancel,
            controllers::clean::clean_apply,
            controllers::clean::clean_apply_cancel,
            controllers::clean::clean_reveal_in_finder,
            controllers::clean::mole_purge,
            controllers::clean::mole_purge_paths_read,
            controllers::clean::mole_purge_paths_write,
            controllers::clean::mole_installer_scan,
            controllers::clean::mole_installer_trash,
            controllers::clean::mole_whitelist_read,
            controllers::clean::mole_whitelist_write,
            controllers::clean::mole_whitelist_predefined,
            controllers::privilege::mole_privilege_capabilities,
            controllers::privilege::mole_request_admin_session,
            controllers::privilege::mole_revoke_admin_session,
            // Optimize — 优化（含 check/touchid）
            controllers::optimize::mole_optimize,
            controllers::optimize::mole_check,
            controllers::optimize::mole_check_fix,
            controllers::optimize::mole_touchid_status,
            controllers::optimize::mole_touchid_enable,
            // Uninstall — 卸载
            controllers::uninstall::mole_list_apps,
            controllers::uninstall::mole_uninstall,
            controllers::uninstall::mole_uninstall_batch,
            controllers::uninstall::mole_open_uninstall_window,
            controllers::uninstall::mole_get_uninstall_history,
            controllers::uninstall::mole_clear_uninstall_history,
            controllers::uninstall::mole_reveal_in_trash,
            // Orphan — 孤儿残留扫描（对齐 PureMac ReversePathsFetch）
            controllers::uninstall::mole_orphan_scan,
            controllers::uninstall::mole_orphan_delete,
            // Updates — 应用更新（对齐 Burrow Updates 标签页）
            controllers::updates::mole_updates_brew_outdated,
            controllers::updates::mole_updates_check,
            controllers::updates::mole_updates_apply,
            controllers::updates::mole_updates_brew_upgrade,
            // Startup — 启动项（对齐 Burrow StartupView）
            controllers::startup::mole_startup_scan,
            controllers::startup::mole_startup_action,
            // settings
            controllers::settings::set_window_size,
            controllers::settings::get_window_size,
            // analyze window
            controllers::analyze::mole_open_analyze_window,
            controllers::analyze::mole_open_shell_window,
            // analyze trash
            controllers::analyze::mole_analyze_trash,
            controllers::analyze::mole_get_protected_analyze_paths,
            // quick look
            mole_quick_look,
            // system confirm — 自定义原生确认弹窗（替代 tauri-plugin-dialog 的 ask）
            controllers::system_confirm::mole_system_confirm,
            controllers::system_confirm::mole_system_confirm_reply
        ])
        .setup(|app| {
            use tauri::Manager;
            let handle = app.handle().clone();

            crate::tray::create_tray(app.handle())?;

            // 注册登录自启：重启电脑后自动恢复运行，
            // 与"关闭窗口→隐藏托盘"配合，保证 sudo 会话生命周期内持久有效。
            let _ = (|| -> Result<(), Box<dyn std::error::Error>> {
                let current_exe = std::env::current_exe()?;
                let auto = auto_launch::AutoLaunchBuilder::new()
                    .set_app_name("MoleStudio")
                    .set_app_path(&current_exe.to_string_lossy())
                    .build()?;
                if !auto.is_enabled()? {
                    auto.enable()?;
                    log::info!("[setup] login item registered");
                }
                Ok(())
            })();

            if let Some(main_window) = handle.get_webview_window("MoleStudio") {
                // 关闭主窗口时隐藏到托盘，不退出进程。
                // 这样 sudo keepalive 持续有效，用户下次从托盘打开时
                // 不需要重新输入密码。
                let w = main_window.clone();
                main_window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });

                #[cfg(target_os = "macos")]
                {
                    use objc2::msg_send;
                    use objc2::runtime::Bool;
                    use objc2_app_kit::NSWindowButton;

                    if let Ok(ptr) = main_window.ns_window() {
                        let ns_win = ptr as *mut objc2_app_kit::NSWindow;
                        unsafe {
                            let zoom_btn: *mut objc2_app_kit::NSButton = msg_send![
                                ns_win,
                                standardWindowButton: NSWindowButton::ZoomButton
                            ];
                            if !zoom_btn.is_null() {
                                let _: () = msg_send![zoom_btn, setHidden: Bool::YES];
                            }
                        }
                    }
                }
            }

            if let Some(analyze_window) = handle.get_webview_window("analyze") {
                let w = analyze_window.clone();
                analyze_window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle: &tauri::AppHandle, event| {
            // Dock 图标点击恢复：主窗口 hide 到托盘后，用户点 Dock 图标时
            // macOS 会发送 Reopen 事件；若无可见窗口则恢复主窗口。
            if let tauri::RunEvent::Reopen { has_visible_windows, .. } = event {
                if !has_visible_windows {
                    use tauri::Manager;
                    if let Some(window) = app_handle.get_webview_window("MoleStudio") {
                        let _ = window.show();
                        let _ = window.set_focus();
                        log::info!("[reopen] main window restored from dock click");
                    }
                }
            }
        });
}
