// src-tauri/src/tray.rs
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, Position, Runtime, WebviewWindow, WindowEvent,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::controllers::status::{start_status_watch, stop_status_watch};
use crate::core::busy_state;
use crate::events;

/// 显隐代际号：每次 request_show / request_hide 递增。动画线程与延迟隐藏线程捕获自身 gen，
/// 唤醒后仅当 gen 未变才执行，保证「出场中途重新点击托盘」能安全取消。
static DASH_GEN: AtomicU64 = AtomicU64::new(0);
/// 隐藏进行标志：request_hide 期间为 true，去重「失焦 + 托盘点击」双触发，
/// 并确保 start/stop_status_watch 引用计数严格一次配对。
static DASH_HIDE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
/// 气泡顶部距菜单栏底部（工作区顶）的偏移（逻辑 px）。
const DASHBOARD_TOP_OFFSET: f64 = 0.0;
/// 气泡右缘与屏幕工作区右缘的间距（逻辑 px）；0 = 紧贴屏幕右缘。
const DASHBOARD_RIGHT_MARGIN: f64 = 0.0;
/// 原生窗口滑动动画时长（ms）。入场 ease-out cubic，出场 ease-in cubic。
const DASHBOARD_ANIM_DURATION_MS: u64 = 280;
/// 动画帧数（280ms / 16 ≈ 17.5ms/帧 ≈ 57fps）。
const DASHBOARD_ANIM_FRAMES: u64 = 16;
/// 滑入/滑出水平位移占窗口宽度的比例：只把窗口向右移出这一部分（非整窗移出屏幕），
/// 保证窗口原点始终落在目标显示器内 → macOS 不会把窗口误关联到主屏 →
/// 从根本上规避双屏「窗口在两屏来回画」的跨屏合成竞态。0.5 = 半窗移出、半窗滑入。
const DASHBOARD_SLIDE_OFFSET_RATIO: f64 = 0.5;
/// 延迟隐藏时长 = 动画时长 + 30ms 余量，确保原生滑出放完再 hide。
const DASHBOARD_HIDE_DELAY_MS: u64 = DASHBOARD_ANIM_DURATION_MS + 30;

/// 计算气泡在屏幕工作区右上角的目标位置（物理像素），返回 (target_x, target_y)。
/// 这是滑入动画的终点；起点为 target_x + slide_offset（右侧移出部分窗宽、原点仍在目标屏内），
/// 从而规避「整窗移到屏幕外真空区」触发的 macOS 跨显示器合成竞态。
/// 显示器选择：优先包含托盘点击坐标者，回退 current → primary。
/// 尺寸动态读 inner_size()（禁止硬编码，遵循窗口尺寸 SSOT）。
fn compute_dashboard_target<R: Runtime>(
    dashboard: &WebviewWindow<R>,
    tray_pos: Option<PhysicalPosition<f64>>,
) -> (i32, i32) {
    let size = dashboard
        .inner_size()
        .unwrap_or_else(|_| tauri::PhysicalSize::new(360u32, 580u32));
    let monitors = dashboard.available_monitors().unwrap_or_default();
    let picked = tray_pos.and_then(|p| {
        monitors
            .iter()
            .find(|m| {
                let a = m.work_area();
                let ax = a.position.x as f64;
                let ay = a.position.y as f64;
                p.x >= ax
                    && p.x <= ax + a.size.width as f64
                    && p.y >= ay
                    && p.y <= ay + a.size.height as f64
            })
            .cloned()
    });
    let monitor = picked
        .or_else(|| dashboard.current_monitor().ok().flatten())
        .or_else(|| dashboard.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        log::warn!("[tray] no monitor available for dashboard positioning");
        return (0, 0);
    };
    let area = monitor.work_area();
    let scale = monitor.scale_factor();
    let right_margin = (DASHBOARD_RIGHT_MARGIN * scale).round() as i32;
    let top_offset = (DASHBOARD_TOP_OFFSET * scale).round() as i32;
    // 右缘紧贴工作区右缘（right_margin=0）；顶部距菜单栏底部 top_offset。
    let target_x = area.position.x + area.size.width as i32 - size.width as i32 - right_margin;
    let target_y = area.position.y + top_offset;
    (target_x, target_y)
}

/// 滑入/滑出的水平位移量（物理像素）= 窗口宽 × DASHBOARD_SLIDE_OFFSET_RATIO。
fn slide_offset<R: Runtime>(dashboard: &WebviewWindow<R>) -> i32 {
    let size = dashboard
        .inner_size()
        .unwrap_or_else(|_| tauri::PhysicalSize::new(360u32, 580u32));
    (size.width as f64 * DASHBOARD_SLIDE_OFFSET_RATIO).round() as i32
}

