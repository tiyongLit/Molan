//! Clean 域数据模型 — 扫描/清理结果契约类型。
//! 自 `controllers/clean.rs` 纯搬迁（行为不变）；serde 派生决定前端 JSON 形状。

use serde::Serialize;

#[derive(Serialize)]
pub struct CleanOutput {
    pub mode: String,
    pub collected_at: String,
    /// 扫描唯一标识。前端 apply 时须回传此 ID，后端据此从 SCAN_REGISTRY 取快照验证。
    /// 仅 dry_run（扫描）模式返回；execute 模式为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scan_id: Option<String>,
    /// 本次扫描/清理是否被用户取消。前端据此区分"正常完成"与"已取消"。
    pub cancelled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whitelist: Option<WhitelistInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<CleanCategory>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<CleanResult>>,
    pub summary: CleanSummary,
}

#[derive(Serialize)]
pub struct WhitelistInfo {
    /// 对齐 `perform_cleanup`：`WHITELIST_PATTERNS` 总数。
    pub active_patterns: usize,
    /// 对齐 `DEFAULT_WHITELIST_PATTERNS` 命中条数（Shell 文案里的 core）。
    pub core_pattern_count: usize,
    /// 非预设条目数（Shell 文案里的 custom）。
    pub custom_pattern_count: usize,
    /// 对齐 Shell：仅在 dry-run 下列出；执行清理时不序列化空数组。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<String>,
    /// 对齐 clean.sh:1098-1103：白名单加载时的验证警告。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
pub struct CleanCategory {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tips: Option<String>,
    pub recommend: bool,
    pub cautious: bool,
    pub requires_sudo: bool,
    pub whitelist_matched: bool,
    pub items: Vec<CleanItem>,
}

#[derive(Serialize)]
pub struct CleanItem {
    pub id: String,
    pub path: String,
    pub size: u64,
    pub size_human: String,
    pub file_count: u64,
    pub status: String,
    /// 该子项是否命中白名单（路径匹配 / item: 前缀）。
    /// 前端据此禁用勾选，实现二级勾选控制。
    pub whitelist_matched: bool,
    /// 后端权威计算的默认勾选状态。
    /// 规则：status==cleanable && size>0 && !whitelist_matched && cat.recommend && !cautious
    /// 前端据此初始化 selectedItemIds，用户可自由修改。
    pub default_selected: bool,
    /// 真实文件系统路径；Some(path) 时前端可渲染「在 Finder 中显示」按钮。
    /// 对齐 Lemon：只有背后对应真实、可定位路径的项才显示打开按钮。
    /// 聚合型 / 无单一路径的项为 None，前端不显示按钮。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub real_path: Option<String>,
}

#[derive(Serialize)]
pub struct CleanResult {
    pub category_id: String,
    pub item_id: String,
    pub path: String,
    pub size_cleaned: u64,
    pub size_cleaned_human: String,
    pub file_count: u64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct CleanSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cleanable_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cleanable_size_human: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cleaned_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cleaned_size_human: Option<String>,
    pub total_file_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_free_space: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_free_space_human: Option<String>,
    /// 对齐 clean.sh `emit_free_space_summary` 的「Free space change」。
    /// 仅在 execute 模式且有初始值时可算。dry-run 时不输出。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_space_change: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_space_change_human: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub movie_equivalent: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CleanStatusInfo {
    /// 执行闸门状态（由 `ensure_execution_allowed` 派生，单一事实来源）。
    pub execution_allowed: bool,
    /// 执行被阻断时的原因码；放行（正常流程）时不序列化该字段。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_blocked_reason: Option<&'static str>,
    /// 当前是否拥有管理员会话（影响 system_caches / apple_silicon 等需 sudo 的分类）。
    pub sudo_session_active: bool,
    /// 最近一次扫描完成时间（ISO8601）；进程重启后为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_scan_at: Option<String>,
}
