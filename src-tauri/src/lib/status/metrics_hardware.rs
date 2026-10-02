use serde::Serialize;
use std::process::Command;

use super::metrics_disk::DiskStatus;

#[derive(Debug, Clone, Serialize)]
pub struct HardwareInfo {
    pub model: String,
    pub cpu_model: String,
    pub total_ram: String,
    pub disk_size: String,
    pub os_version: String,
    pub refresh_rate: String,
}

pub fn collect_hardware(total_ram: u64, disks: &[DiskStatus]) -> HardwareInfo {
    let (model, cpu_model) = read_sp_hardware();
    // 复用 metrics::os_version()（sysctlbyname + OnceLock），
    // 不再单独 fork 一次 `sw_vers`。
    let os_version = format!("macOS {}", super::metrics::os_version());
    let refresh_rate = read_refresh_rate();

    let disk_size = if let Some(d) = disks.first() {
        human_bytes(d.total)
    } else {
        "Unknown".into()
    };

    HardwareInfo {
        model,
        cpu_model,
        total_ram: human_bytes(total_ram),
        disk_size,
        os_version,
        refresh_rate,
    }
}

fn read_sp_hardware() -> (String, String) {
    let out = match Command::new("system_profiler")
        .args(["SPHardwareDataType"])
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return (String::new(), String::new()),
    };

    let mut model = String::new();
    let mut cpu_model = String::new();

    for line in out.lines() {
        let lower = line.trim().to_lowercase();
        if lower.starts_with("model name:") {
            if let Some(after) = line.split(':').nth(1) {
                model = after.trim().to_string();
            }
        }
        if lower.starts_with("chip:") {
            if let Some(after) = line.split(':').nth(1) {
                cpu_model = after.trim().to_string();
            }
        }
        if lower.starts_with("processor name:") && cpu_model.is_empty() {
            if let Some(after) = line.split(':').nth(1) {
                cpu_model = after.trim().to_string();
            }
        }
    }

    (model, cpu_model)
}

fn read_refresh_rate() -> String {
    let out = match Command::new("system_profiler")
        .args(["-detailLevel", "mini", "SPDisplaysDataType"])
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_lowercase(),
        Err(_) => return String::new(),
    };

    let mut max_hz = 0;
    for line in out.lines() {
        if !line.contains("hz") {
            continue;
        }
        let line = line.trim();
        let words: Vec<&str> = line.split_whitespace().collect();
        for word in words {
            if let Some(hz_str) = word.strip_suffix("hz") {
                let num: String = hz_str
                    .chars()
                    .filter(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                if let Ok(hz) = num.parse::<f64>() {
                    let h = hz as i32;
                    if h > max_hz && h < 500 {
                        max_hz = h;
                    }
                }
            }
        }
    }

    if max_hz > 0 {
        format!("{}Hz", max_hz)
    } else {
        String::new()
    }
}

fn human_bytes(bytes: u64) -> String {
    const GB: f64 = 1_073_741_824.0;
    const MB: f64 = 1_048_576.0;
    if bytes as f64 >= GB {
        format!("{:.1} GB", bytes as f64 / GB)
    } else if bytes as f64 >= MB {
        format!("{:.1} MB", bytes as f64 / MB)
    } else {
        format!("{}B", bytes)
    }
}