/// 原生窗口滑入动画：逐帧 set_position 从 from_x 移到 to_x（ease-out cubic）。
/// 每帧检查 DASH_GEN，被取消时立即停止。set_position 内部由 tao 调度到主线程，线程安全。
fn native_slide_in<R: Runtime>(
    dashboard: &WebviewWindow<R>,
    from_x: i32,
    to_x: i32,
    y: i32,
    gen: u64,
) {
    let win = dashboard.clone();
    std::thread::spawn(move || {
        let frame_delay = DASHBOARD_ANIM_DURATION_MS / DASHBOARD_ANIM_FRAMES;
        for i in 1..=DASHBOARD_ANIM_FRAMES {
            std::thread::sleep(std::time::Duration::from_millis(frame_delay));
            if DASH_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            let t = i as f64 / DASHBOARD_ANIM_FRAMES as f64;
            let ease = 1.0 - (1.0 - t).powi(3); // ease-out cubic
            let x = (from_x as f64 + (to_x as f64 - from_x as f64) * ease).round() as i32;
            let _ = win.set_position(Position::Physical(PhysicalPosition { x, y }));
        }
    });
}

/// 原生窗口滑出动画：逐帧 set_position 从 from_x 移到 to_x（ease-in cubic）。
fn native_slide_out<R: Runtime>(
    dashboard: &WebviewWindow<R>,
    from_x: i32,
    to_x: i32,
    y: i32,
    gen: u64,
) {
    let win = dashboard.clone();
    std::thread::spawn(move || {
        let frame_delay = DASHBOARD_ANIM_DURATION_MS / DASHBOARD_ANIM_FRAMES;
        for i in 1..=DASHBOARD_ANIM_FRAMES {
            std::thread::sleep(std::time::Duration::from_millis(frame_delay));
            if DASH_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            let t = i as f64 / DASHBOARD_ANIM_FRAMES as f64;
            let ease = t * t * t; // ease-in cubic
            let x = (from_x as f64 + (to_x as f64 - from_x as f64) * ease).round() as i32;
            let _ = win.set_position(Position::Physical(PhysicalPosition { x, y }));
        }
    });
}

/// 显示气泡：定位到「目标位右侧移出 offset」处（窗口原点仍在目标屏内）→ show → 原生滑入到目标位。
/// 只移出部分窗宽（非整窗到屏幕外真空区），故 macOS 不会把窗口误关联到主屏，规避双屏跨屏竞态。
/// 递增 DASH_GEN 并清 in-flight，取消任何待隐藏/滑出线程。
fn request_show<R: Runtime>(
    app: &AppHandle<R>,
    dashboard: &WebviewWindow<R>,
    tray_pos: Option<PhysicalPosition<f64>>,
) {
    let my_gen = DASH_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    DASH_HIDE_IN_FLIGHT.store(false, Ordering::SeqCst);

    let (target_x, target_y) = compute_dashboard_target(dashboard, tray_pos);
    let was_visible = dashboard.is_visible().unwrap_or(false);
    // 托盘点击只控制气泡显隐，绝不触碰主窗口 / 激活策略（不得调 show_dock_icon，
    // 否则 setActivationPolicy(.regular) 会经 Reopen 连带唤起主窗口）。
    if !was_visible {
        // 首次显示：起点 = 目标位向右移出 offset（半窗在屏、半窗在屏幕右缘外），show 后滑入。
        let start_x = target_x + slide_offset(dashboard);
        let _ = dashboard.set_position(Position::Physical(PhysicalPosition {
            x: start_x,
            y: target_y,
        }));
        let _ = dashboard.show();
        let _ = dashboard.set_focus();
        start_status_watch(app.clone());
        native_slide_in(dashboard, start_x, target_x, target_y, my_gen);
    } else {
        // 出场动画中途重开：从当前位置滑回目标位（窗口已可见，不重复 start_status_watch）。
        let current = dashboard
            .outer_position()
            .unwrap_or(PhysicalPosition { x: target_x, y: target_y });
        let _ = dashboard.set_focus();
        native_slide_in(dashboard, current.x, target_x, target_y, my_gen);
    }
}

