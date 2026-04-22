//! 原生 NSAlert 确认/提示通道 —— 全应用唯一的确认与提示机制。
//!
//! 视觉形态（macOS Big Sur+ 的 stock NSAlert，对齐 Lemon Cleaner「退出状态栏」弹窗）：
//!   行 1  大号 app 图标（居中；编译期内嵌 PNG，dev 环境亦可靠）
//!   行 2  居中粗体文案（messageText，多行自动折行）
//!   行 3  原生按钮（主按钮在右＝默认蓝色、回车触发；取消在左＝Esc 触发）
//!
//! 实现要点：
//! - **不注入 accessoryView、不顶掉默认图标**：macOS 11+ 的 NSAlert 自身就是
//!   「图标 / 文案 / 按钮」垂直布局，stock 即目标形态（Lemon 同款，勿再自绘）；
//! - **messageText 必须显式设置**：NSAlert init 后默认值是 "Alert"（历史遗留），
//!   不覆盖会在顶部多出一行 "Alert" 标题；
//! - 呈现位置：调用方窗口可见（非最小化）时以 sheet 挂载；否则回退 app-modal runModal；
//! - 线程桥接：Tauri async 命令（tokio 线程）→ `DispatchQueue::main().exec_sync` 切主线程
//!   构建并呈现 → block2 完成回调 → tokio oneshot 把结果回传给命令；
//! - App 图标编译期内嵌（`icons/128x128@2x.png`）：dev 环境裸二进制运行时
//!   `NSApplicationIcon` 不可靠，显式 `setIcon` 保证显示真图标，
//!   与 embedded_rules.rs 的内嵌哲学一致；
//! - 无超时：原生对话框不存在「前端崩溃导致永久挂起」的 webview 场景，
//!   与旧 mole_system_confirm 的 60s 超时设计不同。
//!
//! 首个调用方：Analyze Toolbar 返回按钮（「退出后需要重新对磁盘进行分析」）。

use core::ffi::c_void;
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSImage, NSModalResponse, NSWindow,
};
use objc2_foundation::{NSData, NSString};
use tokio::sync::oneshot;

/// 弹窗语义（对齐 NSAlertStyle）。Critical 由 AppKit 自动加警告角标。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Info,
    Warning,
    Critical,
}

impl Default for DialogKind {
    fn default() -> Self {
        Self::Info
    }
}

/// 呈现参数。
#[derive(Clone, Debug)]
pub struct DialogRequest {
    /// 主文案（messageText：居中粗体、自动折行）。
    pub message: String,
    /// 副文案（informativeText：常规字重居中，置于主文案下方）；`None` = 不显示。
    pub informative_text: Option<String>,
    /// 主按钮文案（右侧、默认蓝、回车触发）。
    pub ok_label: String,
    /// 取消按钮文案（左侧、Esc 触发）；`None` = 单按钮提示模式。
    pub cancel_label: Option<String>,
    pub kind: DialogKind,
}

/// Esc 键等效字符（NSButton.keyEquivalent）。
const KEY_EQUIVALENT_ESC: &str = "\u{1b}";

/// App 图标（编译期内嵌；路径相对本文件）。
const APP_ICON_PNG: &[u8] = include_bytes!("../../../icons/128x128@2x.png");

/// 调用方窗口指针（`WebviewWindow::ns_window()` 原样透传）。
///
/// 裸指针不是 `Send`，包装后才能作为 async 命令参数并进入 `exec_sync` 闭包；
/// 指针仅在主线程内解引用，跨线程传递本身不产生数据竞争。
pub struct ParentWindow(*mut c_void);

// Safety: 见上；包装类型只携带地址，无共享可变状态。
unsafe impl Send for ParentWindow {}

impl ParentWindow {
    /// 包装窗口指针；`None` / 空指针 = 无父窗口（app-modal 呈现）。
    pub fn new(ptr: Option<*mut c_void>) -> Option<Self> {
        ptr.filter(|p| !p.is_null()).map(Self)
    }

    /// 取回指针。必须经方法而非直接访问 `.0`：
    /// Rust 2021 精准捕获会让闭包只捕裸指针字段（非 Send），方法调用（&self）强制捕获整个包装体。
    fn ptr(&self) -> *mut c_void {
        self.0
    }
}

