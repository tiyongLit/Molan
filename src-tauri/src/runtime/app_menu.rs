//! 应用菜单：接管 macOS 默认菜单的 Quit 项，使 Cmd+Q / 菜单栏「退出」
//! 走应用内退出流程（真退出），与 Dock 右键退出（隐藏到托盘）区分开。
//!
//! 背景：muda 的 `PredefinedMenuItem::quit` 直通 `-[NSApplication terminate:]`，
//! 与 Dock 右键退出是同一条系统路径，无法在 `applicationShouldTerminate:` 层区分；
//! 替换为普通 `MenuItem` 后，Cmd+Q 改走 Tauri 菜单事件（`lib.rs` 的 `on_menu_event`），
//! 复用与 BottomBar「退出应用」一致的退出守卫。

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::Menu;
use tauri::{AppHandle, Runtime};

#[cfg(target_os = "macos")]
use tauri::menu::{MenuItem, MenuItemKind};

use crate::core::busy_state;
use crate::runtime::macos_dock_quit::confirm_tray_exit;

/// 自定义退出菜单项 id（`lib.rs` 的 `on_menu_event` 中匹配）。
pub const MENU_QUIT_ID: &str = "app_quit";

/// 退出确认弹窗防重入（Cmd+Q 连按时不叠加多个 modal）。
static QUIT_CONFIRM_OPEN: AtomicBool = AtomicBool::new(false);

/// 构建应用菜单（`lib.rs` 的 `Builder::menu` 回调）。
pub fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    #[cfg(target_os = "macos")]
    {
        // `Menu::default()` 打底：保留 Edit/Window 等系统标准项
        //（webview 的 Cmd+C/V 依赖 Edit 菜单），仅替换 App 子菜单末尾的 Quit 项。
        let menu = Menu::default(app)?;
        if let Err(e) = replace_quit_item(app, &menu) {
            log::warn!("[app_menu] replace quit item failed: {e}（Cmd+Q 保持 Dock 退出行为）");
        }
        Ok(menu)
    }
    #[cfg(not(target_os = "macos"))]
    {
        // 本产品仅发布 macOS；非 macOS 返回空菜单，不引入额外菜单栏。
        Menu::new(app)
    }
}

/// 把 App 子菜单末尾的默认 Quit 项（`terminate:` 直通）替换为自定义项：
/// 「退出 Molan」+ Cmd+Q —— 点击后走 Tauri 菜单事件，而非系统终止流程。
#[cfg(target_os = "macos")]
fn replace_quit_item<R: Runtime>(app: &AppHandle<R>, menu: &Menu<R>) -> tauri::Result<()> {
    // App 子菜单是 macOS 默认菜单的第一项（顶栏以 App 名开头的菜单）。
    let Some(MenuItemKind::Submenu(app_submenu)) = menu.items()?.into_iter().next() else {
        return Ok(());
    };

    let items = app_submenu.items()?;
    // 默认 Quit 项位于 App 子菜单末尾，标题形如 "Quit <AppName>"；
    // 不匹配时保守跳过（Tauri 默认菜单结构变化时不误伤其他项）。
    let Some(MenuItemKind::Predefined(default_quit)) = items.last() else {
        return Ok(());
    };
    if !default_quit.text()?.starts_with("Quit") {
        return Ok(());
    }

    let position = items.len() - 1;
    app_submenu.remove(default_quit)?;
    let custom_quit = MenuItem::with_id(
        app,
        MENU_QUIT_ID,
        "退出 Molan",
        true,
        Some("CmdOrCtrl+Q"),
    )?;
    app_submenu.insert(&custom_quit, position)?;
    log::info!("[app_menu] quit item replaced (Cmd+Q → in-app quit flow)");
    Ok(())
}

/// 应用内退出请求入口（Cmd+Q / 菜单栏「退出 Molan」）。
///
/// 与 BottomBar「退出应用」同一守卫口径：
/// - 空闲 → 直接退出（置放行标志后 `app.exit(0)`，经 `lib.rs` 的 ExitRequested 闸门放行）；
/// - 有任务运行 → 弹原生确认（app-modal），确认则强制退出、取消则留在托盘。
pub fn request_quit<R: Runtime>(app: &AppHandle<R>) {
    if !busy_state::is_busy() {
        log::info!("[app_menu] idle, quitting via menu");
        exit_now(app);
        return;
    }

    // 有任务在跑：弹确认；防重入——连按 Cmd+Q 不叠加多个 modal。
    if QUIT_CONFIRM_OPEN
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    log::warn!(
        "[app_menu] exit blocked: {} task(s) running, awaiting confirmation",
        busy_state::busy_count()
    );

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        #[cfg(target_os = "macos")]
        {
            use crate::platform::macos_dialog::{self, DialogKind, DialogRequest};

            let request = DialogRequest {
                message: "当前有任务正在运行，强制退出将中断操作。确定要退出吗？".to_string(),
                informative_text: None,
                ok_label: "强制退出".to_string(),
                cancel_label: Some("取消".to_string()),
                kind: DialogKind::Warning,
            };
            // parent=None → app-modal 兜底（不依赖任何窗口的可见状态，
            // 气泡/主窗口隐藏时同样可弹）。
            let ok = macos_dialog::present(request, None).await.unwrap_or(false);
            QUIT_CONFIRM_OPEN.store(false, Ordering::SeqCst);
            if ok {
                log::info!("[app_menu] user confirmed forced quit");
                exit_now(&app);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = app;
            QUIT_CONFIRM_OPEN.store(false, Ordering::SeqCst);
        }
    });
}

/// 真退出收尾：停状态采集 → 置放行标志 → 退出进程。
fn exit_now<R: Runtime>(app: &AppHandle<R>) {
    crate::controllers::status::stop_status_watch();
    confirm_tray_exit();
    app.exit(0);
}
