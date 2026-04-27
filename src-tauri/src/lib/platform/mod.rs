//! 平台相关能力（权限安装路由提示等）。
//!
//! 特权 Helper 的真实安装须走原生 `ServiceManagement`（SMAppService / SMJobBless），
//! 见 `docs/privileged_helper_strategy.md`。此处提供运行时版本探测与推荐路径。

#[cfg(target_os = "macos")]
pub mod macos_file_icon;
#[cfg(target_os = "macos")]
pub mod macos_mditem;
pub mod macos_reveal;
pub mod macos_running_apps;
pub mod privileged_route;
#[cfg(target_os = "macos")]
pub mod smc;
