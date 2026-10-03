//! Molan 应用版本检查（App Version）业务逻辑层。
//!
//! 与 `controllers/app_version.rs`（薄层 Tauri command 入口）分离：
//! - 本模块包含所有业务逻辑：版本检查、下载安装、MAS 检测、App Store 引导。
//! - Controller 只做参数解析、调用本模块、发射事件。
//!
//! ## 双版本策略
//!
//! | 版本 | feature | 更新源 | 检测方式 | 安装方式 |
//! |------|---------|--------|---------|---------|
//! | 官网完整版 | `full` | Gitee 主 + GitHub 备 | `GET latest.json` | tauri-plugin-updater |
//! | MAS 精简版 | `mas` | App Store | `_MASReceipt` 检测 | `open macappstore://...` |
//!
//! ## 对齐 Mole CLI update.sh
//!
//! 本模块保留了 `update.sh` 中与 GUI 相关的 brew 过期检测逻辑
//! (`populate_brew_update_counts_if_unset` / `format_brew_update_detail` 等)。
//! 文件锁 / sudo 管理 / install.sh 下载执行等 CLI 特有逻辑不移植，
//! 由 tauri-plugin-updater + MAS sandbox 替代。

use serde::Serialize;
use tauri::AppHandle;

// ──────────────────────────── 类型定义 ────────────────────────────

/// 版本检查结果，前后端共享。
#[derive(Serialize, Clone, Debug)]
pub struct AppVersionCheckResult {
    /// 是否有新版本
    pub available: bool,
    /// 更新源: "gitee" | "github" | "app_store" | "none"
    pub source: String,
    /// 远程最新版本 (available=true 时)
    pub latest_version: Option<String>,
    /// 当前运行版本
    pub current_version: String,
    /// 更新日志/Release Notes (Markdown)
    pub release_notes: Option<String>,
    /// 下载 URL (官网版)
    pub download_url: Option<String>,
    /// 发布日期 (ISO8601)
    pub published_at: Option<String>,
}

/// 下载进度回调。
///
/// 参数: `(chunk_length, content_length)`
/// - `chunk_length`: 当前分块结束时的累计字节数
/// - `content_length`: 总字节数（可能为 None）
pub type ProgressCallback = dyn Fn(usize, Option<u64>) + Send + Sync;

// ──────────────────────────── MAS 检测 ────────────────────────────

/// 检测当前构建是否为 Mac App Store 版本。
///
/// 通过检查 `.app/Contents/_MASReceipt/receipt` 文件是否存在来判断。
/// 运行时检测而非编译期 feature gate，对 MAS 审核更友好。
#[cfg(target_os = "macos")]
pub fn is_mas_build() -> bool {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };
    // .app/Contents/MacOS/binary → .app/Contents/_MASReceipt/receipt
    let receipt = exe
        .parent() // MacOS/
        .and_then(|p| p.parent()) // Contents/
        .map(|contents| contents.join("_MASReceipt").join("receipt"));
    receipt.map_or(false, |p| p.exists())
}

#[cfg(not(target_os = "macos"))]
pub fn is_mas_build() -> bool {
    false
}

// ──────────────────────────── 版本检查 ────────────────────────────

/// 检查 Molan 自身是否有新版本。
///
/// - 官网版：使用 tauri-plugin-updater 查询配置好的 endpoints (Gitee → GitHub)。
///   插件自动按数组顺序 fallback。
/// - MAS 版：返回 `source="app_store"`，前端引导 App Store。
pub async fn check_for_update(app: &AppHandle) -> Result<AppVersionCheckResult, String> {
    let current_version = app.package_info().version.to_string();

    // MAS 版：引导 App Store
    if is_mas_build() {
        return Ok(AppVersionCheckResult {
            available: false,
            source: "app_store".to_string(),
            latest_version: None,
            current_version,
            release_notes: None,
            download_url: None,
            published_at: None,
        });
    }

    // 官网版：使用 tauri-plugin-updater 检查
    use tauri_plugin_updater::UpdaterExt;

    let updater = app
        .updater()
        .map_err(|e| format!("初始化 updater 失败: {e}"))?;

    match updater.check().await {
        Ok(Some(update)) => {
            let source = update.download_url.host_str().unwrap_or("").to_string();
            let source_label = if source.contains("gitee") {
                "gitee"
            } else if source.contains("github") {
                "github"
            } else {
                "other"
            };

            Ok(AppVersionCheckResult {
                available: true,
                source: source_label.to_string(),
                latest_version: Some(update.version.clone()),
                current_version,
                release_notes: update.body.clone(),
                download_url: Some(update.download_url.to_string()),
                published_at: update.date.map(|d| d.to_string()),
            })
        }
        Ok(None) => Ok(AppVersionCheckResult {
            available: false,
            source: "none".to_string(),
            latest_version: None,
            current_version,
            release_notes: None,
            download_url: None,
            published_at: None,
        }),
        Err(e) => Err(format!("检查更新失败: {e}")),
    }
}

