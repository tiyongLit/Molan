//! macOS 原生系统通知（UNUserNotificationCenter）：发送"卸载残留"提醒，
//! 并接收**点击回调**——这是自研而非官方插件的原因：tauri-plugin-notification
//! 桌面端没有点击回调能力（action 相关选项一律忽略，见选型记录）。
//!
//! 链路：residual_watch 检测到新 .app → [`send_residual_notification`]（userInfo
//! 携带 appName/bundleId）→ 用户点击通知本体 → delegate `didReceive` →
//! 写 pending 快照 + 唤起主窗 + emit `uninstall::residual-open` →
//! 前端 layout 取 pending 并跳转卸载页定向扫描。
//!
//! 使用约束：
//! - delegate 必须在应用启动早期注册（`run()` 开头，Builder 之前），
//!   覆盖"应用未运行 → 点击通知冷启动"场景：晚注册会错过本次点击回调；
//! - `pnpm tauri dev`（裸二进制、无合法 bundle）下原生通知完全不可用，
//!   相关函数直接返回失败，由调用方降级为事件兜底（见 residual_watch.rs）；
//! - `UNUserNotificationCenter.delegate` 是 **weak** 属性，delegate 实例
//!   必须在本模块强引用持有，否则回调静默失效。

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, define_class, msg_send};
use objc2_foundation::{NSDictionary, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
    UNNotificationRequest, UNNotificationResponse, UNNotificationSettings,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use tauri::AppHandle;

use crate::events;
use crate::runtime::residual_watch::{self, ResidualTarget};

/// 通知 userInfo 的键（点击回调据此还原目标 app）。
const USER_INFO_KEY_APP_NAME: &str = "appName";
const USER_INFO_KEY_BUNDLE_ID: &str = "bundleId";

/// 授权状态缓存：0=未知 1=未决定 2=已拒绝 3=已授权（含临时授权）。
const AUTH_UNKNOWN: u8 = 0;
const AUTH_NOT_DETERMINED: u8 = 1;
const AUTH_DENIED: u8 = 2;
const AUTH_AUTHORIZED: u8 = 3;

static AUTH_STATE: AtomicU8 = AtomicU8::new(AUTH_UNKNOWN);
/// 是否已发起过授权请求（异步弹框，防重复触发）。
static AUTH_REQUESTED: AtomicBool = AtomicBool::new(false);
/// delegate 强引用（`delegate` 属性为 weak，不持有则回调失效）。
static DELEGATE: OnceLock<Retained<MoleNotificationDelegate>> = OnceLock::new();
/// 点击回调所需的 AppHandle（`.setup()` 阶段绑定）。
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

// ────────────────────────── delegate ──────────────────────────

define_class!(
    // SAFETY: NSObject 无子类约束；本类无 ivars、不实现 Drop。
    #[unsafe(super(NSObject))]
    #[ivars = ()]
    struct MoleNotificationDelegate;

    unsafe impl NSObjectProtocol for MoleNotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for MoleNotificationDelegate {
        /// 用户点击通知（本体或行动按钮）后触发；点击通知本体即 default action，
        /// 无需注册自定义按钮。
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive_notification_response(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion_handler: &DynBlock<dyn Fn()>,
        ) {
            handle_notification_click(response);
            completion_handler.call(());
        }
    }
);

// SAFETY: delegate 无状态（ivars 为空），回调仅访问原子量与 OnceLock 中的 AppHandle。
unsafe impl Send for MoleNotificationDelegate {}
unsafe impl Sync for MoleNotificationDelegate {}

impl MoleNotificationDelegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

// ────────────────────────── 对外接口 ──────────────────────────

/// 注册原生通知 delegate。必须在 `run()` 开头（`tauri::Builder` 构建前）调用。
///
/// 失败（非 .app bundle 环境）仅记录日志，调用方不需要处理。
pub fn install_early() -> Result<(), String> {
    if !native_available() {
        return Err("not running inside a .app bundle (dev mode)".to_string());
    }

    let delegate = MoleNotificationDelegate::new();
    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let _ = DELEGATE.set(delegate);

    // 异步刷新初始授权状态，供 send 时同步读取。
    let handler = RcBlock::new(|settings: std::ptr::NonNull<UNNotificationSettings>| {
        // SAFETY: 系统回调保证 settings 非空，且生命周期覆盖本次调用。
        let status = unsafe { settings.as_ref() }.authorizationStatus();
        AUTH_STATE.store(auth_status_to_u8(status), Ordering::Relaxed);
    });
    center.getNotificationSettingsWithCompletionHandler(&handler);

    log::info!("[notifications] native delegate installed");
    Ok(())
}

/// 绑定 AppHandle，供点击回调唤起主窗与发事件使用（`.setup()` 中调用）。
pub fn bind_app(app: AppHandle) {
    let _ = APP_HANDLE.set(app);
}

/// 发送"卸载残留"系统通知。返回是否**已投递**：
///
/// - `true`：请求已被系统接收（授权已就绪），调用方**不要**再发兜底事件；
/// - `false`：非 .app 环境 / 授权被拒 / 首次未决定（本次发起授权弹框）/
///   投递失败——调用方降级为事件兜底（卸载页提示条）。
pub fn send_residual_notification(app_name: &str, bundle_id: Option<&str>, lang: &str) -> bool {
    if !native_available() {
        return false;
    }

    match AUTH_STATE.load(Ordering::Relaxed) {
        AUTH_AUTHORIZED => {}
        AUTH_DENIED => return false,
        // 未决定/未知：发起一次性授权请求（异步、不阻塞），本次走事件兜底。
        _ => {
            request_authorization();
            return false;
        }
    }

    let (title, body) = notification_copy(lang, app_name);
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&title));
    content.setBody(&NSString::from_str(&body));

    // userInfo 携带点击回跳数据（delegate 回调从这里还原目标 app）。
    let key_app = NSString::from_str(USER_INFO_KEY_APP_NAME);
    let val_app: Retained<AnyObject> = NSString::from_str(app_name).into();
    let key_bid = NSString::from_str(USER_INFO_KEY_BUNDLE_ID);
    let val_bid: Retained<AnyObject> = NSString::from_str(bundle_id.unwrap_or("")).into();
    let user_info =
        NSDictionary::from_retained_objects(&[&*key_app, &*key_bid], &[val_app, val_bid]);
    // SAFETY: 键值均为 NSString，符合 `userInfo` 对属性列表值类型的要求。
    // `from_retained_objects` 受 `CopyingHelper` 约束只能产出 `NSDictionary<NSString, AnyObject>`，
    // 而 `setUserInfo` 要求 `NSDictionary<AnyObject, AnyObject>`：泛型参数仅存在于类型层
    // （同一个 NSDictionary 类，内存布局一致），运行时键确实是 NSString，且仅作只读共享引用传入。
    let user_info: &NSDictionary<AnyObject, AnyObject> =
        unsafe { &*(Retained::as_ptr(&user_info) as *const NSDictionary<AnyObject, AnyObject>) };
    unsafe { content.setUserInfo(user_info) };

    let identifier = NSString::from_str(&format!("mole.residual.{app_name}"));
    let request =
        UNNotificationRequest::requestWithIdentifier_content_trigger(&identifier, &content, None);

    // add 的结果异步返回：用 channel 同步短等（回调正常毫秒级；超时按成功处理，
    // 避免"通知已发出但兜底事件也触发"的双通道打扰）。
    let (tx, rx) = std::sync::mpsc::channel::<bool>();
    let handler = RcBlock::new(move |error: *mut NSError| {
        if error.is_null() {
            let _ = tx.send(true);
        } else {
            // SAFETY: 非空 error 由系统提供，仅读取描述文本用于日志。
            let message = unsafe { (*error).localizedDescription() }.to_string();
            log::warn!("[notifications] add request failed: {message}");
            let _ = tx.send(false);
        }
    });

    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.addNotificationRequest_withCompletionHandler(&request, Some(&handler));
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .unwrap_or(true)
}

