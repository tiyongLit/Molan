// 对齐 cmd/status/diagnosis.go: statusDiagnosisLine()
// 根据系统指标输出诊断摘要，用于 Dashboard 顶部状态行。

use super::metrics::MetricsSnapshot;

// 阈值对齐 metrics_health.go
const CPU_HIGH: f64 = 85.0;
const MEM_HIGH: f64 = 88.0;
const DISK_CRIT: f64 = 93.0;
const THERMAL_NORMAL: f64 = 65.0;
const IO_HIGH: f64 = 150.0;

/// 从快照生成诊断摘要行。
/// 优先级：CPU > Memory > Disk > Thermal > IO > HealthScore > All clear
pub fn diagnosis_line(snap: &MetricsSnapshot) -> String {
    // CPU
    if snap.cpu.usage > CPU_HIGH {
        if let Some(proc) = leading_cpu_process(&snap.top_processes, 50.0) {
            return format!("{} high CPU", shorten(&proc.name, 18));
        }
        return "CPU load high".into();
    }

    // Memory
    let pressure = snap.memory.pressure.as_str();
    if pressure == "warn" || pressure == "critical" || snap.memory.used_percent > MEM_HIGH {
        if let Some(proc) = leading_memory_process(&snap.top_processes) {
            if proc.memory > 0.0 {
                return format!("{} memory pressure", shorten(&proc.name, 18));
            }
        }
        return "Memory pressure high".into();
    }

    // Disk
    if let Some(disk) = root_disk(&snap.disks) {
        if disk.used_percent > DISK_CRIT {
            let free = disk.total.saturating_sub(disk.used);
            return format!("Disk low, {} free", human_bytes_short(free));
        }
    }

    // Thermal
    if snap.thermal.cpu_temp > THERMAL_NORMAL {
        return "CPU temperature high".into();
    }

    // Disk IO
    if snap.disk_io.read_rate + snap.disk_io.write_rate > IO_HIGH {
        return "Disk I/O busy".into();
    }

    // Health score issues
    if snap.health_score_msg.contains(':') {
        return snap.health_score_msg.clone();
    }

    "All clear".into()
}

fn leading_cpu_process(
    procs: &[super::metrics_process::ProcessInfo],
    threshold: f64,
) -> Option<&super::metrics_process::ProcessInfo> {
    let best = procs.iter().max_by(|a, b| {
        a.cpu
            .partial_cmp(&b.cpu)
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    if best.cpu < threshold {
        None
    } else {
        Some(best)
    }
}

fn leading_memory_process(
    procs: &[super::metrics_process::ProcessInfo],
) -> Option<&super::metrics_process::ProcessInfo> {
    procs.iter().max_by(|a, b| {
        a.memory
            .partial_cmp(&b.memory)
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

fn root_disk(
    disks: &[super::metrics_disk::DiskStatus],
) -> Option<&super::metrics_disk::DiskStatus> {
    disks
        .iter()
        .find(|d| d.mount == "/")
        .or_else(|| disks.first())
}

fn shorten(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len - 1])
    }
}

fn human_bytes_short(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1000.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} B", bytes)
    } else {
        format!("{:.1} {}", size, UNITS[unit_idx])
    }
}
