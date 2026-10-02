//! 退出守卫：只有系统托盘菜单才能退出进程。
//!
//! 两道防线：
//! 1. ObjC 注入 `applicationShouldTerminate:` —— Dock 右键退出的第一道拦截
//! 2. Tauri `ExitRequested` —— 所有退出路径的终极闸门（见 lib.rs）
//!
//! 显式确认退出的入口（托盘右键菜单 / BottomBar「退出应用」/ Cmd+Q 菜单项）
//! 均经 `confirm_tray_exit()` 置位放行；Dock 右键退出永不置位。

use std::ffi::{c_char, c_uchar, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter};

use crate::core::busy_state;
use crate::events;

/// 显式退出确认标志位。
/// 托盘菜单 / BottomBar / Cmd+Q 菜单项均经 `confirm_tray_exit()` 置为 true；
/// ExitRequested 闸门据此放行。
static TRAY_EXIT_CONFIRMED: AtomicBool = AtomicBool::new(false);

/// 查询托盘是否已确认退出（供 lib.rs ExitRequested 闸门使用）。
pub fn is_tray_exit_confirmed() -> bool {
    TRAY_EXIT_CONFIRMED.load(Ordering::SeqCst)
}

/// 托盘菜单确认退出：置标志，供 ExitRequested 闸门放行。
/// 注意：此函数**不**调用 `app.exit(0)`，退出由调用方执行。
pub fn confirm_tray_exit() {
    TRAY_EXIT_CONFIRMED.store(true, Ordering::SeqCst);
}

/// 保存 AppHandle 引用，供回调中 emit 事件。
static ACTIVE_APP: std::sync::Mutex<Option<AppHandle>> = std::sync::Mutex::new(None);

// ── Objective-C Runtime FFI ──
//
// 来自 /usr/lib/libobjc.A.dylib（macOS 系统库）。
// 所有 Objective-C 程序隐式链接此库。

#[cfg(target_os = "macos")]
#[link(name = "objc", kind = "dylib")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn objc_msgSend(receiver: *mut c_void, sel: *mut c_void, ...) -> *mut c_void;
    fn class_addMethod(
        cls: *mut c_void,
        sel: *mut c_void,
        imp: *mut c_void,
        types: *const c_char,
    ) -> c_uchar;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn class_getSuperclass(cls: *mut c_void) -> *mut c_void;
    fn object_getClass(obj: *mut c_void) -> *mut c_void;
}

/// `applicationShouldTerminate:` 回调实现。
///
/// Dock 右键退出 → **一律取消**，让用户走托盘菜单退出。
/// 如果 ObjC 注入失败（tao delegate 已定义该方法），
/// lib.rs 的 ExitRequested 闸门作为第二道防线兜底。
///
/// 返回值语义（Apple NSApplicationDelegate 文档）：
/// - 1 (`NSTerminateNow`) → 立即退出
/// - 0 (`NSTerminateCancel`) → 取消退出
///
/// # Safety
/// 由 Objective-C runtime 调用，签名必须匹配 `@@:@`（id self, SEL _cmd, id sender）→ BOOL。
#[cfg(target_os = "macos")]
unsafe extern "C" fn should_terminate_handler(
    _self_ptr: *mut c_void,
    _sel: *mut c_void,
    _sender: *mut c_void,
) -> u8 {
    // 托盘已确认退出 → 放行
    if TRAY_EXIT_CONFIRMED.load(Ordering::SeqCst) {
        return 1;
    }

    // Dock 右键退出 → 取消终止 + 隐藏窗口 + 移除 Dock 图标
    // 效果：用户看到 Dock 图标消失、窗口关闭，但进程保留在系统托盘中。

    if let Some(handle) = ACTIVE_APP.lock().unwrap().clone() {
        use tauri::Manager;
        // 隐藏主窗口
        if let Some(win) = handle.get_webview_window("MoleStudio") {
            let _ = win.hide();
        }
        // 隐藏 dashboard 气泡
        if let Some(win) = handle.get_webview_window("dashboard") {
            let _ = win.hide();
        }
        // 移除 Dock 图标：切换到 Accessory 模式
        hide_dock_icon();

        // 如果有任务在跑，通知前端
        if busy_state::is_busy() {
            let _ = handle.emit(events::EVT_DOCK_QUIT_REQUESTED, ());
        }
    }

    0 // NSTerminateCancel
}

/// 隐藏 Dock 图标（切换到 NSApplicationActivationPolicyAccessory）。
/// 进程继续运行，托盘图标不受影响。
#[cfg(target_os = "macos")]
pub fn hide_dock_icon() {
    unsafe {
        let ns_app_cls = objc_getClass(b"NSApplication\0".as_ptr() as *const _);
        if ns_app_cls.is_null() {
            return;
        }
        let shared_sel = sel_registerName(b"sharedApplication\0".as_ptr() as *const _);
        let ns_app: *mut c_void = objc_msgSend(ns_app_cls, shared_sel);
        if ns_app.is_null() {
            return;
        }
        // NSApplicationActivationPolicyAccessory = 1
        let set_policy_sel = sel_registerName(b"setActivationPolicy:\0".as_ptr() as *const _);
        let _: *mut c_void = objc_msgSend(ns_app, set_policy_sel, 1i64);
    }
}

