//! 用户级数据清理 — 严格对齐 lib/clean/user.sh
//!
//! 全面避坑:
//!   - **绝不**直接 `safe_remove(~/Library/Caches)` / `~/Library/Logs`,这些目录本身要保留,
//!     只能通过 `safe_clean(~/Library/Caches/*, …)` 删除子项
//!   - **绝不**碰 `~/Library/Safari/History` / `~/Library/Safari/LocalStorage` 等用户数据
//!   - Docker VM 数据(`com.docker.docker/Data/vms`)由 Docker 自己管,绝不可删
//!   - 浏览器清理仅限 `~/Library/Caches/` 下的 HTTP 缓存;绝不触碰 Application Support
//!     下的任何浏览器 profile 数据(Service Worker / Code Cache / GPUCache 等统一回避),
//!     对齐腾讯柠檬 garbage1_zh.xml「上网垃圾」口径
//!
//! 大多数函数返回 `(total_kb, total_count)` 让 GUI 聚合。

use regex::Regex;
use std::path::Path;
use std::process::Command;

use crate::core::app_protection::{
    is_critical_system_component, is_path_whitelisted, is_path_whitelisted_from_global,
    should_protect_data, should_protect_path,
};
use crate::core::base::{
    MOLE_MAIL_AGE_DAYS, MOLE_MAIL_DOWNLOADS_MIN_KB, bytes_to_human, end_section, get_epoch_seconds,
    get_file_mtime, get_path_size_kb, home_dir, is_dry_run, note_activity, pgrep_x, start_section,
    start_section_spinner, stop_section_spinner, update_progress_if_needed,
};
use crate::core::dry_run_registry::dry_run_register_cleanup_target;
use crate::core::file_ops::{
    MOLE_OK, expand_glob_paths, safe_clean, safe_find_delete, safe_remove, safe_sudo_find_delete,
    safe_sudo_remove,
};
use crate::core::log::{debug_log, log_info, log_operation, log_warning};
use crate::core::sudo::is_admin_authorized;
use crate::core::timeout::run_with_timeout_capture;
use crate::core::timeout::run_with_timeout_capture_lossy;
use crate::events::{CleanupHintItem, CleanupHintsResultPayload};

use super::apps::clean_ds_store_tree;

// =============================================================================
// user_cache 通配清理 —— `~/Library/Caches/*` 去重
// =============================================================================

/// `~/Library/Caches/*` 通配清理时跳过的子目录名（相对 `Library/Caches/`）。
///
/// 这些子目录已被其他**默认勾选**的专属条目覆盖：
///   - browser_cache_main（clean_browsers）：Safari / Google(Chrome) / Chromium / Edge / Arc / Dia /
///     Brave / Helium / Yandex / Firefox / Opera / Vivaldi / Comet / Kagi / Zen
///   - dev_homebrew_cache：Homebrew
///   - app_caches_system：photoanalysisd / akd / WebKit.Networking / QuickLook / iconservices
///   - app_caches_sandbox：helpd / GeoServices / AppleMediaServices / duetexpertd / parsecd /
///     python / e5bundlecache
///
/// 若仍参与通配，同一路径会被两个条目各统计一次，导致前端"已选中"大小虚高，
/// 且会绕过专属条目自身的检查（如 Firefox 运行中跳过 `Caches/Firefox` 缓存）。
/// 维护约定：新增默认条目若覆盖 `Library/Caches/` 下的子目录，须同步登记到本表。
const USER_CACHE_EXCLUDED_SUBDIRS: &[&str] = &[
    // 关键系统缓存（对齐柠檬 filter id=5,7：删除会导致 Spotlight 重建索引 / LaunchServices 关联丢失）
    "com.apple.Spotlight",
    "com.apple.LaunchServices",
    // browser_cache_main
    "com.apple.Safari",
    "Google",
    "Chromium",
    "com.microsoft.edgemac",
    "company.thebrowser.Browser",
    "company.thebrowser.dia",
    "BraveSoftware",
    "net.imput.helium",
    "Yandex",
    "Firefox",
    "com.operasoftware.Opera",
    "com.vivaldi.Vivaldi",
    "Comet",
    "com.kagi.kagimacOS",
    "zen",
    // dev_homebrew_cache
    "Homebrew",
    // app_caches_system
    "com.apple.photoanalysisd",
    "com.apple.akd",
    "com.apple.WebKit.Networking",
    "com.apple.QuickLook.thumbnailcache",
    "Quick Look",
    "com.apple.iconservices",
    "com.apple.iconservices.store",
    // app_caches_sandbox
    "com.apple.helpd",
    "GeoServices",
    "com.apple.AppleMediaServices",
    "com.apple.duetexpertd",
    "com.apple.parsecd",
    "com.apple.python",
    "com.apple.e5rt.e5bundlecache",
];

/// 展开 `{h}/Library/Caches/*`，剔除 [`USER_CACHE_EXCLUDED_SUBDIRS`] 覆盖的子目录后
/// 交给 `safe_clean`。扫描（dry-run）与 item 执行共用，保证两侧口径一致、无重复统计。
fn clean_user_caches_wildcard(h: &str) -> (u64, u64) {
    let cache_root = format!("{h}/Library/Caches");
    let prefix = format!("{cache_root}/");
    let mut kept: Vec<String> = Vec::new();
    for path in expand_glob_paths(&format!("{cache_root}/*")) {
        let name = match path.strip_prefix(&prefix) {
            Some(n) => n,
            None => continue,
        };
        let excluded = USER_CACHE_EXCLUDED_SUBDIRS
            .iter()
            .any(|ex| name == *ex || name.starts_with(&format!("{ex}/")));
        if !excluded {
            kept.push(path);
        }
    }
    if kept.is_empty() {
        return (0, 0);
    }
    let refs: Vec<&str> = kept.iter().map(|s| s.as_str()).collect();
    safe_clean(&refs, "User app cache")
}

// =============================================================================
// clean_user_essentials —— 缓存/日志/废纸篓/Recent items/Mail downloads
// =============================================================================

/// 对齐 SH 第 4-65 行。
pub fn clean_user_essentials() -> super::ModuleScanResult {
    let h = home_dir();
    let mut items: Vec<super::SubItemResult> = Vec::new();

    let (kb, c) = clean_user_caches_wildcard(&h);
    items.push(super::SubItemResult::new(
        "user_cache",
        "User app cache",
        kb,
        c,
    ));

    let (kb, c) = safe_clean(&[&format!("{h}/Library/Logs/*")], "User app logs");
    items.push(super::SubItemResult::new(
        "user_logs",
        "User app logs",
        kb,
        c,
    ));

    let (kb, c) = clean_darwin_user_runtime_dirs();
    items.push(super::SubItemResult::new(
        "darwin_runtime",
        "Darwin runtime temp",
        kb,
        c,
    ));

    let trash_dir = format!("{h}/.Trash");
    let mut trash_count = 0;
    if !is_path_whitelisted_from_global(&trash_dir) && Path::new(&trash_dir).is_dir() {
        let dry_run = is_dry_run();
        let test_mode = std::env::var("MOLE_TEST_MODE").unwrap_or_default() == "1"
            || std::env::var("MOLE_TEST_NO_AUTH").unwrap_or_default() == "1";

        trash_count = if !test_mode {
            run_with_timeout_capture(
                3.0,
                "osascript",
                &["-e", "tell application \"Finder\" to count items in trash"],
            )
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or_else(|| trash_item_count(&trash_dir))
        } else {
            trash_item_count(&trash_dir)
        };

        if !dry_run && trash_count > 0 {
            let mut emptied = false;
            if !test_mode {
                let rc = crate::core::timeout::run_with_timeout(
                    5.0,
                    "osascript",
                    &["-e", "tell application \"Finder\" to empty trash"],
                );
                if rc == 0 {
                    emptied = true;
                    log_info(&format!("Trash · emptied, {trash_count} items"));
                    note_activity();
                }
            }
            if !emptied {
                debug_log("Finder empty trash failed, falling back to direct deletion");
                let mut cleaned = 0u64;
                if let Ok(rd) = std::fs::read_dir(&trash_dir) {
                    for entry in rd.flatten() {
                        let path = entry.path().to_string_lossy().to_string();
                        if safe_remove(&path, true) {
                            cleaned += 1;
                        }
                    }
                }
                if cleaned > 0 {
                    log_info(&format!("Trash · emptied, {cleaned} items"));
                    note_activity();
                }
            }
        }
    }
    items.push(super::SubItemResult::new("trash", "Trash", 0, trash_count));

    let (kb, c) = clean_recent_items();
    items.push(super::SubItemResult::new(
        "recent_items",
        "Recent items",
        kb,
        c,
    ));

    clean_mail_downloads();

    super::ModuleScanResult { items }
}

// =============================================================================
// Darwin User Runtime Dirs —— /var/folders/*/*/{T,C} 清理
// =============================================================================

/// 对齐 SH `_clean_darwin_user_runtime_dirs()` 第 294-306 行。
pub fn clean_darwin_user_runtime_dirs() -> (u64, u64) {
    // 测试模式下默认跳过，除非设置了 MOLE_ENABLE_DARWIN_RUNTIME_CLEANUP_IN_TESTS
    let test_mode = std::env::var("MOLE_TEST_MODE").unwrap_or_default() == "1"
        || std::env::var("MOLE_TEST_NO_AUTH").unwrap_or_default() == "1";
    if test_mode
        && std::env::var("MOLE_ENABLE_DARWIN_RUNTIME_CLEANUP_IN_TESTS").unwrap_or_default() != "1"
    {
        return (0, 0);
    }

    let temp_dir =
        run_with_timeout_capture(1.0, "getconf", &["DARWIN_USER_TEMP_DIR"]).unwrap_or_default();
    let cache_dir =
        run_with_timeout_capture(1.0, "getconf", &["DARWIN_USER_CACHE_DIR"]).unwrap_or_default();

    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;

    let (kb, c) = clean_darwin_user_runtime_dir(temp_dir.trim(), "temp", "Darwin user temp files");
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);

    let (kb, c) =
        clean_darwin_user_runtime_dir(cache_dir.trim(), "cache", "Darwin user cache files");
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);

    (total_kb, total_count)
}

