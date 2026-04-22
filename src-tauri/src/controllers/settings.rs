use tauri::Window;

// ── 窗口尺寸 ──

#[tauri::command(rename_all = "snake_case")]
pub async fn set_window_size(window: Window, width: f64, height: f64) -> Result<(), String> {
    window
        .set_size(tauri::Size::Logical(tauri::LogicalSize { width, height }))
        .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct WindowSize {
    width: f64,
    height: f64,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_window_size(window: Window) -> Result<WindowSize, String> {
    let physical_size = window.inner_size().map_err(|e| e.to_string())?;

    let scale_factor = window.scale_factor().map_err(|e| e.to_string())?;

    let logical_width = physical_size.width as f64 / scale_factor;
    let logical_height = physical_size.height as f64 / scale_factor;

    Ok(WindowSize {
        width: logical_width,
        height: logical_height,
    })
}

// ── 设置窗口 ──

/// 打开/聚焦独立设置窗口（复用 Analyze 窗口模式：懒创建 + prevent_close → hide）。
#[tauri::command(rename_all = "snake_case")]
pub fn mole_open_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;

    if let Some(window) = app.get_webview_window("settings") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    use tauri::webview::PageLoadEvent;
    use tauri::{TitleBarStyle, WebviewUrl, WebviewWindowBuilder};

    let window = WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("/settings".into()))
        .title("设置")
        .inner_size(680.0, 600.0)
        .resizable(false)
        .visible(false)
        .transparent(true)
        .title_bar_style(TitleBarStyle::Overlay)
        .hidden_title(true)
        .on_page_load(|webview, payload| {
            if let PageLoadEvent::Finished = payload.event() {
                let _ = webview.show();
                let _ = webview.set_focus();
                log::info!("[settings] window revealed on page load");
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
            log::info!("[settings] window revealed by 3s fallback");
        }
    });

    Ok(())
}

// ── 开机自启动（双路径：macOS 13+ SMAppService / macOS 12 Launch Agent）──

/// 判断当前 macOS 是否 >= 13（Ventura），复用 platform 模块已有的版本探测。
fn is_macos_13_plus() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::platform::macos_privileged_route::macos_semantic_version()
            .map(|(major, _, _)| major >= 13)
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// macOS 12 降级路径：构建 Launch Agent 模式的 auto_launch 实例。
/// 显式 `set_use_launch_agent(true)` 确保写 ~/Library/LaunchAgents/ plist，不走 AppleScript。
#[cfg(target_os = "macos")]
fn build_launch_agent() -> Result<auto_launch::AutoLaunch, String> {
    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    auto_launch::AutoLaunchBuilder::new()
        .set_app_name("MoleStudio")
        .set_app_path(&current_exe.to_string_lossy())
        .set_use_launch_agent(true)
        .build()
        .map_err(|e| e.to_string())
}

/// 查询开机自启动是否已启用。
#[tauri::command(rename_all = "snake_case")]
pub fn mole_auto_launch_status() -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        if is_macos_13_plus() {
            use smappservice_rs::{AppService, ServiceStatus, ServiceType};
            let svc = AppService::new(ServiceType::MainApp);
            Ok(svc.status() == ServiceStatus::Enabled)
        } else {
            build_launch_agent().and_then(|a| a.is_enabled().map_err(|e| e.to_string()))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(false)
    }
}

/// 切换开机自启动状态。
#[tauri::command(rename_all = "snake_case")]
pub fn mole_auto_launch_toggle(enable: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if is_macos_13_plus() {
            use smappservice_rs::{AppService, ServiceType};
            let svc = AppService::new(ServiceType::MainApp);
            if enable {
                svc.register().map_err(|e| {
                    log::warn!("[settings] SMAppService register failed: {e}");
                    e.to_string()
                })?;
                log::info!("[settings] auto launch enabled (SMAppService)");
            } else {
                svc.unregister().map_err(|e| {
                    log::warn!("[settings] SMAppService unregister failed: {e}");
                    e.to_string()
                })?;
                log::info!("[settings] auto launch disabled (SMAppService)");
            }
        } else {
            let agent = build_launch_agent()?;
            let currently_enabled = agent.is_enabled().map_err(|e| e.to_string())?;
            if enable && !currently_enabled {
                agent.enable().map_err(|e| e.to_string())?;
                log::info!("[settings] auto launch enabled (LaunchAgent)");
            } else if !enable && currently_enabled {
                agent.disable().map_err(|e| e.to_string())?;
                log::info!("[settings] auto launch disabled (LaunchAgent)");
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = enable;
        Ok(())
    }
}

// ── 废纸篓提醒「清空废纸篓」 ──

/// 清空当前用户废纸篓（提醒浮窗主按钮）。
/// 独立于 Clean 缓存清理链路与 CLEAN_EXECUTION_BLOCKED 门禁：路径服务端硬编码为
/// ~/.Trash，不接受任何前端路径参数；用原生 std::fs 删除（非外部 rm）；调用方须为浮窗。
/// 清空废纸篓本质是永久删除（文件已在废纸篓内，无法再次移入），属红线4记录在案的例外：
/// 仅作用于用户自己已丢弃的 ~/.Trash，且前端强制二次确认。
#[tauri::command]
pub async fn mole_trash_empty(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<crate::trash_empty::EmptyTrashResult, String> {
    require_window(window.label(), &["trash-reminder"])?;
    let result = tauri::async_runtime::spawn_blocking(crate::trash_empty::empty_trash)
        .await
        .map_err(|e| e.to_string())??;
    // 清空成功后立即让当前提醒失效隐藏（废纸篓已空必低于阈值）。
    crate::trash_watch::service(&app).on_trash_emptied(&app);
    Ok(result)
}

fn require_window(label: &str, allowed: &[&str]) -> Result<(), String> {
    if allowed.contains(&label) {
        Ok(())
    } else {
        Err("TRASH_CALLER_DENIED".into())
    }
}

#[tauri::command]
pub async fn mole_trash_reminder_get_state(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<crate::trash_watch::Snapshot, String> {
    require_window(
        window.label(),
        &["trash-reminder", "MoleStudio", "settings"],
    )?;
    let refresh = window.label() == "settings";
    let ready = window.label() == "trash-reminder";
    tauri::async_runtime::spawn_blocking(move || {
        let service = crate::trash_watch::service(&app);
        // 设置窗加载 Store 前先确认磁盘配置健康，不能把损坏文件当成首次运行。
        if refresh {
            service.refresh(&app);
        }
        service.snapshot(ready)
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn mole_trash_reminder_action(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    args: crate::trash_watch::ActionArgs,
) -> Result<crate::trash_watch::Snapshot, String> {
    use crate::trash_watch::{self, Action};
    require_window(window.label(), &["trash-reminder"])?;
    if matches!(args.action, Action::Show | Action::Hide) {
        return trash_watch::window_action(app, args).await;
    }
    tauri::async_runtime::spawn_blocking(move || trash_watch::service(&app).action(&app, args))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn mole_trash_reminder_update_settings(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    args: crate::trash_watch::Config,
) -> Result<crate::trash_watch::Snapshot, String> {
    require_window(window.label(), &["settings"])?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::trash_watch::service(&app).update_settings(&app, args)
    })
    .await
    .map_err(|e| e.to_string())?
}
