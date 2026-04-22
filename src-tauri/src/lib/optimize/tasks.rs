//! 对齐 `lib/optimize/tasks.sh`。
//!
//! GUI 端不需要 spinner / TTY 颜色装饰,本模块按 SH 严格翻译业务逻辑。
//! 凡是在 SH 中通过 `should_protect_path` / `is_path_whitelisted` 拒绝的路径,
//! 这里同样要拒绝;凡是 SH 中通过 `run_with_timeout` 套了超时的子进程,
//! 这里也走 `run_with_timeout_capture` 等价物。

use std::ffi::CString;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use walkdir::WalkDir;

use crate::core::app_protection::{is_path_whitelisted_from_global, should_protect_path};
use crate::core::base::{
    bytes_to_human, command_available, get_epoch_seconds, get_lsregister_path, get_path_size_kb,
    home_dir, is_dry_run,
};
use crate::core::bundle_resolver::bundle_has_installed_app_checked;
use crate::core::file_ops::safe_remove;
use crate::core::log::{
    debug_file_action, debug_log, debug_operation_detail, debug_operation_start, debug_risk_level,
};
use crate::core::sudo::sudo_output;
use crate::core::timeout::{
    run_with_timeout, run_with_timeout_capture, run_with_timeout_capture_rc,
};
use crate::optimize::outcome::OptimizeOutcome;

/// 对齐 SH 第 7 行常量。
pub const MOLE_TM_THIN_TIMEOUT: u32 = 180;
/// 对齐 SH 第 8 行常量。
pub const MOLE_TM_THIN_VALUE: u64 = 9_999_999_999;
/// 对齐 SH 第 9 行常量。
pub const MOLE_SQLITE_MAX_SIZE: u64 = 104_857_600;

static DNS_FLUSHED: AtomicBool = AtomicBool::new(false);

fn debug_enabled() -> bool {
    std::env::var("MO_DEBUG").unwrap_or_default() == "1"
}

/// 对齐 SH `optimize_sudo_available`：测试模式硬拒绝，其余看控制器预设的环境变量。
pub fn optimize_sudo_available() -> bool {
    if std::env::var("MOLE_TEST_MODE").unwrap_or_default() == "1"
        || std::env::var("MOLE_TEST_NO_AUTH").unwrap_or_default() == "1"
    {
        return false;
    }
    std::env::var("MOLE_OPTIMIZE_SUDO_AVAILABLE").unwrap_or_else(|_| "true".into()) == "true"
}

/// 对齐 SH 第 12-19 行 `opt_msg`。
pub fn opt_msg(message: &str) {
    if is_dry_run() {
        println!("  → {message}");
    } else {
        println!("  ✓ {message}");
    }
}

/// 打印失败警告并记录原因（控制器经 `optimize::take_failure` 取出，
/// 随 `task_done` 事件透传给前端展示失败明细）。
fn note_failure(message: &str) {
    println!("  ⚠ {message}");
    super::record_failure(message);
}

/// `note_failure` + 返回 `Failed`：探测失败直接判失败的分支专用。
fn optimize_fail(message: &str) -> OptimizeOutcome {
    note_failure(message);
    OptimizeOutcome::Failed
}

/// 对齐 SH 第 71-96 行 `run_launchctl_unload`(26f4d47a 加固后)。
/// 加固点:sudo 分支先查 `optimize_sudo_available`(不可用直接跳过),
/// 两条路径都套 `run_with_timeout`(5s,对齐 MOLE_TIMEOUT_MEDIUM_PROBE_SEC);
/// 超时/信号返回 false,调用方折算任务失败。
pub fn run_launchctl_unload(plist: &str, need_sudo: bool) -> bool {
    if is_dry_run() {
        return true;
    }
    let rc = if need_sudo {
        if !optimize_sudo_available() {
            return true;
        }
        let _ = sudo_output(&["/bin/launchctl", "unload", plist]);
        // sudo_output 无退出码面(授权票据直跑 root);unload 失败不阻断删除,
        // 与 SH 中 `sudo launchctl unload ... || true` 的降级语义一致。
        return true;
    } else {
        run_with_timeout(5.0, "launchctl", &["unload", plist])
    };
    // 对齐 SH:仅超时(124)/信号(>=128)向上传播;普通失败降级继续
    !(rc == 124 || rc >= 128)
}

