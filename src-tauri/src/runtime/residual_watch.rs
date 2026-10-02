//! 卸载残留自动检测：后台轮询 `~/.Trash/` 目录，检测新进入废纸篓的 `.app`，
//! 通过 macOS 原生系统通知提醒用户（点击通知 → 唤起主窗 → 卸载页定向扫描残留），
//! 通知不可用时降级为事件（卸载页内提示条兜底）。
//!
//! 设计原则：
//! - 常驻轮询：应用启动即开始，5s 间隔，开销极小（单次 readdir ≈ 0.1ms）
//! - 后端决策开关：读取 settings.json 的 `uninstall.autoDetectResidual`，
//!   关闭时完全静默；无论开关与否 seen 都更新（防开关切换后积压补发）
//! - 判重口径：`文件名|bundleId`（bundleId 读自 Info.plist，同名不同源不误判）
//! - 启动种子：首次轮询前收集已有 .app 填充 seen set，避免对已存在的废纸篓 app 误报
//! - 送达与点击回调：见 `lib/platform/macos_notifications.rs`

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::events;

/// 轮询间隔（秒）。
const POLL_INTERVAL_SECS: u64 = 5;

/// 后台线程是否已启动（防止重复 spawn）。
static WATCHER_STARTED: AtomicBool = AtomicBool::new(false);

/// 待前端消费的"点击通知 → 定向扫描"请求快照。
///
/// 由 `macos_notifications` 的点击回调写入，前端（layout）经
/// `mole_residual_take_pending` 取走并跳转移交——快照是事实源，事件仅作到达信号。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidualTarget {
    /// 进入废纸篓的 .app 名称（不含 .app 后缀）。
    pub app_name: String,
    /// 从 Info.plist 解析的 bundle id（解析失败为 None）。
    pub bundle_id: Option<String>,
    /// 检测/点击时间（Unix 秒）。
    pub detected_at: i64,
}

/// pending 单槽（新请求覆盖旧请求）。
static PENDING: Mutex<Option<ResidualTarget>> = Mutex::new(None);

/// 写入 pending（由通知点击回调调用）。
pub fn set_pending(target: ResidualTarget) {
    *PENDING.lock().unwrap() = Some(target);
}

/// 取走 pending（take 语义：消费即清空，重复调用返回 None）。
pub fn take_pending() -> Option<ResidualTarget> {
    PENDING.lock().unwrap().take()
}

/// 启动卸载残留检测后台线程。
///
/// 应在 `lib.rs` 的 `.setup()` 中调用，整个应用生命周期仅调用一次。
/// 线程内部通过 `POLL_INTERVAL_SECS` 间隔轮询 `~/.Trash/`，
/// 检测到新 `.app` 时按设置决定系统通知/事件兜底。
pub fn start_residual_watch(app: AppHandle) {
    if WATCHER_STARTED.swap(true, Ordering::SeqCst) {
        log::warn!("[residual_watch] already started, skipping");
        return;
    }

    std::thread::Builder::new()
        .name("residual-watch".into())
        .spawn(move || {
            // 启动种子：首次轮询前收集已有 .app，避免对废纸篓中已存在的 app 误报
            let mut seen = HashSet::<String>::new();
            if let Some(trash) = trash_dir() {
                seen = collect_trash_apps(&trash).into_keys().collect();
            }

            loop {
                std::thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));

                let Some(trash) = trash_dir() else { continue };
                let current = collect_trash_apps(&trash);

                // 检测新增的 .app（判重 key = "文件名|bundleId"）
                for (key, (file_name, bundle_id)) in &current {
                    if seen.contains(key) {
                        continue;
                    }
                    let app_name = file_name.strip_suffix(".app").unwrap_or(file_name);
                    log::info!("[residual_watch] new .app detected: {app_name} (bundle: {bundle_id:?})");
                    notify_new_app(&app, app_name, bundle_id.as_deref());
                }

                seen = current.into_keys().collect();
            }
        })
        .expect("failed to spawn residual-watch thread");
}