/// 隐藏气泡：原生滑出动画 → 延迟后真正 hide + stop_status_watch。
/// 动画开始前先 emit HIDE_REQUESTED 通知前端复位瞬态 UI（此刻窗口仍可见，投递可靠）。
/// 严格一次隐藏由三重守卫保证：调用前 is_visible、swap 去重、唤醒后 gen 匹配。
fn request_hide<R: Runtime>(app: &AppHandle<R>, dashboard: &WebviewWindow<R>, animated: bool) {
    if !dashboard.is_visible().unwrap_or(false) {
        return;
    }
    if DASH_HIDE_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return;
    }
    let my_gen = DASH_GEN.fetch_add(1, Ordering::SeqCst) + 1;

    // 通知前端：气泡即将离场，立即复位瞬态 UI 状态（齿轮下拉等）。
    // 时序保证：上方守卫确保此刻窗口仍可见、webview 活跃，事件立即投递执行；
    // DASHBOARD_HIDE_DELAY_MS 后 hide() 挂起 webview 时，状态早已重置完毕。
    let _ = app.emit(events::EVT_DASHBOARD_HIDE_REQUESTED, ());

    if animated {
        // 原生滑出：从当前位置向右移出 offset（部分移出屏幕右缘，原点仍在目标屏），随后延迟 hide。
        let current = dashboard
            .outer_position()
            .unwrap_or(PhysicalPosition { x: 0, y: 0 });
        let (target_x, _) = compute_dashboard_target(dashboard, None);
        let end_x = target_x + slide_offset(dashboard);
        native_slide_out(dashboard, current.x, end_x, current.y, my_gen);
    }

    let delay = if animated { DASHBOARD_HIDE_DELAY_MS } else { 0 };
    let app2 = app.clone();
    let win2 = dashboard.clone();
    std::thread::spawn(move || {
        if delay > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay));
        }
        let _ = app2.run_on_main_thread(move || {
            if DASH_GEN.load(Ordering::SeqCst) == my_gen {
                if win2.is_visible().unwrap_or(false) {
                    let _ = win2.hide();
                    stop_status_watch();
                }
                DASH_HIDE_IN_FLIGHT.store(false, Ordering::SeqCst);
            }
        });
    });
}

/// 前端主动隐藏气泡（BottomBar「打开主窗口」路径）：原生滑出 + 严格配对 stop_status_watch。
#[tauri::command(rename_all = "snake_case")]
pub fn mole_dashboard_hide(app: AppHandle, window: WebviewWindow) {
    request_hide(&app, &window, true);
}

pub fn create_tray<R: Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<()> {
    // 托盘右键菜单：退出
    let quit_item = MenuItem::with_id(app, "tray_quit", "退出 MoleStudio", true, None::<&str>)?;
    let tray_menu = Menu::with_items(app, &[&quit_item])?;

    let app_for_quit = app.clone();
    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .show_menu_on_left_click(false)
        .menu(&tray_menu)
        .on_menu_event(move |_app, event| {
            if event.id.as_ref() == "tray_quit" {
                log::info!("[tray] quit requested from tray menu");
                stop_status_watch();
                if busy_state::is_busy() {
                    // 有任务在跑：通知前端弹确认框
                    log::warn!(
                        "[tray] exit blocked: {} task(s) running",
                        busy_state::busy_count()
                    );
                    let _ = app_for_quit.emit(events::EVT_DOCK_QUIT_REQUESTED, ());
                } else {
                    // 空闲状态：置标志 + 退出（ExitRequested 闸门会放行）
                    log::info!("[tray] idle, exiting via tray menu");
                    crate::macos_dock_quit::confirm_tray_exit();
                    app_for_quit.exit(0);
                }
            }
        })
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
                    // 右上角锚定：显示中且无隐藏在途 → 收起（播出场动画）；
                    // 隐藏中 或 出场动画进行中 → (重新)显示（request_show 会取消待隐藏线程）。
                    let visible = dashboard.is_visible().unwrap_or(false);
                    let hiding = DASH_HIDE_IN_FLIGHT.load(Ordering::SeqCst);
                    if visible && !hiding {
                        log::info!("[tray] hiding dashboard popover (animated)");
                        request_hide(app, &dashboard, true);
                    } else {
                        log::info!("[tray] showing dashboard popover at top-right");
                        request_show(app, &dashboard, Some(position));
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
        let app_for_focus = app.clone();
        dashboard.on_window_event(move |event| {
            if let WindowEvent::Focused(focused) = event {
                // 失焦收起：交给 request_hide 播出场动画 + 延迟隐藏（严格配对 stop_status_watch）。
                // 三重守卫防重复：确实失焦 && 窗口仍可见 && 无隐藏在途
                // （托盘点击/延迟 hide() 触发的 resignKey 都会再进本回调，in-flight 与 is_visible
                //   联合去重，避免 CONSUMER_COUNT 原子下溢、watch 线程永久无法停止）。
                if !focused
                    && dashboard_clone.is_visible().unwrap_or(false)
                    && !DASH_HIDE_IN_FLIGHT.load(Ordering::SeqCst)
                {
                    log::info!("[tray] dashboard lost focus, hiding popover (animated)");
                    request_hide(&app_for_focus, &dashboard_clone, true);
                }
            }
        });
    }

    Ok(())
}
