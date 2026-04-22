//! Cache cleanup module — 严格对齐 lib/clean/caches.sh
//!
//! 关键约束:
//!   - `scan_project_cache_root` 必须用 `find -P -mount` 限定挂载边界,并接 `MOLE_PROJECT_CACHE_SCAN_TIMEOUT`
//!   - `discover_project_cache_roots` 必须用 `mole_path_identity` 去重,避免 case-insensitive 卷重复
//!   - `clean_service_worker_cache` 必须 `find -depth 2`,并对每个 cache_dir 做白名单二次校验
//!   - 所有删除入口必须走 `safe_clean`,**不要**直接 `safe_remove`
//!   - **不要**自创 `clean_browser_caches` / `clean_firefox_cache` — 浏览器清理由 user.rs::clean_browsers 负责

use std::path::Path;

use super::purge_shared::{
    MOLE_PURGE_DEFAULT_SEARCH_PATHS, MOLE_PURGE_PROJECT_INDICATORS, mole_purge_is_project_root,
    mole_purge_read_paths_config,
};
use crate::core::app_protection::{PROTECTED_SW_DOMAINS, is_path_whitelisted_from_global};
use crate::core::base::home_dir;
use crate::core::common::{mole_identity_in_list, mole_path_identity};
use crate::core::file_ops::safe_clean;
use crate::core::log::debug_log;
use crate::core::timeout::run_with_timeout_capture_lossy;

// =============================================================================
// check_tcc_permissions — SH 第 8-43 行
// =============================================================================

pub fn check_tcc_permissions() {
    let home = home_dir();
    let cache_flag = format!("{home}/.cache/mole/permissions_granted");
    if Path::new(&cache_flag).is_file() {
        return;
    }
    // 只做轻量探测,与 SH 一样:每个目录调用 read_dir 触发 TCC,但不递归。
    let tcc_dirs = [
        format!("{home}/Library/Caches"),
        format!("{home}/Library/Logs"),
        format!("{home}/Library/Application Support"),
        format!("{home}/Library/Containers"),
        format!("{home}/.cache"),
    ];
    for dir in &tcc_dirs {
        if Path::new(dir).is_dir() {
            let _ = std::fs::read_dir(dir);
        }
    }
    if let Some(parent) = Path::new(&cache_flag).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&cache_flag, "1");
}

// =============================================================================
// clean_service_worker_cache — SH 第 46-109 行
// =============================================================================