/// 对齐 SH `_clean_darwin_user_runtime_dir()` 第 199-292 行。
fn clean_darwin_user_runtime_dir(runtime_dir: &str, kind: &str, label: &str) -> (u64, u64) {
    // 解析环境变量配置
    let age_days: u32 = std::env::var("MOLE_DARWIN_USER_RUNTIME_AGE_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7);
    let max_items: u64 = std::env::var("MOLE_DARWIN_USER_RUNTIME_MAX_ITEMS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    let scan_timeout: f64 = std::env::var("MOLE_DARWIN_USER_RUNTIME_SCAN_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8.0);

    // 检查目录是否存在
    if runtime_dir.is_empty() || !Path::new(runtime_dir).is_dir() {
        return (0, 0);
    }

    // 安全检查：验证路径格式和所有者
    if !darwin_user_runtime_dir_is_safe(runtime_dir, kind) {
        return (0, 0);
    }

    // 获取当前用户 ID
    let current_uid = run_with_timeout_capture(1.0, "id", &["-u"]).unwrap_or_default();
    let current_uid = current_uid.trim();
    if current_uid.is_empty() {
        return (0, 0);
    }

    let dry_run = is_dry_run();
    let mut count: u64 = 0;
    let mut total_size_kb: u64 = 0;
    let mut hit_cap = false;

    // 1) 清理旧文件（排除数据库文件）
    if count < max_items {
        if let Some(stdout) = run_with_timeout_capture(
            scan_timeout,
            "find",
            &[
                "-P",
                runtime_dir,
                "-xdev",
                "-mindepth",
                "1",
                "-user",
                current_uid,
                "-type",
                "f",
                "-mtime",
                &format!("+{}", age_days),
                "!",
                "-name",
                "*.sqlite",
                "!",
                "-name",
                "*.sqlite-shm",
                "!",
                "-name",
                "*.sqlite-wal",
                "!",
                "-name",
                "*.db",
                "!",
                "-name",
                "*.plist",
                "-print0",
            ],
        ) {
            for entry in stdout.split('\0') {
                let f = entry.trim();
                if f.is_empty()
                    || !Path::new(f).exists()
                    || Path::new(f)
                        .symlink_metadata()
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false)
                {
                    continue;
                }

                // 跳过受保护的路径
                if should_protect_path(f) || is_path_whitelisted_from_global(f) {
                    continue;
                }

                let item_size_kb = get_path_size_kb(f);

                if dry_run {
                    if dry_run_register_cleanup_target(f) {
                        count += 1;
                        total_size_kb = total_size_kb.saturating_add(item_size_kb);
                    }
                } else if safe_remove(f, true) {
                    count += 1;
                    total_size_kb = total_size_kb.saturating_add(item_size_kb);
                }

                if count >= max_items {
                    hit_cap = true;
                    break;
                }
            }
        }
    }

    // 2) 清理空目录（仅当文件清理未达到上限时）
    if !hit_cap && count < max_items {
        if let Some(stdout) = run_with_timeout_capture(
            scan_timeout,
            "find",
            &[
                "-P",
                runtime_dir,
                "-xdev",
                "-mindepth",
                "1",
                "-user",
                current_uid,
                "-type",
                "d",
                "-empty",
                "-mtime",
                &format!("+{}", age_days),
                "-print0",
            ],
        ) {
            for entry in stdout.split('\0') {
                let d = entry.trim();
                if d.is_empty()
                    || !Path::new(d).is_dir()
                    || Path::new(d)
                        .symlink_metadata()
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false)
                {
                    continue;
                }

                // 跳过受保护的路径
                if should_protect_path(d) || is_path_whitelisted_from_global(d) {
                    continue;
                }

                if dry_run {
                    if dry_run_register_cleanup_target(d) {
                        count += 1;
                    }
                } else if safe_remove(d, true) {
                    count += 1;
                }

                if count >= max_items {
                    hit_cap = true;
                    break;
                }
            }
        }
    }

    if count > 0 {
        let cap_note = if hit_cap { ", capped" } else { "" };
        if dry_run {
            log_info(&format!(
                "{} · {} old items, {} dry{}",
                label,
                count,
                bytes_to_human(total_size_kb.saturating_mul(1024)),
                cap_note
            ));
        } else {
            log_info(&format!(
                "{} · {} old items, {}{}",
                label,
                count,
                bytes_to_human(total_size_kb.saturating_mul(1024)),
                cap_note
            ));
            note_activity();
        }
    }

    (total_size_kb, count)
}

/// 对齐 SH `_darwin_user_runtime_dir_is_safe()` 第 179-197 行。
fn darwin_user_runtime_dir_is_safe(runtime_dir: &str, kind: &str) -> bool {
    if runtime_dir.is_empty() || !Path::new(runtime_dir).is_dir() {
        return false;
    }
    if Path::new(runtime_dir).is_symlink() {
        return false;
    }
    let resolved = match std::fs::canonicalize(runtime_dir) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => return false,
    };

    // 验证路径格式
    let pattern = match kind {
        "temp" => r"^/private/var/folders/[^/]+/[^/]+/T$",
        "cache" => r"^/private/var/folders/[^/]+/[^/]+/C$",
        _ => return false,
    };

    if !regex::Regex::new(pattern)
        .map(|re| re.is_match(&resolved))
        .unwrap_or(false)
    {
        debug_log(&format!(
            "Skipping unexpected Darwin user runtime dir: {} -> {}",
            runtime_dir, resolved
        ));
        return false;
    }

    // 验证目录所有者
    let owner_uid = run_with_timeout_capture(1.0, "stat", &["-f%u", &resolved]).unwrap_or_default();
    let owner_uid = owner_uid.trim();
    let current_uid = run_with_timeout_capture(1.0, "id", &["-u"]).unwrap_or_default();
    let current_uid = current_uid.trim();

    !owner_uid.is_empty() && owner_uid == current_uid
}

pub fn clean_recent_items() -> (u64, u64) {
    let h = home_dir();
    let shared = format!("{h}/Library/Application Support/com.apple.sharedfilelist");
    let mut targets: Vec<String> = Vec::new();
    if Path::new(&shared).is_dir() {
        for f in [
            "com.apple.LSSharedFileList.RecentApplications.sfl2",
            "com.apple.LSSharedFileList.RecentDocuments.sfl2",
            "com.apple.LSSharedFileList.RecentServers.sfl2",
            "com.apple.LSSharedFileList.RecentHosts.sfl2",
            "com.apple.LSSharedFileList.RecentApplications.sfl",
            "com.apple.LSSharedFileList.RecentDocuments.sfl",
            "com.apple.LSSharedFileList.RecentServers.sfl",
            "com.apple.LSSharedFileList.RecentHosts.sfl",
        ] {
            targets.push(format!("{shared}/{f}"));
        }
    }
    targets.push(format!(
        "{h}/Library/Preferences/com.apple.recentitems.plist"
    ));
    let refs: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
    safe_clean(&refs, "Recent items lists")
}