/// 在主线程呈现弹窗并等待用户操作。
///
/// 返回值：`Ok(true)` = 主按钮（确认）；`Ok(false)` = 取消 / 关闭 / Esc。
/// 调用方须为异步上下文（Tauri async 命令天然满足）。
pub async fn present(request: DialogRequest, parent: Option<ParentWindow>) -> Result<bool, String> {
    // 防御：调用方若已在主线程，`exec_sync`（主队列同步等待）会立即死锁，
    // 且 sheet 完成回调也依赖主线程空闲。此时退化为同步 app-modal。
    // 当前架构（async 命令）不会走到这里，仅作兜底。
    if let Some(mtm) = MainThreadMarker::new() {
        let alert = build_alert(mtm, &request);
        return Ok(alert.runModal() == NSAlertFirstButtonReturn);
    }

    let (tx, rx) = oneshot::channel::<bool>();
    let tx_slot = Arc::new(Mutex::new(Some(tx)));
    let slot_sheet = Arc::clone(&tx_slot);
    let slot_modal = Arc::clone(&tx_slot);

    dispatch2::DispatchQueue::main().exec_sync(move || {
        let Some(mtm) = MainThreadMarker::new() else {
            // 主队列上拿不到 marker 属不可能；通道 drop → 上层报错。
            return;
        };
        let alert = build_alert(mtm, &request);

        let parent_win = parent
            .as_ref()
            .and_then(|p| unsafe { visible_window(p.ptr()) });
        match parent_win {
            Some(win) => {
                // sheet：alert 须活到回调结束，闭包捕获 Retained 保活；
                // handler 由 AppKit copy 持有，RcBlock 局部变量 drop 不影响回调。
                let keep_alive = alert.clone();
                let block = RcBlock::new(move |response: NSModalResponse| {
                    let _keep = &keep_alive;
                    let confirmed = response == NSAlertFirstButtonReturn;
                    if let Ok(mut guard) = slot_sheet.lock() {
                        if let Some(tx) = guard.take() {
                            let _ = tx.send(confirmed);
                        }
                    }
                });
                let handler: &block2::DynBlock<dyn Fn(NSModalResponse)> = &block;
                alert.beginSheetModalForWindow_completionHandler(&win, Some(handler));
            }
            None => {
                // 窗口不可见（隐藏/最小化）→ app-modal 兜底（主线程跑 modal loop）。
                let confirmed = alert.runModal() == NSAlertFirstButtonReturn;
                if let Ok(mut guard) = slot_modal.lock() {
                    if let Some(tx) = guard.take() {
                        let _ = tx.send(confirmed);
                    }
                }
            }
        }
    });

    rx.await
        .map_err(|_| "dialog: main-thread result channel closed（未收到用户响应）".to_string())
}

/// 解析调用方窗口指针；不可见（隐藏/最小化）时返回 `None`，交由 app-modal 兜底。
///
/// # Safety
/// 指针须为有效的 NSWindow（Tauri `WebviewWindow::ns_window()` 的原样透传）。
unsafe fn visible_window(ptr: *mut c_void) -> Option<Retained<NSWindow>> {
    if ptr.is_null() {
        return None;
    }
    let win = unsafe { Retained::retain(ptr.cast::<NSWindow>())? };
    (win.isVisible() && !win.isMiniaturized()).then_some(win)
}

/// 构建 stock NSAlert（须主线程）。
fn build_alert(mtm: MainThreadMarker, request: &DialogRequest) -> Retained<NSAlert> {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(match request.kind {
        DialogKind::Info => NSAlertStyle::Informational,
        DialogKind::Warning => NSAlertStyle::Warning,
        DialogKind::Critical => NSAlertStyle::Critical,
    });

    // 必设：NSAlert init 后 messageText 默认 "Alert"，不覆盖会在顶部多出一行标题。
    alert.setMessageText(&NSString::from_str(&request.message));

    if let Some(informative) = &request.informative_text {
        alert.setInformativeText(&NSString::from_str(informative));
    }

    // 显式图标：dev 环境（裸二进制）的 NSApplicationIcon 不可靠，内嵌 PNG 保证真图标。
    if let Some(icon) = load_app_icon() {
        unsafe { alert.setIcon(Some(&icon)) };
    }

    // 行 3：按钮。AppKit 横向排布为 trailing → leading，添加顺序即「最重要 → 次重要」：
    // 先 add 主按钮（最右＝默认蓝、回车自动触发），再 add 取消（左侧）。
    alert.addButtonWithTitle(&NSString::from_str(&request.ok_label));
    if let Some(cancel_label) = &request.cancel_label {
        let cancel = alert.addButtonWithTitle(&NSString::from_str(cancel_label));
        // 标题非「Cancel」时 AppKit 不会自动绑 Esc，这里显式绑定。
        cancel.setKeyEquivalent(&NSString::from_str(KEY_EQUIVALENT_ESC));
    }

    alert
}

/// 内嵌 PNG → NSImage；数据合法时不会失败，`None` 仅作防御（回退系统默认图标）。
fn load_app_icon() -> Option<Retained<NSImage>> {
    let data = NSData::with_bytes(APP_ICON_PNG);
    NSImage::initWithData(NSImage::alloc(), &data)
}
