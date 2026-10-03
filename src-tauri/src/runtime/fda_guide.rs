//! 完全磁盘访问权限（FDA）引导服务。
//!
//! 背景：ad-hoc 签名时代，涉及隐私资源的功能（废纸篓提醒 / 清空废纸篓）需要用户
//! 手动授权「完全磁盘访问权限」；无权限时功能静默失效（trash_watch 上报
//! TRASH_PERMISSION_DENIED），用户不进设置页就无感知。本模块把"权限被拒"变成
//! 可操作引导（骨架对齐腾讯柠檬 GetFullDiskPopVC，细节对齐 Burrow 的成熟实践）：
//!
//! - 引导窗（强通道）：启动 5s 固定延迟 / 主窗聚焦 / 测量失败三处触发，统一入口
//!   `check_and_prompt`；总展示 ≤2 次、每开机周期 ≤1 次、需有聚焦窗口
//!   （不在用户使用其他应用时抢焦点打断）；
//! - Home 软提示横幅（常驻通道）：未授权即显示（命令层查询），**dismiss 为会话级**
//!   （每次启动重现直到授权成功——持久化 dismiss 会让用户与功能永久失联）；
//! - 一键「退出并重新打开」：FDA 授权绑定进程启动时刻，运行中翻转的授权经常对
//!   当前进程不可见，重启是可靠兜底（对标 Burrow 的 Quit & Reopen）；
//! - 额度自动重置：构建变化（exe mtime）/ 授权撤销（已授权→未授权）时重置，
//!   保证重新构建、更新安装、误删隐私条目等场景都能重新被引导到。
//!
//! 状态持久化：`app_data_dir/fda_guide_state.json`（独立小文件，避免与
//! settings.json 的跨模块并发写竞争）。探测为 `~/.Trash` 目录的能力探测
//!（仅 open 即判，不读内容）；判态语义对齐 Burrow：仅"内核拒绝（EPERM/EACCES）"
//! 才视为未授权，路径不存在等歧义情况宁可漏报不误报。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::thread::ThreadId;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// 引导窗总展示上限（对齐柠檬 LemonFullAccessMaxDisplayAfterInstallation = 2）。
const MAX_GUIDE_SHOWN: u32 = 2;

/// 状态文件读写锁（引导检查与命令层并发访问的进程内互斥）。
static STATE_LOCK: Mutex<()> = Mutex::new(());

/// 主线程 id（lib.rs 启动时标记）。窗口枚举 / 可见性查询 / 建窗均为主线程专属操作：
/// 后台线程直接调用会触发 AppKit 的 foreign exception 并 abort 进程
///（2026-10-02 崩溃报告：trash-reminder 线程在测量后触发弹窗链路即崩）。
static MAIN_THREAD_ID: OnceLock<ThreadId> = OnceLock::new();

/// 由 lib.rs 在 `run()` 起点标记主线程（窗口操作线程校验用）。
pub fn mark_main_thread() {
    let _ = MAIN_THREAD_ID.set(std::thread::current().id());
}

fn on_main_thread() -> bool {
    MAIN_THREAD_ID
        .get()
        .is_some_and(|id| *id == std::thread::current().id())
}

/// 引导状态（持久化）。
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GuideState {
    /// 引导窗累计展示次数。
    shown_count: u32,
    /// 上次展示时的开机周期键（分钟级）；同周期不重复弹。
    last_boot_key: u64,
    /// 记录时的 app 二进制 mtime（秒）；构建/安装变化时用于重置引导机会。
    #[serde(default)]
    exe_mtime: u64,
    /// 最近一次记录的授权状态；"已授权 → 未授权"（用户删除隐私条目 / 关闭开关）
    /// 视为新发生的故障，重置引导机会。
    #[serde(default)]
    last_authorized: bool,
}

/// app 二进制 mtime（秒）；取不到时为 0（此时不做构建变化判定）。
fn exe_mtime_secs() -> u64 {
    std::env::current_exe()
        .and_then(|path| std::fs::metadata(path))
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 命令层返回的 FDA 状态。
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FdaStatus {
    /// 当前是否已授权（~/.Trash 可打开）。
    pub authorized: bool,
    /// 是否应显示 Home 软提示横幅（未授权 && 用户未处理）。
    pub show_banner: bool,
}

fn state_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join("fda_guide_state.json"))
}