/// 对齐 SH `_clean_mail_downloads()` 第 112-171 行。
fn clean_mail_downloads() -> (u64, u64) {
    // 对齐 SH: pgrep -x "Mail" —— Mail 运行时跳过清理，避免损坏附件
    if Command::new("pgrep")
        .args(["-x", "Mail"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        debug_log("Mail is running, skipping Mail Downloads cleanup");
        return (0, 0);
    }

    let h = home_dir();
    // 对齐 SH: 支持环境变量覆盖默认值
    let mail_age_days: u32 = std::env::var("MOLE_MAIL_AGE_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MOLE_MAIL_AGE_DAYS);
    let min_kb: u64 = std::env::var("MOLE_MAIL_DOWNLOADS_MIN_KB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MOLE_MAIL_DOWNLOADS_MIN_KB);

    let mail_dirs = [
        format!("{h}/Library/Mail Downloads"),
        format!("{h}/Library/Containers/com.apple.mail/Data/Library/Mail Downloads"),
    ];

    let mut total_kb: u64 = 0;
    let mut count: u64 = 0;
    let dry_run = is_dry_run();

    for target_path in &mail_dirs {
        if !Path::new(target_path).is_dir() {
            continue;
        }
        let dir_size_kb = get_path_size_kb(target_path);
        if dir_size_kb < min_kb {
            continue;
        }
        // find -type f -mtime +N
        let Some(stdout) = run_with_timeout_capture(
            30.0,
            "find",
            &[
                target_path,
                "-type",
                "f",
                "-mtime",
                &format!("+{mail_age_days}"),
                "-print0",
            ],
        ) else {
            continue;
        };
        for entry in stdout.split('\0') {
            let f = entry.trim();
            if f.is_empty() || !Path::new(f).is_file() {
                continue;
            }
            let size_kb = get_path_size_kb(f);
            if !dry_run {
                if safe_remove(f, true) {
                    count += 1;
                    total_kb = total_kb.saturating_add(size_kb);
                }
            } else if dry_run_register_cleanup_target(f) {
                count += 1;
                total_kb = total_kb.saturating_add(size_kb);
            }
        }
    }
    if count > 0 {
        log_info(&format!(
            "Cleaned {count} mail attachments older than {mail_age_days}d, about {}",
            bytes_to_human(total_kb.saturating_mul(1024))
        ));
        note_activity();
    }
    (total_kb, count)
}

// =============================================================================
// 浏览器旧版本清理(Chrome / Edge / Brave / EdgeUpdater)
// =============================================================================

/// 对齐 SH `clean_*_old_versions()` 共享核心逻辑。
/// `keep_newest` 为 true 时额外保留 mtime 最晚的版本目录（Chrome 专用，避免删除刚下载尚未切换 Current 的版本）。
/// 对齐 Mole `_clean_chromium_old_versions()`：
///   - 成功删除才计入 total_size / count（修复"删除失败仍上报为已清理"的幽灵计数 bug）
///   - 非 dry-run 时每个旧版本目录删除前重新调用 `is_running()` 探测进程状态，
///     浏览器在清理过程中启动时立即中止剩余目录（对齐 Mole 逐目录二次检查）
fn clean_old_browser_versions(
    label: &str,
    app_paths: &[String],
    versions_subpath: &str,
    keep_newest: bool,
    is_running: &dyn Fn() -> bool,
) -> (u64, u64) {
    let dry_run = is_dry_run();
    let mut total_size = 0u64;
    let mut count = 0u64;

    for app_path in app_paths {
        if !Path::new(app_path).is_dir() {
            continue;
        }

        let versions_dir = format!("{app_path}/{versions_subpath}");
        if !Path::new(&versions_dir).is_dir() {
            continue;
        }

        let current_link = format!("{versions_dir}/Current");
        if !Path::new(&current_link).is_symlink() {
            continue;
        }

        let current_version = match std::fs::read_link(&current_link) {
            Ok(target) => target
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            Err(_) => continue,
        };
        if current_version.is_empty() {
            continue;
        }

        let current_path = format!("{versions_dir}/{current_version}");
        if !Path::new(&current_path).is_dir() {
            log_warning(&format!(
                "{} Current symlink is broken · skipping version cleanup",
                label
            ));
            continue;
        }

        let newest_version = if keep_newest {
            find_newest_version_dir(&versions_dir, &current_version)
        } else {
            None
        };

        let mut old_versions: Vec<String> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&versions_dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if !p.is_dir() {
                    continue;
                }
                let name = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if name == "Current" {
                    continue;
                }
                if name == current_version {
                    continue;
                }
                if let Some(ref newest) = newest_version {
                    if name == *newest {
                        continue;
                    }
                }
                let p_str = p.to_string_lossy().to_string();
                if is_path_whitelisted_from_global(&p_str) {
                    continue;
                }
                old_versions.push(p_str);
            }
        }

        if old_versions.is_empty() {
            continue;
        }

        for dir in &old_versions {
            // 非 dry-run 时逐目录二次检查进程状态（对齐 Mole L536-554）：
            // 浏览器可能在删除过程中启动，继续删除会破坏活跃版本。
            if !dry_run && is_running() {
                log_warning(&format!("{label} started · remaining old versions skipped"));
                break;
            }
            let size_kb = get_path_size_kb(dir);
            if dry_run {
                // dry-run：只统计，不删除
                total_size = total_size.saturating_add(size_kb);
                count += 1;
            } else {
                // 实际删除：成功才计数（对齐 Mole L567-579，修复幽灵计数 bug）
                let removed = if is_admin_authorized() {
                    safe_sudo_remove(dir, Some(size_kb)) == MOLE_OK
                } else {
                    safe_remove(dir, true)
                };
                if removed {
                    total_size = total_size.saturating_add(size_kb);
                    count += 1;
                } else {
                    debug_log(&format!("{label} old version removal failed: {dir}"));
                }
            }
        }
    }

    if count > 0 {
        let size_human = bytes_to_human(total_size.saturating_mul(1024));
        if dry_run {
            log_info(&format!("{label} · {count} dirs, {size_human} dry"));
        } else {
            log_info(&format!("{label} · {count} dirs, {size_human}"));
        }
        note_activity();
    }

    (total_size, count)
}

/// 在 versions 目录中找出 mtime 比当前版本更新的版本目录（Chrome 自动更新保护）。
fn find_newest_version_dir(versions_dir: &str, current_version: &str) -> Option<String> {
    let mut newest_name: Option<String> = None;
    let mut newest_mtime: u64 = 0;

    if let Ok(rd) = std::fs::read_dir(versions_dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if name == "Current" {
                continue;
            }
            if let Ok(metadata) = p.metadata() {
                if let Ok(mtime) = metadata.modified() {
                    let secs = mtime
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    if secs > newest_mtime {
                        newest_mtime = secs;
                        newest_name = Some(name);
                    }
                }
            }
        }
    }

    let current_path = format!("{versions_dir}/{current_version}");
    if let Ok(metadata) = std::fs::metadata(&current_path) {
        if let Ok(mtime) = metadata.modified() {
            let current_secs = mtime
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if newest_mtime <= current_secs {
                return None;
            }
        }
    }

    newest_name
}

fn is_google_chrome_running() -> bool {
    pgrep_x("Google Chrome")
        || Command::new("pgrep")
            .args(["-x", "Google Chrome Helper"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        || Command::new("pgrep")
            .args(["-f", "/Google Chrome.app/"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

pub fn clean_chrome_old_versions() -> (u64, u64) {
    if is_google_chrome_running() {
        log_warning("Google Chrome running · old versions cleanup skipped");
        return (0, 0);
    }
    let h = home_dir();
    let app_paths = if let Ok(env) = std::env::var("MOLE_CHROME_APP_PATHS") {
        env.split(':').map(|s| s.to_string()).collect()
    } else {
        vec![
            "/Applications/Google Chrome.app".to_string(),
            format!("{h}/Applications/Google Chrome.app"),
        ]
    };
    clean_old_browser_versions(
        "Chrome old versions",
        &app_paths,
        "Contents/Frameworks/Google Chrome Framework.framework/Versions",
        true,
        &|| is_google_chrome_running(),
    )
}

pub fn clean_edge_old_versions() -> (u64, u64) {
    if pgrep_x("Microsoft Edge") {
        log_warning("Microsoft Edge running · old versions cleanup skipped");
        return (0, 0);
    }
    let h = home_dir();
    let app_paths = if let Ok(env) = std::env::var("MOLE_EDGE_APP_PATHS") {
        env.split(':').map(|s| s.to_string()).collect()
    } else {
        vec![
            "/Applications/Microsoft Edge.app".to_string(),
            format!("{h}/Applications/Microsoft Edge.app"),
        ]
    };
    clean_old_browser_versions(
        "Edge old versions",
        &app_paths,
        "Contents/Frameworks/Microsoft Edge Framework.framework/Versions",
        false,
        &|| pgrep_x("Microsoft Edge"),
    )
}

pub fn clean_brave_old_versions() -> (u64, u64) {
    if pgrep_x("Brave Browser") {
        log_warning("Brave Browser running · old versions cleanup skipped");
        return (0, 0);
    }
    let h = home_dir();
    let app_paths = if let Ok(env) = std::env::var("MOLE_BRAVE_APP_PATHS") {
        env.split(':').map(|s| s.to_string()).collect()
    } else {
        vec![
            "/Applications/Brave Browser.app".to_string(),
            format!("{h}/Applications/Brave Browser.app"),
        ]
    };
    clean_old_browser_versions(
        "Brave old versions",
        &app_paths,
        "Contents/Frameworks/Brave Browser Framework.framework/Versions",
        false,
        &|| pgrep_x("Brave Browser"),
    )
}

pub fn clean_edge_updater_old_versions() -> (u64, u64) {
    let h = home_dir();
    let updater_dir =
        format!("{h}/Library/Application Support/Microsoft/EdgeUpdater/apps/msedge-stable");
    if !Path::new(&updater_dir).is_dir() {
        return (0, 0);
    }
    if pgrep_x("Microsoft Edge") {
        log_warning("Microsoft Edge running · updater cleanup skipped");
        return (0, 0);
    }
    let mut versions: Vec<(String, String)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&updater_dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let name = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                versions.push((name, p.to_string_lossy().to_string()));
            }
        }
    }
    if versions.len() < 2 {
        return (0, 0);
    }
    // sort -V:简化为按字符串 lexicographic + numeric 段比较
    versions.sort_by(|a, b| version_compare(&a.0, &b.0));
    let latest = versions.last().map(|(n, _)| n.clone()).unwrap_or_default();
    let dry_run = is_dry_run();
    let mut total_size = 0u64;
    let mut count = 0u64;
    for (name, path) in &versions {
        if name == &latest {
            continue;
        }
        if is_path_whitelisted_from_global(path) {
            continue;
        }
        let size_kb = get_path_size_kb(path);
        total_size = total_size.saturating_add(size_kb);
        count += 1;
        if !dry_run {
            let _ = safe_remove(path, true);
        }
    }
    if count > 0 {
        let size_human = bytes_to_human(total_size.saturating_mul(1024));
        if dry_run {
            log_info(&format!(
                "Edge updater old versions · {count} dirs, {size_human} dry"
            ));
        } else {
            log_info(&format!(
                "Edge updater old versions · {count} dirs, {size_human}"
            ));
        }
        note_activity();
    }
    (total_size, count)
}

/// 简化 sort -V 实现:按数字段顺序比较
fn version_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let mut ai = a.split('.');
    let mut bi = b.split('.');
    loop {
        match (ai.next(), bi.next()) {
            (Some(x), Some(y)) => {
                let cmp = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(xn), Ok(yn)) => xn.cmp(&yn),
                    _ => x.cmp(y),
                };
                if cmp != std::cmp::Ordering::Equal {
                    return cmp;
                }
            }
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (None, None) => return std::cmp::Ordering::Equal,
        }
    }
}

pub fn clean_finder_metadata() -> (u64, u64) {
    if std::env::var("PROTECT_FINDER_METADATA").unwrap_or_default() == "true" {
        return (0, 0);
    }
    clean_ds_store_tree(&home_dir(), "Home directory, .DS_Store")
}

// =============================================================================
// clean_support_app_data
// =============================================================================

