pub mod all;
pub mod configuration;
pub mod dev_environment;
pub mod health_json;
pub mod security;
pub mod system_health;

/// macOS 检查子进程用的 PATH 环境（系统基础路径 + 继承当前 PATH）。
#[cfg(target_os = "macos")]
pub(crate) fn macos_path_env() -> String {
    let tail = std::env::var("PATH").unwrap_or_default();
    format!("/usr/bin:/bin:/usr/sbin:/sbin:{tail}")
}
