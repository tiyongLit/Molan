use super::metrics::DiskIOStatus;
use super::metrics_battery::ThermalStatus;
use super::metrics_cpu::CPUStatus;
use super::metrics_disk::DiskStatus;
use super::metrics_memory::MemoryStatus;

// 对齐 cmd/status/metrics_health.go 阈值
const CPU_NORMAL_THRESHOLD: f64 = 50.0;
const CPU_HIGH_THRESHOLD: f64 = 85.0;
const MEM_NORMAL_THRESHOLD: f64 = 70.0;
const MEM_HIGH_THRESHOLD: f64 = 88.0;
const DISK_WARN_THRESHOLD: f64 = 80.0;
const DISK_CRIT_THRESHOLD: f64 = 93.0;
const THERMAL_NORMAL_THRESHOLD: f64 = 65.0;
const THERMAL_HIGH_THRESHOLD: f64 = 85.0;
const IO_NORMAL_THRESHOLD: f64 = 50.0;
const IO_HIGH_THRESHOLD: f64 = 150.0;
pub fn calculate_health_score(
    cpu: &CPUStatus,
    mem: &MemoryStatus,
    disks: &[DiskStatus],
    disk_io: &DiskIOStatus,
    thermal: &ThermalStatus,
) -> (i32, String) {
    let mut score = 100.0;
    let mut issues: Vec<String> = Vec::new();

    if cpu.usage > CPU_NORMAL_THRESHOLD {
        let penalty = if cpu.usage > CPU_HIGH_THRESHOLD {
            30.0 * (cpu.usage - CPU_NORMAL_THRESHOLD) / CPU_HIGH_THRESHOLD
        } else {
            15.0 * (cpu.usage - CPU_NORMAL_THRESHOLD) / (CPU_HIGH_THRESHOLD - CPU_NORMAL_THRESHOLD)
        };
        score -= penalty;
    }
    if cpu.usage > CPU_HIGH_THRESHOLD {
        issues.push("High CPU".into());
    }

    if mem.used_percent > MEM_NORMAL_THRESHOLD {
        let penalty = if mem.used_percent > MEM_HIGH_THRESHOLD {
            25.0 * (mem.used_percent - MEM_NORMAL_THRESHOLD) / MEM_NORMAL_THRESHOLD
        } else {
            12.5 * (mem.used_percent - MEM_NORMAL_THRESHOLD)
                / (MEM_HIGH_THRESHOLD - MEM_NORMAL_THRESHOLD)
        };
        score -= penalty;
    }
    if mem.used_percent > MEM_HIGH_THRESHOLD {
        issues.push("High Memory".into());
    }

    match mem.pressure.as_str() {
        "warn" => {
            score -= 5.0;
            issues.push("Memory Pressure".into());
        }
        "critical" => {
            score -= 15.0;
            issues.push("Critical Memory".into());
        }
        _ => {}
    }

    if let Some(d) = disks.first() {
        if d.used_percent > DISK_WARN_THRESHOLD {
            let penalty = if d.used_percent > DISK_CRIT_THRESHOLD {
                20.0 * (d.used_percent - DISK_WARN_THRESHOLD) / (100.0 - DISK_WARN_THRESHOLD)
            } else {
                10.0 * (d.used_percent - DISK_WARN_THRESHOLD)
                    / (DISK_CRIT_THRESHOLD - DISK_WARN_THRESHOLD)
            };
            score -= penalty;
        }
        if d.used_percent > DISK_CRIT_THRESHOLD {
            issues.push("Disk Almost Full".into());
        }
    }

    if thermal.cpu_temp > 0.0 {
        if thermal.cpu_temp > THERMAL_NORMAL_THRESHOLD {
            let penalty = if thermal.cpu_temp > THERMAL_HIGH_THRESHOLD {
                15.0
            } else {
                15.0 * (thermal.cpu_temp - THERMAL_NORMAL_THRESHOLD)
                    / (THERMAL_HIGH_THRESHOLD - THERMAL_NORMAL_THRESHOLD)
            };
            score -= penalty;
        }
        if thermal.cpu_temp > THERMAL_HIGH_THRESHOLD {
            issues.push("Overheating".into());
        }
    }

    let total_io = disk_io.read_rate + disk_io.write_rate;
    if total_io > IO_NORMAL_THRESHOLD {
        let penalty = if total_io > IO_HIGH_THRESHOLD {
            10.0
        } else {
            10.0 * (total_io - IO_NORMAL_THRESHOLD) / (IO_HIGH_THRESHOLD - IO_NORMAL_THRESHOLD)
        };
        score -= penalty;
        if total_io > IO_HIGH_THRESHOLD {
            issues.push("Heavy Disk IO".into());
        }
    }

    let score = score.clamp(0.0, 100.0) as i32;
    // 对齐 Go scoreExcellentThreshold=85, scoreGoodThreshold=65, scoreFairThreshold=45
    let msg = if score >= 85 {
        "Excellent"
    } else if score >= 65 {
        "Good"
    } else if score >= 45 {
        "Fair"
    } else {
        "Poor"
    };

    let msg = if issues.is_empty() {
        msg.to_string()
    } else {
        format!("{}: {}", msg, issues.join(", "))
    };

    (score, msg)
}