// ──────────────────────────── 下载安装 ────────────────────────────

/// 执行更新下载安装（仅官网版）。
///
/// 1. 先检查是否有可用更新
/// 2. 通过 `download_and_install` 下载并安装
/// 3. `on_progress` 回调在每个 chunk 下载时触发
/// 4. `on_complete` 回调在下载完成、安装开始前触发
/// 5. 安装完成后请求应用重启
pub async fn perform_update<F1, F2>(
    app: &AppHandle,
    on_progress: F1,
    on_complete: F2,
) -> Result<(), String>
where
    F1: Fn(usize, Option<u64>) + Send + Sync + 'static,
    F2: FnOnce() + Send + Sync + 'static,
{
    use tauri_plugin_updater::UpdaterExt;

    let updater = app
        .updater()
        .map_err(|e| format!("初始化 updater 失败: {e}"))?;

    let update = match updater.check().await {
        Ok(Some(u)) => u,
        Ok(None) => return Err("没有可用的更新".to_string()),
        Err(e) => return Err(format!("检查更新失败: {e}")),
    };

    update
        .download_and_install(
            move |chunk_length, content_length| {
                on_progress(chunk_length, content_length);
            },
            move || {
                on_complete();
            },
        )
        .await
        .map_err(|e| format!("下载安装失败: {e}"))?;

    // 请求重启
    app.restart();

    Ok(())
}

// ──────────────────────────── App Store 引导 ────────────────────────────

/// MAS 版：打开 App Store 更新页面。
pub fn open_appstore() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("macappstore://showUpdatesPage")
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("打开 App Store 失败: {e}"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("App Store 仅支持 macOS".to_string())
    }
}

// ──────────────────────────── Brew 过期检测（对齐 update.sh） ────────────────────────────

use crate::core::base::home_dir;
use crate::core::timeout::run_with_timeout_capture;

/// 对齐 SH 中 `reset_mole_cache`(定义在 `lib/check/all.sh:219-221`)的语义:
/// 仅清空 `$CACHE_DIR/mole_version`。
pub fn reset_mole_cache() {
    let home = home_dir();
    let _ = std::fs::remove_file(format!("{home}/.cache/mole/mole_version"));
}

