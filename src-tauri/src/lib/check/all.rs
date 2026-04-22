//! 与 `lib/check/all.sh` 对齐的聚合检查模块。
//! 提供 `generate_check_report_json_value()` 及各个子检查。

use crate::check::configuration;
use crate::check::dev_environment;
use crate::check::health_json;
use crate::check::security;
use crate::check::system_health;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy)]
pub struct CheckReportOptions {
    pub dev_environment: dev_environment::DevEnvironmentOptions,
    pub system_health: system_health::SystemHealthOptions,
}

impl Default for CheckReportOptions {
    fn default() -> Self {
        Self {
            dev_environment: dev_environment::DevEnvironmentOptions::default(),
            system_health: system_health::SystemHealthOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckReport {
    pub schema_version: u32,
    pub health: Value,
    pub system_health: Value,
    pub configuration: Value,
    pub security: Value,
    pub dev: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mole_update: Option<MoleUpdateStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MoleUpdateStatus {
    Unknown {
        current_version: String,
    },
    UpToDate {
        current_version: String,
    },
    UpdateAvailable {
        current_version: String,
        latest_version: String,
    },
    CheckFailed {
        reason: String,
    },
}

pub fn generate_check_report_json_value() -> Result<Value, String> {
    generate_check_report_json_value_with_options(CheckReportOptions::default())
}

pub fn generate_check_report_json_value_with_options(
    opt: CheckReportOptions,
) -> Result<Value, String> {
    let r = collect_check_report_with_options(opt)?;
    serde_json::to_value(&r).map_err(|e| e.to_string())
}

pub fn collect_check_report() -> Result<CheckReport, String> {
    collect_check_report_with_options(CheckReportOptions::default())
}

pub fn collect_check_report_with_options(opt: CheckReportOptions) -> Result<CheckReport, String> {
    Ok(CheckReport {
        schema_version: 2,
        health: health_json::generate_health_json_value()?,
        system_health: serde_json::to_value(
            &system_health::collect_system_health_report_with_options(opt.system_health)?,
        )
        .map_err(|e| e.to_string())?,
        configuration: configuration::generate_configuration_json_value(),
        security: security::generate_security_json_value(),
        dev: serde_json::to_value(
            &dev_environment::collect_dev_environment_report_with_options(opt.dev_environment)?,
        )
        .map_err(|e| e.to_string())?,
        mole_update: check_mole_update(),
    })
}

pub fn check_mole_update() -> Option<MoleUpdateStatus> {
    let home = std::env::var("HOME").unwrap_or_else(|_| {
        dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default()
    });

    // 对齐 SH 第 484 行:进入函数即先 export `MOLE_UPDATE_AVAILABLE=false`,
    // 后续命中 update available 才覆盖为 true。
    std::env::set_var("MOLE_UPDATE_AVAILABLE", "false");

    if crate::whitelist_optimize::is_whitelisted_optimize(
        "check_mole_update",
        &crate::whitelist_optimize::load_optimize_whitelist_patterns(&std::path::PathBuf::from(
            &home,
        )),
        &std::path::PathBuf::from(&home),
    ) {
        return None;
    }

    let current_version = detect_mole_current_version();
    let cache_file = format!("{home}/.cache/mole/mole_version");
    let cache_ttl: u64 = 600;

    let latest_version = if is_cache_valid(&cache_file, cache_ttl) {
        std::fs::read_to_string(&cache_file)
            .ok()
            .map(|s| s.trim().to_string())
    } else {
        fetch_mole_latest_version_from_github().and_then(|v| {
            if let Some(parent) = Path::new(&cache_file).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&cache_file, &v);
            Some(v)
        })
    };

    let current_norm = current_version
        .trim_start_matches('v')
        .trim_start_matches('V')
        .to_string();
    let latest_norm = latest_version.as_deref().map(|v| {
        v.trim_start_matches('v')
            .trim_start_matches('V')
            .to_string()
    });

    match latest_norm {
        Some(ref latest) if current_norm != *latest => {
            let is_newer = compare_semver(&current_norm, latest).unwrap_or(false);
            if is_newer {
                std::env::set_var("MOLE_UPDATE_AVAILABLE", "true");
                Some(MoleUpdateStatus::UpdateAvailable {
                    current_version: current_norm,
                    latest_version: latest.clone(),
                })
            } else {
                Some(MoleUpdateStatus::UpToDate {
                    current_version: current_norm,
                })
            }
        }
        Some(_) => Some(MoleUpdateStatus::UpToDate {
            current_version: current_norm,
        }),
        None => Some(MoleUpdateStatus::Unknown {
            current_version: current_norm,
        }),
    }
}

fn detect_mole_current_version() -> String {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION
        .get_or_init(|| {
            let candidates = [
                "/usr/local/bin/mo",
                "/opt/homebrew/bin/mo",
                "/usr/local/bin/mole",
            ];
            for bin in &candidates {
                if let Ok(content) = std::fs::read_to_string(bin) {
                    for line in content.lines() {
                        if let Some(v) = line.strip_prefix("VERSION=") {
                            let v = v.trim().trim_matches('"').trim_matches('\'');
                            if !v.is_empty() {
                                return v.to_string();
                            }
                        }
                    }
                }
            }
            std::env::var("MOLE_VERSION").unwrap_or_else(|_| "unknown".to_string())
        })
        .clone()
}

fn is_cache_valid(cache_file: &str, ttl_seconds: u64) -> bool {
    let p = Path::new(cache_file);
    if !p.exists() {
        return false;
    }
    let mtime = crate::core::base::get_file_mtime(cache_file);
    let now = crate::core::base::get_epoch_seconds();
    let age = now.saturating_sub(mtime);
    age < ttl_seconds
}

fn fetch_mole_latest_version_from_github() -> Option<String> {
    let output = Command::new("curl")
        .args([
            "-fsSL",
            "--connect-timeout",
            "3",
            "--max-time",
            "5",
            "https://api.github.com/repos/tw93/mole/releases/latest",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let body = String::from_utf8_lossy(&output.stdout);
    for line in body.lines() {
        if line.contains("\"tag_name\"") {
            let v = line
                .split('"')
                .nth(3)
                .unwrap_or("")
                .trim_start_matches('v')
                .trim_start_matches('V');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn compare_semver(current: &str, latest: &str) -> Option<bool> {
    let parse = |s: &str| -> Option<Vec<u32>> {
        s.split('.')
            .map(|p| p.parse::<u32>().ok())
            .collect::<Option<Vec<_>>>()
    };
    let cv = parse(current)?;
    let lv = parse(latest)?;
    let len = cv.len().max(lv.len());
    for i in 0..len {
        let a = cv.get(i).copied().unwrap_or(0);
        let b = lv.get(i).copied().unwrap_or(0);
        if a != b {
            return Some(a < b);
        }
    }
    Some(false)
}