/// 对齐 SH `clean_support_app_data()` 第 559-587 行。
pub fn clean_support_app_data() -> (u64, u64) {
    let h = home_dir();
    let support_age_days: u32 = std::env::var("MOLE_SUPPORT_CACHE_AGE_DAYS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(30);

    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;

    let crash_dir = format!("{h}/Library/Application Support/CrashReporter");
    if Path::new(&crash_dir).is_dir() && !Path::new(&crash_dir).is_symlink() {
        let pre_kb = get_path_size_kb(&crash_dir);
        safe_find_delete(&crash_dir, "*", support_age_days, "f");
        if pre_kb > 0 {
            total_kb = total_kb.saturating_add(pre_kb);
            total_count += 1;
        }
    }

    let idle_dir = format!("{h}/Library/Application Support/com.apple.idleassetsd");
    if Path::new(&idle_dir).is_dir() && !Path::new(&idle_dir).is_symlink() {
        let pre_kb = get_path_size_kb(&idle_dir);
        safe_find_delete(&idle_dir, "*", support_age_days, "f");
        if pre_kb > 0 {
            total_kb = total_kb.saturating_add(pre_kb);
            total_count += 1;
        }
    }

    let test_mode = std::env::var("MOLE_TEST_MODE").unwrap_or_default() == "1"
        || std::env::var("MOLE_TEST_NO_AUTH").unwrap_or_default() == "1";
    if !test_mode {
        let sys_idle = "/Library/Application Support/com.apple.idleassetsd/Customer";
        let sudo_dir_exists = crate::core::sudo::sudo_output(&["/bin/test", "-d", sys_idle])
            .status
            .success();
        if sudo_dir_exists {
            let (kb, cnt) = safe_sudo_find_delete(sys_idle, "*", support_age_days, "f");
            if kb > 0 || cnt > 0 {
                total_kb = total_kb.saturating_add(kb);
                total_count = total_count.saturating_add(cnt);
            }
        }
    }

    let jobs: Vec<(Vec<String>, &'static str)> = vec![
        (
            vec![format!("{h}/Library/Messages/StickerCache/*")],
            "Messages sticker cache",
        ),
        (
            vec![format!(
                "{h}/Library/Messages/Caches/Previews/Attachments/*"
            )],
            "Messages preview attachment cache",
        ),
        (
            vec![format!(
                "{h}/Library/Messages/Caches/Previews/StickerCache/*"
            )],
            "Messages preview sticker cache",
        ),
    ];
    let (jkb, jcnt) = run_jobs(&jobs);
    total_kb = total_kb.saturating_add(jkb);
    total_count = total_count.saturating_add(jcnt);

    (total_kb, total_count)
}

pub fn clean_app_caches() -> super::ModuleScanResult {
    let h = home_dir();
    let mut items: Vec<super::SubItemResult> = Vec::new();

    macro_rules! add_item {
        ($kb:expr, $cnt:expr, $id:literal, $title:literal) => {
            items.push(super::SubItemResult::new($id, $title, $kb, $cnt));
        };
    }

    stop_section_spinner();

    start_section_spinner("app_caches", "Scanning app caches...");
    let macos_jobs_part1: Vec<(Vec<String>, &'static str)> = vec![
        (
            vec![format!("{h}/Library/Saved Application State/*")],
            "Saved application states",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.photoanalysisd")],
            "Photo analysis cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.akd")],
            "Apple ID cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.WebKit.Networking/*")],
            "WebKit network cache",
        ),
        (
            vec![format!("{h}/Library/DiagnosticReports/*")],
            "Diagnostic reports",
        ),
        (
            vec![format!(
                "{h}/Library/Caches/com.apple.QuickLook.thumbnailcache"
            )],
            "QuickLook thumbnails",
        ),
        (
            vec![format!("{h}/Library/Caches/Quick Look/*")],
            "QuickLook cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.iconservices*")],
            "Icon services cache",
        ),
    ];
    let (kb, c) = run_jobs(&macos_jobs_part1);
    add_item!(kb, c, "app_caches_system", "System app caches");

    let (kb, c) = clean_incomplete_downloads();
    add_item!(kb, c, "app_caches_downloads", "Incomplete downloads");

    let macos_jobs_part2: Vec<(Vec<String>, &'static str)> = vec![
        (
            vec![format!("{h}/Library/IdentityCaches/*")],
            "Identity caches",
        ),
        (
            vec![format!("{h}/Library/Suggestions/*")],
            "Siri suggestions cache",
        ),
        (
            vec![format!("{h}/Library/Calendars/Calendar Cache")],
            "Calendar cache",
        ),
        (
            vec![format!(
                "{h}/Library/Application Support/AddressBook/Sources/*/Photos.cache"
            )],
            "Address Book photo cache",
        ),
    ];
    let (kb, c) = run_jobs(&macos_jobs_part2);
    add_item!(kb, c, "app_caches_identity", "System identity caches");

    let (kb, c) = clean_support_app_data();
    add_item!(kb, c, "app_caches_support", "App support data");

    stop_section_spinner();

    start_section_spinner("app_caches", "Scanning sandboxed apps...");
    let sandbox_jobs: Vec<(Vec<String>, &'static str)> = vec![
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.wallpaper.agent/Data/Library/Caches/*"
            )],
            "Wallpaper agent cache",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.mediaanalysisd/Data/Library/Caches/*"
            )],
            "Media analysis cache",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.mediaanalysisd/Data/tmp/*"
            )],
            "Media analysis temp files",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.AppStore/Data/Library/Caches/*"
            )],
            "App Store cache",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.configurator.xpc.InternetService/Data/tmp/*"
            )],
            "Apple Configurator temp files",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.wallpaper.extension.aerials/Data/tmp/*"
            )],
            "Wallpaper aerials temp files",
        ),
        (
            vec![format!("{h}/Library/Containers/com.apple.geod/Data/tmp/*")],
            "Geod temp files",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.stocks/Data/Library/Caches/*"
            )],
            "Stocks cache",
        ),
        (
            vec![format!(
                "{h}/Library/Application Support/com.apple.wallpaper/aerials/thumbnails/*"
            )],
            "Wallpaper aerials thumbnails",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.helpd/*")],
            "macOS Help system cache",
        ),
        (
            vec![format!("{h}/Library/Caches/GeoServices/*")],
            "Maps geo tile cache",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.AvatarUI.AvatarPickerMemojiPicker/Data/Library/Caches/*"
            )],
            "Memoji picker cache",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.AMPArtworkAgent/Data/Library/Caches/*"
            )],
            "Music album art cache",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.CoreDevice.CoreDeviceService/Data/Library/Caches/*"
            )],
            "CoreDevice service cache",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.NeptuneOneExtension/Data/Library/Caches/*"
            )],
            "Apple Intelligence extension cache",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.AppleMediaServicesUI.UtilityExtension/Data/tmp/*"
            )],
            "Apple Media Services temp files",
        ),
    ];
    let (kb1, c1) = run_jobs(&sandbox_jobs);

    let extra_jobs: Vec<(Vec<String>, &'static str)> = vec![
        (
            vec![format!("{h}/Library/Caches/com.apple.AppleMediaServices/*")],
            "Apple Media Services cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.duetexpertd/*")],
            "Duet Expert cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.parsecd/*")],
            "Parsecd cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.python/*")],
            "Apple Python cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.e5rt.e5bundlecache/*")],
            "Apple Intelligence runtime cache",
        ),
    ];
    let (kb2, c2) = run_jobs(&extra_jobs);

    let containers_dir = format!("{h}/Library/Containers");
    let mut kb3 = 0u64;
    let mut c3 = 0u64;
    if Path::new(&containers_dir).is_dir() {
        let (kb, c) = process_container_cache();
        kb3 += kb;
        c3 += c;
        let (kb, c) = clean_group_container_caches();
        kb3 += kb;
        c3 += c;
    }

    let sandbox_total = kb1 + kb2 + kb3;
    let sandbox_cnt = c1 + c2 + c3;
    add_item!(
        sandbox_total,
        sandbox_cnt,
        "app_caches_sandbox",
        "Sandbox & container caches"
    );

    stop_section_spinner();

    super::ModuleScanResult { items }
}

fn clean_incomplete_downloads() -> (u64, u64) {
    let h = home_dir();
    let groups = [
        (
            "Safari incomplete downloads",
            format!("{h}/Downloads/*.download"),
        ),
        (
            "Chrome incomplete downloads",
            format!("{h}/Downloads/*.crdownload"),
        ),
        (
            "Partial incomplete downloads",
            format!("{h}/Downloads/*.part"),
        ),
    ];
    let mut total_kb = 0u64;
    let mut total_count = 0u64;
    for (label, pattern) in &groups {
        for f in expand_glob_paths(pattern) {
            // lsof 看是否被打开
            let in_use = Command::new("lsof")
                .args(["-F", "n", "--", &f])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if in_use {
                log_warning(&format!(
                    "Skipping active download: {}",
                    Path::new(&f)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("?")
                ));
                continue;
            }
            let (kb, c) = safe_clean(&[&f], label);
            total_kb = total_kb.saturating_add(kb);
            total_count = total_count.saturating_add(c);
        }
    }
    (total_kb, total_count)
}

fn process_container_cache() -> (u64, u64) {
    let h = home_dir();
    let containers_dir = format!("{h}/Library/Containers");
    if !Path::new(&containers_dir).is_dir() {
        return (0, 0);
    }
    let mut total_kb = 0u64;
    let mut total_count = 0u64;
    let mut total_size_partial = false;
    let dry_run = is_dry_run();

    // SH: precise_size_limit / precise_size_used — 限制精确 du 次数
    let precise_size_limit: u64 = std::env::var("MOLE_CONTAINER_CACHE_PRECISE_SIZE_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64);
    let mut precise_size_used = 0u64;

    let Ok(rd) = std::fs::read_dir(&containers_dir) else {
        return (0, 0);
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_dir() || p.is_symlink() {
            continue;
        }
        let bundle_id = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        // 关键检查:系统组件 / 受保护数据应用
        if is_critical_system_component(&bundle_id) {
            continue;
        }
        if should_protect_data(&bundle_id) {
            continue;
        }
        let cache_dir = p.join("Data/Library/Caches");
        if !cache_dir.is_dir() || cache_dir.is_symlink() {
            continue;
        }
        let cache_str = cache_dir.to_string_lossy().to_string();

        // SH: cache_top_level_entry_count_capped 快速检查
        let item_count = cache_top_level_entry_count_capped(&cache_str, 101);
        if item_count == 0 {
            continue;
        }

        // SH: 每个 container 计 1 count
        total_count += 1;

        // SH: 精确 du 只在 ≤100 条目且未达限制时执行
        if item_count <= 100 && precise_size_used < precise_size_limit {
            let size = get_path_size_kb(&cache_str);
            total_kb = total_kb.saturating_add(size);
            precise_size_used += 1;
        } else {
            total_size_partial = true;
        }

        // SH: dry_run 时跳过删除，但仍计数
        if dry_run {
            dry_run_register_cleanup_target(&cache_str);
            continue;
        }

        // SH: 删除 cache_dir 下所有子项
        let pattern = format!("{cache_str}/*");
        for item in expand_glob_paths(&pattern) {
            let _ = safe_remove(&item, true);
        }
    }
    if total_count > 0 {
        if total_size_partial {
            log_info("Sandboxed app caches (partial)");
        } else {
            log_info(&format!(
                "Sandboxed app caches, {}",
                bytes_to_human(total_kb.saturating_mul(1024))
            ));
        }
        note_activity();
    }
    (total_kb, total_count)
}

/// 对齐 SH `clean_group_container_caches()` 第 966-1058 行。
fn clean_group_container_caches() -> (u64, u64) {
    let h = home_dir();
    let group_dir = format!("{h}/Library/Group Containers");
    if !Path::new(&group_dir).is_dir() {
        return (0, 0);
    }
    let mut total_kb = 0u64;
    let mut total_count = 0u64;
    let mut total_size_partial = false;
    let mut found_any = false;
    let dry_run = is_dry_run();
    let Ok(rd) = std::fs::read_dir(&group_dir) else {
        return (0, 0);
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_dir() || p.is_symlink() {
            continue;
        }
        // Skip unreadable containers (avoids TCC/privacy prompts)
        if std::fs::read_dir(&p).is_err() {
            continue;
        }
        let cid = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        // Skip Apple-owned shared containers
        if cid.starts_with("com.apple.")
            || cid.starts_with("group.com.apple.")
            || cid.starts_with("systemgroup.com.apple.")
        {
            continue;
        }

        // Safari Web Extension protection: 清理其缓存会触发扩展重初始化
        let safari_containers = format!("{h}/Library/Containers/{cid}");
        if Path::new(&safari_containers).is_dir() {
            if let Ok(rd2) = std::fs::read_dir(&safari_containers) {
                if rd2.flatten().any(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .to_lowercase()
                        .contains("safari")
                }) {
                    continue;
                }
            }
        }

        let normalized = cid.strip_prefix("group.").unwrap_or(&cid).to_string();
        let protected_container = should_protect_data(&cid) || should_protect_data(&normalized);

        let mut candidates: Vec<String> = vec![
            p.join("Logs").to_string_lossy().to_string(),
            p.join("Library/Logs").to_string_lossy().to_string(),
        ];
        if !protected_container {
            candidates.push(p.join("tmp").to_string_lossy().to_string());
            candidates.push(p.join("Library/tmp").to_string_lossy().to_string());
            candidates.push(p.join("Caches").to_string_lossy().to_string());
            candidates.push(p.join("Library/Caches").to_string_lossy().to_string());
        }
        for cand in &candidates {
            if !Path::new(cand).is_dir() || Path::new(cand).is_symlink() {
                continue;
            }
            if is_path_whitelisted_from_global(cand) {
                continue;
            }

            let quick_count = cache_top_level_entry_count_capped(cand, 101);
            if quick_count == 0 {
                continue;
            }

            let mut candidate_changed = false;
            let mut candidate_size_kb = 0u64;

            let pat = format!("{cand}/*");
            for item in expand_glob_paths(&pat) {
                if Path::new(&item).is_symlink() {
                    continue;
                }
                if should_protect_path(&item) || is_path_whitelisted_from_global(&item) {
                    continue;
                }

                candidate_changed = true;

                if quick_count > 100 {
                    total_size_partial = true;
                    if !dry_run {
                        let _ = safe_remove(&item, true);
                    }
                } else {
                    let size = get_path_size_kb(&item);
                    if dry_run {
                        candidate_size_kb = candidate_size_kb.saturating_add(size);
                    } else if safe_remove(&item, true) {
                        candidate_size_kb = candidate_size_kb.saturating_add(size);
                    }
                }
            }

            if candidate_changed {
                total_count += 1;
                total_kb = total_kb.saturating_add(candidate_size_kb);
                found_any = true;
            }
        }
    }
    if found_any {
        if total_size_partial {
            log_info("Group Containers logs/caches (partial)");
        } else {
            log_info(&format!(
                "Group Containers logs/caches, {}",
                bytes_to_human(total_kb.saturating_mul(1024))
            ));
        }
        note_activity();
    }
    (total_kb, total_count)
}

/// 清理主流浏览器缓存。
///
/// 口径（对齐腾讯柠檬 garbage1_zh.xml「上网垃圾」）：
///   - 仅清理 `~/Library/Caches/` 下的 HTTP 缓存与更新器/旧版本残留
///   - **不触碰** `~/Library/Application Support/` 下的任何浏览器 profile 数据
///     （Service Worker / Code Cache / GPUCache 等目录级统一回避），
///     从根上排除误伤站点数据与登录态的可能
pub fn clean_browsers() -> (u64, u64) {
    let h = home_dir();
    let mut total_kb = 0u64;
    let mut total_count = 0u64;

    // 先清理不依赖浏览器运行状态的缓存
    let mut jobs: Vec<(Vec<String>, &'static str)> = vec![
        (
            vec![format!("{h}/Library/Caches/com.apple.Safari/*")],
            "Safari cache",
        ),
        // Chrome Caches（可以安全清理，不依赖运行状态）
        (
            vec![format!("{h}/Library/Caches/Google/Chrome/*")],
            "Chrome cache",
        ),
    ];
    let (kb, c) = run_jobs(&jobs);
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);

    // GoogleUpdater 缓存（不依赖运行状态）
    jobs = vec![
        (
            vec![format!(
                "{h}/Library/Application Support/Google/GoogleUpdater/crx_cache/*"
            )],
            "GoogleUpdater CRX cache",
        ),
        (
            vec![format!(
                "{h}/Library/Application Support/Google/GoogleUpdater/*.old"
            )],
            "GoogleUpdater old files",
        ),
    ];
    let (kb, c) = run_jobs(&jobs);
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);

    // Chromium / Puppeteer / Edge cache（对齐 SH：在 GoogleUpdater 之后、Arc 之前）
    jobs = vec![
        (
            vec![format!("{h}/Library/Caches/Chromium/*")],
            "Chromium cache",
        ),
        (
            vec![format!("{h}/.cache/puppeteer/*")],
            "Puppeteer browser cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.microsoft.edgemac/*")],
            "Edge cache",
        ),
    ];
    let (kb, c) = run_jobs(&jobs);
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);

    // Arc
    if Path::new(&format!("{h}/Library/Application Support/Arc")).is_dir() {
        let (kb, c) = safe_clean(
            &[&format!("{h}/Library/Caches/company.thebrowser.Browser/*")],
            "Arc cache",
        );
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
    }

    let (kb, c) = safe_clean(
        &[&format!("{h}/Library/Caches/company.thebrowser.dia/*")],
        "Dia cache",
    );
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);

    // Brave
    if Path::new(&format!("{h}/Library/Application Support/BraveSoftware")).is_dir() {
        let (kb, c) = safe_clean(
            &[&format!("{h}/Library/Caches/BraveSoftware/Brave-Browser/*")],
            "Brave cache",
        );
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
    }

    // Helium / Yandex
    if Path::new(&format!("{h}/Library/Application Support/net.imput.helium")).is_dir() {
        let (kb, c) = safe_clean(
            &[&format!("{h}/Library/Caches/net.imput.helium/*")],
            "Helium cache",
        );
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
    }
    if Path::new(&format!("{h}/Library/Application Support/Yandex")).is_dir() {
        let (kb, c) = safe_clean(
            &[&format!("{h}/Library/Caches/Yandex/YandexBrowser/*")],
            "Yandex cache",
        );
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
    }

    // Firefox
    let firefox_running = pgrep_x("Firefox");
    if firefox_running {
        log_warning("Firefox is running · cache cleanup skipped");
    } else {
        let (kb, c) = safe_clean(&[&format!("{h}/Library/Caches/Firefox/*")], "Firefox cache");
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
    }

    let (kb, c) = safe_clean(
        &[&format!("{h}/Library/Caches/com.operasoftware.Opera/*")],
        "Opera cache",
    );
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);

    // Vivaldi
    if Path::new(&format!("{h}/Library/Application Support/Vivaldi")).is_dir() {
        let (kb, c) = safe_clean(
            &[&format!("{h}/Library/Caches/com.vivaldi.Vivaldi/*")],
            "Vivaldi cache",
        );
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
    }

    let final_jobs: Vec<(Vec<String>, &'static str)> = vec![
        (vec![format!("{h}/Library/Caches/Comet/*")], "Comet cache"),
        (
            vec![format!("{h}/Library/Caches/com.kagi.kagimacOS/*")],
            "Orion cache",
        ),
        (vec![format!("{h}/Library/Caches/zen/*")], "Zen cache"),
    ];
    let (kb, c) = run_jobs(&final_jobs);
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);

    let (kb, c) = clean_chrome_old_versions();
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);
    let (kb, c) = clean_edge_old_versions();
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);
    let (kb, c) = clean_edge_updater_old_versions();
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);
    let (kb, c) = clean_brave_old_versions();
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);

    (total_kb, total_count)
}

// =============================================================================
// clean_cloud_storage / clean_office_applications / clean_virtualization_tools
// =============================================================================

/// 对齐 SH `clean_cloud_storage()` 第 1456-1484 行。
/// 注意 Dropbox / Google Drive / OneDrive 运行时跳过缓存清理。
pub fn clean_cloud_storage() -> (u64, u64) {
    debug_log("Cleaning cloud storage caches...");
    let h = home_dir();
    let mut total_kb = 0u64;
    let mut total_count = 0u64;

    // Dropbox — 运行中跳过；对齐 SH 分两条 safe_clean 调用
    if pgrep_x("Dropbox") {
        log_warning("Dropbox is running · cache cleanup skipped");
    } else {
        let (kb, c) = safe_clean(
            &[&format!("{h}/Library/Caches/com.dropbox.*")],
            "Dropbox cache",
        );
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
        let (kb, c) = safe_clean(
            &[&format!("{h}/Library/Caches/com.getdropbox.dropbox")],
            "Dropbox cache",
        );
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
    }

    // Google Drive — 运行中跳过
    if pgrep_x("Google Drive") {
        log_warning("Google Drive is running · cache cleanup skipped");
    } else {
        let p = format!("{h}/Library/Caches/com.google.GoogleDrive");
        let (kb, c) = safe_clean(&[&p], "Google Drive cache");
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
    }

    // Baidu Netdisk / Alibaba Cloud / Box — 无条件清理
    let (kb, c) = run_jobs(&[
        (
            vec![format!("{h}/Library/Caches/com.baidu.netdisk")],
            "Baidu Netdisk cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.alibaba.teambitiondisk")],
            "Alibaba Cloud cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.box.desktop")],
            "Box cache",
        ),
    ]);
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);

    // OneDrive — 运行中跳过
    if pgrep_x("OneDrive") {
        log_warning("OneDrive is running · cache cleanup skipped");
    } else {
        let p = format!("{h}/Library/Caches/com.microsoft.OneDrive");
        let (kb, c) = safe_clean(&[&p], "OneDrive cache");
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
    }

    (total_kb, total_count)
}

/// 对齐 SH `clean_office_applications()` 第 1486-1508 行。添加 debug_log 匹配 MO_DEBUG。
pub fn clean_office_applications() -> (u64, u64) {
    let h = home_dir();
    let mut total_kb = 0u64;
    let mut total_count = 0u64;

    // Word 系列
    let (kb, c) = run_jobs(&[(
        vec![format!("{h}/Library/Caches/com.microsoft.Word")],
        "Microsoft Word cache",
    )]);
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);

    debug_log("Cleaning Word container cache...");
    let (kb, c) = run_jobs(&[
        (
            vec![format!(
                "{h}/Library/Containers/com.microsoft.Word/Data/Library/Caches/*"
            )],
            "Microsoft Word container cache",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.microsoft.Word/Data/tmp/*"
            )],
            "Microsoft Word temp files",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.microsoft.Word/Data/Library/Logs/*"
            )],
            "Microsoft Word container logs",
        ),
    ]);
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);

    // Excel 系列
    let (kb, c) = run_jobs(&[(
        vec![format!("{h}/Library/Caches/com.microsoft.Excel")],
        "Microsoft Excel cache",
    )]);
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);

    debug_log("Cleaning Excel container cache...");
    let (kb, c) = run_jobs(&[
        (
            vec![format!(
                "{h}/Library/Containers/com.microsoft.Excel/Data/Library/Caches/*"
            )],
            "Microsoft Excel container cache",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.microsoft.Excel/Data/tmp/*"
            )],
            "Microsoft Excel temp files",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.microsoft.Excel/Data/Library/Logs/*"
            )],
            "Microsoft Excel container logs",
        ),
    ]);
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);

    // 其余项
    let (kb, c) = run_jobs(&[
        (
            vec![format!("{h}/Library/Caches/com.microsoft.Powerpoint")],
            "Microsoft PowerPoint cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.microsoft.Outlook/*")],
            "Microsoft Outlook cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.iWork.*")],
            "Apple iWork cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.kingsoft.wpsoffice.mac")],
            "WPS Office cache",
        ),
        (
            vec![format!("{h}/Library/Caches/org.mozilla.thunderbird/*")],
            "Thunderbird cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.mail/*")],
            "Apple Mail cache",
        ),
    ]);
    total_kb = total_kb.saturating_add(kb);
    total_count = total_count.saturating_add(c);

    (total_kb, total_count)
}

/// 对齐 SH 第 1264-1270 行。**注意**:绝不删 com.docker.docker/Data/vms!
pub fn clean_virtualization_tools() -> (u64, u64) {
    let h = home_dir();
    run_jobs(&[
        (
            vec![format!("{h}/Library/Caches/com.vmware.fusion")],
            "VMware Fusion cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.parallels.*")],
            "Parallels cache",
        ),
        (
            vec![format!("{h}/VirtualBox VMs/.cache")],
            "VirtualBox cache",
        ),
        (
            vec![format!("{h}/.vagrant.d/tmp/*")],
            "Vagrant temporary files",
        ),
    ])
}

pub fn clean_application_support_logs() -> (u64, u64) {
    let h = home_dir();
    let root = format!("{h}/Library/Application Support");

    // 权限检查（对齐 SH 第 1575-1579 行）
    if !Path::new(&root).is_dir() {
        log_warning("Application Support: No permission to access");
        note_activity();
        return (0, 0);
    }

    // 额外的权限探测：尝试打开目录确认可访问性（对齐 SH 的 ls 检查）
    if std::fs::read_dir(&root).is_err() {
        log_warning("Application Support: Permission denied");
        note_activity();
        return (0, 0);
    }

    let mut total_kb = 0u64;
    let mut total_count = 0u64;
    let dry_run = is_dry_run();

    // 与 SH `start_candidates` 严格对齐
    let candidates = [
        "Code Cache",
        "GPUCache",
        "DawnCache",
        "GrShaderCache",
        "GraphiteDawnCache",
        "Crashpad/completed",
    ];

    // 大小计算超时（秒），对应 SH `MOLE_APP_SUPPORT_ITEM_SIZE_TIMEOUT_SEC`
    let size_timeout: f64 = std::env::var("MOLE_APP_SUPPORT_ITEM_SIZE_TIMEOUT_SEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.4);

    // 统计总 app 数用于进度
    let total_apps: u64 = std::fs::read_dir(&root)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .count() as u64
        })
        .unwrap_or(0);

    let mut app_count: u64 = 0;
    let mut last_progress_update = get_epoch_seconds();
    let mut total_size_partial = false;

    let Ok(rd) = std::fs::read_dir(&root) else {
        return (0, 0);
    };

    for entry in rd.flatten() {
        let app_dir = entry.path();
        if !app_dir.is_dir() {
            continue;
        }

        app_count += 1;

        // 进度更新（对齐 SH `update_progress_if_needed`）
        update_progress_if_needed(app_count, total_apps, &mut last_progress_update, 1);

        let app_dir_str = app_dir.to_string_lossy().to_string();
        let app_name = app_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        // 保护检查
        if is_path_whitelisted_from_global(&app_dir_str)
            || should_protect_path(&app_dir_str)
            || should_protect_data(&app_name)
            || should_protect_data(&app_name.to_ascii_lowercase())
            || is_critical_system_component(&app_name)
        {
            continue;
        }

        for candidate_sub in &candidates {
            let cand = format!("{app_dir_str}/{candidate_sub}");
            if !Path::new(&cand).is_dir() {
                continue;
            }
            if should_protect_path(&cand) || is_path_whitelisted_from_global(&cand) {
                continue;
            }

            // 快速计数：项数 > 100 时整目录删除（对齐 SH bulk clean）
            let quick_count = cache_top_level_entry_count_capped(&cand, 101);
            if quick_count > 100 {
                total_size_partial = true;
                let size_before = get_path_size_kb(&cand);
                if dry_run {
                    if dry_run_register_cleanup_target(&cand) {
                        total_kb = total_kb.saturating_add(size_before);
                        total_count += 1;
                    }
                } else if safe_remove(&cand, true) {
                    total_kb = total_kb.saturating_add(size_before);
                    total_count += 1;
                }
                continue;
            }

            // 逐项处理（带大小超时），对齐 SH 逐项循环 + 累加每候选目录的 size
            let pat = format!("{cand}/*");
            let matches = expand_glob_paths(&pat);
            if matches.is_empty() {
                continue;
            }
            let mut candidate_size_kb: u64 = 0;
            let mut candidate_size_partial = false;
            let mut candidate_item_count: u64 = 0;
            let mut item_found = false;
            for item in &matches {
                if should_protect_path(item) || is_path_whitelisted_from_global(item) {
                    continue;
                }

                item_found = true;
                candidate_item_count += 1;

                // 进度更新（每 250 项刷新一次，对齐 SH 第 1762-1774 行）
                if candidate_item_count % 250 == 0 {
                    let now = get_epoch_seconds();
                    if now.saturating_sub(last_progress_update) >= 1 {
                        let label = if app_name.len() > 24 {
                            format!("{}...", &app_name[..21])
                        } else {
                            app_name.clone()
                        };
                        start_section_spinner(
                            "scan",
                            &format!(
                                "Scanning Application Support... {app_count}/{total_apps} [{label}, {} items]",
                                candidate_item_count
                            ),
                        );
                        last_progress_update = now;
                    }
                }

                // 带超时的大小计算（对齐 SH `app_support_item_size_bytes`）
                let size_kb = get_path_size_kb_timeout(item, size_timeout);
                if size_kb.is_none() {
                    candidate_size_partial = true;
                }
                let size_kb_val = size_kb.unwrap_or(0);
                candidate_size_kb = candidate_size_kb.saturating_add(size_kb_val);

                if dry_run {
                    dry_run_register_cleanup_target(item);
                } else {
                    safe_remove(item, true);
                }
            }
            if item_found {
                total_kb = total_kb.saturating_add(candidate_size_kb);
                total_count += 1;
                if candidate_size_partial {
                    total_size_partial = true;
                }
            }
        }
    }

    // Group Containers（仅白名单容器）
    for container in &["group.com.apple.contentdelivery"] {
        for sub in &["Logs", "Library/Logs"] {
            let cand = format!("{h}/Library/Group Containers/{container}/{sub}");
            if !Path::new(&cand).is_dir() {
                continue;
            }

            // 也支持 bulk clean
            let quick_count = cache_top_level_entry_count_capped(&cand, 101);
            if quick_count > 100 {
                total_size_partial = true;
                let size_before = get_path_size_kb(&cand);
                if dry_run {
                    if dry_run_register_cleanup_target(&cand) {
                        total_kb = total_kb.saturating_add(size_before);
                        total_count += 1;
                    }
                } else if safe_remove(&cand, true) {
                    total_kb = total_kb.saturating_add(size_before);
                    total_count += 1;
                }
                continue;
            }

            let matches = expand_glob_paths(&format!("{cand}/*"));
            if matches.is_empty() {
                continue;
            }
            let mut candidate_size_kb: u64 = 0;
            let mut candidate_size_partial = false;
            let mut item_found = false;
            for item in &matches {
                item_found = true;
                let size_kb = get_path_size_kb_timeout(&item, size_timeout);
                if size_kb.is_none() {
                    candidate_size_partial = true;
                }
                let size_kb_val = size_kb.unwrap_or(0);
                candidate_size_kb = candidate_size_kb.saturating_add(size_kb_val);

                if dry_run {
                    dry_run_register_cleanup_target(&item);
                } else {
                    safe_remove(&item, true);
                }
            }
            if item_found {
                total_kb = total_kb.saturating_add(candidate_size_kb);
                total_count += 1;
                if candidate_size_partial {
                    total_size_partial = true;
                }
            }
        }
    }

    if total_count > 0 {
        let size_str = if total_size_partial {
            format!("at least {}", bytes_to_human(total_kb.saturating_mul(1024)))
        } else {
            bytes_to_human(total_kb.saturating_mul(1024))
        };
        log_info(&format!("Application Support logs/caches, {size_str}"));
        note_activity();
    }
    (total_kb, total_count)
}

/// 带超时的文件/目录大小计算（KB）。超时返回 None。
/// 对齐 SH `app_support_item_size_bytes` + `size_timeout_seconds`。
fn get_path_size_kb_timeout(path: &str, timeout_secs: f64) -> Option<u64> {
    let p = Path::new(path);
    if !p.exists() {
        return Some(0);
    }

    if p.is_file() || p.is_symlink() {
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            // iCloud dataless（云端占位）本地占用为 0，逻辑口径下跳过，避免高估可回收空间
            if crate::core::base::is_dataless(&meta) {
                return Some(0);
            }
            let bytes = meta.len();
            if bytes > 0 {
                return Some((bytes + 1023) / 1024);
            }
        }
    }

    let out = run_with_timeout_capture_lossy(timeout_secs, "du", &["-skP", path])?;
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return None;
    }
    // du 输出格式: "size_kb\tpath"
    let kb_str = trimmed.split('\t').next().unwrap_or(trimmed);
    kb_str.parse::<u64>().ok()
}

// =============================================================================
// clean_cached_device_firmware  / iOS backup info / 大文件提示 / Apple Silicon
// =============================================================================
pub fn clean_cached_device_firmware() -> (u64, u64) {
    let h = home_dir();
    let dry_run = is_dry_run();

    let shallow_dirs = [
        format!("{h}/Library/iTunes/iPhone Software Updates"),
        format!("{h}/Library/iTunes/iPad Software Updates"),
        format!("{h}/Library/iTunes/iPod Software Updates"),
    ];
    let mut configurator_dirs: Vec<String> = Vec::new();
    for p in expand_glob_paths(&format!(
        "{h}/Library/Group Containers/*.group.com.apple.configurator"
    )) {
        if Path::new(&p).is_dir() {
            configurator_dirs.push(p);
        }
    }

    let mut total_kb = 0u64;
    let mut count = 0u64;

    let mut process = |ipsw: &str| {
        if !Path::new(ipsw).is_file() {
            return;
        }
        if is_path_whitelisted_from_global(ipsw) {
            return;
        }
        let size_kb = get_path_size_kb(ipsw);
        if dry_run {
            if dry_run_register_cleanup_target(ipsw) {
                total_kb = total_kb.saturating_add(size_kb);
                count += 1;
            }
            return;
        }
        if safe_remove(ipsw, true) {
            total_kb = total_kb.saturating_add(size_kb);
            count += 1;
        }
    };

    // shallow: -maxdepth 1
    for dir in &shallow_dirs {
        if !Path::new(dir).is_dir() {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("ipsw") {
                    process(&p.to_string_lossy());
                }
            }
        }
    }
    // configurator:深度找
    for dir in &configurator_dirs {
        if let Some(out) = run_with_timeout_capture(
            10.0,
            "find",
            &[dir, "-type", "f", "-name", "*.ipsw", "-print0"],
        ) {
            for line in out.split('\0') {
                let f = line.trim();
                if !f.is_empty() {
                    process(f);
                }
            }
        }
    }

    if count > 0 {
        log_info(&format!(
            "Cached device firmware, {count} files, {}",
            bytes_to_human(total_kb.saturating_mul(1024))
        ));
        note_activity();
    }
    (total_kb, count)
}

pub fn check_ios_device_backups() -> CleanupHintsResultPayload {
    let h = home_dir();
    let backup_dir = format!("{h}/Library/Application Support/MobileSync/Backup");
    let noop = CleanupHintsResultPayload {
        section: "Device backups & firmware".into(),
        phase: "ios-backups".into(),
        title: "iOS backups".into(),
        detected: false,
        review_hint: String::new(),
        items: Vec::new(),
    };
    if !Path::new(&backup_dir).is_dir() {
        return noop;
    }
    let kb = get_path_size_kb(&backup_dir);
    if kb > 102_400 {
        note_activity();
        let size_human = bytes_to_human(kb.saturating_mul(1024));
        CleanupHintsResultPayload {
            section: "Device backups & firmware".into(),
            phase: "ios-backups".into(),
            title: "iOS backups".into(),
            detected: true,
            review_hint: format!("Path: {backup_dir}"),
            items: vec![CleanupHintItem {
                label: "iOS backups".into(),
                size_bytes: kb.saturating_mul(1024),
                size_human,
                path: backup_dir,
                detail: None,
            }],
        }
    } else {
        noop
    }
}

pub fn check_large_file_candidates() -> CleanupHintsResultPayload {
    let h = home_dir();
    let threshold_kb: u64 = 1024 * 1024;
    let mut items: Vec<CleanupHintItem> = Vec::new();

    let timeout_secs: f64 = std::env::var("MOLE_LARGE_CANDIDATE_SIZE_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3.0);

    for (path, label) in [
        (format!("{h}/Library/Mail"), "Mail data".to_string()),
        (
            format!("{h}/Library/Mail Downloads"),
            "Mail downloads".to_string(),
        ),
        (
            format!("{h}/Library/Updates"),
            "macOS updates cache".to_string(),
        ),
    ] {
        if Path::new(&path).is_dir() {
            if let Some(kb) = get_path_size_kb_timeout(&path, timeout_secs) {
                if kb >= threshold_kb {
                    let size_human = bytes_to_human(kb.saturating_mul(1024));
                    items.push(CleanupHintItem {
                        label,
                        size_bytes: kb.saturating_mul(1024),
                        size_human,
                        path: path.clone(),
                        detail: None,
                    });
                }
            }
        }
    }

    for installer in expand_glob_paths("/Applications/Install macOS*.app") {
        if let Some(kb) = get_path_size_kb_timeout(&installer, timeout_secs) {
            if kb > 0 {
                let size_human = bytes_to_human(kb.saturating_mul(1024));
                items.push(CleanupHintItem {
                    label: "macOS installer".into(),
                    size_bytes: kb.saturating_mul(1024),
                    size_human,
                    path: installer,
                    detail: None,
                });
            }
        }
    }

    let system_clean = std::env::var("SYSTEM_CLEAN").unwrap_or_default() == "true";
    if !system_clean {
        let has_tmutil = Command::new("which")
            .arg("tmutil")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if has_tmutil {
            let auto = Command::new("defaults")
                .args([
                    "read",
                    "/Library/Preferences/com.apple.TimeMachine",
                    "AutoBackup",
                ])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();
            if auto.lines().any(|l| matches!(l.trim(), "0" | "1")) {
                let snaps = run_with_timeout_capture(3.0, "tmutil", &["listlocalsnapshots", "/"])
                    .unwrap_or_default();
                let tm_re =
                    Regex::new(r"com\.apple\.TimeMachine\.\d{4}-\d{2}-\d{2}-\d{6}").unwrap();
                let count = snaps.lines().filter(|l| tm_re.is_match(l)).count();
                if count > 0 {
                    items.push(CleanupHintItem {
                        label: "Time Machine local snapshots".into(),
                        size_bytes: 0,
                        size_human: count.to_string(),
                        path: "tmutil listlocalsnapshots /".into(),
                        detail: Some("review with: tmutil listlocalsnapshots /".into()),
                    });
                }
            }
        }
    }

    if Command::new("which")
        .arg("docker")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        if let Some(out) = run_with_timeout_capture(
            3.0,
            "docker",
            &[
                "system",
                "df",
                "--format",
                "{{.Type}}\t{{.Size}}\t{{.Reclaimable}}",
            ],
        ) {
            if !out.trim().is_empty() {
                for line in out.lines() {
                    let parts: Vec<&str> = line.split('\t').collect();
                    if parts.len() >= 3 {
                        items.push(CleanupHintItem {
                            label: format!("Docker {}", parts[0]),
                            size_bytes: 0,
                            size_human: parts[1].to_string(),
                            path: format!("Reclaimable: {}", parts[2]),
                            detail: None,
                        });
                    }
                }
            } else if let Some(out) = run_with_timeout_capture(3.0, "docker", &["system", "df"]) {
                if !out.trim().is_empty() {
                    items.push(CleanupHintItem {
                        label: "Docker storage".into(),
                        size_bytes: 0,
                        size_human: String::new(),
                        path: "docker system df".into(),
                        detail: Some("Review: docker system df".into()),
                    });
                }
            }
        }
    }

    for (path, label) in [
        (
            format!("{h}/Library/Developer/Xcode/Archives"),
            "Xcode archives (review only)",
        ),
        (
            format!("{h}/Library/Application Support/MobileSync/Backup"),
            "iOS backups (review only)",
        ),
        (
            format!("{h}/.lmstudio/models"),
            "LM Studio models (review only)",
        ),
        (format!("{h}/OrbStack"), "OrbStack data (review only)"),
        (format!("{h}/.lima"), "Lima data (review only)"),
        (
            format!("{h}/.m2/repository"),
            "Maven local repository (review only)",
        ),
        (
            format!("{h}/Library/pnpm/store"),
            "pnpm store (review only)",
        ),
        (format!("{h}/.conda/pkgs"), "Conda packages (review only)"),
        (
            format!("{h}/anaconda3/pkgs"),
            "Anaconda packages (review only)",
        ),
    ] {
        if Path::new(&path).is_dir() {
            if let Some(kb) = get_path_size_kb_timeout(&path, timeout_secs) {
                if kb >= threshold_kb {
                    let size_human = bytes_to_human(kb.saturating_mul(1024));
                    items.push(CleanupHintItem {
                        label: label.to_string(),
                        size_bytes: kb.saturating_mul(1024),
                        size_human,
                        path,
                        detail: None,
                    });
                }
            }
        }
    }

    let review_hint = if items.is_empty() {
        String::new()
    } else {
        "Review files >1 GB in home directory; not directly cleaned by mole.".into()
    };

    if !items.is_empty() {
        let total_kb: u64 = items.iter().map(|i| i.size_bytes / 1024).sum();
        let total_human = bytes_to_human(total_kb.saturating_mul(1024));
        log_info(&format!(
            "Large file candidates: {}, {} items",
            total_human,
            items.len()
        ));
        note_activity();
    }

    CleanupHintsResultPayload {
        section: "Large files".into(),
        phase: "large-files".into(),
        title: "Large file candidates".into(),
        detected: !items.is_empty(),
        review_hint,
        items,
    }
}

pub fn clean_apple_silicon_caches() -> (u64, u64) {
    if std::env::var("IS_M_SERIES").unwrap_or_default() != "true" {
        return (0, 0);
    }
    start_section("Apple Silicon updates");
    let h = home_dir();
    let result = run_jobs(&[
        (
            vec!["/Library/Apple/usr/share/rosetta/rosetta_update_bundle".to_string()],
            "Rosetta 2 cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.rosetta.update")],
            "Rosetta 2 user cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.amp.mediasevicesd")],
            "Apple Silicon media service cache",
        ),
    ]);
    end_section();
    result
}

// =============================================================================
// Item-level dispatch wrappers (供 controller 的 execute_category_items 按 item 精确清理)
// =============================================================================

/// 清理 ~/Library/Caches/* (item-level dispatch: "user_cache")，跳过被专属条目覆盖的子目录
pub fn clean_user_app_cache() -> (u64, u64) {
    let h = home_dir();
    clean_user_caches_wildcard(&h)
}

/// 清理 ~/Library/Logs/* (item-level dispatch: "user_logs")
pub fn clean_user_app_logs() -> (u64, u64) {
    let h = home_dir();
    safe_clean(&[&format!("{h}/Library/Logs/*")], "User app logs")
}

/// 清理废纸篓（item-level dispatch: "trash"）
pub fn clean_user_trash() -> (u64, u64) {
    let h = home_dir();
    let trash_dir = format!("{h}/.Trash");
    let mut trash_count = 0u64;
    if !is_path_whitelisted_from_global(&trash_dir) && Path::new(&trash_dir).is_dir() {
        let dry_run = is_dry_run();
        let test_mode = std::env::var("MOLE_TEST_MODE").unwrap_or_default() == "1"
            || std::env::var("MOLE_TEST_NO_AUTH").unwrap_or_default() == "1";
        trash_count = if !test_mode {
            run_with_timeout_capture(
                3.0,
                "osascript",
                &["-e", "tell application \"Finder\" to count items in trash"],
            )
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or_else(|| trash_item_count(&trash_dir))
        } else {
            trash_item_count(&trash_dir)
        };
        if !dry_run && trash_count > 0 {
            let mut emptied = false;
            if !test_mode {
                let rc = crate::core::timeout::run_with_timeout(
                    5.0,
                    "osascript",
                    &["-e", "tell application \"Finder\" to empty trash"],
                );
                if rc == 0 {
                    emptied = true;
                    log_info(&format!("Trash · emptied, {trash_count} items"));
                    note_activity();
                }
            }
            if !emptied {
                debug_log("Finder empty trash failed, falling back to direct deletion");
                let mut cleaned = 0u64;
                if let Ok(rd) = std::fs::read_dir(&trash_dir) {
                    for entry in rd.flatten() {
                        let path = entry.path().to_string_lossy().to_string();
                        if safe_remove(&path, true) {
                            cleaned += 1;
                        }
                    }
                }
                if cleaned > 0 {
                    log_info(&format!("Trash · emptied, {cleaned} items"));
                    note_activity();
                }
            }
        }
    }
    (0, trash_count)
}

/// 清理 app_caches_system: System app caches (item-level dispatch)
pub fn clean_app_caches_system() -> (u64, u64) {
    let h = home_dir();
    let targets: Vec<String> = vec![
        format!("{h}/Library/Saved Application State/*"),
        format!("{h}/Library/Caches/com.apple.photoanalysisd"),
        format!("{h}/Library/Caches/com.apple.akd"),
        format!("{h}/Library/Caches/com.apple.WebKit.Networking/*"),
        format!("{h}/Library/DiagnosticReports/*"),
        format!("{h}/Library/Caches/com.apple.QuickLook.thumbnailcache"),
        format!("{h}/Library/Caches/Quick Look/*"),
        format!("{h}/Library/Caches/com.apple.iconservices*"),
    ];
    let refs: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
    safe_clean(&refs, "System app caches")
}

/// 清理 app_caches_downloads: Incomplete downloads (item-level dispatch)
pub fn clean_app_caches_downloads() -> (u64, u64) {
    clean_incomplete_downloads()
}

/// 清理 app_caches_identity: System identity caches (item-level dispatch)
pub fn clean_app_caches_identity() -> (u64, u64) {
    let h = home_dir();
    let targets: Vec<String> = vec![
        format!("{h}/Library/IdentityCaches/*"),
        format!("{h}/Library/Suggestions/*"),
        format!("{h}/Library/Calendars/Calendar Cache"),
        format!("{h}/Library/Application Support/AddressBook/Sources/*/Photos.cache"),
    ];
    let refs: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
    safe_clean(&refs, "System identity caches")
}

