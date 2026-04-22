//! Project Purge Module — 严格对齐 lib/clean/project.sh
//!
//! 关键要求(SH 第 252-403 行):
//!   - `is_safe_project_artifact`:必须在 search_path 下,至少一层深度,
//!     symlink 走物理路径回退(`/private/...`)
//!   - `is_recently_modified`:7 天内动过的工件默认不删
//!   - `is_protected_purge_artifact`:`bin/` 仅 .NET 才删,`vendor/` 仅 PHP Composer 才删,
//!     `DerivedData` 不能命中 `~/Library/Developer/Xcode/DerivedData`
//!   - `filter_nested_artifacts`:父目录已选中时,子目录跳过
//!
//! GUI 端非交互入口 `clean_project_artifacts(root)` 默认排除 recent items
//! (对齐 SH `clean_project_artifacts` 第 1583-1590 行非交互分支)。

use std::path::Path;
use std::process::Command;

use super::purge_shared::{
    MOLE_PURGE_DEFAULT_SEARCH_PATHS, MOLE_PURGE_PROJECT_INDICATORS, MOLE_PURGE_TARGETS,
    mole_purge_is_project_root, mole_purge_read_paths_config, mole_purge_resolve_path_case,
};
use crate::core::base::{get_epoch_seconds, get_file_mtime, home_dir};
use crate::core::file_ops::safe_clean;
use crate::core::log::debug_log;

/// SH `MIN_AGE_DAYS=7`(第 17 行)
pub const MIN_AGE_DAYS: u64 = 7;
pub const PURGE_MIN_DEPTH_DEFAULT: usize = 1;
pub const PURGE_MAX_DEPTH_DEFAULT: usize = 6;

// =============================================================================
// 项目根识别 / 项目容器识别 / 路径发现
// =============================================================================

/// SH `is_purge_project_root` (第 256-258 行)
pub fn is_purge_project_root(dir: &str) -> bool {
    mole_purge_is_project_root(dir)
}

/// SH `is_project_container` (第 37-70 行)
pub fn is_project_container(dir: &str, max_depth: usize) -> bool {
    let p = Path::new(dir);
    if !p.is_dir() {
        return false;
    }
    let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if name.starts_with('.') {
        return false;
    }
    if matches!(
        name,
        "Library" | "Applications" | "Movies" | "Music" | "Pictures" | "Public"
    ) {
        return false;
    }

    // find -maxdepth max_depth ( -name X -o -name Y ... ) -print -quit
    let mut args: Vec<String> = vec![
        dir.to_string(),
        "-maxdepth".to_string(),
        max_depth.to_string(),
        "(".to_string(),
    ];
    let mut first = true;
    for ind in MOLE_PURGE_PROJECT_INDICATORS.iter() {
        if !first {
            args.push("-o".to_string());
        }
        args.push("-name".to_string());
        args.push((*ind).to_string());
        first = false;
    }
    args.push(")".to_string());
    args.push("-print".to_string());
    args.push("-quit".to_string());

    let str_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    if let Ok(out) = Command::new("find").args(&str_args).output() {
        if !String::from_utf8_lossy(&out.stdout).trim().is_empty() {
            return true;
        }
    }
    false
}

/// SH `discover_project_dirs` (第 72-107 行)
pub fn discover_project_dirs() -> Vec<String> {
    let home = home_dir();
    let mut discovered: Vec<String> = Vec::new();

    for path in MOLE_PURGE_DEFAULT_SEARCH_PATHS {
        let expanded = path.replacen('~', &home, 1);
        if Path::new(&expanded).is_dir() {
            discovered.push(mole_purge_resolve_path_case(&expanded));
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
            let resolved = mole_purge_resolve_path_case(&ep.to_string_lossy());
            if discovered.iter().any(|d| d == &resolved) {
                continue;
            }
            if is_project_container(&resolved, 2) {
                discovered.push(resolved);
            }
        }
    }
    discovered.sort();
    discovered.dedup();
    discovered
}

