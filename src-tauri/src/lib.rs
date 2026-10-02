pub mod cmd;
pub mod constants;
pub mod controllers;
pub mod embedded_rules;
pub mod events;
pub mod vendor;
pub mod whitelist_optimize;

pub mod runtime;
pub mod trash_empty;

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
    // 原生通知 delegate 必须最早注册（Builder 构建前），覆盖"应用未运行 →
    // 点击通知冷启动"场景；失败（dev 裸二进制无合法 bundle）不影响启动，
    // 由 residual_watch 降级为事件兜底。
    #[cfg(target_os = "macos")]
    {
        if let Err(err) = crate::platform::macos_notifications::install_early() {
            log::info!("[notifications] native notifications unavailable: {err}");
        }
    }

    tauri::Builder::default()
        // 单实例守卫：必须最先注册（官方约束）。第二实例启动即退出，并在已有实例中
        // 执行回调——按「用户再次打开应用」语义唤起主窗口。覆盖 open -n、直接 exec
        // 裸二进制、macOS 12 Launch Agent 等不经 LaunchServices 的去重旁路。
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            log::info!("[single_instance] duplicate launch blocked; restoring main window");
            crate::runtime::macos_dock_quit::restore_main_window(app);
        }))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(if cfg!(debug_assertions) {
                    log::LevelFilter::Debug
                } else {
                    log::LevelFilter::Warn
                })
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // 应用菜单接管：Cmd+Q / 菜单栏「退出」走应用内退出流程（真退出），
        // 与 Dock 右键退出（隐藏到托盘）区分；实现见 app_menu.rs。
        .menu(|app| crate::runtime::app_menu::build_menu(app))
        .on_menu_event(|app, event| {
            if event.id.as_ref() == crate::runtime::app_menu::MENU_QUIT_ID {
                crate::runtime::app_menu::request_quit(app);
            }
        })
        .invoke_handler(tauri::generate_handler![
            // Status — 系统监控
            controllers::status::mole_status_start_watch,
            controllers::status::mole_status_stop_watch,
            controllers::status::mole_status_once,
            controllers::status::mole_kill_process,
            controllers::platform::mole_native_icons_resolve,
            // Analyze — 磁盘分析
            controllers::analyze::mole_analyze,
            controllers::analyze::mole_analyze_cancel,
            controllers::analyze::mole_analyze_navigate,
            controllers::analyze::mole_analyze_clear_session,
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
            // Clean Job — 任务状态机（后端唯一事实来源）
            controllers::clean::clean_job_start,
            controllers::clean::clean_job_state,
            controllers::clean::clean_job_result,
            controllers::clean::clean_job_cancel,
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
            // 卸载残留定向链路 — 通知点击 → pending 快照消费 → 定向扫描
            controllers::uninstall::mole_residual_take_pending,
            controllers::uninstall::mole_orphan_scan_for,
            // Updates — 应用更新（Updates 标签页后端）
            controllers::updates::mole_updates_brew_outdated,
            controllers::updates::mole_updates_check,
            controllers::updates::mole_updates_apply,
            controllers::updates::mole_updates_brew_upgrade,
            // Self-Update — MoleStudio 自身更新
            controllers::app_version::mole_app_version_check,
            controllers::app_version::mole_app_version_install,
            controllers::app_version::mole_app_version_open_appstore,
            // Startup — 启动项
            controllers::startup::mole_startup_scan,
            controllers::startup::mole_startup_action,
            // settings
            controllers::settings::set_window_size,
            controllers::settings::get_window_size,
            controllers::settings::mole_open_settings_window,
            controllers::settings::mole_auto_launch_status,
            controllers::settings::mole_auto_launch_toggle,
            controllers::settings::mole_trash_empty,
            controllers::settings::mole_trash_reminder_get_state,
            controllers::settings::mole_trash_reminder_action,
            controllers::settings::mole_trash_reminder_update_settings,
            // analyze window
            controllers::analyze::mole_open_analyze_window,
            controllers::analyze::mole_open_shell_window,
            // analyze trash
            controllers::analyze::mole_analyze_trash,
            controllers::analyze::mole_get_protected_analyze_paths,
            // quick look
            mole_quick_look,
            // dialog — 原生 NSAlert 确认/提示（唯一确认通道，垂直三行流；Lemon 形态）
            controllers::dialog::mole_dialog,
            // UI 时序埋点 — 前端日志转发（卡顿分析用，纯信息记录）
            controllers::ui_log::mole_ui_log,
            // Dock 退出拦截 + 忙碌状态查询
            runtime::macos_dock_quit::mole_confirm_dock_quit,
            runtime::macos_dock_quit::mole_is_busy,
            runtime::macos_dock_quit::mole_show_dock_icon,
            // 托盘气泡出场动画隐藏（BottomBar「打开主窗口」路径经此严格配对 stop_status_watch）
            runtime::tray::mole_dashboard_hide,
            // Platform info
            controllers::platform::mole_get_platform_info
        ])
        .setup(|app| {
            use tauri::Manager;
            let handle = app.handle().clone();

            // 原生通知点击回调需要 AppHandle（唤起主窗 + emit 事件）。
            #[cfg(target_os = "macos")]
            crate::platform::macos_notifications::bind_app(handle.clone());

            crate::runtime::tray::create_tray(app.handle())?;

            // 注：登录项不再强制注册。
            // 早期为保 sudo 会话生命周期曾在此处强制 enable；现在由用户在设置页显式
            // 开关（useSettings 挂载时按 store 偏好同步 OS 状态，toggle 走
            // mole_auto_launch_toggle 命令），托盘常驻仍覆盖运行期内 sudo 保活需求。

            // 迁移：清除旧 auto-launch AppleScript 模式注册的 LSSharedFileList 条目。
            // 旧版 build_auto_launch() 未设 use_launch_agent(true)，走 osascript 操作
            // LSSharedFileList；新版 macOS 13+ 走 SMAppService、macOS 12 走 Launch Agent
            // plist，两者均不再写 LSSharedFileList。若不清除，系统设置 → 登录项里
            // 会同时出现旧条目和新条目。此迁移幂等，每次启动安全执行。
            #[cfg(target_os = "macos")]
            {
                if let Ok(exe) = std::env::current_exe() {
                    if let Ok(legacy) = auto_launch::AutoLaunchBuilder::new()
                        .set_app_name("MoleStudio")
                        .set_app_path(&exe.to_string_lossy())
                        .build()
                    {
                        if legacy.is_enabled().unwrap_or(false) {
                            let _ = legacy.disable();
                            log::info!("[setup] migrated: removed legacy AppleScript login item");
                        }
                    }
                }
            }

            if let Some(main_window) = handle.get_webview_window("MoleStudio") {
                // 关闭主窗口时隐藏到托盘，不退出进程（对齐 Lemon Cleaner）。
                // 这样 sudo keepalive 持续有效，用户下次从托盘/Dock 打开时
                // 不需要重新输入密码。真正退出走托盘右键菜单或 Dock→Quit。
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

            // 日志轮转：启动时清理过期日志 + 后台每 24 小时兜底清理
            // （macOS 用户通常不关机，app 可能数周不重启）
            crate::vendor::log_cleanup::cleanup_old_logs(&handle);
            crate::vendor::log_cleanup::start_periodic_cleanup(&handle);

            // Dock 退出拦截：有长任务在跑时拦截 Dock 右键退出
            #[cfg(target_os = "macos")]
            crate::runtime::macos_dock_quit::install(&handle);

            // 卸载残留自动检测：后台轮询废纸篓，检测新 .app 并通知前端
            crate::runtime::residual_watch::start_residual_watch(handle.clone());

            // 废纸篓超阈值提醒：后台低频轮询 ~/.Trash 体积，超阈值时通知前端弹右上角浮窗
            crate::runtime::trash_watch::start_trash_watch(handle.clone());

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
            if matches!(event, tauri::RunEvent::Exit) {
                crate::runtime::trash_watch::service(app_handle).stop();
            }
            // Dock 图标点击 / 已运行时再次双击 .app：统一唤起主窗口（含 accessory
            // 驻留态的 Dock 图标还原），与 single-instance 回调共用 restore_main_window。
            if let tauri::RunEvent::Reopen { .. } = event {
                log::info!("[reopen] restoring main window");
                runtime::macos_dock_quit::restore_main_window(app_handle);
            }

            // ── 退出终极闸门 ───────────────────────────────────
            // 拦截所有退出路径（Dock 右键 / app.exit() 等），
            // 只有显式确认退出的入口（托盘菜单 / BottomBar / Cmd+Q 菜单项）
            // 经 confirm_tray_exit() 置位后才放行。
            // 这是 ObjC 注入失败时的第二道防线。
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !runtime::macos_dock_quit::is_tray_exit_confirmed() {
                    log::warn!("[exit_gate] exit blocked — no confirmed exit request");
                    api.prevent_exit();
                } else {
                    log::info!("[exit_gate] exit allowed (confirmed by user)");
                }
            }
        });
}
