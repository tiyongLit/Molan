//! 提醒窗口展示：仅在主线程调用 AppKit，不激活应用、不修改 Dock 策略。

use std::sync::Mutex;
use tauri::{PhysicalPosition, WebviewWindow};

static DISPLAY_LAYOUT: Mutex<Option<String>> = Mutex::new(None);

pub fn show(window: &WebviewWindow) -> Result<(), String> {
    let primary = window
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("TRASH_DISPLAY_UNAVAILABLE")?;
    let monitors = window.available_monitors().map_err(|e| e.to_string())?;
    let signature = monitors
        .iter()
        .map(|m| {
            format!(
                "{:?}/{:?}/{}",
                m.position(),
                m.work_area(),
                m.scale_factor()
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    let position = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let on_screen = monitors.iter().any(|m| {
        let area = m.work_area();
        position.x >= area.position.x
            && position.y >= area.position.y
            && position.x as i64 + size.width as i64
                <= area.position.x as i64 + area.size.width as i64
            && position.y as i64 + size.height as i64
                <= area.position.y as i64 + area.size.height as i64
    });
    let mut layout = DISPLAY_LAYOUT
        .lock()
        .map_err(|_| "TRASH_DISPLAY_UNAVAILABLE")?;
    if !window.is_visible().map_err(|e| e.to_string())?
        || layout.as_ref() != Some(&signature)
        || !on_screen
    {
        let area = primary.work_area();
        let scale = primary.scale_factor();
        // 20pt 为可见卡片边缘距离；外壳两侧透明留白各 12pt。
        let x = area.position.x as f64 + area.size.width as f64 - (20.0 + 320.0 + 12.0) * scale;
        let y = area.position.y as f64 + (20.0 - 12.0) * scale;
        window
            .set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
            .map_err(|e| e.to_string())?;
    }
    *layout = Some(signature);
    if objc2::MainThreadMarker::new().is_none() {
        return Err("TRASH_MAIN_THREAD_REQUIRED".into());
    }
    let pointer = window.ns_window().map_err(|e| e.to_string())?;
    if pointer.is_null() {
        return Err("TRASH_WINDOW_UNAVAILABLE".into());
    }
    // NSWindow.orderFront: 不会 makeKey，也不会把应用设为 active。
    // 不设置 canJoinAllSpaces/fullScreenAuxiliary，不侵入全屏应用的 Space。
    unsafe {
        use objc2::{msg_send, runtime::Bool};
        let native = pointer.cast::<objc2_app_kit::NSWindow>();
        let _: () = msg_send![native, setHasShadow: Bool::NO];
        let _: () = msg_send![native, orderFront: std::ptr::null::<objc2_foundation::NSObject>()];
        let shown: Bool = msg_send![native, isVisible];
        if !shown.as_bool() {
            return Err("TRASH_SHOW_FAILED".into());
        }
    }
    Ok(())
}
