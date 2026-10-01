//! 系统级清理 — 严格对齐 lib/clean/system.sh
//!
//! 注意:这一层全部需要 sudo,所有删除统一走 `safe_sudo_*`,内部已经做了:
//!   - should_protect_path
//!   - is_path_whitelisted
//!   - SIP / 只读 FS / 认证失败的退出码区分
//! 不要绕过去自己 spawn `find -delete`,否则保护会被旁路。

use regex::Regex;
use std::path::Path;
use std::process::Command;

use crate::core::base::{
    MOLE_CRASH_REPORT_AGE_DAYS, MOLE_GPU_CACHE_AGE_DAYS, MOLE_LOG_AGE_DAYS,
    MOLE_TEMP_FILE_AGE_DAYS, MOLE_TM_BACKUP_SAFE_HOURS, STAT_BSD, begin_activity_probe,
    bytes_to_human_kb, command_available, current_spinner_app_handle, get_epoch_seconds,
    get_file_mtime, get_path_size_kb, note_activity, pgrep_f, run_cmd, start_section_spinner,
    stop_section_spinner,
};
use crate::core::dry_run_registry::dry_run_register_cleanup_target;
use crate::core::file_ops::{MOLE_OK, safe_sudo_find_delete, safe_sudo_remove};
use crate::core::log::{debug_log, log_info, log_operation, log_warning, oplog_enabled};
use crate::core::sudo::sudo_output;
use crate::core::timeout::run_with_timeout_capture;
use crate::events::{
    CleanupPhaseResultPayload, PHASE_SYSTEM_BROWSER_CODE_SIGN_CACHES, PHASE_SYSTEM_CACHES,
    PHASE_SYSTEM_CRASH_REPORTS, PHASE_SYSTEM_DIAGNOSTIC_LOGS, PHASE_SYSTEM_LIBRARY_UPDATES,
    PHASE_SYSTEM_LOGS, PHASE_SYSTEM_MACOS_INSTALLERS, PHASE_SYSTEM_MEMORY_EXCEPTION_REPORTS,
    PHASE_SYSTEM_POWER_LOGS, PHASE_SYSTEM_REBUILDABLE_GPU_CACHES,
    PHASE_SYSTEM_REBUILDABLE_SERVICE_CACHES, PHASE_SYSTEM_TEMP_FILES,
    PHASE_SYSTEM_THIRD_PARTY_LOGS, emit_cleanup_phase_result,
};

