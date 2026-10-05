//! 更新（Updates）Tauri command 薄层。
//! 行为基线：见同目录 `updates.md` 的「行为基线（权威语义）」章节。
//!
//! 数据流：
//!   mole_list_apps（已带 update_source，检测零网络）
//!     → 前端自动 surface：mole_updates_brew_outdated（每会话一次）
//!     → 用户点 Check：mole_updates_check（并发 6：appcast / iTunes，完成后必刷新 brew 行）
//!     → 行内 Update：mole_updates_apply（open 深链）
//!     → brew 行 Upgrade：mole_updates_brew_upgrade（流式，进度走 updates::brew-progress 事件）
//!
//! 隐私边界：appcast/iTunes 只在用户点击 Check 后请求；
//! brew outdated 是用户自己的工具，允许自动展示。

use serde::Serialize;

use crate::updates::brew::BrewOutdatedItem;
use crate::updates::brew::BrewUpgradeOutcome;
use crate::updates::detect::UpdateSource;

/// 单个 app 的检查结果（网络检查部分）。
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
    /// 当前 macOS 版本（sw_vers -productVersion），供前端做系统兼容门判断。
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

/// 手动 Check：并发 6 检查每个 app（受控并发上限），
/// 全部完成后必刷新 brew 行。
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

/// 深链更新动作（对齐 Burrow 的 `.handedOff` 语义：交接必须「可确认」）：
/// - open_app：Sparkle/Electron → `open <app>`
/// - open_url：App Store 有 trackViewUrl → `open <url>`
/// - macappstore：App Store 无 pageURL → `open macappstore://showUpdatesPage`
///
/// 原实现 spawn 后不看退出码：open 失败（URL 无效 / 应用不存在 / scheme
/// 不被支持）会被静默成功化，前端只剩「点了没反应」。改为等待退出码并
/// 回传 stderr，让前端能给出失败反馈；同时走绝对路径（GUI 环境不依赖 PATH）。
/// spawn_blocking 隔离等待，避免阻塞 IPC 线程。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_updates_apply(action: String, target: Option<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let url = match action.as_str() {
            "open_app" => target.ok_or("缺少应用路径")?,
            "open_url" => target.ok_or("缺少链接")?,
            "macappstore" => "macappstore://showUpdatesPage".to_string(),
            other => return Err(format!("未知 action: {other}")),
        };
        let out = std::process::Command::new("/usr/bin/open")
            .arg(&url)
            .output()
            .map_err(|e| format!("启动失败: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(format!("打开失败: {}", err.trim()))
        }
    })
    .await
    .map_err(|e| format!("打开任务失败: {e}"))?
}

/// brew 流式升级：`name` 为 None = 全部。单包 1800s / 全部 3600s 超时。
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

// ── 第三方 App 原地安装（更新执行引擎，P0：Sparkle 2 + zip） ──

/// `mole_updates_install` 返回摘要（前端持有，用于展示新版本号）。
#[derive(Serialize)]
pub struct InstallPrepareResult {
    pub app_path: String,
    pub new_version: String,
    pub bundle_id: String,
}

/// 原地安装·prepare：拉 feed → 下载 → 五道门验证 → 暂存就绪。
/// 进度经 `updates::install-progress` 事件推送；就绪后等待 `_commit`。
/// 失败（含"不支持原地更新"）时不触碰系统文件——由前端决定回退深链。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_updates_install(
    app: tauri::AppHandle,
    app_path: String,
    current_version: String,
) -> Result<InstallPrepareResult, String> {
    use crate::updates::engine::session::{self, InstallStage};

    let _slot = session::InstallSlotGuard::acquire()?;
    let handle = app.clone();
    let path_for_task = app_path.clone();
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        let mut emit_stage = |stage: InstallStage, bytes: Option<u64>| {
            crate::events::emit_updates_install_progress(
                &handle,
                crate::events::InstallProgressPayload {
                    app_path: path_for_task.clone(),
                    stage: session::stage_str(stage).to_string(),
                    bytes,
                    message: None,
                },
            );
        };
        session::prepare_install(&path_for_task, &current_version, &mut emit_stage)
    })
    .await
    .map_err(|e| format!("安装任务失败: {e}"))??;

    let summary = InstallPrepareResult {
        app_path: prepared.app_path.clone(),
        new_version: prepared.new_version.clone(),
        bundle_id: prepared.bundle_id.clone(),
    };
    session::stash_session(prepared);
    Ok(summary)
}

/// 原地安装·commit：退出目标 App → 替换 → 复验 → 重启（失败自动回滚）。
/// 需先经 `mole_updates_install` 完成下载与验证。失败时暂存保留可重试。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_updates_install_commit(
    app: tauri::AppHandle,
    app_path: String,
) -> Result<(), String> {
    use crate::updates::engine::session::{self, InstallStage};

    let prepared = session::take_session(&app_path)
        .ok_or_else(|| "没有待安装的会话（请先下载并验证更新）".to_string())?;
    let _slot = session::InstallSlotGuard::acquire()?;
    let handle = app.clone();
    let path_for_task = app_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut emit_stage = |stage: InstallStage, bytes: Option<u64>| {
            crate::events::emit_updates_install_progress(
                &handle,
                crate::events::InstallProgressPayload {
                    app_path: path_for_task.clone(),
                    stage: session::stage_str(stage).to_string(),
                    bytes,
                    message: None,
                },
            );
        };
        session::commit_prepared(prepared, &mut emit_stage)
    })
    .await
    .map_err(|e| format!("安装任务失败: {e}"))?
}

/// 原地安装·cancel：丢弃待安装会话的暂存（不触碰系统文件）。
#[tauri::command(rename_all = "snake_case")]
pub fn mole_updates_install_cancel(app_path: String) -> Result<(), String> {
    use crate::updates::engine::session;
    if let Some(prepared) = session::take_session(&app_path) {
        session::cancel_prepared(&prepared);
    }
    Ok(())
}

// ── 内部实现 ──

/// 并发上限 6 跑单 app 检查（inFlight 计数器法：
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

/// 单个 app 的网络检查：
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
            let bundle_id =
                crate::core::bundle_id_anchor::read_bundle_id_of_app(std::path::Path::new(path))
                    .unwrap_or_default();
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
/// `-sSL`：静默 + 跟随重定向（appcast 常 302）；`--max-time 10` 为 fetch 超时；
/// curl 无磁盘缓存，天然满足「忽略本地缓存」语义。
fn fetch_text(url: &str) -> Option<String> {
    crate::core::timeout::run_with_timeout_capture_lossy(
        15.0,
        "curl",
        &["-sSL", "--max-time", "10", url],
    )
}
