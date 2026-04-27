//! 高风险 dotfile/dotdir 黑名单（对齐 PureMac `Conditions.swift#highRiskHomeDotPaths`）。
//!
//! 防止"名为 Claude 的 web app 把 `~/.claude` 当残留删了"这类灾难性事故。
//! 核心原则（对齐 PureMac `Locations.swift` L43-47）：
//! - bare `$HOME` 永不参与残留扫描根目录
//! - 任何卸载残留扫描结果命中黑名单即丢弃
//!
//! 与现有安全机制的关系：
//! - **不替代** `app_protection::path_belongs_to_independent_cli()`（仅覆盖 4 个 CLI 工具名）
//! - **不替代** `batch::file_path_is_sensitive()`（检测文件内容敏感性）
//! - 本模块是**全局兜底黑名单**，在扫描输出和删除执行两个环节双重过滤

/// 编译期黑名单：绝对禁止作为卸载残留被扫描或删除的 `$HOME` dotfile/dotdir。
/// 对齐 PureMac `Conditions.swift#highRiskHomeDotPaths`（36 条）。
///
/// 匹配规则：`path == "$HOME/<dot>"` 或 `path.starts_with("$HOME/<dot>/")`
pub const HIGH_RISK_HOME_DOTPATHS: &[&str] = &[
    // 开发工具配置
    ".claude",
    ".ssh",
    ".aws",
    ".gnupg",
    ".gpg",
    ".kube",
    ".docker",
    ".config",
    ".git",
    ".gitconfig",
    ".git-credentials",
    ".netrc",
    ".npmrc",
    ".yarnrc",
    ".pnpmrc",
    ".pip",
    ".pypirc",
    ".rbenv",
    ".pyenv",
    ".nvm",
    ".cargo",
    ".rustup",
    ".gem",
    ".local",
    ".password-store",
    ".mozilla",
    ".wine",
    ".vscode",
    ".vim",
    ".viminfo",
    // Shell 配置与历史
    ".zshrc",
    ".zsh_history",
    ".bash_history",
    ".bashrc",
    ".bash_profile",
    ".profile",
];

/// 判断路径是否命中高风险 dotfile/dotdir 黑名单。
///
/// 匹配规则：
/// - `path == "$HOME/<dot>"`（精确命中 dotfile/dotdir 本身）
/// - `path.starts_with("$HOME/<dot>/")`（命中 dotdir 下的任何子路径）
///
/// 不匹配 `~/Library/...` 下的同名目录（如 `~/Library/Application Support/Claude`），
/// 因为它们不在 `$HOME` 根下。
pub fn is_high_risk_dotpath(path: &str, home: &str) -> bool {
    if path.is_empty() || home.is_empty() {
        return false;
    }
    // 规范化：去尾随 `/`
    let path = path.trim_end_matches('/');
    let home = home.trim_end_matches('/');

    for dot in HIGH_RISK_HOME_DOTPATHS {
        let full = format!("{home}/{dot}");
        if path == full || path.starts_with(&format!("{full}/")) {
            return true;
        }
    }
    false
}

/// 批量过滤：从路径列表中移除所有命中黑名单的条目。
///
/// 用于 `leftovers::scan_deep_leftovers()` 和 `app_protection::find_app_files()`
/// 返回前的最终安全过滤。
pub fn filter_high_risk_paths(paths: Vec<String>, home: &str) -> Vec<String> {
    let (kept, blocked): (Vec<String>, Vec<String>) = paths
        .into_iter()
        .partition(|p| !is_high_risk_dotpath(p, home));
    if !blocked.is_empty() {
        log::warn!(
            "[high_risk_dotpaths] blocked {} path(s) from uninstall residual list: {:?}",
            blocked.len(),
            blocked
        );
    }
    kept
}