/// 主入口,对齐 SH `clean_deep_system()` 阶段顺序与文案。
/// 返回值：(清理的大小KB, 清理的文件数量)
pub fn clean_deep_system() -> super::ModuleScanResult {
    const SECTION: &str = "system";
    const PHASES: &[(&str, &str, &str, fn() -> (u64, u64))] = &[
        (
            PHASE_SYSTEM_CACHES,
            "system_caches",
            "System caches",
            clean_system_caches,
        ),
        (
            PHASE_SYSTEM_TEMP_FILES,
            "system_temp_files",
            "System temp files",
            clean_system_temp_files,
        ),
        (
            PHASE_SYSTEM_CRASH_REPORTS,
            "system_crash_reports",
            "System crash reports",
            clean_system_crash_reports,
        ),
        (
            PHASE_SYSTEM_LOGS,
            "system_logs",
            "System logs",
            clean_system_logs,
        ),
        (
            PHASE_SYSTEM_THIRD_PARTY_LOGS,
            "system_third_party_logs",
            "Third-party system logs",
            clean_third_party_system_logs,
        ),
        (
            PHASE_SYSTEM_LIBRARY_UPDATES,
            "system_library_updates",
            "System library updates",
            clean_system_library_updates,
        ),
        (
            PHASE_SYSTEM_MACOS_INSTALLERS,
            "system_macos_installers",
            "macOS installer files",
            clean_macos_installer_files,
        ),
        (
            PHASE_SYSTEM_BROWSER_CODE_SIGN_CACHES,
            "system_browser_code_sign",
            "Browser code sign caches",
            clean_browser_code_sign_caches,
        ),
        (
            PHASE_SYSTEM_REBUILDABLE_SERVICE_CACHES,
            "system_rebuildable_service",
            "Rebuildable service caches",
            clean_rebuildable_system_service_caches,
        ),
        (
            PHASE_SYSTEM_REBUILDABLE_GPU_CACHES,
            "system_rebuildable_gpu",
            "Rebuildable GPU caches",
            clean_accessible_rebuildable_gpu_caches,
        ),
        (
            PHASE_SYSTEM_DIAGNOSTIC_LOGS,
            "system_diagnostic_logs",
            "Diagnostic logs",
            clean_system_diagnostic_logs,
        ),
        (
            PHASE_SYSTEM_POWER_LOGS,
            "system_power_logs",
            "Power logs",
            clean_power_logs,
        ),
        (
            PHASE_SYSTEM_MEMORY_EXCEPTION_REPORTS,
            "system_memory_exception",
            "Memory exception reports",
            clean_memory_exception_reports,
        ),
    ];

    let mut items: Vec<super::SubItemResult> = Vec::new();

    stop_section_spinner();
    for &(phase, item_id, item_title, work) in PHASES {
        begin_activity_probe();
        start_section_spinner(SECTION, item_title);
        let (kb, cnt) = work();
        items.push(super::SubItemResult::new(item_id, item_title, kb, cnt));
        stop_section_spinner();
        let cleaned = kb > 0 || cnt > 0;
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(
                &app,
                &CleanupPhaseResultPayload {
                    section: SECTION.to_string(),
                    phase: phase.to_string(),
                    title: item_title.to_string(),
                    cleaned,
                    size_kb: Some(kb),
                    file_count: Some(cnt),
                },
            );
        }
    }

    super::ModuleScanResult { items }
}

/// 顶层 Time Machine 入口:对齐 SH `clean_time_machine_failed_backups`。
/// 与 `clean_deep_system` 分开调用是因为它涉及外接磁盘,需要单独 spinner 节奏。
pub fn clean_time_machine_failed_backups() -> (u64, u64) {
    if !command_available("tmutil") {
        return (0, 0);
    }
    if !time_machine_configured() {
        return (0, 0);
    }

    start_section_spinner("time_machine", "Checking Time Machine configuration...");
    let info = run_with_timeout_capture(2.0, "tmutil", &["destinationinfo"]).unwrap_or_default();
    if info.contains("No destinations configured") || info.is_empty() {
        stop_section_spinner();
        return (0, 0);
    }
    if !Path::new("/Volumes").is_dir() {
        stop_section_spinner();
        return (0, 0);
    }
    if tm_is_running() == TmStatus::Running {
        stop_section_spinner();
        log_warning("Time Machine backup in progress, skipping cleanup");
        return (0, 0);
    }
    stop_section_spinner();

    start_section_spinner("time_machine", "Checking backup volumes...");
    let mut backup_volumes: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        for entry in rd.flatten() {
            let path = entry.path();
            let path_str = path.to_string_lossy().to_string();
            if !path.is_dir() || path.is_symlink() {
                continue;
            }
            if path_str == "/Volumes/MacintoshHD" || path_str == "/" {
                continue;
            }
            let backupdb = format!("{path_str}/Backups.backupdb");
            let mobile = format!("{path_str}/.MobileBackups");
            if Path::new(&backupdb).is_dir() || Path::new(&mobile).is_dir() {
                backup_volumes.push(path_str);
            }
        }
    }
    if backup_volumes.is_empty() {
        stop_section_spinner();
        return (0, 0);
    }
    stop_section_spinner();

    start_section_spinner("time_machine", "Scanning backup volumes...");
    let mut spinner_active = true;
    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;
    for volume in &backup_volumes {
        let fs_type = run_with_timeout_capture(1.0, "df", &["-T", volume])
            .unwrap_or_default()
            .lines()
            .last()
            .and_then(|l| l.split_whitespace().nth(1).map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        if matches!(
            fs_type.as_str(),
            "nfs" | "smbfs" | "afpfs" | "cifs" | "webdav" | "unknown"
        ) {
            continue;
        }

        let backupdb_dir = format!("{volume}/Backups.backupdb");
        if Path::new(&backupdb_dir).is_dir() && spinner_active {
            stop_section_spinner();
            spinner_active = false;
        }
        if Path::new(&backupdb_dir).is_dir() {
            let (kb, cnt) = scan_and_delete_inprogress(&backupdb_dir);
            total_kb = total_kb.saturating_add(kb);
            total_count = total_count.saturating_add(cnt);
        }

        for ext in [".backupbundle", ".sparsebundle"] {
            if let Ok(rd) = std::fs::read_dir(volume) {
                for entry in rd.flatten() {
                    let p = entry.path();
                    if !p.is_dir() {
                        continue;
                    }
                    let name = p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string();
                    if !name.ends_with(ext) {
                        continue;
                    }
                    if let Some(mounted) = hdiutil_mount_point_for(&name) {
                        if spinner_active {
                            stop_section_spinner();
                            spinner_active = false;
                        }
                        let (kb, cnt) = scan_and_delete_inprogress(&mounted);
                        total_kb = total_kb.saturating_add(kb);
                        total_count = total_count.saturating_add(cnt);
                    }
                }
            }
        }
    }
    if spinner_active {
        stop_section_spinner();
    }
    if total_count == 0 {
        log_info("No incomplete backups found");
    }
    (total_kb, total_count)
}

