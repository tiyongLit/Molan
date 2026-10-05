//! 目标 App 的退出与重启。
//!
//! - **退出**：`NSRunningApplication.terminate()` 优雅请求 + 轮询等待（对齐
//!   Burrow 的 terminate + deadline 纪律；**不强制 kill**——未退出则中止安装，
//!   绝不硬杀用户进程）；
//! - **重启**：走 `/usr/bin/open`（与深链交接通道同一工具，已验证等待真实
//!   退出码语义）。

use std::time::{Duration, Instant};

/// 请求 `bundle_id` 对应的全部运行中实例退出，并等待至多 `timeout`。
/// - 未在运行 → 直接 `Ok`（无需退出，可直接替换）；
/// - 超时未全部退出 → `Err`（调用方中止安装，不触碰文件系统）。
#[cfg(target_os = "macos")]
pub fn quit_and_wait(bundle_id: &str, timeout: Duration) -> Result<(), String> {
    use objc2_app_kit::NSRunningApplication;
    use objc2_foundation::NSString;

    let bid = NSString::from_str(bundle_id);

    // 首次遍历：发送优雅退出请求（没有实例则直接通过）。
    let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(&bid);
    if apps.iter().next().is_none() {
        return Ok(());
    }
    for app in apps.iter() {
        app.terminate();
    }

    // 轮询等待全部退出。
    let deadline = Instant::now() + timeout;
    loop {
        let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(&bid);
        let alive = apps.iter().any(|app| !app.isTerminated());
        if !alive {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "目标应用未在 {timeout:?} 内退出（bundle id: {bundle_id}），已中止安装"
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(not(target_os = "macos"))]
pub fn quit_and_wait(_bundle_id: &str, _timeout: Duration) -> Result<(), String> {
    Err("仅支持 macOS".to_string())
}

/// 重启目标 App（等待 `/usr/bin/open` 真实退出码）。
pub fn relaunch_app(app_path: &str) -> Result<(), String> {
    let out = std::process::Command::new("/usr/bin/open")
        .arg(app_path)
        .output()
        .map_err(|e| format!("启动应用失败: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "启动应用失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn not_running_bundle_is_ok() {
        // 不存在的 bundle id：无运行实例 → 直接 Ok（安全，不触碰任何进程）
        let r = quit_and_wait("com.molan.probe.nonexistent.9f3a", Duration::from_millis(500));
        assert!(r.is_ok(), "{r:?}");
    }

    #[test]
    fn relaunch_nonexistent_path_fails() {
        let r = relaunch_app("/nonexistent-molan/probe-9f3a.app");
        assert!(r.is_err());
    }
}