/// 处理检测到的新 .app：读设置 → 主通道系统通知；不可用时事件兜底。
fn notify_new_app(app: &AppHandle, app_name: &str, bundle_id: Option<&str>) {
    let settings = read_settings_json(app);

    // 开关关闭 → 完全静默（不打扰；seen 已在主循环统一更新）
    if let Some(document) = &settings {
        if !auto_detect_enabled(document) {
            log::info!("[residual_watch] autoDetectResidual disabled; silent skip: {app_name}");
            return;
        }
    }

    let lang = settings
        .as_ref()
        .map(read_language)
        .unwrap_or_else(|| "zh-CN".to_string());

    // 主通道：macOS 原生通知（成功即送达，不再发事件，避免双通道打扰）
    #[cfg(target_os = "macos")]
    let delivered =
        crate::platform::macos_notifications::send_residual_notification(app_name, bundle_id, &lang);
    #[cfg(not(target_os = "macos"))]
    let delivered = false;

    // 兜底通道：通知不可用（dev / 未授权 / 发送失败）→ 事件（卸载页提示条）
    if !delivered {
        events::emit_residual_detected(app, app_name, bundle_id);
    }
}

/// 获取废纸篓目录路径。
fn trash_dir() -> Option<PathBuf> {
    crate::core::base::home_dir_opt().map(|h| h.join(".Trash"))
}

/// 收集废纸篓中的 .app：key = `"文件名|bundleId"`（判重口径），
/// value = (文件名, bundleId)。删除项不会出现在结果中。
fn collect_trash_apps(trash: &Path) -> HashMap<String, (String, Option<String>)> {
    let mut apps = HashMap::new();
    let Ok(read_dir) = trash.read_dir() else {
        return apps;
    };
    for entry in read_dir.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.ends_with(".app") {
            continue;
        }
        let bundle_id = crate::core::bundle_id_anchor::read_bundle_id_of_app(&entry.path());
        let key = format!("{file_name}|{}", bundle_id.as_deref().unwrap_or(""));
        apps.insert(key, (file_name, bundle_id));
    }
    apps
}

/// 读取 settings.json（缺失/损坏返回 None，调用方按缺省值处理）。
fn read_settings_json(app: &AppHandle) -> Option<serde_json::Value> {
    let path = app.path().app_data_dir().ok()?.join("settings.json");
    let data = std::fs::read(path).ok()?;
    serde_json::from_slice(&data).ok()
}

/// `uninstall.autoDetectResidual` 开关（缺失/非法值缺省开启）。
fn auto_detect_enabled(document: &serde_json::Value) -> bool {
    document
        .get("uninstall")
        .and_then(|uninstall| uninstall.get("autoDetectResidual"))
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

/// 界面语言（缺失/非法值缺省 zh-CN），用于通知文案。
fn read_language(document: &serde_json::Value) -> String {
    document
        .get("language")
        .and_then(|value| value.as_str())
        .unwrap_or("zh-CN")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_detect_default_on() {
        assert!(auto_detect_enabled(&serde_json::json!({})));
        assert!(auto_detect_enabled(
            &serde_json::json!({"uninstall": {"autoDetectResidual": true}})
        ));
        assert!(!auto_detect_enabled(
            &serde_json::json!({"uninstall": {"autoDetectResidual": false}})
        ));
    }

    #[test]
    fn language_fallback() {
        assert_eq!(
            read_language(&serde_json::json!({"language": "en-US"})),
            "en-US"
        );
        assert_eq!(read_language(&serde_json::json!({})), "zh-CN");
    }

    #[test]
    fn pending_take_is_atomic() {
        set_pending(ResidualTarget {
            app_name: "Foo".to_string(),
            bundle_id: Some("com.example.foo".to_string()),
            detected_at: 1,
        });
        let taken = take_pending();
        assert_eq!(
            taken.map(|target| (target.app_name, target.bundle_id)),
            Some(("Foo".to_string(), Some("com.example.foo".to_string())))
        );
        // 消费即清空
        assert!(take_pending().is_none());
    }
}