/// 清理 app_caches_support: App support data (item-level dispatch)
pub fn clean_app_caches_support() -> (u64, u64) {
    clean_support_app_data()
}

/// 清理 app_caches_sandbox: Sandbox & container caches (item-level dispatch)
pub fn clean_app_caches_sandbox() -> (u64, u64) {
    let h = home_dir();
    let targets: Vec<String> = vec![
        format!("{h}/Library/Containers/com.apple.wallpaper.agent/Data/Library/Caches/*"),
        format!("{h}/Library/Containers/com.apple.mediaanalysisd/Data/Library/Caches/*"),
        format!("{h}/Library/Containers/com.apple.mediaanalysisd/Data/tmp/*"),
        format!("{h}/Library/Containers/com.apple.AppStore/Data/Library/Caches/*"),
        format!("{h}/Library/Containers/com.apple.configurator.xpc.InternetService/Data/tmp/*"),
        format!("{h}/Library/Containers/com.apple.wallpaper.extension.aerials/Data/tmp/*"),
        format!("{h}/Library/Containers/com.apple.geod/Data/tmp/*"),
        format!("{h}/Library/Containers/com.apple.stocks/Data/Library/Caches/*"),
        format!("{h}/Library/Application Support/com.apple.wallpaper/aerials/thumbnails/*"),
        format!("{h}/Library/Caches/com.apple.helpd/*"),
        format!("{h}/Library/Caches/GeoServices/*"),
        format!(
            "{h}/Library/Containers/com.apple.AvatarUI.AvatarPickerMemojiPicker/Data/Library/Caches/*"
        ),
        format!("{h}/Library/Containers/com.apple.AMPArtworkAgent/Data/Library/Caches/*"),
        format!(
            "{h}/Library/Containers/com.apple.CoreDevice.CoreDeviceService/Data/Library/Caches/*"
        ),
        format!("{h}/Library/Containers/com.apple.NeptuneOneExtension/Data/Library/Caches/*"),
        format!(
            "{h}/Library/Containers/com.apple.AppleMediaServicesUI.UtilityExtension/Data/tmp/*"
        ),
        format!("{h}/Library/Caches/com.apple.AppleMediaServices/*"),
        format!("{h}/Library/Caches/com.apple.duetexpertd/*"),
        format!("{h}/Library/Caches/com.apple.parsecd/*"),
        format!("{h}/Library/Caches/com.apple.python/*"),
        format!("{h}/Library/Caches/com.apple.e5rt.e5bundlecache/*"),
    ];
    let refs: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
    let (mut kb, mut cnt) = safe_clean(&refs, "Sandbox & container caches");
    if Path::new(&format!("{h}/Library/Containers")).is_dir() {
        let (ckb, ccnt) = process_container_cache();
        kb = kb.saturating_add(ckb);
        cnt = cnt.saturating_add(ccnt);
        let (gkb, gcnt) = clean_group_container_caches();
        kb = kb.saturating_add(gkb);
        cnt = cnt.saturating_add(gcnt);
    }
    (kb, cnt)
}

fn run_jobs(jobs: &[(Vec<String>, &str)]) -> (u64, u64) {
    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;
    for (paths, label) in jobs {
        let p_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        let (kb, c) = safe_clean(&p_refs, label);
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
    }
    (total_kb, total_count)
}

fn cache_top_level_entry_count_capped(dir: &str, cap: u64) -> u64 {
    match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().take(cap as usize).count() as u64,
        Err(_) => 0,
    }
}

fn trash_item_count(dir: &str) -> u64 {
    if let Ok(rd) = std::fs::read_dir(dir) {
        rd.filter_map(|e| e.ok()).count() as u64
    } else {
        0
    }
}

#[allow(dead_code)]
fn _silence_unused() {
    // 这些 helper 在外部 lib::clean::caches 等模块中也可能被引用,导出后保留
    let _ = log_operation;
    let _ = get_epoch_seconds;
    let _ = get_file_mtime;
    let _ = is_path_whitelisted as fn(&str, &[String]) -> bool;
}
