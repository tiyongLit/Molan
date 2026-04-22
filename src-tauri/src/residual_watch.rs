//! 卸载残留自动检测：后台轮询 `~/.Trash/` 目录，检测新进入废纸篓的 `.app`，
//! 通知前端提示用户扫描残留文件（复用 `mole_orphan_scan`）。
//!
//! 设计原则：
//! - 零新依赖：复用 `std::fs::read_dir`（与 `collect_trash_size` 同口径）
//! - 常驻轮询：应用启动即开始，5s 间隔，开销极小（单次 readdir ≈ 0.1ms）
//! - 前端决策：Rust 侧始终 emit 事件，前端根据 `uninstall.autoDetectResidual` 设置决定是否展示
//! - 启动种子：首次轮询前收集已有 .app 填充 seen set，避免对已存在的废纸篓 app 误报

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::AppHandle;

use crate::events;

/// 轮询间隔（秒）。
const POLL_INTERVAL_SECS: u64 = 5;

/// 后台线程是否已启动（防止重复 spawn）。
static WATCHER_STARTED: AtomicBool = AtomicBool::new(false);

/// 启动卸载残留检测后台线程。
///
/// 应在 `lib.rs` 的 `.setup()` 中调用，整个应用生命周期仅调用一次。
/// 线程内部通过 `POLL_INTERVAL_SECS` 间隔轮询 `~/.Trash/`，
/// 检测到新 `.app` 时 emit `uninstall::residual-detected` 事件。
pub fn start_residual_watch(app: AppHandle) {
    if WATCHER_STARTED.swap(true, Ordering::SeqCst) {
        log::warn!("[residual_watch] already started, skipping");
        return;
    }

    std::thread::Builder::new()
        .name("residual-watch".into())
        .spawn(move || {
            let mut seen = HashSet::<String>::new();

            // 启动种子：首次收集已有 .app，避免对废纸篓中已存在的 app 误报
            if let Some(trash) = trash_dir() {
                if let Ok(rd) = trash.read_dir() {
                    for entry in rd.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.ends_with(".app") {
                            seen.insert(name);
                        }
                    }
                }
            }

            loop {
                std::thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));

                let Some(trash) = trash_dir() else { continue };
                let Ok(rd) = trash.read_dir() else { continue };

                // 收集当前废纸篓中所有 .app 名称
                let mut current = HashSet::<String>::new();
                for entry in rd.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.ends_with(".app") {
                        current.insert(name);
                    }
                }

                // 检测新增的 .app
                for name in &current {
                    if !seen.contains(name) {
                        let app_name = name.strip_suffix(".app").unwrap_or(name);
                        log::info!("[residual_watch] new .app detected: {}", app_name);
                        events::emit_residual_detected(&app, app_name);
                    }
                }

                seen = current;
            }
        })
        .expect("failed to spawn residual-watch thread");
}

/// 获取废纸篓目录路径。
fn trash_dir() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".Trash"))
}
