//! 对齐 `Mole/bin/clean.sh`：`DRY_RUN_SEEN_IDENTITIES` + `register_dry_run_cleanup_target`。
//! 单次 `mole_clean` 会话内按 `mole_path_identity` 去重，避免跨模块 / 多次调用重复累计 KB、count。
//!
//! - `clear_seen_cleanup_targets`：对齐 `start_cleanup` 开头的 `DRY_RUN_SEEN_IDENTITIES=()`
//! - `dry_run_register_cleanup_target`：对齐 dry-run 路径登记；非 dry-run 恒为 `true`

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use super::common::mole_path_identity;

fn seen_mutex() -> &'static Mutex<HashSet<String>> {
    static STORE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashSet::new()))
}

#[inline]
pub fn mole_dry_run_active() -> bool {
    std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1"
        || std::env::var("DRY_RUN").unwrap_or_default() == "true"
}

/// 每一轮 GUI / `mole_clean` 开始前清空（对齐 bash `start_cleanup`）。
pub fn clear_seen_cleanup_targets() {
    if let Ok(mut g) = seen_mutex().lock() {
        g.clear();
    }
}

/// Dry-run：首次见到的 path identity 返回 `true`；重复返回 `false`。非 dry-run：恒 `true`。
pub fn dry_run_register_cleanup_target(path: &str) -> bool {
    if !mole_dry_run_active() {
        return true;
    }
    let id = mole_path_identity(path);
    let mut g = seen_mutex().lock().unwrap_or_else(|e| e.into_inner());
    if g.contains(&id) {
        false
    } else {
        g.insert(id);
        true
    }
}