fn load_state(app: &AppHandle) -> GuideState {
    let Some(path) = state_path(app) else {
        return GuideState::default();
    };
    let mut state = std::fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice::<GuideState>(&data).ok())
        .unwrap_or_default();
    // 构建感知重置：app 二进制 mtime 变化 = 重新编译 / 更新安装；ad-hoc 签名下
    // FDA 授权随 cdhash 失效，用户重新回到"功能不可用"状态，引导机会必须重置
    //（否则"总 2 次"的旧额度耗尽后，重新构建 / 更新后的版本将永远不再引导）。
    let current_mtime = exe_mtime_secs();
    if state.exe_mtime != current_mtime {
        state.shown_count = 0;
        state.last_boot_key = 0;
        state.exe_mtime = current_mtime;
        save_state(app, &state);
    }
    state
}

fn save_state(app: &AppHandle, state: &GuideState) {
    let Some(path) = state_path(app) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_vec_pretty(state) {
        Ok(data) => {
            if let Err(e) = std::fs::write(&path, data) {
                log::warn!("[fda-guide] save state failed: {e}");
            }
        }
        Err(e) => log::warn!("[fda-guide] serialize state failed: {e}"),
    }
}

/// 单探针判定结果（判态语义对齐 Burrow Privacy.ProbeOutcome）：
/// - `Granted`：打开成功——授权对当前进程有效；
/// - `Denied`：EPERM/EACCES——内核拒绝了读取，授权确实没开；
/// - `Unavailable`：ENOENT/ENOTDIR——路径不存在，什么都不能说明（不为难用户）；
/// - `Inconclusive`：其他 errno——不视为证据，不下结论也不打扰。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeOutcome {
    Granted,
    Denied,
    Unavailable,
    Inconclusive,
}

/// 由 open(2) 结果与 errno 分类探针结果（纯函数，可单测）。
fn classify_probe(opened: bool, errno: Option<i32>) -> ProbeOutcome {
    if opened {
        return ProbeOutcome::Granted;
    }
    match errno {
        Some(libc::EPERM) | Some(libc::EACCES) => ProbeOutcome::Denied,
        Some(libc::ENOENT) | Some(libc::ENOTDIR) => ProbeOutcome::Unavailable,
        _ => ProbeOutcome::Inconclusive,
    }
}

/// bool 语义映射（纯函数）：仅 `Denied` 视为"无权限"——`Unavailable` 是
/// "路径不存在什么都不能说明"、`Inconclusive` 是"不下结论"，二者都不打扰用户
///（宁可漏报不误报；旧实现对任意 errno 一律报"无权限"，会把罕见 IO 异常误报成
/// 权限问题）。
fn outcome_blocks_access(outcome: ProbeOutcome) -> bool {
    matches!(outcome, ProbeOutcome::Denied)
}

/// 对 `~/.Trash` 做能力探测：仅打开目录（read_dir 即 open，立即释放，不读内容；
/// 无权限时内核静默拒绝，探测本身不会触发任何系统弹窗）。
pub fn probe_trash_outcome() -> ProbeOutcome {
    let Some(home) = crate::core::base::home_dir_opt() else {
        return ProbeOutcome::Inconclusive;
    };
    match std::fs::read_dir(home.join(".Trash")) {
        Ok(_) => ProbeOutcome::Granted,
        Err(e) => classify_probe(false, e.raw_os_error()),
    }
}

/// 当前是否需要 FDA 引导（bool 便捷语义：未授权 ⇔ Denied）。
pub fn probe_full_disk_access() -> bool {
    !outcome_blocks_access(probe_trash_outcome())
}

/// 开机周期键（分钟级）：同一开机周期内取值一致，跨重启变化。
fn boot_key() -> u64 {
    let uptime = sysinfo::System::uptime();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now.saturating_sub(uptime) / 60
}

/// 查询 FDA 状态与横幅显隐（Home 横幅共用）。
pub fn status(app: &AppHandle) -> FdaStatus {
    let authorized = probe_full_disk_access();
    {
        let _guard = STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        sync_authorization_locked(app, authorized);
    }
    FdaStatus {
        authorized,
        // 软提示横幅：未授权即应显示（dismiss 为会话级、由前端状态管理——每次启动
        // 重现的温和提醒，直到授权成功；持久化 dismiss 会造成永久静默，已弃用）。
        show_banner: !authorized,
    }
}

