pub mod app_protection;
pub mod base;
pub mod bundle_id_anchor;
pub mod bundle_resolver;
pub mod busy_state;
pub mod commands;
pub mod common;
pub mod debug_trace;
pub mod dry_run_registry;
pub mod file_ops;
pub mod help;
pub mod high_risk_dotpaths;
pub mod keep_awake;
pub mod log;
pub mod pkg_receipts;
pub mod safety;
pub mod sudo;
pub mod timeout;
pub mod ui;

use std::sync::atomic::{AtomicBool, Ordering};

static CORE_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// 统一初始化入口,对齐 lib/core/common.sh 顶部那串 `source` + 自动调用。
/// GUI / Tauri runner 应该在 setup hook 中调一次,后端命令(clean/uninstall/...)调用前生效。
///
/// 行为:
///   - 准备 mole 自己的 TMPDIR(对齐 SH `prepare_mole_tmpdir`)
///   - 触发一次 log rotation
///   - MO_DEBUG=1 时输出系统信息
///   - 注册 SIGINT/SIGTERM/SIGHUP 钩子,确保 GUI 被强杀时也能 stop_sudo_session
///
/// 幂等:多次调用只会执行一次。
pub fn init() {
    if CORE_INITIALIZED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = base::prepare_mole_tmpdir();
    log::rotate_log_once();
    if std::env::var("MO_DEBUG").unwrap_or_default() == "1" {
        log::log_system_info();
    }
    let _ = sudo::register_sudo_cleanup();
}

/// 设置全局白名单。GUI 在加载 `~/.cache/mole/whitelist.txt` 等配置文件后调用。
/// 之后 `safe_find_delete` / `safe_sudo_find_delete` 都会自动尊重它。
pub fn set_whitelist(patterns: Vec<String>) {
    app_protection::set_global_whitelist(patterns);
}
