//! 与 `src/lib/check/health_json.sh` 中 `generate_health_json` 对齐的系统健康 JSON（GUI / CLI 共用）。
//!
//! 权威语义以当前 shell 实现为准；非 macOS 平台返回错误。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthJson {
    pub memory_used_gb: f64,
    pub memory_total_gb: f64,
    pub disk_used_gb: f64,
    pub disk_total_gb: f64,
    pub disk_used_percent: f64,
    pub uptime_days: f64,
    pub optimizations: Vec<HealthOptimizationItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthOptimizationItem {
    pub category: String,
    pub name: String,
    pub description: String,
    pub action: String,
    pub safe: bool,
}

pub fn generate_health_json_value() -> Result<Value, String> {
    let doc = collect_health_json()?;
    serde_json::to_value(&doc).map_err(|e| e.to_string())
}

pub fn collect_health_json() -> Result<HealthJson, String> {
    #[cfg(target_os = "macos")]
    {
        collect_health_json_macos()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("health_json 仅在 macOS 上可用（与 health_json.sh 一致）".to_string())
    }
}

#[cfg(target_os = "macos")]
fn collect_health_json_macos() -> Result<HealthJson, String> {
    let (mem_used, mem_total) = memory_info_gb()?;
    let home = crate::core::base::home_dir_opt().ok_or_else(|| "无法解析主目录".to_string())?;
    let (disk_used, disk_total, disk_pct) = disk_info_gb(&home)?;
    let uptime = uptime_days()?;
    Ok(HealthJson {
        memory_used_gb: mem_used,
        memory_total_gb: mem_total,
        disk_used_gb: disk_used,
        disk_total_gb: disk_total,
        disk_used_percent: disk_pct,
        uptime_days: uptime,
        optimizations: default_optimizations(),
    })
}

fn default_optimizations() -> Vec<HealthOptimizationItem> {
    const ROWS: &[(&str, &str, &str, bool)] = &[
        (
            "system_maintenance",
            "DNS & Spotlight Check",
            "Refresh DNS cache & verify Spotlight status",
            true,
        ),
        (
            "cache_refresh",
            "Finder Cache Refresh",
            "Refresh QuickLook thumbnails & icon services cache",
            true,
        ),
        (
            "saved_state_cleanup",
            "App State Cleanup",
            "Remove old saved application states (30+ days)",
            true,
        ),
        (
            "fix_broken_configs",
            "Broken Config Repair",
            "Fix corrupted preferences files",
            true,
        ),
        (
            "network_optimization",
            "Network Cache Refresh",
            "Optimize DNS cache & restart mDNSResponder",
            true,
        ),
        (
            "sqlite_vacuum",
            "Database Optimization",
            "Compress SQLite databases for Mail, Safari & Messages (skips if apps are running)",
            true,
        ),
        (
            "launch_services_rebuild",
            "LaunchServices Repair",
            "Repair \"Open with\" menu & file associations",
            true,
        ),
        (
            "prevent_network_dsstore",
            "Prevent Finder .DS_Store",
            "Set a persistent Finder preference to stop writing .DS_Store on SMB/AFP/NFS and USB volumes",
            true,
        ),
        (
            "legacy_overrides_audit",
            "Legacy Overrides",
            "Remove hidden App Nap and disk-image verification overrides left by old tweak tools",
            true,
        ),
        (
            "network_stack_optimize",
            "Network Stack Refresh",
            "Flush routing table and ARP cache to resolve network issues",
            true,
        ),
        (
            "disk_permissions_repair",
            "Permission Repair",
            "Fix user directory permission issues",
            true,
        ),
        (
            "spotlight_index_optimize",
            "Spotlight Optimization",
            "Rebuild index if search is slow (smart detection)",
            true,
        ),
        (
            "spotlight_orphan_rules_cleanup",
            "Spotlight Orphan Rules",
            "Remove Spotlight search-rule entries for apps that are no longer installed",
            true,
        ),
        (
            "periodic_maintenance",
            "Periodic Maintenance",
            "Run macOS daily/weekly/monthly maintenance scripts if stale",
            true,
        ),
        (
            "shared_file_list_repair",
            "Shared File Lists",
            "Repair corrupted Finder favorites and recent documents",
            true,
        ),
        (
            "disk_verify",
            "Disk Health",
            "Verify filesystem integrity",
            true,
        ),
        (
            "login_items_audit",
            "Login Items",
            "Audit login items for broken entries",
            true,
        ),
        (
            "quarantine_cleanup",
            "Quarantine Database Cleanup",
            "Clear Gatekeeper download tracking history",
            true,
        ),
        (
            "launch_agents_cleanup",
            "Launch Agents Cleanup",
            "Remove broken LaunchAgents whose binaries no longer exist",
            true,
        ),
        (
            "notification_cleanup",
            "Notifications",
            "Clean old delivered notifications to reduce database bloat",
            true,
        ),
        (
            "coreduet_cleanup",
            "Usage Data",
            "Clean old usage tracking data",
            true,
        ),
    ];
    ROWS.iter()
        .map(|(action, name, desc, safe)| HealthOptimizationItem {
            category: "system".to_string(),
            name: (*name).to_string(),
            description: (*desc).to_string(),
            action: (*action).to_string(),
            safe: *safe,
        })
        .collect()
}

