//! LaunchServices stale registration cleanup — 严格对齐 `Mole/lib/clean/launch_services.sh`

use std::path::Path;

use crate::core::base::{home_dir, is_dry_run, note_activity};
use crate::core::log::{debug_log, log_success};
use crate::core::timeout::run_with_timeout_capture_lossy;

/// SH `get_lsregister_path` — locate lsregister binary.
fn get_lsregister_path() -> Option<String> {
    let candidates = [
        "/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister",
        "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister",
    ];
    for c in &candidates {
        if Path::new(c).is_file() {
            return Some(c.to_string());
        }
    }
    // Fallback: find using system framework paths
    for c in candidates {
        if Path::new(c).exists() {
            return Some(c.to_string());
        }
    }
    None
}

/// SH `launch_services_extract_app_path_from_line`
fn launch_services_extract_app_path_from_line(line: &str) -> Option<String> {
    if !line.contains(".app") {
        return None;
    }
    // The path starts after the first '/' — SH: path="/${line#*/}"
    if let Some(idx) = line.find('/') {
        let path = &line[idx..];
        // SH: path="${path%%.app*}.app"
        if let Some(app_end) = path.find(".app") {
            let app_path = format!("{}.app", &path[..app_end + 4]);
            if app_path.starts_with('/') && app_path.ends_with(".app") {
                return Some(app_path.trim_end_matches('/').to_string());
            }
        }
    }
    None
}

/// SH `launch_services_stale_app_path_is_safe`
fn launch_services_stale_app_path_is_safe(path: &str) -> bool {
    if path.is_empty() || !path.starts_with('/') || !path.ends_with(".app") {
        return false;
    }
    if path.contains('\n') || path.contains('\r') {
        return false;
    }
    // SH case: */../* | */.. | ../* | /System/* | /Library/Apple/*
    if path.contains("/../")
        || path.ends_with("/..")
        || path.starts_with("../")
        || path.starts_with("/System/")
        || path.starts_with("/Library/Apple/")
    {
        return false;
    }
    // Must NOT exist on disk
    !Path::new(path).exists()
}

/// SH `collect_stale_launch_services_app_paths`
fn collect_stale_launch_services_app_paths(lsregister: &str) -> Vec<String> {
    let output = match run_with_timeout_capture_lossy(10.0, lsregister, &["-dump"]) {
        Some(o) => o,
        None => return Vec::new(),
    };

    // SH: Parse lsregister dump output, find "Bundle node not found on disk" blocks
    let mut record_paths: Vec<String> = Vec::new();
    let mut missing_record = false;
    let mut current_paths: Vec<String> = Vec::new();

    for line in output.lines() {
        if line.is_empty() {
            flush_launch_services_record(
                &mut missing_record,
                &mut current_paths,
                &mut record_paths,
            );
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("bundle ") {
            flush_launch_services_record(
                &mut missing_record,
                &mut current_paths,
                &mut record_paths,
            );
        }
        if line.contains("Bundle node not found on disk") {
            missing_record = true;
        }
        if let Some(app_path) = launch_services_extract_app_path_from_line(line) {
            current_paths.push(app_path);
        }
    }
    flush_launch_services_record(&mut missing_record, &mut current_paths, &mut record_paths);

    // Dedup and sort
    let mut paths = Vec::new();
    for p in &record_paths {
        if launch_services_stale_app_path_is_safe(p) && !paths.contains(p) {
            paths.push(p.to_string());
        }
    }
    paths
}

fn flush_launch_services_record(
    missing: &mut bool,
    current: &mut Vec<String>,
    record: &mut Vec<String>,
) {
    if *missing && !current.is_empty() {
        for p in current.drain(..) {
            record.push(p);
        }
    } else {
        current.clear();
    }
    *missing = false;
}

/// SH `clean_stale_launch_services_registrations` — aligned with launch_services.sh:88-153
pub fn clean_stale_launch_services_registrations() -> (u64, u64) {
    let lsregister = match get_lsregister_path() {
        Some(p) => p,
        None => {
            debug_log("[launch_services] lsregister not found");
            return (0, 0);
        }
    };

    let stale_apps = collect_stale_launch_services_app_paths(&lsregister);
    if stale_apps.is_empty() {
        return (0, 0);
    }

    let max_items: usize = std::env::var("MOLE_LAUNCH_SERVICES_STALE_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    let max_items = max_items.max(1);

    let mut limited: Vec<&String> = Vec::new();
    for app_path in &stale_apps {
        limited.push(app_path);
        if limited.len() >= max_items {
            break;
        }
    }

    note_activity();

    let count = limited.len();
    let count_label = if stale_apps.len() > count {
        format!("{count}+")
    } else {
        count.to_string()
    };

    if is_dry_run() {
        let h = home_dir();
        let example = limited
            .first()
            .map(|p| p.replacen(&h, "~", 1))
            .unwrap_or_default();
        log::info!(
            "LaunchServices stale app registrations · would unregister {count_label} (example: {example})"
        );
        note_activity();
        return (0, 0);
    }

    let mut success_count: u64 = 0;
    let mut failed_count: u64 = 0;
    for app_path in &limited {
        debug_log(&format!(
            "[launch_services] Unregistering stale app: {app_path}"
        ));
        let result = std::process::Command::new(&lsregister)
            .args(["-u", app_path])
            .output();
        match result {
            Ok(o) if o.status.success() => {
                success_count += 1;
            }
            _ => {
                failed_count += 1;
                debug_log(&format!(
                    "[launch_services] Failed to unregister: {app_path}"
                ));
            }
        }
    }

    if success_count > 0 {
        log_success(&format!(
            "LaunchServices stale app registrations, {success_count} removed"
        ));
    }
    if failed_count > 0 {
        log::info!("LaunchServices stale app registrations, {failed_count} failed");
    }

    (0, success_count)
}