/// 记录授权状态变化（须在持有 STATE_LOCK 时调用）。
/// "已授权 → 未授权"（用户删除隐私条目 / 关闭开关）视为新发生的故障：
/// 重置引导窗机会，让用户能重新被引导到。
fn sync_authorization_locked(app: &AppHandle, authorized: bool) {
    let mut state = load_state(app);
    if state.last_authorized != authorized {
        if state.last_authorized && !authorized {
            state.shown_count = 0;
            state.last_boot_key = 0;
            log::info!("[fda-guide] authorization revoked; guide chances reset");
        }
        state.last_authorized = authorized;
        save_state(app, &state);
    }
}

/// 启动后固定延迟（秒）：横幅在启动约 2 秒内出现，5 秒时弹窗衔接——用户刚看清
/// 软提示、节奏紧凑且不唐突（用户反馈 10 秒偏长后改短；仍留足界面渲染时间）。
const STARTUP_GUIDE_DELAY: Duration = Duration::from_secs(5);

/// 启动引导检查：lib.rs setup 调用一次；固定延迟后做一次引导决策，
/// 保证"打开应用约 5 秒后弹出引导窗"的确定性（不依赖后台测量时序）。
pub fn start_startup_check(app: &AppHandle) {
    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("fda-guide-startup".into())
        .spawn(move || {
            std::thread::sleep(STARTUP_GUIDE_DELAY);
            check_and_prompt(&handle);
        });
    if let Err(e) = spawned {
        log::warn!("[fda-guide] start startup check failed: {e}");
    }
}

/// 权限引导检查统一入口：未授权时走引导决策（prompt_on_main 内部含频率与
/// "应用聚焦"条件）。调用点：启动延迟检查、主窗聚焦补弹、trash_watch 测量后
///（权限被拒的即时发现）；任何线程可调，窗口操作统一调度到主线程执行。
pub fn check_and_prompt(app: &AppHandle) {
    let handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        if probe_full_disk_access() {
            return;
        }
        prompt_on_main(&handle);
    }) {
        log::warn!("[fda-guide] schedule prompt check failed: {e}");
    }
}

/// 引导决策主体（仅主线程执行）。
fn prompt_on_main(app: &AppHandle) {
    // 仅当本应用有"聚焦窗口"时弹（用户正看着 Molan 的某个界面）：
    // - 纯托盘驻留 / 开机自启（无窗口）不弹；
    // - 用户在其他应用前台工作时（我们的窗口只是"可见"在后台）也不弹——
    //   避免突然抢焦点打断用户；等用户切回 Molan 后的下一轮测量再弹。
    let any_focused = app
        .webview_windows()
        .values()
        .any(|w| w.is_focused().unwrap_or(false));
    if !any_focused {
        return;
    }

    let current_boot = boot_key();
    let shown = {
        let _guard = STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = load_state(app);
        if state.shown_count >= MAX_GUIDE_SHOWN || state.last_boot_key == current_boot {
            return;
        }
        state.shown_count += 1;
        state.last_boot_key = current_boot;
        save_state(app, &state);
        state.shown_count
    };

    log::info!("[fda-guide] prompting full disk access guide (shown={shown})");
    if let Err(e) = open_guide_window(app) {
        log::warn!("[fda-guide] open guide window failed: {e}");
    }
    // 广播给所有 Webview：Home 软提示横幅收到后立即让位（避免同屏双层提示）。
    let _ = app.emit(crate::events::EVT_FDA_GUIDE_SHOWN, ());
}