pub fn load_purge_config() -> Vec<String> {
    let cfg = format!("{}/.config/mole/purge_paths", home_dir());
    let paths = mole_purge_read_paths_config(&cfg);
    if paths.is_empty() {
        discover_project_dirs()
    } else {
        paths
    }
}

/// SH `save_discovered_paths` (第 161-167 行,经 `write_purge_config` 第 116-152 行)
/// 用 tmp + rename 模拟 SH 的 atomic write。
pub fn save_discovered_paths(paths: &[String]) {
    let cfg = format!("{}/.config/mole/purge_paths", home_dir());
    if let Some(parent) = Path::new(&cfg).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut content = String::from(
        "# Mole Purge Paths - Auto-discovered project directories\n\
         # Edit this file to customize, or run: mo purge --paths\n\
         # Add one path per line (supports ~ for home directory)\n",
    );
    let home = home_dir();
    for p in paths {
        let display = p.replacen(&home, "~", 1);
        content.push_str(&format!("{display}\n"));
    }
    let tmp = format!("{cfg}.tmp.{}", std::process::id());
    if std::fs::write(&tmp, content).is_ok() {
        let _ = std::fs::rename(&tmp, &cfg);
    }
}

// =============================================================================
// 安全检查 helpers — SH 第 260-403 行
// =============================================================================

/// SH `is_safe_project_artifact` (第 262-307 行)
///
/// 返回 true 表示 `path` 可以安全清理(在 search_path 内,至少一层深度,
/// 或 search_path 本身就是一个 project root 时允许直接子目录)。
pub fn is_safe_project_artifact(path: &str, search_path: &str) -> bool {
    if path.is_empty() || !path.starts_with('/') {
        return false;
    }
    let mut sp = search_path.to_string();
    if sp != "/" {
        while sp.ends_with('/') {
            sp.pop();
        }
    }

    let prefix = if sp == "/" {
        "/".to_string()
    } else {
        format!("{sp}/")
    };

    let (effective_path, effective_search) = if path.starts_with(&prefix) || path == sp {
        (path.to_string(), sp.clone())
    } else {
        // 物理路径回退:fd / find 可能给出 /private/var,而 search_path 用 /var
        let p_canon = match (Path::new(path).is_dir(), Path::new(&sp).is_dir()) {
            (true, true) => {
                let pp = std::fs::canonicalize(path)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());
                let pps = std::fs::canonicalize(&sp)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());
                (pp, pps)
            }
            _ => (None, None),
        };
        match p_canon {
            (Some(pp), Some(pps)) => {
                let pps_with_slash = if pps == "/" {
                    "/".to_string()
                } else {
                    format!("{pps}/")
                };
                if pp.starts_with(&pps_with_slash) || pp == pps {
                    (pp, pps)
                } else {
                    return false;
                }
            }
            _ => return false,
        }
    };

    // 等于 search_root 自身时,depth=0 → 仅当本身是 project root 才放行
    if effective_path == effective_search {
        return is_purge_project_root(&effective_search);
    }

    let strip_prefix = if effective_search == "/" {
        "/".to_string()
    } else {
        format!("{effective_search}/")
    };
    let rel = effective_path
        .strip_prefix(&strip_prefix)
        .unwrap_or(&effective_path);
    let depth = rel.chars().filter(|c| *c == '/').count();
    if depth < 1 {
        return is_purge_project_root(&effective_search);
    }
    true
}

/// SH `is_recently_modified` (第 555-575 行)
///
/// `current_time=0` 时使用当前时间。返回 true 表示 7 天内动过(最近)。
pub fn is_recently_modified(path: &str, current_time: u64) -> bool {
    if !Path::new(path).exists() && !Path::new(path).is_symlink() {
        return false;
    }
    let mtime = get_file_mtime(path);
    if mtime == 0 {
        return false;
    }
    let now = if current_time > 0 {
        current_time
    } else {
        get_epoch_seconds()
    };
    let age_seconds = now.saturating_sub(mtime);
    age_seconds < MIN_AGE_DAYS * 86400
}

