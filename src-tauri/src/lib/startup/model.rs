//! 启动项管理数据模型。
//! 对齐 Launchdeck `model.rs`：Service / Status / Provenance / Safety / Elevation / LaunchConfig。
//! 增加 Lemon 式 AppAssociation 与 AppGroup 分组输出。

use serde::Serialize;

// ── 服务来源 ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceSource {
    Launchd,
    Homebrew,
    Both,
}

// ── 作用域 ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceScope {
    UserAgent,
    GlobalAgent,
    SystemDaemon,
}

impl ServiceScope {
    pub fn domain(&self, uid: u32) -> String {
        match self {
            Self::UserAgent | Self::GlobalAgent => format!("gui/{uid}"),
            Self::SystemDaemon => "system".to_string(),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::UserAgent => "user agent",
            Self::GlobalAgent => "global agent",
            Self::SystemDaemon => "system daemon",
        }
    }
}

// ── 7 种运行状态 ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    Running,
    Scheduled,
    Stopped,
    Failed,
    Unloaded,
    Disabled,
    Unknown,
}

// ── 安全分级 ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyLevel {
    UserWritable,
    AdminRequired,
    ReadonlySystem,
    ProtectedVendor,
}

// ── 来源追溯 ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    Homebrew,
    UserPlist,
    VendorApp,
    System,
    RuntimeOnly,
    Unknown,
}

impl Provenance {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Homebrew => "homebrew",
            Self::UserPlist => "user-plist",
            Self::VendorApp => "vendor-app",
            Self::System => "system",
            Self::RuntimeOnly => "runtime-only",
            Self::Unknown => "unknown",
        }
    }

    /// 如何永久修改此服务（而非只是临时停止）。
    pub fn change_hint(&self) -> &'static str {
        match self {
            Self::Homebrew => "通过 `brew services` 管理",
            Self::UserPlist => "编辑 plist 或使用 launchctl",
            Self::VendorApp => "在所属应用的设置中修改",
            Self::System => "Apple 管理，仅可查看",
            Self::RuntimeOnly => "磁盘上无 plist，修改前请先检查",
            Self::Unknown => "来源未知，修改前请先检查",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Guess,
}

#[derive(Debug, Clone, Serialize)]
pub struct Origin {
    pub kind: Provenance,
    pub confidence: Confidence,
    /// 分类依据（可审计）。
    pub evidence: Vec<String>,
}

impl Origin {
    pub fn unknown() -> Self {
        Self {
            kind: Provenance::Unknown,
            confidence: Confidence::Guess,
            evidence: Vec::new(),
        }
    }
}

// ── 权限需求（3 轴）──

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ElevationNeeds {
    /// launchctl 操作是否需要 root。
    pub runtime: bool,
    /// 编辑 plist 是否需要 root。
    pub plist_write: bool,
    /// 删除 plist 是否需要 root。
    pub plist_remove: bool,
}

impl ElevationNeeds {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn needs_any(&self) -> bool {
        self.runtime || self.plist_write || self.plist_remove
    }
}

// ── plist 配置解析 ──

#[derive(Debug, Clone, Serialize)]
pub struct CalendarSchedule {
    pub minute: Option<u64>,
    pub hour: Option<u64>,
    pub day: Option<u64>,
    pub weekday: Option<u64>,
    pub month: Option<u64>,
}

impl CalendarSchedule {
    pub fn describe(&self) -> String {
        if self.month.is_none() && self.day.is_none() && self.weekday.is_none() {
            return match (self.hour, self.minute) {
                (Some(h), Some(m)) => format!("{h:02}:{m:02}"),
                (Some(h), None) => format!("{h:02}:*"),
                (None, Some(m)) => format!("*:{m:02}"),
                (None, None) => "calendar".to_string(),
            };
        }
        let mut parts = Vec::new();
        if let Some(mo) = self.month {
            parts.push(format!("mo{mo}"));
        }
        if let Some(d) = self.day {
            parts.push(format!("d{d}"));
        }
        if let Some(wd) = self.weekday {
            parts.push(weekday_name(wd).to_string());
        }
        match (self.hour, self.minute) {
            (Some(h), Some(m)) => parts.push(format!("{h:02}:{m:02}")),
            (Some(h), None) => parts.push(format!("{h:02}:*")),
            (None, Some(m)) => parts.push(format!("*:{m:02}")),
            (None, None) => {}
        }
        if parts.is_empty() {
            "calendar".to_string()
        } else {
            parts.join(" ")
        }
    }
}