fn read_count_env(name: &str) -> Option<u32> {
    std::env::var(name)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

fn brew_available() -> bool {
    run_with_timeout_capture(2.0, "which", &["brew"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

fn count_outdated_lines(out: &str) -> u32 {
    out.lines().filter(|l| !l.trim().is_empty()).count() as u32
}

/// 对齐 `update.sh:38-65` 中的 `populate_brew_update_counts_if_unset`。
///
/// 当任意一个 BREW_* 环境变量未设置时,运行 `brew outdated` 探测,并把
/// `BREW_FORMULA_OUTDATED_COUNT` / `BREW_CASK_OUTDATED_COUNT` / `BREW_OUTDATED_COUNT`
/// 写回到环境变量。
pub fn populate_brew_update_counts_if_unset() {
    let need_probe = std::env::var("BREW_OUTDATED_COUNT").is_err()
        || std::env::var("BREW_FORMULA_OUTDATED_COUNT").is_err()
        || std::env::var("BREW_CASK_OUTDATED_COUNT").is_err();
    if !need_probe {
        return;
    }

    let mut formula_count = read_count_env("BREW_FORMULA_OUTDATED_COUNT").unwrap_or(0);
    let mut cask_count = read_count_env("BREW_CASK_OUTDATED_COUNT").unwrap_or(0);

    if brew_available() {
        if let Some(out) =
            run_with_timeout_capture(8.0, "brew", &["outdated", "--formula", "--quiet"])
        {
            formula_count = count_outdated_lines(&out);
        }
        if let Some(out) = run_with_timeout_capture(8.0, "brew", &["outdated", "--cask", "--quiet"])
        {
            cask_count = count_outdated_lines(&out);
        }
    }

    let total = formula_count + cask_count;
    unsafe {
        std::env::set_var("BREW_FORMULA_OUTDATED_COUNT", formula_count.to_string());
        std::env::set_var("BREW_CASK_OUTDATED_COUNT", cask_count.to_string());
        std::env::set_var("BREW_OUTDATED_COUNT", total.to_string());
    }
}

/// 对齐 `update.sh:8-29` 中的 `format_brew_update_detail`。
///
/// 注意:不带 `Homebrew, ` 前缀(前缀属于 label,见 `format_brew_update_label`)。
/// 输入读自 `BREW_OUTDATED_COUNT` / `BREW_FORMULA_OUTDATED_COUNT` /
/// `BREW_CASK_OUTDATED_COUNT` 环境变量。
pub fn format_brew_update_detail() -> String {
    let total = read_count_env("BREW_OUTDATED_COUNT").unwrap_or(0);
    if total == 0 {
        return String::new();
    }
    let formulas = read_count_env("BREW_FORMULA_OUTDATED_COUNT").unwrap_or(0);
    let casks = read_count_env("BREW_CASK_OUTDATED_COUNT").unwrap_or(0);

    let mut details: Vec<String> = Vec::new();
    if formulas > 0 {
        details.push(format!("{formulas} formula"));
    }
    if casks > 0 {
        details.push(format!("{casks} cask"));
    }

    if details.is_empty() {
        format!("{total} updates")
    } else {
        details.join(", ")
    }
}

/// 对齐 `update.sh:32-36` 中的 `format_brew_update_label`,保留旧调用方/测试兼容。
pub fn format_brew_update_label() -> String {
    let detail = format_brew_update_detail();
    if detail.is_empty() {
        String::new()
    } else {
        format!("Homebrew, {detail}")
    }
}

/// 对齐 `update.sh:67-76` 中的 `brew_has_outdated`。
/// `kind = "cask"` 仅看 cask,其它默认看全部 outdated。
pub fn brew_has_outdated(kind: &str) -> bool {
    if !brew_available() {
        return false;
    }
    let out = if kind == "cask" {
        run_with_timeout_capture(8.0, "brew", &["outdated", "--cask", "--quiet"])
    } else {
        run_with_timeout_capture(8.0, "brew", &["outdated", "--quiet"])
    };
    out.map(|s| s.lines().any(|l| !l.trim().is_empty()))
        .unwrap_or(false)
}

/// 对齐 `update.sh:80-130` 中的 `ask_for_updates`。
///
/// SH 端会通过 `read_key` 读取回车 / ESC 来确认 Mole 更新;GUI 端无 TTY,
/// 这里只判断"是否存在任意一种待更新源"——具体确认行为由前端弹窗承担。
pub fn ask_for_updates() -> bool {
    populate_brew_update_counts_if_unset();

    let mut has_updates = false;
    if read_count_env("BREW_OUTDATED_COUNT").unwrap_or(0) > 0 {
        has_updates = true;
    }
    if read_count_env("APPSTORE_UPDATE_COUNT").unwrap_or(0) > 0 {
        has_updates = true;
    }
    if std::env::var("MACOS_UPDATE_AVAILABLE").unwrap_or_default() == "true" {
        has_updates = true;
    }
    if std::env::var("MOLE_UPDATE_AVAILABLE").unwrap_or_default() == "true" {
        has_updates = true;
    }

    if !has_updates {
        return false;
    }

    // SH 第 104-117 行:仅 Mole 走交互确认。GUI 没有 TTY,直接返回 false,
    // 由前端单独通过 Tauri command 触发 perform_update。
    false
}

/// 对齐 `update.sh:134-169` 中的 `perform_updates`(Mole CLI 版)。
///
/// GUI 版的 Molan 自更新已迁移到 `check_for_update` + `perform_update`
/// (基于 tauri-plugin-updater)。本函数保留用于 Mole CLI 兼容性检测场景。
/// 返回 true 表示存在更新且全部成功;无更新或失败均返回 false。
pub fn perform_cli_updates() -> bool {
    let mut updated_count = 0u32;
    let mut total_count = 0u32;

    if std::env::var("MOLE_UPDATE_AVAILABLE").unwrap_or_default() == "true" {
        total_count = 1;

        let mole_bin = run_with_timeout_capture(2.0, "command", &["-v", "mole"])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && std::path::Path::new(&s).exists());

        if let Some(bin) = mole_bin {
            let out = run_with_timeout_capture(60.0, &bin, &["update"]).unwrap_or_default();
            if out.contains("Updated") || out.contains("latest version") {
                reset_mole_cache();
                updated_count += 1;
            }
        }
    }

    if total_count == 0 {
        return true;
    }
    updated_count == total_count
}