/// 用 `access(W_OK)` 判断当前进程能否写,语义对齐 SH `[[ ! -w "$path" ]]`。
/// 之前 Rust 用 `permissions().readonly()` 看的是 mode bit (0o222),
/// 不等价于"当前进程能否写",会漏掉 owner-mismatch 但 mode 0o644 的目录。
fn writable_by_me(path: &str) -> bool {
    let Ok(c) = CString::new(path) else {
        return false;
    };
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// 对齐 SH 第 36-56 行 `needs_permissions_repair`。
pub fn needs_permissions_repair() -> bool {
    let home = home_dir();
    if let Ok(out) = Command::new("/usr/bin/stat")
        .args(["-f", "%Su", &home])
        .output()
    {
        let owner = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let user = std::env::var("USER").unwrap_or_default();
        if !owner.is_empty() && owner != user {
            return true;
        }
    }

    let paths = [
        home.clone(),
        format!("{home}/Library"),
        format!("{home}/Library/Preferences"),
    ];
    for p in &paths {
        if Path::new(p).exists() && !writable_by_me(p) {
            return true;
        }
    }
    false
}

/// 对齐 SH 第 58-70 行 `has_bluetooth_hid_connected`。
/// 注意:bluetooth_reset 已从 catalog 移除,此函数不再被调用。
pub fn has_bluetooth_hid_connected() -> bool {
    let out = Command::new("system_profiler")
        .arg("SPBluetoothDataType")
        .output();
    if let Ok(o) = out {
        let s = String::from_utf8_lossy(&o.stdout);
        if !s.contains("Connected: Yes") {
            return false;
        }
        let lower = s.to_lowercase();
        lower.contains("keyboard")
            || lower.contains("trackpad")
            || lower.contains("mouse")
            || lower.contains("hid")
    } else {
        false
    }
}

/// 对齐 SH 第 72-74 行 `is_ac_power`。
pub fn is_ac_power() -> bool {
    Command::new("pmset")
        .args(["-g", "batt"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("AC Power"))
        .unwrap_or(false)
}

/// 对齐 SH 第 109-148 行 `has_active_vpn_interface`(三态版)。
/// 检查是否有活跃 VPN 连接,避免 network_stack_optimize 打断 VPN 路由:
/// - `Some(true)`:确认有活跃 VPN(系统管理或 full-tunnel)
/// - `Some(false)`:确认无活跃 VPN
/// - `None`:探测异常(scutil/route 不可用、超时或执行失败,SH 中 return 2)
pub fn has_active_vpn_interface() -> Option<bool> {
    // SH 第 126-134 行:允许显式假设(测试/排障用)
    match std::env::var("MOLE_ASSUME_VPN_ACTIVE").as_deref() {
        Ok("1" | "true" | "TRUE" | "yes" | "YES") => return Some(true),
        Ok("0" | "false" | "FALSE" | "no" | "NO") => return Some(false),
        _ => {}
    }

    // 1. scutil --nc list 检查系统管理的 VPN 连接(L2TP/IPsec/IKEv2/Cisco IPSec)
    if !command_available("scutil") {
        return None;
    }
    let Some(out) = run_with_timeout_capture(3.0, "scutil", &["--nc", "list"]) else {
        // SH 第 138-140 行:scutil 超时/失败 → return 2(探测异常)
        return None;
    };
    if out.lines().any(|l| l.starts_with("* (Connected)")) {
        return Some(true);
    }

    // 2. 默认路由接口为 utun* 即表示 full-tunnel VPN(WireGuard/OpenVPN/Tunnelblick 等)
    if !command_available("route") {
        return None;
    }
    let Some(out) = run_with_timeout_capture(3.0, "route", &["-n", "get", "default"]) else {
        return None;
    };
    for line in out.lines() {
        let trimmed = line.trim();
        if let Some(iface) = trimmed.strip_prefix("interface: ") {
            let iface = iface.trim();
            if iface.starts_with("utun") && iface[4..].chars().all(|c| c.is_ascii_digit()) {
                return Some(true);
            }
        }
    }

    Some(false)
}

/// 对齐 SH 第 90-101 行 `flush_dns_cache`。
/// 任意一步成功即视为成功(SH `&&` 是两步都要成功;Rust 这里也保持 `&&`)。
pub fn flush_dns_cache() -> bool {
    if DNS_FLUSHED.load(Ordering::Relaxed) {
        return true;
    }
    if is_dry_run() {
        DNS_FLUSHED.store(true, Ordering::Relaxed);
        return true;
    }
    let a = sudo_output(&["/usr/bin/dscacheutil", "-flushcache"])
        .status
        .success();
    // killall 的用户态过滤按 real uid(501) 排除 root 域进程（mDNSResponder 属
    // _mdnsresponder/uid 65），euid=0 也会报 "No matching processes belonging to
    // you"；pkill -HUP -x 按精确名直达 pid 发信号，内核层面 euid=0 即可。
    let b = sudo_output(&["/usr/bin/pkill", "-HUP", "-x", "mDNSResponder"])
        .status
        .success();
    if a && b {
        DNS_FLUSHED.store(true, Ordering::Relaxed);
        true
    } else {
        false
    }
}

/// 对齐 SH 第 199-227 行 `opt_system_maintenance`。
pub fn opt_system_maintenance() -> OptimizeOutcome {
    if !is_dry_run() && !optimize_sudo_available() {
        opt_msg("DNS & Spotlight check skipped (admin access required)");
        return OptimizeOutcome::Skipped;
    }

    let mut dns_flushed = false;
    if flush_dns_cache() {
        opt_msg("DNS cache flushed");
        dns_flushed = true;
    } else {
        // 该分支原先静默（failed 计数隐含失败但无任何提示），补录原因供结果页追查
        note_failure("Failed to flush DNS cache");
    }

    let mut spotlight_failed = 0u32;
    match run_with_timeout_capture(10.0, "mdutil", &["-s", "/"]) {
        Some(out) => {
            let lower = out.to_lowercase();
            if lower.contains("indexing disabled") {
                println!("  ○ Spotlight indexing disabled");
            } else {
                opt_msg("Spotlight index verified");
            }
        }
        None => {
            note_failure("Failed to verify Spotlight index");
            spotlight_failed = 1;
        }
    }

    let applied = if dns_flushed { 1 } else { 0 };
    let failed = spotlight_failed + if dns_flushed { 0 } else { 1 };
    OptimizeOutcome::from_counts(applied, failed, 0)
}

/// 对齐 SH 第 230-330 行 `opt_cache_refresh`。
pub fn opt_cache_refresh() -> OptimizeOutcome {
    let home = home_dir();
    let cache_targets = [
        format!("{home}/Library/Caches/com.apple.QuickLook.thumbnailcache"),
        format!("{home}/Library/Caches/com.apple.iconservices.store"),
        format!("{home}/Library/Caches/com.apple.iconservices"),
    ];

    if debug_enabled() {
        debug_operation_start(
            "Finder Cache Refresh",
            Some("Refresh QuickLook thumbnails and icon services"),
        );
        debug_operation_detail("Method", "Remove cache files and rebuild via qlmanage");
        debug_operation_detail(
            "Expected outcome",
            "Faster Finder preview generation, fixed icon display issues",
        );
        debug_risk_level("LOW", "Caches are automatically rebuilt");
    }

    let mut quicklook_refreshed = 0u32;
    let mut icons_refreshed = 0u32;
    let mut refresh_failed = 0u32;
    if is_dry_run() {
        quicklook_refreshed = 1;
        icons_refreshed = 1;
    } else {
        if Command::new("qlmanage")
            .args(["-r", "cache"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            quicklook_refreshed = 1;
        } else {
            refresh_failed += 1;
        }
        if Command::new("qlmanage")
            .arg("-r")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            icons_refreshed = 1;
        } else {
            refresh_failed += 1;
        }
    }

    // 先收集可删目标（对齐 SH：尺寸查询与删除分开，删除失败才计 remove_failed）
    let mut removable: Vec<(String, u64)> = Vec::new();
    for target in &cache_targets {
        if Path::new(target).exists() && !should_protect_path(target) {
            removable.push((target.clone(), get_path_size_kb(target)));
        }
    }

    if debug_enabled() {
        if removable.is_empty() {
            debug_operation_detail("Files to be removed", "none");
        } else {
            debug_operation_detail("Files to be removed", "");
            for (target_path, size_kb) in &removable {
                let size_human = if *size_kb > 0 {
                    bytes_to_human(size_kb.saturating_mul(1024))
                } else {
                    "unknown".to_string()
                };
                debug_file_action("  Will remove", target_path, Some(&size_human), None);
            }
        }
    }

    let mut removed_count = 0u32;
    let mut remove_failed = 0u32;
    let mut total_cache_kb: u64 = 0;
    for (target, size_kb) in &removable {
        if safe_remove(target, true) {
            removed_count += 1;
            total_cache_kb = total_cache_kb.saturating_add(*size_kb);
        } else {
            remove_failed += 1;
        }
    }

    unsafe {
        std::env::set_var("OPTIMIZE_CACHE_CLEANED_KB", total_cache_kb.to_string());
    }
    if quicklook_refreshed == 1 {
        opt_msg("QuickLook thumbnails refreshed");
    }
    if icons_refreshed == 1 {
        opt_msg("Icon services cache rebuilt");
    }
    if remove_failed > 0 {
        note_failure(&format!("Failed to remove {remove_failed} Finder cache target(s)"));
    }
    if refresh_failed > 0 {
        note_failure(&format!("Failed to rebuild {refresh_failed} Finder cache service(s)"));
    }
    OptimizeOutcome::from_counts(
        removed_count + quicklook_refreshed + icons_refreshed,
        remove_failed + refresh_failed,
        0,
    )
}

/// 对齐 SH 第 337-394 行 `opt_saved_state_cleanup`。
///
/// 递归 walk(SH `find -type d -name "*.savedState"`)，加 `should_protect_path` 保护。
pub fn opt_saved_state_cleanup() -> OptimizeOutcome {
    let home = home_dir();
    let state_dir = format!("{home}/Library/Saved Application State");
    let age_days = std::env::var("MOLE_SAVED_STATE_AGE_DAYS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30);

    if debug_enabled() {
        debug_operation_start(
            "App Saved State Cleanup",
            Some("Remove old application saved states"),
        );
        debug_operation_detail(
            "Method",
            &format!("Find and remove .savedState folders older than {age_days} days"),
        );
        debug_operation_detail("Location", &state_dir);
        debug_operation_detail(
            "Expected outcome",
            "Reduced disk usage, apps start with clean state",
        );
        debug_risk_level("LOW", "Old saved states, apps will create new ones");
    }

    let mut removed = 0u32;
    let mut scan_failed = 0u32;
    let mut remove_failed = 0u32;

    if Path::new(&state_dir).is_dir() {
        let cutoff = get_epoch_seconds().saturating_sub(age_days.saturating_mul(86400));
        let mut candidates: Vec<String> = Vec::new();
        for entry in WalkDir::new(&state_dir).min_depth(1).into_iter() {
            match entry {
                Ok(e) => {
                    if !e.file_type().is_dir() {
                        continue;
                    }
                    let name = e.file_name().to_string_lossy().to_string();
                    if !name.ends_with(".savedState") {
                        continue;
                    }
                    let path_str = e.path().to_string_lossy().to_string();
                    if should_protect_path(&path_str) {
                        continue;
                    }
                    let mtime = e
                        .metadata()
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    if mtime != 0 && mtime < cutoff {
                        candidates.push(path_str);
                    }
                }
                Err(_) => {
                    scan_failed = 1;
                }
            }
        }

        for path_str in &candidates {
            if safe_remove(path_str, true) {
                removed += 1;
            } else {
                remove_failed += 1;
            }
        }
    }

    if scan_failed == 0 && remove_failed == 0 {
        opt_msg("App saved states optimized");
    } else if removed > 0 {
        opt_msg(&format!("Removed {removed} old saved state(s)"));
    }
    if remove_failed > 0 {
        note_failure(&format!("Failed to remove {remove_failed} old saved state(s)"));
    }
    OptimizeOutcome::from_counts(removed, scan_failed + remove_failed, 0)
}

/// 对齐 SH 第 402-442 行 `opt_fix_broken_configs`。
pub fn opt_fix_broken_configs() -> OptimizeOutcome {
    let (broken, partial) = crate::optimize::maintenance::fix_broken_preferences();
    unsafe {
        std::env::set_var("OPTIMIZE_CONFIGS_REPAIRED", broken.to_string());
    }
    if broken > 0 {
        if partial {
            note_failure(&format!(
                "Preference scan hit its time budget, repaired {broken} so far"
            ));
        } else {
            opt_msg(&format!("Repaired {broken} corrupted preference files"));
        }
    } else if partial {
        note_failure(&format!(
            "Preference scan hit its time budget, repaired {broken} so far"
        ));
    } else {
        opt_msg("All preference files valid");
    }
    OptimizeOutcome::from_counts(broken as u32, if partial { 1 } else { 0 }, 0)
}

/// 对齐 SH 第 445-474 行 `opt_network_optimization`。
pub fn opt_network_optimization() -> OptimizeOutcome {
    if debug_enabled() {
        debug_operation_start(
            "Network Optimization",
            Some("Refresh DNS cache and restart mDNSResponder"),
        );
        debug_operation_detail(
            "Method",
            "Flush DNS cache via dscacheutil and pkill mDNSResponder",
        );
        debug_operation_detail(
            "Expected outcome",
            "Faster DNS resolution, fixed network connectivity issues",
        );
        debug_risk_level("LOW", "DNS cache is automatically rebuilt");
    }

    // SH 第 453-458 行:已经 flush 过就判 unchanged
    if DNS_FLUSHED.load(Ordering::Relaxed) {
        opt_msg("DNS cache already refreshed");
        opt_msg("mDNSResponder already restarted");
        return OptimizeOutcome::Unchanged;
    }
    if !is_dry_run() && !optimize_sudo_available() {
        opt_msg("Network cache refresh skipped (admin access required)");
        return OptimizeOutcome::Skipped;
    }
    if flush_dns_cache() {
        opt_msg("DNS cache refreshed");
        opt_msg("mDNSResponder restarted");
        OptimizeOutcome::Applied
    } else {
        optimize_fail("Failed to refresh DNS cache")
    }
}

/// 对齐 SH 第 477-537 行 `opt_quarantine_cleanup`。
pub fn opt_quarantine_cleanup() -> OptimizeOutcome {
    if debug_enabled() {
        debug_operation_start(
            "Quarantine Database Cleanup",
            Some("Clear Gatekeeper download tracking history"),
        );
        debug_operation_detail(
            "Method",
            "DELETE + VACUUM on QuarantineEventsV2 SQLite database",
        );
        debug_operation_detail(
            "Safety",
            "Only clears download tracking metadata, does not affect file quarantine flags",
        );
        debug_operation_detail(
            "Expected outcome",
            "Reduced database size, cleared download tracking history",
        );
        debug_risk_level("LOW", "Database is automatically recreated by macOS");
    }

    if !command_available("sqlite3") {
        println!("  - Quarantine cleanup skipped, sqlite3 unavailable");
        return OptimizeOutcome::Unavailable;
    }

    let home = home_dir();
    let db = format!("{home}/Library/Preferences/com.apple.LaunchServices.QuarantineEventsV2");
    if !Path::new(&db).is_file() {
        opt_msg("Quarantine database already clean");
        return OptimizeOutcome::Unchanged;
    }
    if should_protect_path(&db) {
        opt_msg("Quarantine database already clean");
        return OptimizeOutcome::Unchanged;
    }

    let count_out = run_with_timeout_capture(
        5.0,
        "sqlite3",
        &[&db, "SELECT COUNT(*) FROM LSQuarantineEvent;"],
    );
    let Some(count_str) = count_out else {
        return optimize_fail("Failed to inspect quarantine database");
    };
    let row_count: u64 = match count_str.trim().parse() {
        Ok(n) => n,
        Err(_) => return optimize_fail("Failed to inspect quarantine database"),
    };
    if row_count == 0 {
        opt_msg("Quarantine database already clean");
        return OptimizeOutcome::Unchanged;
    }

    if !is_dry_run() {
        let rc = run_with_timeout(
            10.0,
            "sqlite3",
            &[&db, "DELETE FROM LSQuarantineEvent; VACUUM;"],
        );
        if rc == 0 {
            opt_msg(&format!("Quarantine history cleared ({row_count} entries)"));
            OptimizeOutcome::Applied
        } else {
            optimize_fail("Failed to clean quarantine database")
        }
    } else {
        opt_msg(&format!("Quarantine history cleared ({row_count} entries)"));
        OptimizeOutcome::Applied
    }
}

/// 对齐 SH 第 540-724 行 `opt_sqlite_vacuum`。
pub fn opt_sqlite_vacuum() -> OptimizeOutcome {
    if debug_enabled() {
        debug_operation_start(
            "Database Optimization",
            Some("Vacuum SQLite databases for Mail, Safari, and Messages"),
        );
        debug_operation_detail(
            "Method",
            "Run VACUUM command on databases after integrity check",
        );
        debug_operation_detail(
            "Safety checks",
            "Skip if apps are running, verify integrity first, 20s timeout",
        );
        debug_operation_detail(
            "Expected outcome",
            "Reduced database size, faster app performance",
        );
        debug_risk_level("LOW", "Only optimizes databases, does not delete data");
    }

    if !command_available("pgrep") {
        println!("  - Database optimization unavailable, process probe unavailable");
        return OptimizeOutcome::Unavailable;
    }

    // 对齐 SH 3ebe4f0d:pgrep 退出码 0=有进程、1=无进程;非 0 非 1 视为探测异常 → 任务失败
    let mut busy: Vec<&str> = Vec::new();
    for app in ["Mail", "Safari", "Messages"] {
        let out = Command::new("pgrep").args(["-x", app]).output();
        match out {
            Ok(o) if o.status.success() => busy.push(app),
            Ok(o) if o.status.code() == Some(1) => {}
            _ => return optimize_fail("Failed to inspect active apps before database optimization"),
        }
    }
    if !busy.is_empty() {
        println!(
            "  ⚠ Close these apps before database optimization: {}",
            busy.join(", ")
        );
        return OptimizeOutcome::Skipped;
    }

    if !command_available("sqlite3") {
        println!("  - Database optimization already optimal, sqlite3 unavailable");
        return OptimizeOutcome::Unavailable;
    }

    let home = home_dir();
    let patterns = [
        format!("{home}/Library/Mail/V*/MailData/Envelope Index*"),
        format!("{home}/Library/Messages/chat.db"),
        format!("{home}/Library/Safari/History.db"),
        format!("{home}/Library/Safari/TopSites.db"),
    ];

    let mut vacuumed = 0u32;
    let mut timed_out = 0u32;
    let mut failed = 0u32;
    let mut policy_skipped = 0u32;
    let mut already_optimal = 0u32;
    // 仅被 100MB 上限拦下的路径：存在时绝不宣称 "already optimized"（issue #1367）。
    let mut policy_skipped_paths: Vec<String> = Vec::new();

    for pattern in &patterns {
        let glob_iter = match glob::glob(pattern) {
            Ok(it) => it,
            Err(_) => continue,
        };
        for entry in glob_iter.flatten() {
            if !entry.is_file() {
                continue;
            }
            let path = entry.to_string_lossy().to_string();
            if path.ends_with("-wal") || path.ends_with("-shm") {
                continue;
            }
            if should_protect_path(&path) {
                continue;
            }

            // 不是 SQLite 库就跳过(SH `file ... | grep -q "SQLite"`)
            let file_out = run_with_timeout_capture(2.0, "file", &[&path]).unwrap_or_default();
            if !file_out.contains("SQLite") {
                continue;
            }

            // 超过 100MB 跳过(SH 第 618-624 行)
            let file_size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            if file_size > MOLE_SQLITE_MAX_SIZE {
                policy_skipped += 1;
                policy_skipped_paths.push(path.clone());
                continue;
            }

            // freelist 太小直接判已 compact(SH 第 626-646 行)；page 查询失败计 failed
            let page_info = run_with_timeout_capture(
                5.0,
                "sqlite3",
                &[&path, "PRAGMA page_count; PRAGMA freelist_count;"],
            );
            let Some(page_info) = page_info else {
                failed += 1;
                continue;
            };
            let mut lines = page_info.lines();
            let page_count: u64 = lines
                .next()
                .and_then(|l| l.split_whitespace().next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let freelist_count: u64 = lines
                .next()
                .and_then(|l| l.split_whitespace().next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            if page_count > 0 && freelist_count.saturating_mul(100) < page_count.saturating_mul(5) {
                already_optimal += 1;
                continue;
            }

            // 完整性检查(SH 第 648-658 行)：失败计 failed
            if !is_dry_run() {
                let integrity =
                    run_with_timeout_capture(10.0, "sqlite3", &[&path, "PRAGMA integrity_check;"]);
                let ok = integrity.as_deref().map_or(false, |s| s.contains("ok"));
                if !ok {
                    failed += 1;
                    continue;
                }
            }

            if !is_dry_run() {
                let rc = run_with_timeout(20.0, "sqlite3", &[&path, "VACUUM;"]);
                if rc == 0 {
                    vacuumed += 1;
                } else if rc == 124 {
                    timed_out += 1;
                } else {
                    failed += 1;
                }
            } else {
                vacuumed += 1;
            }
        }
    }

    unsafe {
        std::env::set_var("OPTIMIZE_DATABASES_COUNT", vacuumed.to_string());
    }

    // 头条消息不宣称 "already optimized"：有 policy 跳过或根本没压到库时（issue #1367）
    if vacuumed > 0 {
        opt_msg(&format!(
            "Optimized {vacuumed} databases for Mail, Safari, Messages"
        ));
    } else if timed_out != 0 || failed != 0 {
        note_failure("Database optimization incomplete");
    } else if policy_skipped > 0 {
        opt_msg("No databases compacted");
    } else if already_optimal > 0 {
        opt_msg("All databases already optimized");
    } else {
        opt_msg("No databases found to optimize");
    }

    if already_optimal > 0 {
        opt_msg(&format!("Already optimal for {already_optimal} databases"));
    }
    if policy_skipped > 0 {
        opt_msg(&format!(
            "Skipped {policy_skipped} databases over the 100 MB safety limit"
        ));
        for skipped_path in &policy_skipped_paths {
            let skipped_size = std::fs::metadata(skipped_path)
                .map(|m| m.len())
                .unwrap_or(0);
            let skipped_display = if skipped_size > 0 {
                bytes_to_human(skipped_size)
            } else {
                "unknown size".to_string()
            };
            let home = home_dir();
            let display_path = skipped_path.replacen(&home, "~", 1);
            println!("  - {display_path} · {skipped_display}");
        }
    }
    if timed_out > 0 {
        note_failure(&format!("Timed out on {timed_out} databases"));
    }
    if failed > 0 {
        note_failure(&format!("Failed on {failed} databases"));
    }

    OptimizeOutcome::from_counts(vacuumed, timed_out + failed, policy_skipped)
}

/// 对齐 SH 第 727-776 行 `opt_launch_services_rebuild`。
/// 修复点:`-r -f -domain local -domain user -domain system` 失败时回退到去掉 system 的版本。
pub fn opt_launch_services_rebuild() -> OptimizeOutcome {
    if debug_enabled() {
        debug_operation_start(
            "LaunchServices Rebuild",
            Some("Rebuild LaunchServices database"),
        );
        debug_operation_detail(
            "Method",
            "Run lsregister -gc then force rescan with -r -f on local, user, and system domains",
        );
        debug_operation_detail(
            "Purpose",
            "Fix \"Open with\" menu issues, file associations, and stale app metadata",
        );
        debug_operation_detail(
            "Expected outcome",
            "Correct app associations, fixed duplicate entries, fewer stale app listings",
        );
        debug_risk_level("LOW", "Database is automatically rebuilt");
    }

    let lsregister = get_lsregister_path();
    if lsregister.is_empty() || !Path::new(&lsregister).exists() {
        println!("  ⚠ lsregister not found");
        return OptimizeOutcome::Unavailable;
    }

    let success: bool;
    if !is_dry_run() {
        let _ = Command::new(&lsregister).arg("-gc").output();
        let primary_ok = Command::new(&lsregister)
            .args([
                "-r", "-f", "-domain", "local", "-domain", "user", "-domain", "system",
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if primary_ok {
            success = true;
        } else {
            // SH 第 474-477 行 fallback:去掉 system domain 重试一次
            success = Command::new(&lsregister)
                .args(["-r", "-f", "-domain", "local", "-domain", "user"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
        }
    } else {
        success = true;
    }

    if success {
        opt_msg("LaunchServices repaired");
        opt_msg("File associations refreshed");
        OptimizeOutcome::Applied
    } else {
        optimize_fail("Failed to rebuild LaunchServices")
    }
}

/// 对齐 SH 第 784-886 行 `opt_network_stack_optimize`。
///
/// 修复点:
/// - 活跃 VPN 时跳过,避免打断连接(SH 第 788-799 行);
/// - 网络健康探测套 3s 超时,超时或异常退出都算检查失败(SH 第 813-833 行);
/// - route/arp 分别计数,`from_counts` 折算结局(SH 第 867-885 行)。
pub fn opt_network_stack_optimize() -> OptimizeOutcome {
    // 对齐 SH 第 788-806 行:有活跃 VPN 时跳过;VPN 状态探测异常时任务失败(fail-closed)
    match has_active_vpn_interface() {
        Some(true) => {
            opt_msg("Network stack refresh skipped, active VPN detected");
            return OptimizeOutcome::Skipped;
        }
        None => return optimize_fail("Failed to inspect active VPN state"),
        Some(false) => {}
    }

    // SH 第 813-833 行:探测超时(124)或异常退出都视为检查失败
    let route_rc = run_with_timeout(3.0, "route", &["-n", "get", "default"]);
    let dns_rc = run_with_timeout(
        3.0,
        "dscacheutil",
        &["-q", "host", "-a", "name", "example.com"],
    );
    if route_rc == 124 || dns_rc == 124 {
        return optimize_fail("Network health check timed out");
    }
    if route_rc > 1 || dns_rc > 1 {
        return optimize_fail("Failed to inspect network health");
    }

    if route_rc == 0 && dns_rc == 0 {
        opt_msg("Network stack already optimal");
        return OptimizeOutcome::Unchanged;
    }

    let mut applied = 0u32;
    let mut failed = 0u32;
    if !is_dry_run() {
        if !optimize_sudo_available() {
            println!("  ⚠ Network stack refresh · skipped (admin access required)");
            return OptimizeOutcome::Skipped;
        }
        if sudo_output(&["/sbin/route", "-n", "flush"])
            .status
            .success()
        {
            opt_msg("Network routing table refreshed");
            applied += 1;
        } else {
            failed += 1;
        }
        if sudo_output(&["/usr/sbin/arp", "-a", "-d"]).status.success() {
            opt_msg("ARP cache cleared");
            applied += 1;
        } else {
            failed += 1;
        }
    } else {
        opt_msg("Network routing table refreshed");
        opt_msg("ARP cache cleared");
        applied = 2;
    }

    if failed > 0 {
        note_failure(&format!(
            "Network stack refresh incomplete ({failed} operation(s) failed)"
        ));
    }
    OptimizeOutcome::from_counts(applied, failed, 0)
}

/// 对齐 SH 第 889-940 行 `opt_disk_permissions_repair`。
///
/// **Bug 修复**:之前用 `std::process::id()` 获取 PID,与 SH `id -u` 语义不符。
/// 改为 `libc::getuid()` 获取真正的当前用户 UID。
pub fn opt_disk_permissions_repair() -> OptimizeOutcome {
    if debug_enabled() {
        debug_operation_start(
            "Disk Permissions Repair",
            Some("Reset user directory permissions"),
        );
        debug_operation_detail(
            "Method",
            "Run diskutil resetUserPermissions on user home directory",
        );
        debug_operation_detail("Condition", "Only runs if permissions issues are detected");
        debug_operation_detail(
            "Expected outcome",
            "Fixed file access issues, correct ownership",
        );
        debug_risk_level("MEDIUM", "Requires sudo, modifies permissions");
    }

    let user_id: u32 = unsafe { libc::getuid() };
    let user_id_str = user_id.to_string();

    // SH 第 901-905 行:先判权限问题(dry-run 同样先判),没有就 UNCHANGED
    if !needs_permissions_repair() {
        opt_msg("User directory permissions already optimal");
        return OptimizeOutcome::Unchanged;
    }

    if !is_dry_run() {
        if !optimize_sudo_available() {
            println!("  ⚠ Disk permissions repair · skipped (admin access required)");
            return OptimizeOutcome::Skipped;
        }
        let ok = sudo_output(&[
            "/usr/sbin/diskutil",
            "resetUserPermissions",
            "/",
            &user_id_str,
        ])
        .status
        .success();
        if ok {
            opt_msg("User directory permissions repaired");
            opt_msg("File access issues resolved");
            OptimizeOutcome::Applied
        } else {
            optimize_fail("Failed to repair permissions, may not be needed")
        }
    } else {
        opt_msg("User directory permissions repaired");
        opt_msg("File access issues resolved");
        OptimizeOutcome::Applied
    }
}

/// 对齐 SH 第 943-1041 行 `opt_spotlight_index_optimize`。
///
/// 修复点:mdutil 状态探测套 3s 超时;两次 mdfind 速度采样判断索引是否真的慢;
/// 电池供电时跳过速度探测(SH 第 963-967 行),只有慢且 AC 供电时才重建。
pub fn opt_spotlight_index_optimize() -> OptimizeOutcome {
    let Some(status_out) = run_with_timeout_capture(3.0, "mdutil", &["-s", "/"]) else {
        return optimize_fail("Failed to inspect Spotlight index");
    };
    let status_lower = status_out.to_lowercase();

    if status_lower.contains("indexing disabled") {
        println!("  ○ Spotlight indexing is disabled");
        return OptimizeOutcome::Skipped;
    }

    let indexing_enabled = status_lower.contains("indexing enabled");
    let indexing_searching_disabled = status_lower.contains("indexing and searching disabled");

    if indexing_enabled && !indexing_searching_disabled {
        // 电池供电时跳过速度探测,结果反正不会用于重建(SH 第 963-967 行)
        if !is_ac_power() {
            opt_msg("Spotlight index already optimal");
            return OptimizeOutcome::Skipped;
        }

        let slow_threshold: u64 = std::env::var("MOLE_OPTIMIZE_SPOTLIGHT_SLOW_SEC")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);

        // 两次速度采样,>阈值算一次慢;两次都慢才认为有问题(SH 第 980-1001 行)
        let mut slow_count = 0u32;
        let mut probe_failed = 0u32;
        for _ in 0..2 {
            let start = get_epoch_seconds();
            // 超时本身就是"慢"的表现(SH 第 985-987 行)
            let probe_rc = run_with_timeout(5.0, "mdfind", &["kMDItemFSName == 'Applications'"]);
            let end = get_epoch_seconds();
            if probe_rc == 124 {
                slow_count += 1;
            } else if probe_rc != 0 {
                probe_failed += 1;
            } else if end.saturating_sub(start) > slow_threshold {
                slow_count += 1;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }

        if probe_failed > 0 {
            return optimize_fail(&format!(
                "Spotlight speed check failed ({probe_failed} probe(s))"
            ));
        }

        if slow_count >= 2 {
            if !is_dry_run() {
                if !optimize_sudo_available() {
                    println!("  ⚠ Spotlight index rebuild · skipped (admin access required)");
                    return OptimizeOutcome::Skipped;
                }
                println!("  ℹ Spotlight search is slow, rebuilding index, may take 1-2 hours");
                if sudo_output(&["/usr/bin/mdutil", "-E", "/"])
                    .status
                    .success()
                {
                    opt_msg("Spotlight index rebuild started");
                    println!("  Indexing will continue in background");
                    OptimizeOutcome::Applied
                } else {
                    optimize_fail("Failed to rebuild Spotlight index")
                }
            } else {
                opt_msg("Spotlight index rebuild started");
                OptimizeOutcome::Applied
            }
        } else {
            opt_msg("Spotlight index already optimal");
            OptimizeOutcome::Unchanged
        }
    } else {
        opt_msg("Spotlight index verified");
        OptimizeOutcome::Unchanged
    }
}

/// 对齐 SH 第 1048-1119 行 `opt_prune_spotlight_orphan_rules`。
/// 读取 Spotlight EnabledPreferenceRules，移除已卸载应用的孤儿规则。
pub fn opt_prune_spotlight_orphan_rules() -> OptimizeOutcome {
    let domain = "com.apple.spotlight";
    let plist = format!("{}/Library/Preferences/{domain}.plist", home_dir());

    // 先检查是否有规则
    let check = Command::new("defaults")
        .args(["read", domain, "EnabledPreferenceRules"])
        .output();
    if !check.map(|o| o.status.success()).unwrap_or(false) {
        opt_msg("Spotlight search rules already clean");
        return OptimizeOutcome::Unchanged;
    }

    let mut keep: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    let mut i = 0u32;
    loop {
        let entry = Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", &format!("Print :EnabledPreferenceRules:{i}"), &plist])
            .output();
        match entry {
            Ok(o) if o.status.success() => {
                let val = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if val.is_empty() {
                    i += 1;
                    continue;
                }
                if val.starts_with("System.") || val.starts_with("com.apple.") {
                    keep.push(val);
                } else if is_reverse_dns_bundle_id(&val) {
                    // 对齐 SH 26f4d47a:resolver 被信号中断(rc>=128)时 fail-closed,
                    // 任务失败,不得当作"未安装"静默删除规则。
                    match bundle_has_installed_app_checked(&val) {
                        None => {
                            return optimize_fail(
                                "Failed to resolve Spotlight rule app, aborting cleanup",
                            );
                        }
                        Some(false) => removed.push(val),
                        Some(true) => keep.push(val),
                    }
                } else {
                    keep.push(val);
                }
                i += 1;
            }
            _ => break,
        }
    }

    if removed.is_empty() {
        opt_msg("Spotlight search rules already clean");
        return OptimizeOutcome::Unchanged;
    }

    if is_dry_run() {
        opt_msg(&format!(
            "Would remove {} orphan Spotlight rule(s)",
            removed.len()
        ));
        return OptimizeOutcome::Applied;
    }

    // 通过 cfprefsd(defaults)重写过滤后的数组,而非就地删除 plist 下标:
    // 避免 cfprefsd 缓存覆盖直接文件编辑,保证系统设置面板立即反映变更(SH 第 1102-1110 行)
    let write_ok = if keep.is_empty() {
        Command::new("defaults")
            .args(["delete", domain, "EnabledPreferenceRules"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    } else {
        let mut args: Vec<String> = vec![
            "write".to_string(),
            domain.to_string(),
            "EnabledPreferenceRules".to_string(),
            "-array".to_string(),
        ];
        args.extend(keep.iter().cloned());
        Command::new("defaults")
            .args(&args)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };

    if write_ok {
        opt_msg(&format!(
            "Removed {} orphan Spotlight rule(s)",
            removed.len()
        ));
        OptimizeOutcome::Applied
    } else {
        optimize_fail("Failed to remove orphan Spotlight rules")
    }
}

/// 对齐 SH 第 1125-1165 行 `opt_prevent_network_dsstore`。
pub fn opt_prevent_network_dsstore() -> OptimizeOutcome {
    let keys = ["DSDontWriteNetworkStores", "DSDontWriteUSBStores"];
    let domain = "com.apple.desktopservices";
    let mut changed = 0u32;
    let mut already = 0u32;
    let mut failed = 0u32;
    for key in &keys {
        let current = Command::new("defaults")
            .args(["read", domain, key])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        if current == "1" {
            already += 1;
            continue;
        }
        if is_dry_run() {
            changed += 1;
            continue;
        }
        if Command::new("defaults")
            .args(["write", domain, key, "-bool", "true"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            changed += 1;
        } else {
            failed += 1;
        }
    }
    if changed == 0 && already > 0 {
        opt_msg(".DS_Store prevention already enabled on network & USB volumes");
    }
    if changed > 0 {
        opt_msg(".DS_Store prevention enabled on network & USB volumes");
    } else if failed > 0 {
        note_failure("Failed to enable .DS_Store prevention");
    }
    if changed > 0 && failed > 0 {
        note_failure(&format!(
            "Failed to enable .DS_Store prevention for {failed} volume type(s)"
        ));
    }
    OptimizeOutcome::from_counts(changed, failed, 0)
}

/// 对齐 SH 第 897-903 行 `launch_agent_volume_mounted`。
/// 当 binary 路径在 /Volumes/ 下时，检查对应卷是否已挂载。
/// 路径不在 /Volumes/ 下则返回 true（视为已挂载，可正常判定）。
fn launch_agent_volume_mounted(binary: &str) -> bool {
    if let Some(rest) = binary.strip_prefix("/Volumes/") {
        let vol = rest.split('/').next().unwrap_or("");
        !vol.is_empty() && Path::new(&format!("/Volumes/{vol}")).is_dir()
    } else {
        true
    }
}

/// 对齐 SH 第 1257-1327 行 `opt_launch_agents_cleanup`。
pub fn opt_launch_agents_cleanup() -> OptimizeOutcome {
    let home = home_dir();
    let agents_dir = format!("{home}/Library/LaunchAgents");
    if !Path::new(&agents_dir).is_dir() {
        opt_msg("Launch Agents all healthy");
        return OptimizeOutcome::Unchanged;
    }

    let mut broken_files: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&agents_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("plist") {
                continue;
            }
            if !p.is_file() {
                continue;
            }
            let binary = match plist_get_program(&p.to_string_lossy()) {
                Ok(binary) => binary,
                Err(()) => {
                    // 对齐 SH 26f4d47a:PlistBuddy 超时/信号 → 任务级失败
                    return optimize_fail(&format!(
                        "Failed to probe Launch Agent (timeout): {}",
                        p.display()
                    ));
                }
            };
            // 对齐 SH 第 908-916 行：仅当路径绝对、文件确实不存在、且卷已挂载时才视为 broken。
            // 未挂载的外置卷上的 plist 不删除（拔盘后路径不存在不是真正的 broken）。
            if let Some(binary) = binary {
                if Path::new(&binary).is_absolute()
                    && !Path::new(&binary).exists()
                    && launch_agent_volume_mounted(&binary)
                {
                    broken_files.push(p.to_string_lossy().to_string());
                }
            }
        }
    }

    if broken_files.is_empty() {
        opt_msg("Launch Agents all healthy");
        return OptimizeOutcome::Unchanged;
    }

    let mut removed_count = 0u32;
    let mut failed = 0u32;
    for plist in &broken_files {
        // 对齐 SH 26f4d47a:unload 超时/信号 → 任务级失败
        if !run_launchctl_unload(plist, false) {
            return optimize_fail(&format!("Failed to unload Launch Agent (timeout): {plist}"));
        }
        if safe_remove(plist, true) {
            removed_count += 1;
        } else {
            failed += 1;
        }
    }
    if removed_count > 0 {
        opt_msg(&format!("Cleaned {removed_count} broken Launch Agent(s)"));
    }
    if failed > 0 {
        note_failure(&format!("Failed to remove {failed} broken Launch Agent(s)"));
    }
    OptimizeOutcome::from_counts(removed_count, failed, 0)
}

/// 对齐 SH 第 1332-1380 行 `opt_periodic_maintenance`。
///
/// 修复点:
/// - 检查 daily.out 修改时间,< 7 天直接跳过(SH 第 1343-1354 行);
/// - 支持 `MOLE_PERIODIC_LOG` 覆盖路径(SH 第 1340 行);
/// - 失败时把 stderr 写入 debug log(SH 第 1362-1375 行)。
pub fn opt_periodic_maintenance() -> OptimizeOutcome {
    if !command_available("periodic") {
        opt_msg("Periodic maintenance skipped (not available on this macOS version)");
        return OptimizeOutcome::Unavailable;
    }

    let daily_log =
        std::env::var("MOLE_PERIODIC_LOG").unwrap_or_else(|_| "/var/log/daily.out".to_string());
    let stale_days: u64 = 7;

    if Path::new(&daily_log).is_file() {
        let last_mod = std::fs::metadata(&daily_log)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let now = get_epoch_seconds();
        let age_days = now.saturating_sub(last_mod) / 86400;
        if age_days < stale_days {
            opt_msg(&format!(
                "Periodic maintenance already current ({age_days}d ago)"
            ));
            return OptimizeOutcome::Unchanged;
        }
    }

    if !is_dry_run() {
        // 没有 sudo 就跳过,与 SH 第 1357-1360 行一致
        if !optimize_sudo_available() {
            opt_msg("Periodic maintenance skipped (requires sudo)");
            return OptimizeOutcome::Skipped;
        }
        let out = sudo_output(&["/usr/sbin/periodic", "daily", "weekly", "monthly"]);
        if out.status.success() {
            opt_msg("Periodic maintenance triggered");
            OptimizeOutcome::Applied
        } else {
            let rc = out.status.code().unwrap_or(-1);
            note_failure(&format!("Failed to run periodic maintenance (exit={rc})"));
            let merged = String::from_utf8_lossy(&out.stderr).to_string();
            if !merged.is_empty() {
                debug_log(&format!("periodic stderr: {merged}"));
            }
            OptimizeOutcome::Failed
        }
    } else {
        opt_msg("Periodic maintenance triggered");
        OptimizeOutcome::Applied
    }
}

/// 对齐 SH 第 1383-1439 行 `opt_shared_file_list_repair`。
pub fn opt_shared_file_list_repair() -> OptimizeOutcome {
    let home = home_dir();
    let sfl_dir = format!("{home}/Library/Application Support/com.apple.sharedfilelist");
    if !Path::new(&sfl_dir).is_dir() {
        opt_msg("Shared file lists directory not found");
        return OptimizeOutcome::Unchanged;
    }

    let mut repaired = 0u32;
    let mut remove_failed = 0u32;
    for entry in WalkDir::new(&sfl_dir)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let p = entry.path();
        let path_str = p.to_string_lossy().to_string();
        let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
        if ext != "sfl2" && ext != "sfl3" {
            continue;
        }
        // 跳过用户数据(最近文档列表)
        if path_str.contains("ApplicationRecentDocuments") {
            continue;
        }
        if !plutil_lint(&path_str) {
            if is_dry_run() {
                repaired += 1;
            } else if safe_remove(&path_str, true) {
                repaired += 1;
            } else {
                remove_failed += 1;
            }
        }
    }

    if repaired > 0 {
        opt_msg(&format!(
            "Repaired {repaired} corrupted shared file list(s)"
        ));
    } else if remove_failed == 0 {
        opt_msg("Shared file lists all healthy");
    }
    if remove_failed > 0 {
        note_failure(&format!(
            "Failed to repair {remove_failed} corrupted shared file list(s)"
        ));
    }
    OptimizeOutcome::from_counts(repaired, remove_failed, 0)
}

/// 对齐 SH 第 1447-1462 行 `resolve_notification_center_db`。
/// macOS 15+ 走 usernoted group container;老系统走 DARWIN_USER_DIR。
/// 优先返回实际存在的路径,避免 usernoted 持库打开时报"数据库不存在"的假阴性(issue #1368)。
fn resolve_notification_center_db() -> Option<String> {
    let home = home_dir();
    let group_db = format!("{home}/Library/Group Containers/group.com.apple.usernoted/db2/db");
    if Path::new(&group_db).is_file() {
        return Some(group_db);
    }

    let darwin_dir = Command::new("getconf")
        .arg("DARWIN_USER_DIR")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if !darwin_dir.is_empty() {
        let db = format!(
            "{}com.apple.notificationcenter/db2/db",
            trailing_slash(&darwin_dir)
        );
        if Path::new(&db).is_file() {
            return Some(db);
        }
    }
    None
}

/// 对齐 SH 第 1465-1512 行 `opt_notification_cleanup`。
///
/// 修复点:
/// - 数据库路径解析优先 group container(issue #1368);
/// - 小于 50MB(51200 KB)直接判健康;
/// - 检查 sqlite3 可用性,失败时输出 "busy or locked"。
pub fn opt_notification_cleanup() -> OptimizeOutcome {
    let Some(nc_db) = resolve_notification_center_db() else {
        // Unavailable,不是健康的空状态:成功路径的"not found"曾把 Sequoia
        // 新路径漏掉变成 no-op(issue #1368)。
        println!("  - Notification Center database unavailable (no supported path)");
        return OptimizeOutcome::Unavailable;
    };
    debug_log(&format!("Notification Center database: {nc_db}"));

    let db_size_kb = du_sk(&nc_db);
    if db_size_kb == 0 {
        return optimize_fail("Failed to inspect Notification Center database size");
    }
    if db_size_kb < 51200 {
        opt_msg(&format!(
            "Notification Center database is healthy ({})",
            bytes_to_human(db_size_kb.saturating_mul(1024))
        ));
        return OptimizeOutcome::Unchanged;
    }

    if !is_dry_run() {
        if !command_available("sqlite3") {
            println!("  ⚠ sqlite3 not available");
            return OptimizeOutcome::Unavailable;
        }
        let rc = run_with_timeout(
            10.0,
            "sqlite3",
            &[
                &nc_db,
                "DELETE FROM record WHERE delivered_date < strftime('%s','now','-30 days'); VACUUM;",
            ],
        );
        if rc == 0 {
            let _ = Command::new("killall").arg("NotificationCenter").output();
            opt_msg(&format!(
                "Notification Center database cleaned (was {})",
                bytes_to_human(db_size_kb.saturating_mul(1024))
            ));
            OptimizeOutcome::Applied
        } else {
            optimize_fail("Notification Center cleanup skipped (database busy or locked)")
        }
    } else {
        opt_msg(&format!(
            "Notification Center database cleaned (was {})",
            bytes_to_human(db_size_kb.saturating_mul(1024))
        ));
        OptimizeOutcome::Applied
    }
}

fn trailing_slash(p: &str) -> String {
    if p.ends_with('/') {
        p.to_string()
    } else {
        format!("{p}/")
    }
}

/// 对齐 SH `du -sk <file> | awk '{print $1}'`,失败返回 0。
fn du_sk(path: &str) -> u64 {
    Command::new("du")
        .args(["-sk", path])
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok())
        })
        .unwrap_or(0)
}

/// 对齐 SH 第 1518-1557 行 `opt_disk_verify`。
///
/// 修复点:
/// - 用 `run_with_timeout 30` 控制超时(SH 第 1536 行);
/// - 同时看 stdout 与 stderr(SH `2>&1`);
/// - 结果分类:OK→unchanged、error/corrupt→attention、其余→failed。
pub fn opt_disk_verify() -> OptimizeOutcome {
    if std::env::var("MOLE_ENABLE_DISK_VERIFY").unwrap_or_default() != "1" {
        opt_msg("Disk verify skipped (set MOLE_ENABLE_DISK_VERIFY=1 to enable)");
        return OptimizeOutcome::Skipped;
    }
    if is_dry_run() {
        opt_msg("Disk verify · skipped in dry-run");
        return OptimizeOutcome::Skipped;
    }

    let Some(output) = run_with_timeout_capture(30.0, "diskutil", &["verifyVolume", "/"]) else {
        return optimize_fail("Disk verification timed out or failed");
    };
    let lower = output.to_lowercase();
    if lower.contains("appears to be ok") || lower.contains("volume appears to be ok") {
        opt_msg("Disk filesystem verified OK");
        OptimizeOutcome::Unchanged
    } else if lower.contains("error") || lower.contains("corrupt") || lower.contains("invalid") {
        println!("  ⚠ Disk issues detected · run: sudo diskutil repairVolume /");
        OptimizeOutcome::Attention
    } else {
        optimize_fail("Disk verification result was not recognized")
    }
}

/// 对齐 SH 第 1560-1645 行 `opt_coreduet_cleanup`。
///
/// 修复点:
/// - db + wal + shm 总大小 < 100MB 时直接判健康(SH 第 1590-1595 行);
/// - 检查 sqlite3 可用性(SH 第 1597-1602 行);
/// - wal/shm 删除与 SQL 各自计数,失败/locked 输出对应警告。
pub fn opt_coreduet_cleanup() -> OptimizeOutcome {
    let home = home_dir();
    let knowledge_dir = format!("{home}/Library/Application Support/Knowledge");
    let knowledge_db = format!("{knowledge_dir}/knowledgeC.db");
    if !Path::new(&knowledge_db).is_file() {
        opt_msg("Knowledge database not found");
        return OptimizeOutcome::Unchanged;
    }

    let wal_file = format!("{knowledge_db}-wal");
    let shm_file = format!("{knowledge_db}-shm");
    let mut total_kb: u64 = 0;
    let mut inspect_failed = false;
    for f in [&knowledge_db, &wal_file, &shm_file] {
        if Path::new(f).is_file() {
            let kb = du_sk(f);
            if kb == 0 {
                inspect_failed = true;
            }
            total_kb = total_kb.saturating_add(kb);
        }
    }
    if inspect_failed {
        return optimize_fail("Failed to inspect Knowledge database size");
    }

    if total_kb < 102_400 {
        opt_msg(&format!(
            "Knowledge database is healthy ({})",
            bytes_to_human(total_kb.saturating_mul(1024))
        ));
        return OptimizeOutcome::Unchanged;
    }

    if !is_dry_run() {
        if !command_available("sqlite3") {
            println!("  ⚠ sqlite3 not available");
            return OptimizeOutcome::Unavailable;
        }
        // 删除 WAL/SHM 文件(SQLite 自动重建)
        let mut removed_count = 0u32;
        let mut remove_failed = 0u32;
        for f in [&wal_file, &shm_file] {
            if Path::new(f).is_file() {
                if safe_remove(f, true) {
                    removed_count += 1;
                } else {
                    remove_failed += 1;
                }
            }
        }
        // 删除 90 天前的 ZOBJECT(CoreTime 是 Mac 纪元:2001-01-01 起秒数)
        let sql_applied: u32;
        let sql_failed: u32;
        let rc = run_with_timeout(
            10.0,
            "sqlite3",
            &[
                &knowledge_db,
                "DELETE FROM ZOBJECT WHERE ZCREATIONDATE < (strftime('%s','now','-90 days') - strftime('%s','2001-01-01')); VACUUM;",
            ],
        );
        if rc == 0 {
            sql_applied = 1;
            sql_failed = 0;
        } else {
            sql_applied = 0;
            sql_failed = 1;
        }

        if sql_failed > 0 {
            note_failure("Knowledge database cleanup skipped (database busy or locked)");
        } else if remove_failed > 0 {
            note_failure("Knowledge database cleanup incomplete");
        } else {
            opt_msg(&format!(
                "Knowledge database cleaned (was {})",
                bytes_to_human(total_kb.saturating_mul(1024))
            ));
        }
        OptimizeOutcome::from_counts(removed_count + sql_applied, remove_failed + sql_failed, 0)
    } else {
        opt_msg(&format!(
            "Knowledge database cleaned (was {})",
            bytes_to_human(total_kb.saturating_mul(1024))
        ));
        OptimizeOutcome::Applied
    }
}

/// 登录项查找三态结果（对齐 SH 的 probe_uncertain 机制）。
#[derive(Debug, PartialEq)]
enum LoginItemLookup {
    Exists,    // 应用确实存在
    NotFound,  // 应用确实不存在（broken）
    Uncertain, // 探测超时/失败，不能下结论
}

/// 对齐 SH L1693-1712 `_login_item_name_matches`：
/// 大小写不敏感 + 去空格 + 去后缀匹配。
fn login_item_name_matches(actual: &str, expected: &str) -> bool {
    if actual.is_empty() {
        return false;
    }
    let actual_lower = actual.to_lowercase();
    let expected_lower = expected.to_lowercase();
    if actual_lower == expected_lower {
        return true;
    }
    let actual_nospace = actual_lower.replace(' ', "");
    let expected_nospace = expected_lower.replace(' ', "");
    if actual_nospace == expected_nospace {
        return true;
    }
    // 去后缀（Client/Helper/Agent/Launcher/Service）
    let mut stripped = expected_nospace.clone();
    for suffix in &["client", "helper", "agent", "launcher", "service"] {
        if let Some(base) = expected_nospace.strip_suffix(suffix) {
            stripped = base.to_string();
            break;
        }
    }
    if !stripped.is_empty() && stripped != expected_nospace && actual_nospace == stripped {
        return true;
    }
    false
}

/// 通过 `plutil -extract` 提取 plist 键的原始字符串值。
fn plutil_extract_raw(plist: &str, key: &str) -> Option<String> {
    Command::new("plutil")
        .args(["-extract", key, "raw", plist])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 扫描 Applications 目录构建应用清单（对齐 SH `_login_item_build_app_inventory`）。
/// 带超时保护，超时返回 None（调用方视为 uncertain）。
fn login_item_build_app_inventory()
-> Option<Vec<(String, Option<String>, Option<String>, Option<String>)>> {
    let home = home_dir();
    let mut inventory = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);

    for root in &["/Applications", &format!("{home}/Applications")] {
        if std::time::Instant::now() >= deadline {
            return None;
        }
        let root_path = Path::new(root);
        if !root_path.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root_path)
            .max_depth(6)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if std::time::Instant::now() >= deadline {
                return None;
            }
            if !entry.file_type().is_dir() {
                continue;
            }
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("app") {
                continue;
            }
            let info = p.join("Contents/Info.plist");
            if !info.is_file() {
                continue;
            }
            let info_str = info.to_string_lossy().to_string();
            let display_name = plutil_extract_raw(&info_str, "CFBundleDisplayName");
            let bundle_name = plutil_extract_raw(&info_str, "CFBundleName");
            let executable = plutil_extract_raw(&info_str, "CFBundleExecutable");
            inventory.push((
                p.to_string_lossy().to_string(),
                display_name,
                bundle_name,
                executable,
            ));
        }
    }
    Some(inventory)
}

/// 对齐 SH `_login_item_app_exists`（增强版）。
///
/// 返回三态：Exists / NotFound / Uncertain。
/// 查找顺序：mdfind 精确/去空格/去后缀 → 文件系统递归 → System Events 路径 → 应用清单匹配。
/// 不引入 `sfltool dumpbtm`（macOS 14+ 弹授权窗口）。
fn login_item_app_exists(
    name: &str,
    item_path: &str,
    inventory: &Option<Vec<(String, Option<String>, Option<String>, Option<String>)>>,
) -> LoginItemLookup {
    // 路径快速判定：path 非空且存在 → 直接健康
    if !item_path.is_empty() && (Path::new(item_path).exists() || Path::new(item_path).is_symlink())
    {
        return LoginItemLookup::Exists;
    }

    // 1-3. mdfind 匹配（精确 / 去空格 / 去后缀）
    let nospace = name.replace(' ', "");
    let mut stripped = nospace.clone();
    for suffix in &["Client", "Helper", "Agent", "Launcher", "Service"] {
        if let Some(base) = stripped.strip_suffix(suffix) {
            stripped = base.to_string();
            break;
        }
    }
    let q1 = format!("kMDItemFSName == '{name}.app'");
    let q2 = if nospace != name {
        Some(format!("kMDItemFSName == '{nospace}.app'"))
    } else {
        None
    };
    let q3 = if stripped != nospace {
        Some(format!("kMDItemFSName == '{stripped}.app'"))
    } else {
        None
    };
    let queries: Vec<String> = std::iter::once(q1)
        .chain(q2.into_iter())
        .chain(q3.into_iter())
        .collect();
    for q in &queries {
        if mdfind_hit(q) {
            return LoginItemLookup::Exists;
        }
    }

    // 4. 文件系统递归查找嵌套 helper app
    let home = home_dir();
    let app_names = [
        format!("{name}.app"),
        format!("{nospace}.app"),
        format!("{stripped}.app"),
    ];
    for root in ["/Applications", &format!("{home}/Applications")] {
        let root_path = Path::new(root);
        if !root_path.is_dir() {
            continue;
        }
        for app_name in &app_names {
            for entry in WalkDir::new(root_path)
                .max_depth(6)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.file_name().to_string_lossy() == app_name.as_str() {
                    return LoginItemLookup::Exists;
                }
            }
        }
    }

    // 5. System Events 路径回退（替代 sfltool dumpbtm，避免 macOS 14+ 弹窗）
    let script = format!(
        "tell application \"System Events\" to get the path of every login item whose name is \"{}\"",
        name.replace('"', "\\\"")
    );
    if let Ok(out) = Command::new("osascript").args(["-e", &script]).output() {
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !path.is_empty() && Path::new(&path).exists() {
            return LoginItemLookup::Exists;
        }
    }

    // 6. 应用清单元数据匹配（对齐 SH `_login_item_build_app_inventory`）
    if let Some(inv) = inventory {
        for (app_path, display_name, bundle_name, executable) in inv {
            if !Path::new(app_path).exists() && !Path::new(app_path).is_symlink() {
                continue;
            }
            // 匹配 .app 文件名（去掉 .app 后缀）
            let app_basename = Path::new(app_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if login_item_name_matches(app_basename, name) {
                return LoginItemLookup::Exists;
            }
            // 匹配 Info.plist 元数据
            for meta in [display_name, bundle_name, executable]
                .iter()
                .filter_map(|m| m.as_ref())
            {
                if login_item_name_matches(meta, name) {
                    return LoginItemLookup::Exists;
                }
            }
        }
    }

    // 如果清单构建超时（None），返回 Uncertain 而非 NotFound，避免误报
    if inventory.is_none() {
        return LoginItemLookup::Uncertain;
    }

    LoginItemLookup::NotFound
}

/// 对齐 SH L1988-2122 `opt_login_items_audit`（增强版）。
///
/// 改进点：
/// - AppleScript 获取 name + path（tab 分隔），而非仅 name；
/// - path 存在时直接判健康（快速路径）；
/// - 应用清单元数据匹配（CFBundleDisplayName/CFBundleName/CFBundleExecutable）；
/// - 探测超时时判 Uncertain 而非 broken，避免误报。
pub fn opt_login_items_audit() -> OptimizeOutcome {
    if std::env::var("MOLE_TEST_NO_AUTH").unwrap_or_default() == "1" {
        opt_msg("Login items audit · skipped in test mode");
        return OptimizeOutcome::Skipped;
    }

    // 对齐 SH `_login_items_snapshot`：获取 name + path（tab 分隔）
    let script = r#"
set oldDelimiters to AppleScript's text item delimiters
set tabChar to ASCII character 9
set linefeedChar to ASCII character 10
set outputLines to {}
tell application "System Events"
    repeat with loginItem in login items
        set itemName to ""
        set itemPath to ""
        try
            set itemName to name of loginItem as text
        end try
        try
            set itemPath to POSIX path of (path of loginItem as alias)
        on error
            try
                set itemPath to path of loginItem as text
            end try
        end try
        set end of outputLines to itemName & tabChar & itemPath
    end repeat
end tell
set AppleScript's text item delimiters to linefeedChar
set outputText to outputLines as text
set AppleScript's text item delimiters to oldDelimiters
return outputText
"#;

    let snapshot = run_with_timeout_capture(10.0, "osascript", &["-e", script]);
    let items_str = match snapshot {
        Some(s) if !s.trim().is_empty() => s,
        Some(_) => {
            opt_msg("No login items found");
            return OptimizeOutcome::Unchanged;
        }
        None => {
            return optimize_fail("Failed to inspect login items (snapshot timed out)");
        }
    };

    // 解析 (name, path) 对
    let items: Vec<(&str, &str)> = items_str
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, '\t');
            let name = parts.next().unwrap_or("").trim();
            let path = parts.next().unwrap_or("").trim();
            if name.is_empty() {
                None
            } else {
                Some((name, path))
            }
        })
        .collect();

    if items.is_empty() {
        opt_msg("No login items found");
        return OptimizeOutcome::Unchanged;
    }

    // 判断是否需要构建应用清单（只有 path 缺失/不存在的项才需要）
    let inventory_needed = items.iter().any(|(_, path)| {
        path.is_empty() || (!Path::new(path).exists() && !Path::new(path).is_symlink())
    });

    let inventory = if inventory_needed {
        login_item_build_app_inventory()
    } else {
        Some(Vec::new())
    };

    let mut broken = 0u32;
    let mut checked = 0u32;
    let mut uncertain = false;

    for (name, path) in &items {
        checked += 1;
        let result = login_item_app_exists(name, path, &inventory);
        match result {
            LoginItemLookup::Exists => continue,
            LoginItemLookup::Uncertain => {
                uncertain = true;
                continue;
            }
            LoginItemLookup::NotFound => {
                println!("  ⚠ Broken login item: {name} (app not found)");
                broken += 1;
            }
        }
    }

    // 探测不完整时不发表结论（对齐 SH L2093-2106）
    if uncertain && broken == 0 {
        return optimize_fail("Login items audit incomplete (time limit reached)");
    }

    if broken == 0 {
        opt_msg(&format!("Login items all healthy ({checked} checked)"));
        OptimizeOutcome::Unchanged
    } else {
        println!(
            "  ⚠ {broken} broken login item(s) · remove via System Settings > General > Login Items"
        );
        OptimizeOutcome::Attention
    }
}

/// 对齐 SH 第 1174-1239 行 `opt_legacy_overrides_audit`(#1242/#1243)。
/// 检测旧优化工具留下的隐藏偏好覆盖:全局 App Nap 开关(NSAppSleepDisabled)
/// 与 DiskImages skip-verify 系列。只删除显式覆盖键,让 macOS 恢复默认行为,
/// 不写入替代偏好,也不动 plist 文件本身。
pub fn opt_legacy_overrides_audit() -> OptimizeOutcome {
    if debug_enabled() {
        debug_operation_start(
            "Legacy Overrides",
            Some("Detect App Nap and disk-image verification overrides"),
        );
        debug_operation_detail(
            "Method",
            "defaults read -g NSAppSleepDisabled; defaults read com.apple.frameworks.diskimages skip-verify*",
        );
        debug_operation_detail(
            "Expected outcome",
            "Overrides removed so macOS defaults apply again",
        );
        debug_risk_level(
            "LOW",
            "Deletes explicit override keys only; macOS falls back to its default behavior",
        );
    }

    // 对齐 SH 第 1187-1189 行 `_opt_defaults_is_truthy`
    fn defaults_is_truthy(v: &str) -> bool {
        matches!(
            v.trim(),
            "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes"
        )
    }

    let home = home_dir();
    // (label, domain, key, plist)
    let mut found: Vec<(String, String, String, String)> = Vec::new();
    let mut push_found = |label: &str, domain: &str, key: &str, plist: String| {
        found.push((
            label.to_string(),
            domain.to_string(),
            key.to_string(),
            plist,
        ));
    };

    let value = Command::new("defaults")
        .args(["read", "-g", "NSAppSleepDisabled"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if defaults_is_truthy(&value) {
        push_found(
            "App Nap disabled globally (NSAppSleepDisabled)",
            "-g",
            "NSAppSleepDisabled",
            format!("{home}/Library/Preferences/.GlobalPreferences.plist"),
        );
    }

    for key in ["skip-verify", "skip-verify-locked", "skip-verify-remote"] {
        let value = Command::new("defaults")
            .args(["read", "com.apple.frameworks.diskimages", key])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        if defaults_is_truthy(&value) {
            push_found(
                &format!("Disk-image verification skipped ({key})"),
                "com.apple.frameworks.diskimages",
                key,
                format!("{home}/Library/Preferences/com.apple.frameworks.diskimages.plist"),
            );
        }
    }

    if found.is_empty() {
        opt_msg("No legacy App Nap or disk-image overrides found");
        return OptimizeOutcome::Unchanged;
    }

    let mut changed = 0u32;
    let mut skipped = 0u32;
    let mut failed = 0u32;
    for (label, domain, key, plist) in &found {
        if is_path_whitelisted_from_global(plist) {
            opt_msg(&format!("Skipped (whitelisted): {label}"));
            skipped += 1;
            continue;
        }
        if is_dry_run() {
            println!("  → Would remove override: {label}");
            changed += 1;
            continue;
        }
        if Command::new("defaults")
            .args(["delete", domain, key])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            opt_msg(&format!("Removed override: {label}"));
            changed += 1;
        } else {
            note_failure(&format!("Could not remove override: {label}"));
            failed += 1;
        }
    }

    OptimizeOutcome::from_counts(changed, failed, skipped)
}

/// 对齐 SH 第 1861-1887 行 `execute_optimization`。
/// 分支顺序对齐 catalog.sh 的 21 任务注册顺序;返回任务级六态结局,
/// 未知 action 返回 None(对齐 SH 返回非零退出码)。
pub fn execute_optimization(action: &str) -> Option<OptimizeOutcome> {
    let outcome = match action {
        "system_maintenance" => opt_system_maintenance(),
        "cache_refresh" => opt_cache_refresh(),
        "saved_state_cleanup" => opt_saved_state_cleanup(),
        "fix_broken_configs" => opt_fix_broken_configs(),
        "network_optimization" => opt_network_optimization(),
        "sqlite_vacuum" => opt_sqlite_vacuum(),
        "launch_services_rebuild" => opt_launch_services_rebuild(),
        "prevent_network_dsstore" => opt_prevent_network_dsstore(),
        "legacy_overrides_audit" => opt_legacy_overrides_audit(),
        "network_stack_optimize" => opt_network_stack_optimize(),
        "disk_permissions_repair" => opt_disk_permissions_repair(),
        "spotlight_index_optimize" => opt_spotlight_index_optimize(),
        "spotlight_orphan_rules_cleanup" => opt_prune_spotlight_orphan_rules(),
        "periodic_maintenance" => opt_periodic_maintenance(),
        "shared_file_list_repair" => opt_shared_file_list_repair(),
        "disk_verify" => opt_disk_verify(),
        "login_items_audit" => opt_login_items_audit(),
        "quarantine_cleanup" => opt_quarantine_cleanup(),
        "launch_agents_cleanup" => opt_launch_agents_cleanup(),
        "notification_cleanup" => opt_notification_cleanup(),
        "coreduet_cleanup" => opt_coreduet_cleanup(),
        _ => return None,
    };
    Some(outcome)
}

// ============================================================================
// Process / file helpers
// ============================================================================

/// 对齐 SH 26f4d47a 加固后的 PlistBuddy 读取:
/// - `Ok(Some(program))`:读到 Program 键值
/// - `Ok(None)`:PlistBuddy 正常执行但两个键都不存在(该 plist 不算 broken,跳过)
/// - `Err(())`:探测超时(124)/信号(>=128),fail-closed,任务不得继续删除
fn plist_get_program(path: &str) -> Result<Option<String>, ()> {
    let (rc, out) = run_with_timeout_capture_rc(
        2.0,
        "/usr/libexec/PlistBuddy",
        &["-c", "Print :ProgramArguments:0", path],
    );
    if rc == 124 || rc >= 128 {
        return Err(());
    }
    if let Some(s) = out.as_deref().map(str::trim) {
        if !s.is_empty() {
            return Ok(Some(s.to_string()));
        }
    }
    let (rc2, out2) = run_with_timeout_capture_rc(
        2.0,
        "/usr/libexec/PlistBuddy",
        &["-c", "Print :Program", path],
    );
    if rc2 == 124 || rc2 >= 128 {
        return Err(());
    }
    Ok(out2
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string()))
}

fn plutil_lint(path: &str) -> bool {
    Command::new("plutil")
        .args(["-lint", path])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn mdfind_hit(query: &str) -> bool {
    // SH 第 1149 行用 `mdfind ... | grep -q .`,Spotlight 没就绪时这条会卡住;
    // GUI 端给个 2s 上限,与 manage 模块一致。
    run_with_timeout_capture(2.0, "mdfind", &[query])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

/// 对齐 SH `mole_is_reverse_dns_bundle_id`：检查字符串是否为 reverse-DNS bundle id 格式。
fn is_reverse_dns_bundle_id(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut count = 0u32;
    for seg in s.split('.') {
        count += 1;
        if seg.is_empty() {
            return false;
        }
        let mut chars = seg.chars();
        match chars.next() {
            Some(c) if c.is_ascii_alphanumeric() => {}
            _ => return false,
        }
        if !chars.all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return false;
        }
    }
    count >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_slash_works() {
        assert_eq!(trailing_slash("/foo"), "/foo/");
        assert_eq!(trailing_slash("/foo/"), "/foo/");
    }
}