/// 恢复 Dock 图标（切换到 NSApplicationActivationPolicyRegular）。
/// 用户点击托盘图标时调用，让应用重新出现在 Dock 中。
#[cfg(target_os = "macos")]
pub fn show_dock_icon() {
    unsafe {
        let ns_app_cls = objc_getClass(b"NSApplication\0".as_ptr() as *const _);
        if ns_app_cls.is_null() {
            return;
        }
        let shared_sel = sel_registerName(b"sharedApplication\0".as_ptr() as *const _);
        let ns_app: *mut c_void = objc_msgSend(ns_app_cls, shared_sel);
        if ns_app.is_null() {
            return;
        }
        // NSApplicationActivationPolicyRegular = 0
        let set_policy_sel = sel_registerName(b"setActivationPolicy:\0".as_ptr() as *const _);
        let _: *mut c_void = objc_msgSend(ns_app, set_policy_sel, 0i64);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn hide_dock_icon() {}
#[cfg(not(target_os = "macos"))]
pub fn show_dock_icon() {}

/// 恢复主窗口到前台：还原 Dock 图标 + 显示并聚焦主窗口。
///
/// 「唤起主窗口」语义的单一事实来源，两类入口共用：
/// - `RunEvent::Reopen`：Dock 图标点击 / 已运行时再次双击 .app（经 LaunchServices）
/// - single-instance 回调：重复启动被拦截（open -n、直接 exec 裸二进制、
///   macOS 12 Launch Agent 等不经 LaunchServices 的旁路）
///
/// 不按 `has_visible_windows` 跳过：气泡/提醒浮窗可见但主窗隐藏时，
/// 用户再次打开应用同样应带出主窗口（气泡会因失焦自行隐藏）。
pub fn restore_main_window(app: &AppHandle) {
    use tauri::Manager;
    // Dock 右键退出后的 accessory 驻留态需切回 regular，否则应用无 Dock 图标
    show_dock_icon();
    if let Some(window) = app.get_webview_window("MoleStudio") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 安装 Dock 退出拦截 handler。在 Tauri `.setup()` 中调用。
///
/// 实现策略：
/// 1. 通过 `NSApplication.sharedApplication.delegate` 获取实际 delegate 对象
/// 2. 用 `object_getClass` 获取 delegate 的真实类
/// 3. 向该类注入 `applicationShouldTerminate:` 方法
/// 4. 如果注入失败（方法已存在），尝试超类链
///
/// 拦截失败不影响正常运行（只是退出时无保护），仅打印警告日志。
#[cfg(target_os = "macos")]
pub fn install(app: &AppHandle) {
    // 保存 AppHandle
    *ACTIVE_APP.lock().unwrap() = Some(app.clone());

    unsafe {
        // 1. NSApplication.sharedApplication
        let ns_app_cls = objc_getClass(b"NSApplication\0".as_ptr() as *const _);
        if ns_app_cls.is_null() {
            log::error!("[dock_quit] NSApplication class not found");
            return;
        }

        let shared_sel = sel_registerName(b"sharedApplication\0".as_ptr() as *const _);
        let ns_app: *mut c_void = objc_msgSend(ns_app_cls, shared_sel);
        if ns_app.is_null() {
            log::error!("[dock_quit] NSApplication.sharedApplication returned null");
            return;
        }

        // 2. delegate
        let delegate_sel = sel_registerName(b"delegate\0".as_ptr() as *const _);
        let delegate: *mut c_void = objc_msgSend(ns_app, delegate_sel);
        if delegate.is_null() {
            log::error!("[dock_quit] NSApplication.delegate returned null");
            return;
        }

        // 3. object_getClass(delegate) — 获取真实类
        let delegate_class = object_getClass(delegate);
        if delegate_class.is_null() {
            log::error!("[dock_quit] object_getClass returned null");
            return;
        }

        // 4. 注入 applicationShouldTerminate: 方法
        let imp = should_terminate_handler
            as unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> u8;
        let imp_ptr = imp as *mut c_void;
        let should_term_sel =
            sel_registerName(b"applicationShouldTerminate:\0".as_ptr() as *const _);
        // ObjC 类型编码：c = BOOL (char), @ = id, : = SEL, @ = id (sender)
        let types = b"c@:@\0".as_ptr() as *const c_char;

        // 尝试注入到 delegate 类
        if class_addMethod(delegate_class, should_term_sel, imp_ptr, types) != 0 {
            return;
        }

        // 方法已存在 → 尝试超类
        let super_class = class_getSuperclass(delegate_class);
        if !super_class.is_null() {
            if class_addMethod(super_class, should_term_sel, imp_ptr, types) != 0 {
                return;
            }
        }

        log::warn!(
            "[dock_quit] failed to inject handler (method may already exist in class chain)"
        );
    }
}

#[cfg(not(target_os = "macos"))]
pub fn install(_app: &AppHandle) {
    // 非 macOS 平台：no-op
}

/// 前端确认退出后调用：置标志 + 退出进程。
/// 托盘菜单和前端 BottomBar 退出均走此命令。
#[tauri::command(rename_all = "snake_case")]
pub fn mole_confirm_dock_quit(app: AppHandle) {
    confirm_tray_exit();
    app.exit(0);
}

/// 查询当前是否处于忙碌状态（前端 BottomBar 退出前调用）。
#[tauri::command(rename_all = "snake_case")]
pub fn mole_is_busy() -> bool {
    busy_state::is_busy()
}

/// 恢复 Dock 图标（前端“打开主窗口”时调用）。
#[tauri::command(rename_all = "snake_case")]
pub fn mole_show_dock_icon() {
    show_dock_icon();
}