/// SH `is_dotnet_bin_dir` (第 330-343 行)
fn is_dotnet_bin_dir(path: &str) -> bool {
    let basename = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if basename != "bin" {
        return false;
    }
    let parent = match Path::new(path).parent() {
        Some(p) => p,
        None => return false,
    };
    let mut has_proj = false;
    if let Ok(rd) = std::fs::read_dir(parent) {
        for entry in rd.flatten() {
            let n = entry.file_name().to_string_lossy().to_string();
            if n.ends_with(".csproj") || n.ends_with(".fsproj") || n.ends_with(".vbproj") {
                has_proj = true;
                break;
            }
        }
    }
    if !has_proj {
        return false;
    }
    Path::new(path).join("Debug").is_dir() || Path::new(path).join("Release").is_dir()
}

/// SH `is_rails_project_root` (第 309-315 行)
fn is_rails_project_root(dir: &str) -> bool {
    Path::new(&format!("{dir}/config/application.rb")).is_file()
        && Path::new(&format!("{dir}/Gemfile")).is_file()
        && (Path::new(&format!("{dir}/bin/rails")).is_file()
            || Path::new(&format!("{dir}/config/environment.rb")).is_file())
}

/// SH `is_go_project_root` (第 317-321 行)
fn is_go_project_root(dir: &str) -> bool {
    Path::new(&format!("{dir}/go.mod")).is_file()
}

/// SH `is_php_project_root` (第 323-327 行)
fn is_php_project_root(dir: &str) -> bool {
    Path::new(&format!("{dir}/composer.json")).is_file()
}

/// SH `is_protected_vendor_dir` (第 345-374 行)
/// 返回 true 表示该 vendor 受保护,**不能删**。
fn is_protected_vendor_dir(path: &str) -> bool {
    let basename = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if basename != "vendor" {
        // 不是 vendor 目录,这里"是否受保护"语义无意义,统一按"未保护"处理
        return false;
    }
    let parent = match Path::new(path).parent() {
        Some(p) => p.to_string_lossy().to_string(),
        None => return true,
    };
    if is_php_project_root(&parent) {
        return false;
    }
    if is_rails_project_root(&parent) {
        return true;
    }
    if is_go_project_root(&parent) {
        return true;
    }
    // 未知类型默认保护(SH 第 372 行)
    true
}

/// SH `is_protected_purge_artifact` (第 376-403 行)
/// 返回 true 表示该工件受保护,**不能删**。
pub fn is_protected_purge_artifact(path: &str) -> bool {
    let base = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    match base {
        "bin" => !is_dotnet_bin_dir(path),
        "vendor" => is_protected_vendor_dir(path),
        "DerivedData" => path.contains("/Library/Developer/Xcode/DerivedData"),
        _ => false,
    }
}

/// SH `filter_nested_artifacts` (第 528-545 行)
/// 排序后扫描,如果当前 path 以前一条已保留的 path 开头,就视为嵌套并丢弃。
pub fn filter_nested_artifacts(paths: &[String]) -> Vec<String> {
    let mut tagged: Vec<String> = paths
        .iter()
        .map(|p| {
            let mut s = p.clone();
            if !s.ends_with('/') {
                s.push('/');
            }
            s
        })
        .collect();
    tagged.sort();
    let mut out: Vec<String> = Vec::new();
    let mut last_kept: String = String::new();
    for s in tagged {
        if last_kept.is_empty() || !s.starts_with(&last_kept) {
            out.push(s.clone());
            last_kept = s;
        }
    }
    out.into_iter()
        .map(|mut s| {
            if s.ends_with('/') {
                s.pop();
            }
            s
        })
        .collect()
}

