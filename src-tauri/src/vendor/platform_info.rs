//! 平台安装形态检测：统一出口。
//!
//! 将分散的渠道判断（运行时沙盒检测 / MAS receipt）
//! 收敛为 `PlatformInfo` 单例。未来所有"官网版 vs MAS 版"的分支逻辑，
//! **只准**查 `PlatformInfo::current().channel`，禁止再散落
//! `is_mas_build()` 之类的裸判断。

use serde::Serialize;
use std::env;
use std::path::PathBuf;
use std::sync::OnceLock;

// ── 枚举 ─

/// 分发渠道。新渠道（如 Homebrew cask）只扩这里 + `detect_channel`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DistributionChannel {
    /// 官网直发（DMG / zip 下载）
    Direct,
    /// Mac App Store（沙盒）
    MacAppStore,
}

/// 运行形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallKind {
    /// 正式 .app 包
    MacosAppBundle,
    /// target/debug 或找不到 .app 结构
    Unknown,
}

// ── 主结构体 ──

/// 运行时的平台安装信息（OnceLock 缓存，首次调用计算，之后零开销）。
#[derive(Debug, Clone, Serialize)]
pub struct PlatformInfo {
    /// 分发渠道
    pub channel: DistributionChannel,
    /// 运行形态
    pub install_kind: InstallKind,
    /// 当前版本号
    pub app_version: String,
    /// 当前 .app bundle 路径（如果找到）
    pub current_app_bundle: Option<String>,
    /// 是否运行在 App Sandbox 中
    pub sandboxed: bool,
}

static CACHE: OnceLock<PlatformInfo> = OnceLock::new();

/// 全库统一出口：渠道分支只准查这里。
///
/// ```text
/// if platform_info::current().channel == DistributionChannel::Direct {
///     // 官网版特有逻辑
/// }
/// ```
pub fn current() -> &'static PlatformInfo {
    CACHE.get_or_init(compute)
}

fn compute() -> PlatformInfo {
    let sandboxed = is_sandboxed();
    let channel = detect_channel(sandboxed);
    let exe = env::current_exe().ok();
    let app_bundle = exe.as_deref().and_then(find_macos_app_bundle);
    let install_kind = if app_bundle.is_some() {
        InstallKind::MacosAppBundle
    } else {
        InstallKind::Unknown
    };

    let app_version = env!("CARGO_PKG_VERSION").to_string();

    PlatformInfo {
        channel,
        install_kind,
        app_version,
        current_app_bundle: app_bundle.map(|p| p.to_string_lossy().into_owned()),
        sandboxed,
    }
}

// ── 辅助函数 ──

/// 运行时检测 App Sandbox（`APP_SANDBOX_CONTAINER_ID` 环境变量）。
#[cfg(target_os = "macos")]
fn is_sandboxed() -> bool {
    env::var("APP_SANDBOX_CONTAINER_ID").is_ok()
}

#[cfg(not(target_os = "macos"))]
fn is_sandboxed() -> bool {
    false
}

/// 检测分发渠道。
///
/// 优先级：
/// 1. 运行时沙盒环境变量 → MAS
/// 2. 其余 → Direct
fn detect_channel(sandboxed: bool) -> DistributionChannel {
    if sandboxed {
        DistributionChannel::MacAppStore
    } else {
        DistributionChannel::Direct
    }
}

/// 从可执行文件路径向上查找 .app bundle 根目录。
fn find_macos_app_bundle(exe: &std::path::Path) -> Option<PathBuf> {
    let mut cur = exe.parent();
    while let Some(p) = cur {
        if p.extension().and_then(|e| e.to_str()) == Some("app") {
            return Some(p.to_path_buf());
        }
        cur = p.parent();
    }
    None
}