/// SH 用 `basename | grep -oE '[a-zA-Z0-9][-a-zA-Z0-9]*\.[a-zA-Z]{2,}' | head -1`
/// 抠出第一段域名。Rust 用最小化扫描复制这条规则。
fn extract_domain_from_dir(dir: &str) -> String {
    let name = Path::new(dir)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if name.is_empty() {
        return String::new();
    }
    // 找出形如 `XXX.YY` 的最长 domain-ish 子串(首字母字母数字 + 跨过 `.`)
    let bytes = name.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        let c = bytes[i];
        let starts_id = c.is_ascii_alphanumeric();
        if starts_id {
            let mut j = i + 1;
            let mut saw_dot = false;
            let mut after_dot_alpha = 0;
            while j < n {
                let cj = bytes[j];
                if cj.is_ascii_alphanumeric() || cj == b'-' {
                    if saw_dot && cj.is_ascii_alphabetic() {
                        after_dot_alpha += 1;
                    }
                    j += 1;
                } else if cj == b'.' && !saw_dot {
                    saw_dot = true;
                    j += 1;
                } else {
                    break;
                }
            }
            if saw_dot && after_dot_alpha >= 2 {
                return String::from_utf8_lossy(&bytes[i..j]).to_string();
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    String::new()
}

fn is_sw_domain_protected(domain: &str) -> bool {
    if domain.is_empty() {
        return false;
    }
    PROTECTED_SW_DOMAINS.iter().any(|p| domain.contains(p))
}

/// 对齐 SH 第 46-109 行。
///
///   - 走 `find <cache_path> -type d -depth 2` 抠出每个 origin 目录(SH 第 85 行)
///   - 域名命中保护表 → 跳过
///   - `is_path_whitelisted` 命中 → 跳过(SH 第 75-78 行)
///   - 其余项走 `safe_clean`(已含 should_protect_path / whitelist 二次保护)
pub fn clean_service_worker_cache(browser_name: &str, cache_path: &str) {
    if !Path::new(cache_path).is_dir() {
        return;
    }

    let raw =
        run_with_timeout_capture_lossy(10.0, "find", &[cache_path, "-type", "d", "-depth", "2"])
            .unwrap_or_default();

    let mut targets: Vec<String> = Vec::new();
    let mut protected_count: u32 = 0;
    for line in raw.lines() {
        let cache_dir = line.trim();
        if cache_dir.is_empty() || !Path::new(cache_dir).is_dir() {
            continue;
        }
        let domain = extract_domain_from_dir(cache_dir);
        if is_sw_domain_protected(&domain) {
            protected_count = protected_count.saturating_add(1);
            debug_log(&format!(
                "SW cache protected by domain {domain} for {browser_name}"
            ));
            continue;
        }
        // SH 第 75-78 行:即使域名不在保护表里,也尊重用户配置的全局白名单
        if is_path_whitelisted_from_global(cache_dir) {
            protected_count = protected_count.saturating_add(1);
            debug_log(&format!(
                "SW cache protected by whitelist for {browser_name}: {cache_dir}"
            ));
            continue;
        }
        targets.push(cache_dir.to_string());
    }

    if targets.is_empty() {
        if protected_count > 0 {
            debug_log(&format!(
                "{browser_name} Service Worker: {protected_count} protected, nothing to clean"
            ));
        }
        return;
    }

    let refs: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
    let (kb, count) = safe_clean(&refs, &format!("{browser_name} Service Worker"));
    if count > 0 {
        debug_log(&format!(
            "{browser_name} Service Worker cleaned: {count} dirs, {kb}KB, {protected_count} protected"
        ));
    }
}

// =============================================================================
// project cache discovery — SH 第 110-200 行
// =============================================================================

/// SH `project_cache_has_indicators` (第 111-131 行)
///
/// 用 `find -maxdepth N (...indicators) -print -quit`,默认 2s 超时。
pub fn project_cache_has_indicators(dir: &str, max_depth: usize) -> bool {
    if !Path::new(dir).is_dir() {
        return false;
    }
    let timeout_secs: f64 = std::env::var("MOLE_PROJECT_CACHE_DISCOVERY_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(2.0);

    let mut args: Vec<String> = vec![
        dir.to_string(),
        "-maxdepth".to_string(),
        max_depth.to_string(),
        "(".to_string(),
    ];
    let mut first = true;
    for indicator in MOLE_PURGE_PROJECT_INDICATORS.iter() {
        if !first {
            args.push("-o".to_string());
        }
        args.push("-name".to_string());
        args.push((*indicator).to_string());
        first = false;
    }
    args.push(")".to_string());
    args.push("-print".to_string());
    args.push("-quit".to_string());

    let str_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match run_with_timeout_capture_lossy(timeout_secs, "find", &str_args) {
        Some(out) => !out.trim().is_empty(),
        None => false,
    }
}

/// SH `discover_project_cache_roots` (第 134-199 行)
///
/// 与 SH 一致:默认搜索路径 + 用户 `purge_paths` 配置 + `$HOME/*/` 中带项目指示器的目录,
/// 最后用 `mole_path_identity` 去重(case-insensitive 卷或 symlink 不会重复扫两次)。
pub fn discover_project_cache_roots() -> Vec<String> {
    let home = home_dir();
    let mut roots: Vec<String> = Vec::new();

    for path in MOLE_PURGE_DEFAULT_SEARCH_PATHS {
        let expanded = path.replacen('~', &home, 1);
        if Path::new(&expanded).is_dir() {
            roots.push(expanded);
        }
    }

    let config = format!("{home}/.config/mole/purge_paths");
    for cfg_path in mole_purge_read_paths_config(&config) {
        if Path::new(&cfg_path).is_dir() {
            roots.push(cfg_path);
        }
    }

    let skip = [
        "Library",
        "Applications",
        "Movies",
        "Music",
        "Pictures",
        "Public",
    ];
    if let Ok(rd) = std::fs::read_dir(&home) {
        for entry in rd.flatten() {
            let ep = entry.path();
            if !ep.is_dir() {
                continue;
            }
            let name = ep.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.starts_with('.') || skip.contains(&name) {
                continue;
            }
            let s = ep.to_string_lossy().to_string();
            if project_cache_has_indicators(&s, 5) {
                roots.push(s);
            }
        }
    }

    if roots.is_empty() {
        return Vec::new();
    }

    // 按 mole_path_identity 去重(SH 第 187-196 行)
    let mut unique: Vec<String> = Vec::new();
    let mut seen_identities: Vec<String> = Vec::new();
    for root in roots {
        let id = mole_path_identity(&root);
        if mole_identity_in_list(&id, &seen_identities) {
            continue;
        }
        seen_identities.push(id);
        unique.push(root);
    }
    unique
}

// =============================================================================
// scan_project_cache_root — SH 第 201-246 行
// =============================================================================

/// SH `scan_project_cache_root` (第 202-246 行)
///
/// 用 `find -P -maxdepth 9 -mount` 限定挂载边界,默认 6s 超时。
/// 返回 `(project_root, cache_dir)` 列表,与 SH 第 234 行 `printf '%s\t%s\n'` 等价。
pub fn scan_project_cache_root(root: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if !Path::new(root).is_dir() {
        return out;
    }

    let timeout_secs: f64 = std::env::var("MOLE_PROJECT_CACHE_SCAN_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(6.0);

    // SH 第 208-215 行:
    //   find -P "$root" -maxdepth 9 -mount
    //     ( -name Library -o -name .Trash -o ... ) -prune -o
    //     -type d ( -name .next -o -name __pycache__ -o -name .dart_tool ) -print
    let prune_names = [
        "Library",
        ".Trash",
        "node_modules",
        ".git",
        ".svn",
        ".hg",
        ".venv",
        "venv",
        ".pnpm-store",
        ".fvm",
        "DerivedData",
        "Pods",
        "miniconda3",
        "anaconda3",
        "miniforge3",
        "mambaforge",
        "site-packages",
    ];
    let target_names = [".next", "__pycache__", ".dart_tool"];

    let mut args: Vec<String> = vec![
        "-P".to_string(),
        root.to_string(),
        "-maxdepth".to_string(),
        "9".to_string(),
        "-mount".to_string(),
        "(".to_string(),
    ];
    let mut first = true;
    for n in prune_names.iter() {
        if !first {
            args.push("-o".to_string());
        }
        args.push("-name".to_string());
        args.push((*n).to_string());
        first = false;
    }
    args.push(")".to_string());
    args.push("-prune".to_string());
    args.push("-o".to_string());
    args.push("-type".to_string());
    args.push("d".to_string());
    args.push("(".to_string());
    let mut first2 = true;
    for n in target_names.iter() {
        if !first2 {
            args.push("-o".to_string());
        }
        args.push("-name".to_string());
        args.push((*n).to_string());
        first2 = false;
    }
    args.push(")".to_string());
    args.push("-print".to_string());

    let str_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let raw = match run_with_timeout_capture_lossy(timeout_secs, "find", &str_args) {
        Some(s) => s,
        None => {
            debug_log(&format!("Project cache scan timed out: {root}"));
            return out;
        }
    };

    for line in raw.lines() {
        let match_path = line.trim();
        if match_path.is_empty() {
            continue;
        }
        // SH 第 226-230 行:跳过没有 .pyc/.pyo 的空 __pycache__
        let basename = Path::new(match_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if basename == "__pycache__" {
            let mut has_bytecode = false;
            if let Ok(rd) = std::fs::read_dir(match_path) {
                for e in rd.flatten() {
                    let n = e.file_name().to_string_lossy().to_string();
                    if n.ends_with(".pyc") || n.ends_with(".pyo") {
                        has_bytecode = true;
                        break;
                    }
                }
            }
            if !has_bytecode {
                continue;
            }
        }
        let project_root = project_cache_group_root(root, match_path);
        out.push((project_root, match_path.to_string()));
    }

    out
}

/// SH `project_cache_group_root` (第 248-264 行)
pub fn project_cache_group_root(scan_root: &str, cache_path: &str) -> String {
    let mut candidate = match Path::new(cache_path).parent() {
        Some(p) => p.to_string_lossy().to_string(),
        None => return scan_root.to_string(),
    };
    while !candidate.is_empty() && candidate != "/" {
        if mole_purge_is_project_root(&candidate) {
            return candidate;
        }
        if candidate == scan_root {
            break;
        }
        match Path::new(&candidate).parent() {
            Some(p) => candidate = p.to_string_lossy().to_string(),
            None => break,
        }
    }
    scan_root.to_string()
}

// =============================================================================
// 清理入口 — SH 第 266-479 行
// =============================================================================

/// SH `clean_project_cache_target` (第 266-288 行)
/// 必须走 `safe_clean`,**不要**直接 `safe_remove`(会绕过白名单/保护)。
pub fn clean_project_cache_target(paths: &[String], description: &str) {
    if paths.is_empty() {
        return;
    }
    let refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
    let _ = safe_clean(&refs, description);
}

/// SH `flush_python_group_if_needed` (第 290-300 行)
pub fn flush_python_group_if_needed(group_root: &str, group_dirs: &mut Vec<String>) {
    if group_root.is_empty() || group_dirs.is_empty() {
        return;
    }
    let dirs = std::mem::take(group_dirs);
    clean_python_bytecode_cache_group(group_root, &dirs);
}

/// SH `process_project_cache_matches` (第 302-343 行)
///
/// 注意 SH 用 `LC_ALL=C sort -u`,这里 Rust 实现也按 `(project_root, cache_dir)`
/// 字典序去重排序,以便相邻的 `__pycache__` 被合并到同一个 group。
pub fn process_project_cache_matches(matches: &[(String, String)]) {
    let mut sorted: Vec<(String, String)> = matches.to_vec();
    sorted.sort();
    sorted.dedup();

    let mut current_python_root = String::new();
    let mut current_python_dirs: Vec<String> = Vec::new();

    for (record_root, cache_dir) in &sorted {
        if record_root.is_empty() || cache_dir.is_empty() {
            continue;
        }
        let cache_name = Path::new(cache_dir)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        match cache_name {
            ".next" => {
                flush_python_group_if_needed(&current_python_root, &mut current_python_dirs);
                current_python_root.clear();
                current_python_dirs.clear();
                let cache = format!("{cache_dir}/cache");
                if Path::new(&cache).is_dir() {
                    // SH 第 317 行:`safe_clean "$cache_dir/cache"/* "Next.js build cache"`
                    let pattern = format!("{cache}/*");
                    clean_project_cache_target(&[pattern], "Next.js build cache");
                }
            }
            "__pycache__" => {
                if record_root != &current_python_root && !current_python_dirs.is_empty() {
                    flush_python_group_if_needed(&current_python_root, &mut current_python_dirs);
                    current_python_dirs.clear();
                }
                current_python_root = record_root.clone();
                if Path::new(cache_dir).is_dir() {
                    current_python_dirs.push(cache_dir.clone());
                }
            }
            ".dart_tool" => {
                flush_python_group_if_needed(&current_python_root, &mut current_python_dirs);
                current_python_root.clear();
                current_python_dirs.clear();
                if Path::new(cache_dir).is_dir() {
                    clean_project_cache_target(
                        &[cache_dir.clone()],
                        "Flutter build cache (.dart_tool)",
                    );
                    if let Some(parent) = Path::new(cache_dir).parent() {
                        let build_dir = parent.join("build").to_string_lossy().to_string();
                        if Path::new(&build_dir).is_dir() {
                            clean_project_cache_target(
                                &[build_dir],
                                "Flutter build cache (build/)",
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
    flush_python_group_if_needed(&current_python_root, &mut current_python_dirs);
}

/// SH `clean_python_bytecode_cache_group` (第 345-439 行)
///
/// 把所有 cache_dir 一次性交给 `safe_clean`,后者已经包含 should_protect_path / whitelist
/// 两道检查,与 SH 第 364-376 行行为等价。
pub fn clean_python_bytecode_cache_group(_project_root: &str, cache_dirs: &[String]) {
    if cache_dirs.is_empty() {
        return;
    }
    let refs: Vec<&str> = cache_dirs.iter().map(|s| s.as_str()).collect();
    let (kb, count) = safe_clean(&refs, "Python bytecode cache");
    if count > 0 {
        let display = Path::new(_project_root)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(_project_root);
        debug_log(&format!(
            "Python bytecode cache · {display}: {count} dirs, {kb}KB"
        ));
    }
}

/// SH `clean_project_caches` (第 441-479 行)
pub fn clean_project_caches() -> (u64, u64) {
    let roots = discover_project_cache_roots();
    if roots.is_empty() {
        return (0, 0);
    }
    for root in &roots {
        let matches = scan_project_cache_root(root);
        if matches.is_empty() {
            continue;
        }
        process_project_cache_matches(&matches);
    }
    (0, 0)
}
