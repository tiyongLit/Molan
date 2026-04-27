//! Apple 特权 Helper **安装**双路径（仅 macOS 有意义）：
//! - **macOS 13+**：优先 `SMAppService`。
//! - **macOS 12 及以下**：`SMJobBless`（deprecated 但仍为旧系统可行路径）。
//!
//! 当前：**路由枚举 + sw_vers**。Bless/register 由原生子工程后续对接。

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegedHelperInstallRoute {
    SmaAppService,
    SmJobBless,
}

pub fn macos_semantic_version() -> Option<(u32, u32, u32)> {
    #[cfg(not(target_os = "macos"))]
    {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout);
        let trimmed = s.trim();
        let mut parts = trimmed.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        Some((major, minor, patch))
    }
}

pub fn recommended_helper_install_route() -> Option<PrivilegedHelperInstallRoute> {
    let (major, _, _) = macos_semantic_version()?;
    Some(if major >= 13 {
        PrivilegedHelperInstallRoute::SmaAppService
    } else {
        PrivilegedHelperInstallRoute::SmJobBless
    })
}