fn weekday_name(wd: u64) -> &'static str {
    match wd {
        0 | 7 => "Sun",
        1 => "Mon",
        2 => "Tue",
        3 => "Wed",
        4 => "Thu",
        5 => "Fri",
        6 => "Sat",
        _ => "?",
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LaunchConfig {
    pub program: Option<String>,
    pub arguments: Vec<String>,
    pub working_directory: Option<String>,
    pub stdout_path: Option<String>,
    pub stderr_path: Option<String>,
    pub run_at_load: Option<bool>,
    pub keep_alive: Option<String>,
    pub start_interval: Option<u64>,
    pub start_calendar_intervals: Vec<CalendarSchedule>,
}

impl LaunchConfig {
    pub fn empty() -> Self {
        Self {
            program: None,
            arguments: Vec::new(),
            working_directory: None,
            stdout_path: None,
            stderr_path: None,
            run_at_load: None,
            keep_alive: None,
            start_interval: None,
            start_calendar_intervals: Vec::new(),
        }
    }

    pub fn command_preview(&self) -> String {
        if !self.arguments.is_empty() {
            self.arguments.join(" ")
        } else {
            self.program.clone().unwrap_or_else(|| "-".to_string())
        }
    }

    pub fn schedule_summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(interval) = self.start_interval {
            parts.push(format_interval(interval));
        }
        parts.extend(self.start_calendar_intervals.iter().map(|c| c.describe()));
        if parts.is_empty() {
            "-".to_string()
        } else {
            parts.join(", ")
        }
    }

    pub fn has_schedule(&self) -> bool {
        self.start_interval.is_some() || !self.start_calendar_intervals.is_empty()
    }
}

pub fn format_interval(seconds: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    if seconds >= DAY && seconds % DAY == 0 {
        format!("{}d", seconds / DAY)
    } else if seconds >= HOUR && seconds % HOUR == 0 {
        format!("{}h", seconds / HOUR)
    } else if seconds >= MINUTE && seconds % MINUTE == 0 {
        format!("{}min", seconds / MINUTE)
    } else {
        format!("{seconds}s")
    }
}

// ── App 关联（Lemon 核心）──

#[derive(Debug, Clone, Serialize)]
pub struct AppAssociation {
    pub app_name: String,
    pub app_path: String,
    pub bundle_id: String,
}

// ── 主实体 ──

#[derive(Debug, Clone, Serialize)]
pub struct Service {
    /// 唯一标识："gui/501:com.docker.docker"
    pub id: String,
    pub label: String,
    pub display_name: String,
    pub source: ServiceSource,
    pub scope: ServiceScope,
    pub domain: String,
    pub plist_path: Option<String>,
    pub config: LaunchConfig,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub status: ServiceStatus,
    pub enabled: Option<bool>,
    pub loaded: Option<bool>,
    pub brew_formula: Option<String>,
    pub brew_status: Option<String>,
    pub safety_level: SafetyLevel,
    pub elevation: ElevationNeeds,
    pub origin: Origin,
    pub app_info: Option<AppAssociation>,
    pub health: Vec<String>,
}

impl Service {
    /// 是否可被用户操作（非只读、非保护）。
    pub fn is_actionable(&self) -> bool {
        !matches!(
            self.safety_level,
            SafetyLevel::ReadonlySystem | SafetyLevel::ProtectedVendor
        )
    }

    /// 搜索用文本（label + display_name + brew_formula + origin + health）。
    pub fn searchable_text(&self) -> String {
        format!(
            "{} {} {} {} {} {}",
            self.label,
            self.display_name,
            self.brew_formula.as_deref().unwrap_or(""),
            self.origin.kind.label(),
            self.scope.label(),
            self.health.join(" ")
        )
        .to_lowercase()
    }
}

// ── 分组输出（前端消费）──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnableStatus {
    AllEnabled,
    SomeEnabled,
    AllDisabled,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppGroup {
    pub app_name: String,
    pub app_path: String,
    pub bundle_id: String,
    pub services: Vec<Service>,
    pub enable_status: EnableStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct StartupInventory {
    /// 能关联到 App 的分组。
    pub app_groups: Vec<AppGroup>,
    /// 关联不到的散装服务。
    pub standalone_services: Vec<Service>,
    /// 扫描过程中的警告。
    pub warnings: Vec<String>,
}

// ── 操作结果 ──

#[derive(Debug, Clone, Serialize)]
pub struct ActionResult {
    pub success: bool,
    pub message: String,
}