// ────────────────────────── 内部实现 ──────────────────────────

/// 点击回调主体：快照 pending → 唤起主窗 → emit 事件。
fn handle_notification_click(response: &UNNotificationResponse) {
    let user_info = response.notification().request().content().userInfo();
    let app_name = user_info_string(&user_info, USER_INFO_KEY_APP_NAME).unwrap_or_default();
    let bundle_id = user_info_string(&user_info, USER_INFO_KEY_BUNDLE_ID).filter(|s| !s.is_empty());

    if app_name.is_empty() {
        log::warn!("[notifications] click without appName in userInfo; ignored");
        return;
    }
    log::info!("[notifications] residual notification clicked: {app_name}");

    // 快照事实源：先落 pending，前端（layout）消费后跳转移交（同 trash_watch 原则）。
    residual_watch::set_pending(ResidualTarget {
        app_name: app_name.clone(),
        bundle_id: bundle_id.clone(),
        detected_at: chrono::Utc::now().timestamp(),
    });

    let Some(app) = APP_HANDLE.get() else {
        log::warn!("[notifications] app handle not bound; skip window restore");
        return;
    };

    // 唤起主窗：delegate 回调线程无保证，统一回主线程执行（已在主线程时为直通）。
    let app_for_restore = app.clone();
    let _ = app.run_on_main_thread(move || {
        crate::runtime::macos_dock_quit::restore_main_window(&app_for_restore);
    });

    // 事件仅作"到达信号"；数据仍以 pending 为准（前端 take 失败时才用 payload 兜底）。
    events::emit_residual_open(app, &app_name, bundle_id.as_deref());
}