/// 隐藏窗口红绿灯中的 Zoom（绿色）按钮（与 lib.rs 主窗口同款处理）：
/// 窗口不可缩放时该按钮在透明 + overlay 样式中呈"空态"，属多余控件；
/// 移除后只保留关闭 / 最小化两个按钮（对齐腾讯柠檬引导窗的克制形态）。
#[cfg(target_os = "macos")]
fn hide_zoom_button(window: &tauri::WebviewWindow) {
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
fn hide_zoom_button(_window: &tauri::WebviewWindow) {}

/// 打开（或聚焦）FDA 引导窗；懒创建，关闭即隐藏复用（对齐设置窗口模式）。
///
/// 线程安全：窗口创建与红绿灯操作均为主线程专属（AppKit 在非主线程调用会抛
/// foreign exception 致 abort）；非主线程调用自动转主线程异步执行。
pub fn open_guide_window(app: &AppHandle) -> Result<(), String> {
    if on_main_thread() {
        return open_guide_window_inner(app);
    }
    let handle = app.clone();
    app.run_on_main_thread(move || {
        if let Err(e) = open_guide_window_inner(&handle) {
            log::warn!("[fda-guide] deferred open failed: {e}");
        }
    })
    .map_err(|e| e.to_string())
}

/// 引导窗打开主体（仅主线程调用）。
fn open_guide_window_inner(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("fda-guide") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    use tauri::webview::PageLoadEvent;
    use tauri::{TitleBarStyle, WebviewUrl, WebviewWindowBuilder};

    let window = WebviewWindowBuilder::new(app, "fda-guide", WebviewUrl::App("/fda-guide".into()))
        .title("完全磁盘访问权限")
        .inner_size(560.0, 430.0)
        .resizable(false)
        .visible(false)
        .transparent(true)
        .title_bar_style(TitleBarStyle::Overlay)
        .hidden_title(true)
        .center()
        .on_page_load(|webview, payload| {
            if let PageLoadEvent::Finished = payload.event() {
                let _ = webview.show();
                let _ = webview.set_focus();
                log::info!("[fda-guide] window revealed on page load");
            }
        })
        .build()
        .map_err(|e| e.to_string())?;
    // 隐藏红绿灯 Zoom 按钮，只保留关闭 / 最小化（对齐主窗口与柠檬形态）。
    hide_zoom_button(&window);

    let w = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = w.hide();
            // 通知 Home 横幅重新查询：未授权且未被用户处理时恢复显示软提示
            //（与 EVT_FDA_GUIDE_SHOWN 的让位成对）。
            let _ = w.emit(crate::events::EVT_FDA_GUIDE_CLOSED, ());
        }
    });

    // 3s fallback：防止 page load 事件延迟时窗口一直不可见
    let fallback_win = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(3));
        if !fallback_win.is_visible().unwrap_or(false) {
            let _ = fallback_win.show();
            let _ = fallback_win.set_focus();
            log::info!("[fda-guide] window revealed by 3s fallback");
        }
    });

    Ok(())
}

/// 重新探测授权状态；已授权时触发 trash_watch 重测（尽快清除错误态并同步各窗口）。
/// 供引导窗获焦重检与命令层复用。
pub fn check_and_refresh(app: &AppHandle) -> bool {
    let authorized = probe_full_disk_access();
    {
        // 记录状态快照：授权被撤销的场景在此被捕捉（引导机会重置）。
        let _guard = STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        sync_authorization_locked(app, authorized);
    }
    if authorized {
        if let Some(svc) = app.try_state::<std::sync::Arc<crate::runtime::trash_watch::Service>>() {
            svc.inner().clone().dirty();
        }
    }
    authorized
}

/// 一键「退出并重新打开」：macOS 把 FDA 授权绑定在"进程启动时刻"，运行中翻转的
/// 授权经常对当前进程不可见——重启是让新授权生效的可靠兜底（对标 Burrow 的
/// Quit & Reopen）。
///
/// 先置退出放行标志（绕过我们的退出守卫闸门：restart 的非主线程路径会走
/// ExitRequested 事件），再走 Tauri restart——主线程路径为"spawn 新进程 +
/// 退出旧进程"（顺序正确，不会被单实例守卫误伤）。
pub fn relaunch_app(app: &AppHandle) {
    crate::runtime::macos_dock_quit::confirm_tray_exit();
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_maps_errno_to_outcome() {
        assert_eq!(classify_probe(true, None), ProbeOutcome::Granted);
        assert_eq!(classify_probe(false, Some(libc::EPERM)), ProbeOutcome::Denied);
        assert_eq!(classify_probe(false, Some(libc::EACCES)), ProbeOutcome::Denied);
        assert_eq!(classify_probe(false, Some(libc::ENOENT)), ProbeOutcome::Unavailable);
        assert_eq!(classify_probe(false, Some(libc::ENOTDIR)), ProbeOutcome::Unavailable);
        assert_eq!(classify_probe(false, Some(libc::EIO)), ProbeOutcome::Inconclusive);
        assert_eq!(classify_probe(false, None), ProbeOutcome::Inconclusive);
    }

    #[test]
    fn only_denied_blocks_access() {
        // 宁可漏报不误报：仅内核拒绝才视为"需要引导"。
        assert!(outcome_blocks_access(ProbeOutcome::Denied));
        assert!(!outcome_blocks_access(ProbeOutcome::Granted));
        assert!(!outcome_blocks_access(ProbeOutcome::Unavailable));
        assert!(!outcome_blocks_access(ProbeOutcome::Inconclusive));
    }
}