pub fn clean_local_snapshots() {
    if !command_available("tmutil") {
        return;
    }
    if !time_machine_configured() {
        return;
    }

    start_section_spinner("system", "Checking Time Machine status...");
    match tm_is_running() {
        TmStatus::Unknown => {
            stop_section_spinner();
            log_warning("Could not determine Time Machine status; skipping snapshot check");
            return;
        }
        TmStatus::Running => {
            stop_section_spinner();
            log_warning("Time Machine is active; skipping snapshot check");
            return;
        }
        TmStatus::NotRunning => {}
    }
    stop_section_spinner();

    start_section_spinner("system", "Checking local snapshots...");
    let snapshot_list =
        run_with_timeout_capture(3.0, "tmutil", &["listlocalsnapshots", "/"]).unwrap_or_default();
    if snapshot_list.is_empty() {
        stop_section_spinner();
        return;
    }
    let re = Regex::new(r"com\.apple\.TimeMachine\.\d{4}-\d{2}-\d{2}-\d{6}").unwrap();
    let count = snapshot_list.lines().filter(|l| re.is_match(l)).count();
    stop_section_spinner();
    if count > 0 {
        log_info(&format!(
            "Time Machine local snapshots: {count}, review with: tmutil listlocalsnapshots /"
        ));
        note_activity();
    }
}

// =============================================================================
// 内部 helpers
// =============================================================================

#[derive(PartialEq, Eq)]
enum TmStatus {
    Running,
    NotRunning,
    Unknown,
}

/// `defaults read /Library/Preferences/com.apple.TimeMachine AutoBackup` 是 0 或 1 即视为已配置。
fn time_machine_configured() -> bool {
    let out = Command::new("defaults")
        .args([
            "read",
            "/Library/Preferences/com.apple.TimeMachine",
            "AutoBackup",
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout);
            s.lines().any(|l| matches!(l.trim(), "0" | "1"))
        }
        _ => false,
    }
}

