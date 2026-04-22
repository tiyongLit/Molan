//! 集中式安全守卫（facade）。
//!
//! 把散落在 `file_ops` / `app_protection` / `whitelist` / `project` 中的删除与路径守卫，
//! 收拢到单一可审计入口。**首版为零行为变更的委托层**：各谓词直接转调既有实现，
//! 语义与迁移前逐条一致；后续新增清理能力统一从此处取守卫，避免规则再次散落。
//!
//! 分域谓词（对齐 Trashly `safety.rs` 的分域思路，但为 Rust 原生重写，不含任何 AGPL 代码）：
//! - [`is_deletable`]：路径是否可安全删除（转调 `file_ops::validate_path_for_deletion`）
//! - [`is_whitelisted`]：是否命中全局白名单（转调 `app_protection::is_path_whitelisted_from_global`）
//! - [`is_protected`]：是否为受保护路径（转调 `app_protection::should_protect_path`）
//! - [`is_user_path`]：是否位于当前用户 HOME 之下（用于区分用户态 / 系统态删除路由）

use super::app_protection::{is_path_whitelisted_from_global, should_protect_path};
use super::base::home_dir;
use super::file_ops::validate_path_for_deletion;

// iCloud dataless（云端占位）判定：实现放在最底层 `base`（紧邻公共 sizer `get_path_size_kb`），
// 在此再导出，使尺寸口径相关的守卫也能从 safety facade 统一取得。
pub use super::base::is_dataless;

/// 路径是否可安全删除。
///
/// 委托 `file_ops::validate_path_for_deletion`：空/相对路径、`..` 穿越、控制字符、
/// 指向系统目录的符号链接、关键系统目录黑名单、`should_protect_path` 保护规则一律拒绝。
pub fn is_deletable(path: &str) -> bool {
    validate_path_for_deletion(path)
}

/// 是否命中全局白名单（用户/内置显式保留，清理时跳过）。
pub fn is_whitelisted(path: &str) -> bool {
    is_path_whitelisted_from_global(path)
}

/// 是否为受保护路径（系统关键 bundle / 运行中应用等）。
pub fn is_protected(path: &str) -> bool {
    should_protect_path(path)
}

/// 是否严格位于当前用户 HOME 之下（HOME 本身返回 false）。
///
/// 用于删除路由：用户态路径可走 Finder 废纸篓（`trash`），系统态 / root 属主路径需走特权删除。
/// HOME 不可得（沙箱 / 异常环境）时 **fail-closed** 返回 false，绝不把未知路径当作用户路径。
pub fn is_user_path(path: &str) -> bool {
    let home = home_dir();
    let home = home.trim_end_matches('/');
    // fail-closed：HOME 为空或退化为根时不认定为用户路径
    if home.is_empty() || home == "/" {
        return false;
    }
    // 严格前缀 + 分量边界：防止 `/Users/foobar` 命中 `/Users/foo`
    path.len() > home.len() + 1 && path.starts_with(home) && path.as_bytes()[home.len()] == b'/'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletable_rejects_empty_and_relative() {
        assert!(!is_deletable(""));
        assert!(!is_deletable("relative/path"));
    }

    #[test]
    fn deletable_rejects_traversal_and_system_roots() {
        assert!(!is_deletable("/Users/foo/../bar"));
        assert!(!is_deletable("/"));
        assert!(!is_deletable("/System"));
        assert!(!is_deletable("/usr/bin"));
        assert!(!is_deletable("/etc"));
    }

    #[test]
    fn deletable_allows_known_safe_private() {
        assert!(is_deletable("/private/tmp"));
        assert!(is_deletable("/private/var/folders"));
    }

    #[test]
    fn user_path_rejects_system_and_empty() {
        // 与运行环境 HOME 无关的断言：系统路径必非用户路径
        assert!(!is_user_path("/System/Library"));
        assert!(!is_user_path("/private/tmp"));
        assert!(!is_user_path(""));
    }

    #[test]
    fn user_path_respects_component_boundary() {
        // 构造以 HOME 为前缀但跨越分量边界的伪兄弟目录，必须判非用户路径
        let home = home_dir();
        let home = home.trim_end_matches('/');
        if home.is_empty() || home == "/" {
            return; // 无 HOME 环境跳过（fail-closed 已由上一用例覆盖）
        }
        assert!(is_user_path(&format!("{home}/Library/Caches")));
        assert!(!is_user_path(&format!("{home}_sibling/x")));
        assert!(!is_user_path(home)); // HOME 本身不可整体删
    }
}
