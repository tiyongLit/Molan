//! 更新（Updates）Tauri command 薄层。
//! 行为权威：Burrow `UpdatesView.swift`（UpdatesModel）。
//!
//! 数据流：
//!   mole_list_apps（已带 update_source，检测零网络）
//!     → 前端自动 surface：mole_updates_brew_outdated（每会话一次）
//!     → 用户点 Check：mole_updates_check（并发 6：appcast / iTunes，完成后必刷新 brew 行）
//!     → 行内 Update：mole_updates_apply（open 深链）
//!     → brew 行 Upgrade：mole_updates_brew_upgrade（流式，进度走 updates::brew-progress 事件）
//!
//! 隐私边界（对齐 Burrow SECURITY.md 网络故事）：appcast/iTunes 只在用户点击 Check 后请求；
//! brew outdated 是用户自己的工具，允许自动展示。

use serde::Serialize;

use crate::updates::brew::BrewOutdatedItem;
use crate::updates::brew::BrewUpgradeOutcome;
use crate::updates::detect::UpdateSource;

/// 单个 app 的检查结果（对齐 Burrow `AppUpdateItem` 的网络部分）。
#[derive(Serialize, Clone)]
pub struct AppCheckResult {
    pub path: String,
    /// "sparkle" | "app_store" | "electron"
    pub source: String,
    /// 请求失败 / electron（v1 只打徽标）→ null，行保留原状（静默）。
    pub latest_version: Option<String>,
    /// 仅 app_store
    pub page_url: Option<String>,
    /// 仅 app_store
    pub minimum_os: Option<String>,
}

/// mole_updates_check 返回。
#[derive(Serialize)]
pub struct UpdatesCheckResult {
    pub checked_at: String,
    /// 当前 macOS 版本（sw_vers -productVersion），供前端 OSUpdateGate 判断。
    pub running_os: String,
    pub apps: Vec<AppCheckResult>,
    pub brew: Vec<BrewOutdatedItem>,
}

/// 自动 surface：`brew outdated --json=v2`（120s 超时）。
/// brew 不存在 / 失败 / 超时 → 空数组。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_updates_brew_outdated() -> Vec<BrewOutdatedItem> {
    tauri::async_runtime::spawn_blocking(crate::updates::brew::brew_outdated)
        .await
        .unwrap_or_default()
}

/// 手动 Check：并发 6 检查每个 app（对齐 Burrow `checkNow` 的 TaskGroup 上限），
/// 全部完成后必刷新 brew 行（`checkNow` 同款语义）。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_updates_check(app_paths: Vec<String>) -> UpdatesCheckResult {
    let apps = tauri::async_runtime::spawn_blocking(move || check_concurrent(app_paths))
        .await
        .unwrap_or_default();
    let brew = tauri::async_runtime::spawn_blocking(crate::updates::brew::brew_outdated)
        .await
        .unwrap_or_default();
    UpdatesCheckResult {
        checked_at: chrono::Local::now().to_rfc3339(),
        running_os: running_os_version(),
        apps,
        brew,
    }
}

fn running_os_version() -> String {
    std::process::Command::new("sw_vers")
        .args(["-productVersion"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// 深链更新动作（对齐 Burrow `UpdatesModel.update`）：
/// - open_app：Sparkle/Electron → `open <app>`
/// - open_url：App Store 有 trackViewUrl → `open <url>`
/// - macappstore：App Store 无 pageURL → `open macappstore://showUpdatesPage`
#[tauri::command(rename_all = "snake_case")]
pub fn mole_updates_apply(action: String, target: Option<String>) -> Result<(), String> {
    let url = match action.as_str() {
        "open_app" => target.ok_or("缺少应用路径")?,
        "open_url" => target.ok_or("缺少链接")?,
        "macappstore" => "macappstore://showUpdatesPage".to_string(),
        other => return Err(format!("未知 action: {other}")),
    };
    std::process::Command::new("open")
        .arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("启动失败: {e}"))
}

/// brew 流式升级：`name` 为 None = 全部。单包 1800s / 全部 3600s 超时（对齐 Burrow）。
/// 进度短语逐行经 `updates::brew-progress` 事件推送。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_updates_brew_upgrade(
    app: tauri::AppHandle,
    name: Option<String>,
) -> Result<BrewUpgradeOutcome, String> {
    let timeout = if name.is_none() { 3600.0 } else { 1800.0 };
    let id = name.clone().unwrap_or_else(|| "brew".to_string());
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::updates::brew::brew_upgrade_streaming(name.as_deref(), timeout, |line| {
            if let Some(phrase) = crate::updates::brew::brew_progress_phrase(line) {
                crate::events::emit_updates_brew_progress(&handle, &id, &phrase);
            }
        })
    })
    .await
    .map_err(|e| format!("升级任务失败: {e}"))
}