fn run_cmd_stdout(cmd: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("执行 {cmd}: {e}"))?;
    if !output.status.success() {
        return Err(format!("{cmd} 退出码 {:?}", output.status.code()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "macos")]
fn run_cmd_stdout_lenient(cmd: &str, args: &[&str]) -> String {
    run_cmd_stdout(cmd, args).unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
#[cfg(target_os = "macos")]
fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

#[cfg(target_os = "macos")]
fn memory_info_gb() -> Result<(f64, f64), String> {
    let mem_s = run_cmd_stdout_lenient("sysctl", &["-n", "hw.memsize"]);
    let total_bytes: u64 = mem_s.trim().parse().unwrap_or(0);
    let total_gb = round2(total_bytes as f64 / 1024_f64.powi(3));
    let page_size: u64 = 4096;
    let vm_out = run_cmd_stdout_lenient("/usr/bin/vm_stat", &[]);
    let active = parse_vm_stat_pages(&vm_out, "Pages active:");
    let wired = parse_vm_stat_pages(&vm_out, "Pages wired down:");
    let compressed = parse_vm_stat_pages(&vm_out, "Pages occupied by compressor:");
    let used_bytes =
        (active.saturating_add(wired).saturating_add(compressed)).saturating_mul(page_size);
    let used_gb = round2(used_bytes as f64 / 1024_f64.powi(3));
    Ok((used_gb, total_gb))
}

#[cfg(target_os = "macos")]
fn parse_vm_stat_pages(text: &str, prefix: &str) -> u64 {
    for line in text.lines() {
        if line.contains(prefix) {
            if let Some(tok) = line.split_whitespace().last() {
                let digits: String = tok
                    .trim_end_matches('.')
                    .chars()
                    .filter(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(n) = digits.parse::<u64>() {
                    return n;
                }
            }
        }
    }
    0
}

#[cfg(target_os = "macos")]
fn disk_info_gb(home: &Path) -> Result<(f64, f64, f64), String> {
    let output = Command::new("/bin/df")
        .args(["-k"])
        .arg(home)
        .output()
        .map_err(|e| format!("df: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .last()
        .unwrap_or("");
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 {
        return Ok((0.0, 0.0, 0.0));
    }
    let mut total_kb: u64 = parts[1].parse().unwrap_or(0);
    let used_kb: u64 = parts[2].parse().unwrap_or(0);
    if total_kb == 0 {
        total_kb = 1;
    }
    let total_gb = round2(total_kb as f64 / 1024_f64.powi(2));
    let used_gb = round2(used_kb as f64 / 1024_f64.powi(2));
    let used_percent = round1((used_kb as f64 / total_kb as f64) * 100.0);
    Ok((used_gb, total_gb, used_percent))
}

#[cfg(target_os = "macos")]
fn uptime_days() -> Result<f64, String> {
    let boot_s = run_cmd_stdout_lenient("sysctl", &["-n", "kern.boottime"]);
    let Some(boot_sec) = parse_boot_sec(&boot_s) else {
        return Ok(0.0);
    };
    if boot_sec == 0 {
        return Ok(0.0);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let uptime_sec = now.saturating_sub(boot_sec);
    Ok(round1(uptime_sec as f64 / 86400.0))
}

#[cfg(target_os = "macos")]
fn parse_boot_sec(sysctl_out: &str) -> Option<u64> {
    let needle = "sec = ";
    let mut start = 0usize;
    while let Some(i) = sysctl_out[start..].find(needle) {
        let abs = start + i + needle.len();
        let rest = &sysctl_out[abs..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(v) = digits.parse::<u64>() {
            if v > 0 {
                return Some(v);
            }
        }
        start = abs;
    }
    None
}