/// 对齐 SH `tm_is_running()` 第 406-417 行。
/// 退出码 0 → Running,1 → NotRunning,2 → Unknown。
fn tm_is_running() -> TmStatus {
    let out = Command::new("tmutil").arg("status").output();
    let Ok(out) = out else {
        return TmStatus::Unknown;
    };
    if !out.status.success() {
        return TmStatus::Unknown;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let has_running_field = stdout
        .lines()
        .any(|l| l.contains("Running") && l.contains('='));
    if !has_running_field {
        return TmStatus::Unknown;
    }
    let running = stdout.lines().any(|l| {
        let t = l.trim();
        // Running = 1; / Running=1; / "Running" = 1
        let cleaned = t.replace(['"', ';', ' '], "");
        cleaned.contains("Running=1")
    });
    if running {
        TmStatus::Running
    } else {
        TmStatus::NotRunning
    }
}

fn hdiutil_mount_point_for(bundle_name: &str) -> Option<String> {
    let out = Command::new("hdiutil").arg("info").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let mut next_volumes = false;
    for line in stdout.lines() {
        if line.contains("image-path") && line.contains(bundle_name) {
            next_volumes = true;
            continue;
        }
        if next_volumes {
            // 期望像 "/Volumes/Time Machine ..." 这样的行
            if let Some(idx) = line.find("/Volumes/") {
                let candidate: String = line[idx..]
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string();
                if !candidate.is_empty() && Path::new(&candidate).is_dir() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn scan_and_delete_inprogress(base: &str) -> (u64, u64) {
    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;
    // find ... -name "*.inProgress" -o -name "*.inprogress"
    let out = run_with_timeout_capture(
        15.0,
        "find",
        &[
            base,
            "-maxdepth",
            "3",
            "-type",
            "d",
            "(",
            "-name",
            "*.inProgress",
            "-o",
            "-name",
            "*.inprogress",
            ")",
        ],
    );
    let Some(stdout) = out else { return (0, 0) };
    for line in stdout.lines() {
        let candidate = line.trim();
        if candidate.is_empty() || !Path::new(candidate).is_dir() {
            continue;
        }
        let mtime = get_file_mtime(candidate);
        let now = get_epoch_seconds();
        let hours_old = now.saturating_sub(mtime) / 3600;
        if hours_old < MOLE_TM_BACKUP_SAFE_HOURS as u64 {
            continue;
        }
        let size_kb = get_path_size_kb(candidate);
        if size_kb == 0 {
            continue;
        }
        let dry_run = std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1"
            || std::env::var("DRY_RUN").unwrap_or_default() == "true";
        if dry_run {
            debug_log(&format!(
                "[DRY RUN] Incomplete backup: {candidate} ({})",
                bytes_to_human_kb(size_kb)
            ));
            total_kb = total_kb.saturating_add(size_kb);
            total_count += 1;
            note_activity();
            continue;
        }
        // 用 tmutil delete(对齐 SH 第 327 行 / 380 行)
        let ok = Command::new("tmutil")
            .args(["delete", candidate])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            log_operation("clean", "REMOVED", candidate, Some("incomplete backup"));
            total_kb = total_kb.saturating_add(size_kb);
            total_count += 1;
            note_activity();
        } else {
            log_warning(&format!(
                "Could not delete incomplete backup: {candidate}, try manually with sudo"
            ));
        }
    }
    (total_kb, total_count)
}

// -----------------------------------------------------------------------------
// clean_deep_system 子步骤
// -----------------------------------------------------------------------------

fn sudo_dir_exists(path: &str) -> bool {
    sudo_output(&["/bin/test", "-d", path]).status.success()
}

fn sudo_path_exists(path: &str) -> bool {
    sudo_output(&["/bin/test", "-e", path]).status.success()
}

fn sudo_find_has_match(args: &[String]) -> bool {
    let all_args: Vec<&str> = std::iter::once("/usr/bin/find")
        .chain(args.iter().map(|s| s.as_str()))
        .collect();
    !sudo_output(&all_args).stdout.is_empty()
}

/// 对齐 `system.sh` `is_rebuildable_gpu_cache_dir`。
fn is_rebuildable_gpu_cache_dir(path: &str) -> bool {
    let p = path.trim();
    if p.is_empty() {
        return false;
    }
    let starts_with_private = p.starts_with("/private/var/folders/");
    let starts_with_var = p.starts_with("/var/folders/");
    if !starts_with_private && !starts_with_var {
        return false;
    }

    let parts: Vec<&str> = p
        .trim_end_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    let expected_parts = if starts_with_private { 8 } else { 7 };
    if parts.len() != expected_parts {
        return false;
    }

    let c_idx = if starts_with_private { 5 } else { 4 };
    if parts[c_idx] != "C" {
        return false;
    }

    matches!(
        *parts.last().unwrap(),
        "com.apple.gpuarchiver" | "com.apple.metal" | "com.apple.metalfe"
    )
}

/// 对齐 `system.sh` `gpu_cache_dir_is_stale`：目录内无「近 age_days 天内修改过」的文件则视为可删。
fn gpu_cache_dir_is_stale(path: &str, age_days: u32) -> bool {
    if !Path::new(path).is_dir() || Path::new(path).is_symlink() {
        return false;
    }
    let age = format!("-{age_days}");
    let out = Command::new("find")
        .args([path, "-type", "f", "-mtime", &age, "-print", "-quit"])
        .output()
        .ok();
    match out {
        Some(o) if o.status.success() => o.stdout.is_empty(),
        _ => false,
    }
}

pub fn clean_system_caches() -> (u64, u64) {
    if !sudo_dir_exists("/Library/Caches") {
        return (0, 0);
    }
    let age = MOLE_TEMP_FILE_AGE_DAYS;
    let log_age = MOLE_LOG_AGE_DAYS;
    let args: Vec<String> = vec![
        "find".to_string(),
        "/Library/Caches".to_string(),
        "-maxdepth".to_string(),
        "5".to_string(),
        "-type".to_string(),
        "f".to_string(),
        "(".to_string(),
        "(".to_string(),
        "-name".to_string(),
        "*.cache".to_string(),
        "-mtime".to_string(),
        format!("+{age}"),
        ")".to_string(),
        "-o".to_string(),
        "(".to_string(),
        "-name".to_string(),
        "*.tmp".to_string(),
        "-mtime".to_string(),
        format!("+{age}"),
        ")".to_string(),
        "-o".to_string(),
        "(".to_string(),
        "-name".to_string(),
        "*.log".to_string(),
        "-mtime".to_string(),
        format!("+{log_age}"),
        ")".to_string(),
        ")".to_string(),
        "-print0".to_string(),
    ];
    let str_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let all_args: Vec<&str> = std::iter::once("/usr/bin/find")
        .chain(str_args.into_iter())
        .collect();
    let out = sudo_output(&all_args);
    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;
    for entry in out.stdout.split(|&b| b == 0) {
        if entry.is_empty() {
            continue;
        }
        let path = String::from_utf8_lossy(entry);
        let size_kb = get_path_size_kb(path.as_ref());
        if safe_sudo_remove(path.as_ref(), None) == MOLE_OK {
            total_kb = total_kb.saturating_add(size_kb);
            total_count += 1;
        }
    }
    if total_count > 0 {
        note_activity();
    }
    (total_kb, total_count)
}

pub fn clean_system_temp_files() -> (u64, u64) {
    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;
    for tmp_dir in ["/private/tmp", "/private/var/tmp"] {
        if !sudo_dir_exists(tmp_dir) {
            continue;
        }
        let probe_args = vec![
            tmp_dir.to_string(),
            "-maxdepth".to_string(),
            "1".to_string(),
            "-type".to_string(),
            "f".to_string(),
            "-mtime".to_string(),
            format!("+{}", MOLE_TEMP_FILE_AGE_DAYS),
            "-print".to_string(),
            "-quit".to_string(),
        ];
        if !sudo_find_has_match(&probe_args) {
            continue;
        }
        let (kb, cnt) = safe_sudo_find_delete(tmp_dir, "*", MOLE_TEMP_FILE_AGE_DAYS, "f");
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(cnt);
    }
    if total_count > 0 {
        note_activity();
    }
    (total_kb, total_count)
}

pub fn clean_system_crash_reports() -> (u64, u64) {
    let dir = "/Library/Logs/DiagnosticReports";
    if !sudo_dir_exists(dir) {
        return (0, 0);
    }
    let probe_args = vec![
        dir.to_string(),
        "-maxdepth".to_string(),
        "1".to_string(),
        "-type".to_string(),
        "f".to_string(),
        "-mtime".to_string(),
        format!("+{}", MOLE_CRASH_REPORT_AGE_DAYS),
        "-print".to_string(),
        "-quit".to_string(),
    ];
    if !sudo_find_has_match(&probe_args) {
        return (0, 0);
    }
    let (kb, count) = safe_sudo_find_delete(dir, "*", MOLE_CRASH_REPORT_AGE_DAYS, "f");
    if count > 0 {
        note_activity();
    }
    (kb, count)
}

pub fn clean_system_logs() -> (u64, u64) {
    if !sudo_dir_exists("/private/var/log") {
        return (0, 0);
    }
    let probe_args = vec![
        "/private/var/log".to_string(),
        "-maxdepth".to_string(),
        "3".to_string(),
        "-type".to_string(),
        "f".to_string(),
        "(".to_string(),
        "-name".to_string(),
        "*.log".to_string(),
        "-o".to_string(),
        "-name".to_string(),
        "*.gz".to_string(),
        "-o".to_string(),
        "-name".to_string(),
        "*.asl".to_string(),
        ")".to_string(),
        "-mtime".to_string(),
        format!("+{}", MOLE_LOG_AGE_DAYS),
        "-print".to_string(),
        "-quit".to_string(),
    ];
    if !sudo_find_has_match(&probe_args) {
        return (0, 0);
    }
    let (kb_log, count_log) =
        safe_sudo_find_delete("/private/var/log", "*.log", MOLE_LOG_AGE_DAYS, "f");
    let (kb_gz, count_gz) =
        safe_sudo_find_delete("/private/var/log", "*.gz", MOLE_LOG_AGE_DAYS, "f");
    let (kb_asl, count_asl) =
        safe_sudo_find_delete("/private/var/log", "*.asl", MOLE_LOG_AGE_DAYS, "f");
    let total_kb = kb_log + kb_gz + kb_asl;
    let total_count = count_log + count_gz + count_asl;
    if total_count > 0 {
        note_activity();
    }
    (total_kb, total_count)
}

pub fn clean_third_party_system_logs() -> (u64, u64) {
    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;
    for dir in ["/Library/Logs/Adobe", "/Library/Logs/CreativeCloud"] {
        if sudo_dir_exists(dir) {
            let probe_args = vec![
                dir.to_string(),
                "-maxdepth".to_string(),
                "5".to_string(),
                "-type".to_string(),
                "f".to_string(),
                "-mtime".to_string(),
                format!("+{}", MOLE_LOG_AGE_DAYS),
                "-print".to_string(),
                "-quit".to_string(),
            ];
            if !sudo_find_has_match(&probe_args) {
                continue;
            }
            let (kb, count) = safe_sudo_find_delete(dir, "*", MOLE_LOG_AGE_DAYS, "f");
            total_kb = total_kb.saturating_add(kb);
            total_count = total_count.saturating_add(count);
        }
    }
    let adobegc_probe = vec![
        "/Library/Logs".to_string(),
        "-maxdepth".to_string(),
        "1".to_string(),
        "-type".to_string(),
        "f".to_string(),
        "-name".to_string(),
        "adobegc.log".to_string(),
        "-mtime".to_string(),
        format!("+{}", MOLE_LOG_AGE_DAYS),
        "-print".to_string(),
        "-quit".to_string(),
    ];
    if sudo_find_has_match(&adobegc_probe) {
        if safe_sudo_remove("/Library/Logs/adobegc.log", None) == MOLE_OK {
            total_kb = total_kb.saturating_add(100);
            total_count += 1;
        }
    }
    if total_count > 0 {
        note_activity();
    }
    (total_kb, total_count)
}

pub fn clean_system_library_updates() -> (u64, u64) {
    let dir = "/Library/Updates";
    if !Path::new(dir).is_dir() || Path::new(dir).is_symlink() {
        return (0, 0);
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, 0);
    };
    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let path_str = path.to_string_lossy().to_string();

        if !path_str.starts_with("/Library/Updates/")
            || path_str["/Library/Updates/".len()..].contains('/')
        {
            debug_log(&format!("Skipping malformed path: {path_str}"));
            continue;
        }

        if let Some(flags) = run_cmd(STAT_BSD, &["-f%Sf", &path_str]) {
            if flags.contains("restricted") {
                continue;
            }
        }
        let size_kb = get_path_size_kb(&path_str);
        if safe_sudo_remove(&path_str, None) == MOLE_OK {
            total_kb = total_kb.saturating_add(size_kb);
            total_count += 1;
            note_activity();
        }
    }
    (total_kb, total_count)
}

pub fn clean_macos_installer_files() -> (u64, u64) {
    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;

    let install_data = "/macOS Install Data";
    if Path::new(install_data).is_dir() {
        let mtime = get_file_mtime(install_data);
        let now = get_epoch_seconds();
        let age_days = now.saturating_sub(mtime) / 86400;
        debug_log(&format!("Found macOS Install Data, age {age_days} days"));
        if age_days >= 14 {
            let size_kb = get_path_size_kb(install_data);
            if size_kb > 0 {
                debug_log(&format!(
                    "Cleaning macOS Install Data: {}, {age_days} days old",
                    bytes_to_human_kb(size_kb)
                ));
                if safe_sudo_remove(install_data, Some(size_kb)) == MOLE_OK {
                    total_kb = total_kb.saturating_add(size_kb);
                    total_count += 1;
                    note_activity();
                }
            }
        }
    }

    let current_version = run_cmd("sw_vers", &["-productVersion"])
        .unwrap_or_default()
        .split('.')
        .next()
        .unwrap_or("")
        .to_string();

    let installer_globs =
        crate::core::file_ops::expand_glob_paths("/Applications/Install macOS*.app");
    for installer in installer_globs {
        if !Path::new(&installer).is_dir() {
            continue;
        }
        let running = pgrep_f(&installer);
        if running {
            debug_log(&format!("Skipping {installer}: currently running"));
            continue;
        }
        if !current_version.is_empty() {
            let plist = format!("{installer}/Contents/Info.plist");
            if Path::new(&plist).is_file() {
                if let Some(out) = run_cmd(
                    "/usr/libexec/PlistBuddy",
                    &["-c", "Print :DTPlatformVersion", &plist],
                ) {
                    let installer_major = out.split('.').next().unwrap_or("");
                    if !installer_major.is_empty() && installer_major.contains(&current_version) {
                        debug_log(&format!("Keeping {installer}: matches current macOS"));
                        continue;
                    }
                }
            }
        }
        let mtime = get_file_mtime(&installer);
        let now = get_epoch_seconds();
        let age_days = now.saturating_sub(mtime) / 86400;
        if age_days < 14 {
            debug_log(&format!("Keeping {installer}: only {age_days} days old"));
            continue;
        }
        let size_kb = get_path_size_kb(&installer);
        if size_kb > 0 {
            debug_log(&format!(
                "Cleaning macOS installer: {installer}, {}",
                bytes_to_human_kb(size_kb)
            ));
            if safe_sudo_remove(&installer, Some(size_kb)) == MOLE_OK {
                total_kb = total_kb.saturating_add(size_kb);
                total_count += 1;
                note_activity();
            }
        }
    }
    (total_kb, total_count)
}

pub fn clean_browser_code_sign_caches() -> (u64, u64) {
    let stdout = run_with_timeout_capture(
        5.0,
        "sh",
        &[
            "-c",
            "sudo find /private/var/folders -maxdepth 5 -type d -name '*.code_sign_clone' -path '*/X/*' -print0 2>/dev/null",
        ],
    )
    .unwrap_or_default();

    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;
    for entry in stdout.split('\0') {
        let p = entry.trim();
        if p.is_empty() {
            continue;
        }
        let size_kb = get_path_size_kb(p);
        if safe_sudo_remove(p, None) == MOLE_OK {
            total_kb = total_kb.saturating_add(size_kb);
            total_count += 1;
        }
    }
    if total_count > 0 {
        debug_log(&format!(
            "Browser code signature caches cleaned, {total_count} items"
        ));
        note_activity();
    }
    (total_kb, total_count)
}

pub fn clean_rebuildable_system_service_caches() -> (u64, u64) {
    let p = "/Library/Caches/com.apple.iconservices.store";
    if sudo_path_exists(p) {
        let size_kb = get_path_size_kb(p);
        if safe_sudo_remove(p, None) == MOLE_OK {
            note_activity();
            return (size_kb, 1);
        }
    }
    (0, 0)
}

pub fn clean_accessible_rebuildable_gpu_caches() -> (u64, u64) {
    let stdout = run_with_timeout_capture(
        8.0,
        "sh",
        &["-c", "sudo find /private/var/folders /var/folders -maxdepth 8 -type d \\( -name 'com.apple.gpuarchiver' -o -name 'com.apple.metal' -o -name 'com.apple.metalfe' \\) -path '*/C/*' -print0 2>/dev/null"],
    )
    .unwrap_or_default();
    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;
    for entry in stdout.split('\0') {
        let p = entry.trim();
        if p.is_empty() {
            continue;
        }
        if !is_rebuildable_gpu_cache_dir(p) {
            continue;
        }
        if Path::new(p).is_symlink() {
            continue;
        }
        if !gpu_cache_dir_is_stale(p, MOLE_GPU_CACHE_AGE_DAYS) {
            continue;
        }
        let size_kb = get_path_size_kb(p);
        if safe_sudo_remove(p, None) == MOLE_OK {
            total_kb = total_kb.saturating_add(size_kb);
            total_count += 1;
        }
    }
    if total_count > 0 {
        note_activity();
    }
    (total_kb, total_count)
}

pub fn clean_system_diagnostic_logs() -> (u64, u64) {
    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;
    let diag_base = "/private/var/db/diagnostics";
    if sudo_dir_exists(diag_base) {
        let (kb1, count1) = safe_sudo_find_delete(diag_base, "*", MOLE_LOG_AGE_DAYS, "f");
        total_kb = total_kb.saturating_add(kb1);
        total_count = total_count.saturating_add(count1);
        let (kb2, count2) = safe_sudo_find_delete(diag_base, "*.tracev3", 30, "f");
        total_kb = total_kb.saturating_add(kb2);
        total_count = total_count.saturating_add(count2);
    }
    let pipeline = "/private/var/db/DiagnosticPipeline";
    if sudo_dir_exists(pipeline) {
        let (kb, count) = safe_sudo_find_delete(pipeline, "*", MOLE_LOG_AGE_DAYS, "f");
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(count);
    }
    if total_count > 0 {
        note_activity();
    }
    (total_kb, total_count)
}

pub fn clean_power_logs() -> (u64, u64) {
    let dir = "/private/var/db/powerlog";
    if sudo_dir_exists(dir) {
        let (kb, count) = safe_sudo_find_delete(dir, "*", MOLE_LOG_AGE_DAYS, "f");
        if count > 0 {
            note_activity();
        }
        return (kb, count);
    }
    (0, 0)
}

pub fn clean_memory_exception_reports() -> (u64, u64) {
    let dir = "/private/var/db/reportmemoryexception/MemoryLimitViolations";
    if !sudo_dir_exists(dir) {
        return (0, 0);
    }

    let stat_args = format!(
        "sudo find '{dir}' -type f -mtime +30 -exec stat -f '%z' {{}} + 2>/dev/null \
         | awk '{{c++; s+=$1}} END {{print c+0, s+0}}'"
    );
    let stats = run_with_timeout_capture(15.0, "sh", &["-c", &stat_args]).unwrap_or_default();
    let mut iter = stats.split_whitespace();
    let file_count: u64 = iter.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let total_bytes: u64 = iter.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let total_size_kb = total_bytes / 1024;

    if file_count == 0 {
        return (0, 0);
    }
    let dry_run = std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1"
        || std::env::var("DRY_RUN").unwrap_or_default() == "true";
    if dry_run {
        if dry_run_register_cleanup_target(dir) {
            log_info(&format!(
                "[DRY-RUN] Would remove {file_count} old memory exception reports, {} KB",
                total_size_kb
            ));
            note_activity();
        }
        return (total_size_kb, file_count);
    }

    let (_, removed_count) = safe_sudo_find_delete(dir, "*", 30, "f");

    if oplog_enabled() && removed_count > 0 {
        log_operation(
            "clean",
            "REMOVED",
            dir,
            Some(&format!(
                "{file_count} files, {}",
                bytes_to_human_kb(total_size_kb)
            )),
        );
    }
    if removed_count > 0 {
        note_activity();
    }
    (total_size_kb, removed_count)
}