// ── 内部实现 ──

/// 并发上限 6 跑单 app 检查（对齐 Burrow `checkNow` 的 inFlight 计数器法：
/// 完成一个补一个，不一次性铺满）。
fn check_concurrent(paths: Vec<String>) -> Vec<AppCheckResult> {
    std::thread::scope(|s| {
        let mut results = Vec::with_capacity(paths.len());
        let mut iter = paths.into_iter();
        let mut handles: Vec<std::thread::ScopedJoinHandle<AppCheckResult>> = Vec::new();
        loop {
            while handles.len() < 6 {
                match iter.next() {
                    Some(p) => handles.push(s.spawn(move || check_one(&p))),
                    None => break,
                }
            }
            if handles.is_empty() {
                break;
            }
            // 等最早入队的完成，再补新的（保并发上限）。
            let done = handles.remove(0);
            match done.join() {
                Ok(r) => results.push(r),
                Err(_) => {}
            }
        }
        results
    })
}

/// 单个 app 的网络检查（对齐 Burrow `check(_:)`）：
/// sparkle → feedURL + fetch + parseAppcast；app_store → iTunes lookup；
/// electron → 不请求（自带更新器，v1 只打徽标）。任何失败静默 → 版本保持 None。
fn check_one(path: &str) -> AppCheckResult {
    let source = crate::updates::detect::detect_update_source(path);
    let mut r = AppCheckResult {
        path: path.to_string(),
        source: source.map(|s| s.as_str().to_string()).unwrap_or_default(),
        latest_version: None,
        page_url: None,
        minimum_os: None,
    };
    match source {
        Some(UpdateSource::Sparkle) => {
            let feed = crate::updates::detect::feed_url(path);
            if !feed.is_empty() {
                if let Some(xml) = fetch_text(&feed) {
                    r.latest_version = crate::updates::appcast::parse_appcast(&xml);
                }
            }
        }
        Some(UpdateSource::AppStore) => {
            let bundle_id = read_bundle_id(path);
            if !bundle_id.is_empty() {
                let url = crate::updates::itunes::itunes_lookup_url(&bundle_id);
                if let Some(json) = fetch_text(&url) {
                    if let Some(m) = crate::updates::itunes::parse_itunes_lookup(&json) {
                        r.latest_version = Some(m.version);
                        r.page_url = m.page_url;
                        r.minimum_os = m.minimum_os_version;
                    }
                }
            }
        }
        Some(UpdateSource::Electron) | None => {}
    }
    r
}

/// 网络请求：curl 子进程（网络层规范：Rust 命令内执行，前端不用任何 HTTP 库）。
/// `-sSL`：静默 + 跟随重定向（appcast 常 302）；`--max-time 10` 对齐 Burrow fetch 超时；
/// curl 无磁盘缓存，天然满足 Burrow `reloadIgnoringLocalCacheData`。
fn fetch_text(url: &str) -> Option<String> {
    crate::core::timeout::run_with_timeout_capture_lossy(
        15.0,
        "curl",
        &["-sSL", "--max-time", "10", url],
    )
}

fn read_bundle_id(app_path: &str) -> String {
    let plist_path = format!("{app_path}/Contents/Info.plist");
    if !std::path::Path::new(&plist_path).is_file() {
        return String::new();
    }
    match plist::Value::from_file(&plist_path)
        .ok()
        .and_then(|v| v.into_dictionary())
        .and_then(|mut d| d.remove("CFBundleIdentifier"))
    {
        Some(plist::Value::String(s)) => s,
        _ => String::new(),
    }
}