/// 请求通知授权（异步弹框，仅发起一次；结果写入 `AUTH_STATE`）。
fn request_authorization() {
    if AUTH_REQUESTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let handler = RcBlock::new(|granted: objc2::runtime::Bool, _error: *mut NSError| {
        AUTH_STATE.store(
            if granted.as_bool() {
                AUTH_AUTHORIZED
            } else {
                AUTH_DENIED
            },
            Ordering::Relaxed,
        );
    });
    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &handler,
    );
    log::info!("[notifications] authorization requested");
}

/// 从 userInfo 字典读取字符串值。
fn user_info_string(dict: &NSDictionary, key: &str) -> Option<String> {
    let key = NSString::from_str(key);
    // SAFETY: objectForKey 的类型契约由写入端保证——本字典由本模块构造，
    // 键值均为 NSString（见 send_residual_notification）。
    let value = unsafe { dict.objectForKey_unchecked(&key) }?;
    value.downcast_ref::<NSString>().map(|s| s.to_string())
}

/// 是否处于可用的 `.app` bundle 环境。
///
/// `pnpm tauri dev`（裸二进制/非 bundle）下 UNUserNotificationCenter 不可用
/// （无合法 bundle identifier），所有原生调用直接跳过，由调用方走事件兜底。
fn native_available() -> bool {
    std::env::current_exe()
        .map(|path| path.to_string_lossy().contains(".app/Contents/MacOS/"))
        .unwrap_or(false)
}

/// UNAuthorizationStatus → 缓存值映射。
fn auth_status_to_u8(status: UNAuthorizationStatus) -> u8 {
    if status == UNAuthorizationStatus::Authorized
        || status == UNAuthorizationStatus::Provisional
        || status == UNAuthorizationStatus::Ephemeral
    {
        AUTH_AUTHORIZED
    } else if status == UNAuthorizationStatus::Denied {
        AUTH_DENIED
    } else {
        AUTH_NOT_DETERMINED
    }
}

/// 通知文案（三语，与前端 i18n 语言设置对齐；后端无通用 i18n 基建，仅此一处）。
fn notification_copy(lang: &str, app_name: &str) -> (String, String) {
    match lang {
        "en-US" => (
            "App Moved to Trash".to_string(),
            format!("“{app_name}” was moved to Trash. Click to scan for leftovers."),
        ),
        "zh-TW" => (
            "偵測到應用程式已卸載".to_string(),
            format!("「{app_name}」已移至垃圾桶，點擊掃描殘留檔案"),
        ),
        _ => (
            "检测到应用已卸载".to_string(),
            format!("「{app_name}」已移入废纸篓，点击扫描残留文件"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_copy_three_locales() {
        assert!(notification_copy("zh-CN", "Foo").0.contains("已卸载"));
        assert!(notification_copy("zh-TW", "Foo").0.contains("已卸載"));
        assert_eq!(notification_copy("en-US", "Foo").0, "App Moved to Trash");
        // 未知语言回落 zh-CN
        assert!(notification_copy("fr-FR", "Foo").0.contains("已卸载"));
        assert!(notification_copy("zh-CN", "Foo").1.contains("「Foo」"));
    }

    #[test]
    fn auth_status_mapping() {
        assert_eq!(
            auth_status_to_u8(UNAuthorizationStatus::Authorized),
            AUTH_AUTHORIZED
        );
        assert_eq!(
            auth_status_to_u8(UNAuthorizationStatus::Provisional),
            AUTH_AUTHORIZED
        );
        assert_eq!(
            auth_status_to_u8(UNAuthorizationStatus::Denied),
            AUTH_DENIED
        );
        assert_eq!(
            auth_status_to_u8(UNAuthorizationStatus::NotDetermined),
            AUTH_NOT_DETERMINED
        );
    }
}
