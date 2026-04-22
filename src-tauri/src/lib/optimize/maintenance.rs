use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::core::app_protection::should_protect_path;
use crate::core::base::home_dir;
use crate::core::file_ops::safe_remove;
use crate::core::log::debug_file_action;

/// 对齐 `lib/optimize/maintenance.sh` `_preference_plist_is_protected`。
fn preference_plist_is_protected(plist_file: &Path, protect_loginwindow: bool) -> bool {
    let filename = plist_file
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if filename.starts_with("com.apple.") || filename.starts_with(".GlobalPreferences") {
        return true;
    }
    if filename == "loginwindow.plist" {
        return protect_loginwindow;
    }
    false
}

/// 对齐 `lib/optimize/maintenance.sh` `_repair_preference_plists_in_dir`：
/// 大批量 lint（512 一批）+ 时间预算（15s）+ partial 返回码。
/// 返回 `(broken_count, partial)`。
fn repair_preference_plists_in_dir(
    search_dir: &str,
    max_depth: usize,
    protect_loginwindow: bool,
) -> (usize, bool) {
    if !Path::new(search_dir).is_dir() {
        return (0, false);
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut collect = |entry_path: &Path| {
        if !entry_path.is_file() {
            return;
        }
        if entry_path.extension().and_then(|s| s.to_str()) != Some("plist") {
            return;
        }
        if preference_plist_is_protected(entry_path, protect_loginwindow) {
            return;
        }
        candidates.push(entry_path.to_path_buf());
    };
    let walker = walkdir::WalkDir::new(search_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok());
    if max_depth > 0 {
        // find -maxdepth 1 只取该目录内的直接文件 → WalkDir depth <= 1。
        for entry in walker {
            if entry.depth() > max_depth {
                continue;
            }
            collect(entry.path());
        }
    } else {
        for entry in walker {
            collect(entry.path());
        }
    }

    let mut broken_count = 0usize;
    let total = candidates.len();
    const BATCH_SIZE: usize = 512;
    const HINT_SCAN_BUDGET_SECS: u64 = 15;
    let deadline = Instant::now() + std::time::Duration::from_secs(HINT_SCAN_BUDGET_SECS);
    let mut partial = false;
    let mut start = 0usize;

    while start < total {
        if Instant::now() >= deadline {
            partial = true;
            break;
        }
        let end = (start + BATCH_SIZE).min(total);
        let batch = &candidates[start..end];
        start = end;

        // plutil -lint 批量:整批通过则跳过;失败批逐文件回退。
        if plutil_lint_batch(batch) {
            continue;
        }
        for candidate in batch {
            if !candidate.is_file() {
                continue;
            }
            if plutil_lint_ok(&candidate.to_string_lossy()) {
                continue;
            }
            if should_protect_path(&candidate.to_string_lossy()) {
                continue;
            }
            if safe_remove(&candidate.to_string_lossy(), true) {
                // 对齐 SH bbac1b50:debug 模式记录实际修复的偏好文件路径
                debug_file_action(
                    "Removed corrupted preference",
                    &candidate.to_string_lossy(),
                    None,
                    None,
                );
                broken_count += 1;
            }
        }
    }

    (broken_count, partial)
}

/// 对齐 `lib/optimize/maintenance.sh` `fix_broken_preferences`：
/// 返回 `(repaired_count, partial)`；partial 为 true 表示扫描命中时间预算、计数不完整。
pub fn fix_broken_preferences() -> (usize, bool) {
    let home = home_dir();
    let prefs_dir = format!("{home}/Library/Preferences");
    if !Path::new(&prefs_dir).is_dir() {
        return (0, false);
    }

    let mut broken_count = 0usize;
    let mut partial = false;

    let (n, p) = repair_preference_plists_in_dir(&prefs_dir, 1, true);
    broken_count += n;
    partial |= p;

    let byhost = format!("{prefs_dir}/ByHost");
    let (n, p) = repair_preference_plists_in_dir(&byhost, 0, false);
    broken_count += n;
    partial |= p;

    (broken_count, partial)
}

fn plutil_lint_ok(path: &str) -> bool {
    Command::new("plutil")
        .args(["-lint", path])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(true)
}

/// 批量 `plutil -lint`：整批通过返回 true；任一批内失败返回 false。
fn plutil_lint_batch(batch: &[PathBuf]) -> bool {
    let mut cmd = Command::new("plutil");
    cmd.arg("-lint");
    for p in batch {
        cmd.arg(p.as_os_str());
    }
    cmd.output().map(|o| o.status.success()).unwrap_or(true)
}
