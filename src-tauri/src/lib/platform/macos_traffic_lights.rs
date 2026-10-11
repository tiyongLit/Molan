//! macOS 窗口红绿灯（traffic lights）统一口径。
//!
//! 应用内所有带标题栏的窗口均为 Overlay titleBarStyle + 不可缩放形态
//!（主窗 / 设置窗 / FDA 引导窗）：绿色 Zoom 按钮在该形态下呈"空态"占位框，
//! 属多余控件。统一隐藏后只保留关闭 / 最小化两枚按钮（对齐腾讯柠檬的克制形态）。
//! decorations:false 的窗口（托盘气泡 / 废纸篓提醒）无红绿灯，不在此列。

use tauri::WebviewWindow;

/// 隐藏窗口红绿灯中的 Zoom（绿色）按钮；幂等，重复调用无副作用。
///
/// 线程约束：AppKit 窗口操作须在主线程执行（非主线程调用会抛 foreign
/// exception 致进程 abort）。Tauri 同步命令、`setup` 回调均为主线程；
/// 后台线程使用前须经 `run_on_main_thread` 转投。
#[cfg(target_os = "macos")]
pub fn hide_zoom_button(window: &WebviewWindow) {
    use objc2::msg_send;
    use objc2::runtime::Bool;
    use objc2_app_kit::NSWindowButton;

    if let Ok(ptr) = window.ns_window() {
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

#[cfg(not(target_os = "macos"))]
pub fn hide_zoom_button(_window: &WebviewWindow) {}