/// 判断一个路径是否是 bare `$HOME`（用于扫描根目录校验）。
///
/// 对齐 PureMac `Locations.swift` 的设计原则：
/// > User home - bare "$HOME" is intentionally NOT scanned.
/// > Scanning bare $HOME matches top-level dotfiles like .claude,
/// > .ssh, .aws, .kube by normalized app-name and invites data loss.
pub fn is_bare_home(path: &str, home: &str) -> bool {
    if path.is_empty() || home.is_empty() {
        return false;
    }
    let path = path.trim_end_matches('/');
    let home = home.trim_end_matches('/');
    path == home
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_ssh_directory() {
        let home = "/Users/testuser";
        assert!(is_high_risk_dotpath("/Users/testuser/.ssh", home));
        assert!(is_high_risk_dotpath(
            "/Users/testuser/.ssh/known_hosts",
            home
        ));
        assert!(is_high_risk_dotpath("/Users/testuser/.ssh/id_rsa", home));
    }

    #[test]
    fn blocks_claude_directory() {
        let home = "/Users/testuser";
        assert!(is_high_risk_dotpath("/Users/testuser/.claude", home));
        assert!(is_high_risk_dotpath(
            "/Users/testuser/.claude/settings.json",
            home
        ));
    }

    #[test]
    fn blocks_cargo_and_rustup() {
        let home = "/Users/testuser";
        assert!(is_high_risk_dotpath("/Users/testuser/.cargo", home));
        assert!(is_high_risk_dotpath(
            "/Users/testuser/.cargo/config.toml",
            home
        ));
        assert!(is_high_risk_dotpath("/Users/testuser/.rustup", home));
        assert!(is_high_risk_dotpath(
            "/Users/testuser/.rustup/toolchains/stable",
            home
        ));
    }

    #[test]
    fn blocks_dotfiles_not_directories() {
        let home = "/Users/testuser";
        assert!(is_high_risk_dotpath("/Users/testuser/.zshrc", home));
        assert!(is_high_risk_dotpath("/Users/testuser/.bash_profile", home));
        assert!(is_high_risk_dotpath("/Users/testuser/.gitconfig", home));
    }

    #[test]
    fn does_not_block_library_paths() {
        let home = "/Users/testuser";
        // ~/Library 下的同名目录不在 $HOME 根下，不应被拦截
        assert!(!is_high_risk_dotpath(
            "/Users/testuser/Library/Application Support/Claude",
            home
        ));
        assert!(!is_high_risk_dotpath(
            "/Users/testuser/Library/Caches/com.docker.docker",
            home
        ));
        assert!(!is_high_risk_dotpath(
            "/Users/testuser/Library/Application Support/Code",
            home
        ));
    }

    #[test]
    fn does_not_block_similar_names() {
        let home = "/Users/testuser";
        // .sshconfig 不等于 .ssh
        assert!(!is_high_risk_dotpath("/Users/testuser/.sshconfig", home));
        // .dockerx 不等于 .docker
        assert!(!is_high_risk_dotpath("/Users/testuser/.dockerx", home));
    }

    #[test]
    fn filter_removes_blocked_paths() {
        let home = "/Users/testuser";
        let paths = vec![
            "/Users/testuser/Library/Caches/com.foo.app".to_string(),
            "/Users/testuser/.ssh".to_string(),
            "/Users/testuser/Library/Preferences/com.foo.app.plist".to_string(),
            "/Users/testuser/.claude/settings.json".to_string(),
        ];
        let kept = filter_high_risk_paths(paths, home);
        assert_eq!(kept.len(), 2);
        assert!(kept.contains(&"/Users/testuser/Library/Caches/com.foo.app".to_string()));
        assert!(
            kept.contains(&"/Users/testuser/Library/Preferences/com.foo.app.plist".to_string())
        );
    }

    #[test]
    fn bare_home_detection() {
        let home = "/Users/testuser";
        assert!(is_bare_home("/Users/testuser", home));
        assert!(is_bare_home("/Users/testuser/", home));
        assert!(!is_bare_home("/Users/testuser/Library", home));
        assert!(!is_bare_home("/Users/testuser/.ssh", home));
    }

    #[test]
    fn empty_inputs_are_safe() {
        assert!(!is_high_risk_dotpath("", "/Users/testuser"));
        assert!(!is_high_risk_dotpath("/Users/testuser/.ssh", ""));
        assert!(!is_bare_home("", "/Users/testuser"));
    }
}
