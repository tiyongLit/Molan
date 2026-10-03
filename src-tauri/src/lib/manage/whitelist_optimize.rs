//! 与 `lib/manage/whitelist.sh` 中 `load_whitelist "optimize"`、`is_whitelisted` 一致（精确字符串匹配，`~` 前缀展开）。

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// 与 `lib/core/base.sh` 中 `DEFAULT_OPTIMIZE_WHITELIST_PATTERNS` 一致。
pub const DEFAULT_OPTIMIZE_WHITELIST_PATTERNS: &[&str] =
    &["check_brew_health", "check_touchid", "check_git_config"];

/// 与 sh 中 `pattern/#\~/$HOME` 一致：只替换句首 `~`。
pub fn expand_leading_tilde(pattern: &str, home: &Path) -> String {
    if pattern == "~" {
        return home.to_string_lossy().into_owned();
    }
    if let Some(rest) = pattern.strip_prefix("~/") {
        return home.join(rest).to_string_lossy().into_owned();
    }
    if pattern.starts_with('~') && pattern.len() > 1 {
        return format!("{}{}", home.display(), &pattern[1..]);
    }
    pattern.to_string()
}

fn patterns_equivalent(a: &str, b: &str, home: &Path) -> bool {
    expand_leading_tilde(a, home) == expand_leading_tilde(b, home)
}

fn dedupe_patterns(patterns: Vec<String>, home: &Path) -> Vec<String> {
    let mut unique: Vec<String> = Vec::new();
    for p in patterns {
        let duplicate = unique
            .iter()
            .any(|existing| patterns_equivalent(&p, existing, home));
        if !duplicate {
            unique.push(p);
        }
    }
    unique
}

fn read_whitelist_file_lines(path: &Path) -> Vec<String> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        out.push(line.to_string());
    }
    out
}

/// 等价于 `load_whitelist "optimize"` 后 `CURRENT_WHITELIST_PATTERNS`（不写入迁移副作用；legacy 仅读取）。
pub fn load_optimize_whitelist_patterns(home: &Path) -> Vec<String> {
    let config = home.join(".config/molan/whitelist_optimize");
    let legacy = home.join(".config/molan/whitelist_checks");

    let path_opt = if config.is_file() {
        Some(config)
    } else if legacy.is_file() {
        Some(legacy)
    } else {
        None
    };

    let patterns: Vec<String> = if let Some(p) = path_opt {
        let raw = read_whitelist_file_lines(&p);
        if raw.is_empty() { Vec::new() } else { raw }
    } else {
        DEFAULT_OPTIMIZE_WHITELIST_PATTERNS
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    };

    dedupe_patterns(patterns, home)
}

/// 与 `is_whitelisted` 一致：`patterns` 为空则永不跳过。
pub fn is_whitelisted_optimize(check_id: &str, patterns: &[String], home: &Path) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let check_expanded = expand_leading_tilde(check_id, home);
    for existing in patterns {
        let ex = expand_leading_tilde(existing, home);
        if check_expanded == ex {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn expand_tilde_roundtrip_like_shell() {
        let home = PathBuf::from("/Users/test");
        assert_eq!(expand_leading_tilde("check_a", &home), "check_a");
        assert_eq!(
            expand_leading_tilde("~/foo", &home),
            "/Users/test/foo".to_string()
        );
    }

    #[test]
    fn default_patterns_not_whitelisting_dev_keys() {
        let home = PathBuf::from("/tmp/x");
        let p: Vec<String> = DEFAULT_OPTIMIZE_WHITELIST_PATTERNS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(!is_whitelisted_optimize("check_launch_agents", &p, &home));
        assert!(!is_whitelisted_optimize("check_dev_tools", &p, &home));
        assert!(!is_whitelisted_optimize(
            "check_version_mismatches",
            &p,
            &home
        ));
    }
}
