//! 平台相关能力（macOS 专属系统 API 封装：objc2 / IOKit / CoreServices FFI）。
//!
//! 特权 Helper 的真实安装须走原生 `ServiceManagement`（SMAppService / SMJobBless），
//! 见 `docs/privileged_helper_strategy.md`。此处提供运行时版本探测与推荐路径。
//!
//! 通用基础设施（渠道检测、配置存储、日志清理等）位于 `crate::vendor`。

#[cfg(target_os = "macos")]
pub mod macos_dialog;
#[cfg(target_os = "macos")]
pub mod macos_file_icon;
#[cfg(target_os = "macos")]
pub mod macos_mditem;
#[cfg(target_os = "macos")]
pub mod macos_privileged_route;
pub mod macos_reveal;
pub mod macos_running_apps;
#[cfg(target_os = "macos")]
pub mod macos_smc;
pub mod native_icon_registry;
#[cfg(target_os = "macos")]
pub mod macos_reminder_window;
#[cfg(target_os = "macos")]
pub mod macos_trash_watch;
