// src-tauri/src/tray.rs
use tauri::{
    Manager, PhysicalPosition, Position, Runtime, WindowEvent,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::controllers::status::{start_status_watch, stop_status_watch};

pub fn create_tray<R: Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<()> {
    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(dashboard) = app.get_webview_window("dashboard") {
                    let scale_factor = dashboard.scale_factor().unwrap_or(1.0);
                    // 与 tauri.conf.json dashboard 窗口尺寸保持一致（V2 六卡：360×680）
                    let dashboard_width = 360.0;
                    let dashboard_height = 680.0;

                    let mut x = position.x as f64 / scale_factor;
                    let mut y = position.y as f64 / scale_factor;

                    x = x - dashboard_width / 2.0;

                    #[cfg(target_os = "macos")]
                    {
                        y = y - dashboard_height;
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        y = y + 20.0;
                    }

                    let _ = dashboard.set_position(Position::Physical(PhysicalPosition {
                        x: (x * scale_factor) as i32,
                        y: (y * scale_factor) as i32,
                    }));

                    if dashboard.is_visible().unwrap_or(false) {
                        log::info!("[tray] hiding dashboard popover");
                        let _ = dashboard.hide();
                        stop_status_watch();
                    } else {
                        log::info!("[tray] showing dashboard popover at ({}, {})", x, y);
                        let _ = dashboard.show();
                        let _ = dashboard.set_focus();
                        start_status_watch(app.clone());
                    }
                } else {
                    log::warn!("[tray] dashboard window not found");
                }
            }
        })
        .build(app)?;

    if let Some(dashboard) = app.get_webview_window("dashboard") {
        #[cfg(target_os = "macos")]
        {
            use objc2::msg_send;
            use objc2_foundation::NSObject;

            unsafe {
                let ptr = dashboard.ns_window().expect("failed to get ns_window");
                if !ptr.is_null() {
                    let ns_window = &*(ptr as *const objc2_app_kit::NSWindow);
                    let content_view: *mut NSObject = msg_send![ns_window, contentView];
                    if !content_view.is_null() {
                        let () = msg_send![content_view, setWantsLayer: true];
                        let layer: *mut NSObject = msg_send![content_view, layer];
                        if !layer.is_null() {
                            let () = msg_send![layer, setCornerRadius: 10.0];
                            let () = msg_send![layer, setMasksToBounds: true];
                        }
                    }
                }
            }
        }

        let dashboard_clone = dashboard.clone();
        dashboard.on_window_event(move |event| {
            if let WindowEvent::Focused(focused) = event {
                // 仅在气泡确实可见时才隐藏并停止采集：
                // 点击托盘图标收起气泡时，hide() 会触发 resignKey → 本回调（Focused(false)），
                // 若无可见性守卫，会与点击分支重复调用 stop_status_watch，
                // 导致 CONSUMER_COUNT 原子下溢、watch 线程永久无法停止。
                if !focused && dashboard_clone.is_visible().unwrap_or(false) {
                    log::info!("[tray] dashboard lost focus, hiding popover");
                    let _ = dashboard_clone.hide();
                    stop_status_watch();
                }
            }
        });
    }

    Ok(())
}