// =============================================================================
// scan_purge_targets — SH 第 405-525 行
// =============================================================================

/// 用 `find` 扫描一个 search_path 的 purge 候选,然后依次走:
///   `is_safe_project_artifact` → `filter_nested_artifacts` → `is_protected_purge_artifact`
///
/// 返回**已经过三道安全过滤**的路径列表。
pub fn scan_purge_targets(search_path: &str) -> Vec<String> {
    if !Path::new(search_path).is_dir() {
        return Vec::new();
    }

    // 与 SH 498-521 行 find 用法一致:
    //   -mindepth N -maxdepth N -type d
    //   ( -name $prune ... ) -prune -o
    //   ( -name $target ... ) -print -prune
    let prune_dirs = [".git", "Library", ".Trash", "Applications"];
    let mut args: Vec<String> = vec![
        search_path.to_string(),
        "-mindepth".to_string(),
        PURGE_MIN_DEPTH_DEFAULT.to_string(),
        "-maxdepth".to_string(),
        PURGE_MAX_DEPTH_DEFAULT.to_string(),
        "-type".to_string(),
        "d".to_string(),
        "(".to_string(),
    ];
    let mut first = true;
    for d in prune_dirs.iter() {
        if !first {
            args.push("-o".to_string());
        }
        args.push("-name".to_string());
        args.push((*d).to_string());
        first = false;
    }
    args.push(")".to_string());
    args.push("-prune".to_string());
    args.push("-o".to_string());
    args.push("(".to_string());
    let mut first2 = true;
    for t in MOLE_PURGE_TARGETS.iter() {
        if !first2 {
            args.push("-o".to_string());
        }
        args.push("-name".to_string());
        args.push((*t).to_string());
        first2 = false;
    }
    args.push(")".to_string());
    args.push("-print".to_string());
    args.push("-prune".to_string());

    let str_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let raw = match Command::new("find").args(&str_args).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return Vec::new(),
    };

    // is_safe_project_artifact
    let mut safe_paths: Vec<String> = Vec::new();
    for line in raw.lines() {
        let p = line.trim();
        if p.is_empty() {
            continue;
        }
        if !is_safe_project_artifact(p, search_path) {
            continue;
        }
        safe_paths.push(p.to_string());
    }

    // filter_nested_artifacts
    let nested = filter_nested_artifacts(&safe_paths);

    // is_protected_purge_artifact(true → 跳过)
    nested
        .into_iter()
        .filter(|p| !is_protected_purge_artifact(p))
        .collect()
}

// =============================================================================
// 清理入口
// =============================================================================

/// 单根非交互清理。默认排除 7 天内动过的工件,对齐 SH 非交互分支(第 1583-1590 行)。
/// 返回 `(count, total_kb)`。
pub fn clean_project_artifacts(root: &str) -> (usize, u64) {
    let now = get_epoch_seconds();
    let candidates = scan_purge_targets(root);

    let mut to_clean: Vec<String> = Vec::new();
    for p in &candidates {
        if is_recently_modified(p, now) {
            debug_log(&format!("Skipping recently-modified artifact: {p}"));
            continue;
        }
        to_clean.push(p.clone());
    }
    if to_clean.is_empty() {
        return (0, 0);
    }
    let refs: Vec<&str> = to_clean.iter().map(|s| s.as_str()).collect();
    let (kb, count) = safe_clean(&refs, "Project artifacts");
    (count as usize, kb)
}

pub fn run_purge() {
    let paths = load_purge_config();
    let mut total_count = 0usize;
    let mut total_kb = 0u64;
    for root in &paths {
        if Path::new(root).is_dir() {
            let (c, kb) = clean_project_artifacts(root);
            total_count += c;
            total_kb += kb;
        }
    }
    debug_log(&format!(
        "Purge: {total_count} artifacts removed, {total_kb}KB"
    ));
}
