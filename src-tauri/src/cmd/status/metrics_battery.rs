use serde::Serialize;

#[derive(Debug, Clone, Serialize, Default)]
pub struct ThermalStatus {
    pub cpu_temp: f64,
    pub fan_speed: i32,
    pub fan_count: i32,
}

/// 风扇与 CPU die 温度：SMC 只读客户端（免 root，Burrow SMC.swift 同款）。
/// SMC 不可用时（虚拟机等）风扇/温度保持 0，前端诚实显示「无数据」。
pub fn collect_thermal() -> ThermalStatus {
    let mut thermal = ThermalStatus {
        cpu_temp: 0.0,
        fan_speed: 0,
        fan_count: 0,
    };

    #[cfg(target_os = "macos")]
    {
        let (count, rpm) = crate::platform::macos_smc::fans();
        thermal.fan_count = count;
        // Burrow 同款：多风扇取最高 RPM（SnapshotPatcher: f.rpm.max()）
        thermal.fan_speed = rpm.iter().max().copied().unwrap_or(0);
        let cpu_t = crate::platform::macos_smc::temps();
        thermal.cpu_temp = cpu_t.unwrap_or(0.0);
    }

    thermal
}
