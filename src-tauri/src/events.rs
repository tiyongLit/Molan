//! 与前端约定的事件名、`payload` 类型及 `emit` 助手。
//! 业务核心层不直接 `emit`；由 [`crate::core::base::set_spinner_app_handle`] / [`SpinnerAppGuard`] 注入 `AppHandle` 后，[`crate::core::base::start_section_spinner`] 等会经此推送。
//!
//! **与 shell 一致**：常出现「先 `stop` 再 `start`」；前端应在收到 `is_active: false` 时结束当前 loading，
//! 再收到 `is_active: true` 时展示新 `message`（同一 `section` 下多阶段仅文案切换）。

use tauri::{AppHandle, Emitter};

/// 清理流程中的小节进度（spinner/阶段文案）。
/// 前端监听此事件名；payload 与 `SpinnerUpdatePayload` 字段一致（camelCase）。
pub const EVT_CLEANUP_SPINNER_UPDATE: &str = "cleanup::spinner-update";
/// 清理阶段结果（是否真的有清理动作），用于 GUI 列表/提示而非 spinner。
pub const EVT_CLEANUP_PHASE_RESULT: &str = "cleanup::phase-result";
/// 清理线索通知（扫描发现，非清理统计）。
/// Payload 携带结构化线索列表，前端可展示"系统数据线索"、"大文件候选"等面板。
pub const EVT_CLEANUP_HINTS_RESULT: &str = "cleanup::hints-result";

/// 磁盘分析：实时扫描进度（与前端 `EVT_ANALYZE_SCAN_PROGRESS` 一致，前端已定义此常量）。
pub const EVT_ANALYZE_SCAN_PROGRESS: &str = "analyze::scan-progress";

/// 磁盘分析：移到废纸篓的实时进度。
pub const EVT_ANALYZE_TRASH_PROGRESS: &str = "analyze::trash-progress";

/// Clean 执行（移废纸篓）阶段进度：当前正在清理的分类、累计已清大小、失败数。
/// 前端监听此事件名，payload 与 `CleanApplyProgressPayload` 字段一致（camelCase）。
pub const EVT_CLEAN_APPLY_PROGRESS: &str = "clean::apply-progress";

/// 卸载进度：当前正在处理的 app、累计进度、当前操作项。
/// 前端监听此事件名，payload 与 `UninstallProgressPayload` 字段一致（camelCase）。
pub const EVT_UNINSTALL_PROGRESS: &str = "uninstall::progress";

/// 卸载完成：单个 app 清理完成的结果。
pub const EVT_UNINSTALL_COMPLETE: &str = "uninstall::complete";

/// System 模块：系统缓存阶段（/Library/Caches 等）。
pub const PHASE_SYSTEM_CACHES: &str = "system.caches";
/// System 模块：系统临时文件阶段（/private/tmp、/private/var/tmp）。
pub const PHASE_SYSTEM_TEMP_FILES: &str = "system.temp_files";
/// System 模块：系统崩溃报告阶段（DiagnosticReports）。
pub const PHASE_SYSTEM_CRASH_REPORTS: &str = "system.crash_reports";
/// System 模块：系统日志阶段（/private/var/log）。
pub const PHASE_SYSTEM_LOGS: &str = "system.logs";
/// System 模块：第三方系统日志阶段（如 Adobe / CreativeCloud）。
pub const PHASE_SYSTEM_THIRD_PARTY_LOGS: &str = "system.third_party_logs";
/// System 模块：系统更新残留阶段（/Library/Updates）。
pub const PHASE_SYSTEM_LIBRARY_UPDATES: &str = "system.library_updates";
/// System 模块：macOS 安装器与 Install Data 阶段。
pub const PHASE_SYSTEM_MACOS_INSTALLERS: &str = "system.macos_installers";
/// System 模块：浏览器/系统 code_sign_clone 缓存阶段。
pub const PHASE_SYSTEM_BROWSER_CODE_SIGN_CACHES: &str = "system.browser_code_sign_caches";
/// System 模块：可重建系统服务缓存阶段（iconservices 等）。
pub const PHASE_SYSTEM_REBUILDABLE_SERVICE_CACHES: &str = "system.rebuildable_service_caches";
/// System 模块：可重建 GPU 缓存阶段（metal/gpuarchiver）。
pub const PHASE_SYSTEM_REBUILDABLE_GPU_CACHES: &str = "system.rebuildable_gpu_caches";
/// System 模块：系统诊断日志阶段（diagnostics/DiagnosticPipeline）。
pub const PHASE_SYSTEM_DIAGNOSTIC_LOGS: &str = "system.diagnostic_logs";
/// System 模块：功耗日志阶段（powerlog）。
pub const PHASE_SYSTEM_POWER_LOGS: &str = "system.power_logs";
/// System 模块：内存异常报告阶段（MemoryLimitViolations）。
pub const PHASE_SYSTEM_MEMORY_EXCEPTION_REPORTS: &str = "system.memory_exception_reports";

/// User 模块：用户基础文件清理阶段（~/Library/Caches、~/Library/Logs、~/.Trash）。
pub const PHASE_USER_ESSENTIALS: &str = "user.essentials";
/// User 模块：Finder 元数据阶段（.DS_Store）。
pub const PHASE_FINDER_METADATA: &str = "user.finder_metadata";
/// App 模块：应用缓存清理阶段。
pub const PHASE_APP_CACHES: &str = "app.caches";
/// Browser 模块：浏览器缓存清理阶段。
pub const PHASE_BROWSERS: &str = "browser.caches";
/// Cloud 模块：云存储缓存清理阶段。
pub const PHASE_CLOUD_STORAGE: &str = "cloud.storage";
/// Office 模块：办公软件缓存清理阶段。
pub const PHASE_OFFICE_CACHES: &str = "office.caches";
/// Dev 模块：开发者工具缓存清理阶段。
pub const PHASE_DEV_TOOLS: &str = "dev.tools";
/// Applications 模块：GUI 应用缓存清理阶段。
pub const PHASE_APPLICATIONS: &str = "applications";
/// Virtualization 模块：虚拟化工具缓存清理阶段。
pub const PHASE_VIRTUALIZATION: &str = "virtualization";
/// Device 模块：设备固件/iOS 备份清理阶段。
pub const PHASE_DEVICE_FIRMWARE: &str = "device.firmware";
/// Maven 模块：Maven 仓库清理阶段。
pub const PHASE_MAVEN: &str = "maven.repository";
/// Apps 模块：孤立应用数据清理阶段。
pub const PHASE_ORPHANED_DATA: &str = "orphaned.data";
/// Brew 模块：Homebrew 缓存清理阶段。
pub const PHASE_BREW: &str = "brew.cache";
/// System 模块：系统缓存清理阶段（顶层）。
pub const PHASE_SYSTEM: &str = "system";
/// System 模块：本地 Time Machine 快照检查阶段。
pub const PHASE_LOCAL_SNAPSHOTS: &str = "system.local_snapshots";
/// System 模块：Time Machine 失败备份清理阶段。
pub const PHASE_TIME_MACHINE: &str = "system.time_machine";
/// User 模块：Application Support 日志/缓存清理阶段。
pub const PHASE_APP_SUPPORT_LOGS: &str = "app_support.logs";
/// Apps 模块：孤立系统服务清理阶段。
pub const PHASE_ORPHANED_SYSTEM_SERVICES: &str = "orphaned.system_services";
/// Apps 模块：孤立容器存根清理阶段。
pub const PHASE_ORPHANED_CONTAINER_STUBS: &str = "orphaned.container_stubs";
/// User 模块：Apple Silicon 专用缓存清理阶段。
pub const PHASE_APPLE_SILICON_CACHES: &str = "apple_silicon.caches";
/// Hints 模块：大文件检查阶段。
pub const PHASE_LARGE_FILES: &str = "large_files";
/// Hints 模块：系统数据线索阶段。
pub const PHASE_SYSTEM_DATA_HINTS: &str = "system_data.hints";
/// Hints 模块：项目产物提示阶段。
pub const PHASE_PROJECT_ARTIFACTS: &str = "project_artifacts";
/// Hints 模块：用户 LaunchAgent 提示阶段。
pub const PHASE_USER_LAUNCH_AGENTS: &str = "user.launch_agents";
/// Hints 模块：孤立 dotdir 提示阶段。
pub const PHASE_ORPHANED_DOTDIR: &str = "orphaned.dotdir";

/// 与前端 `SpinnerUpdate` / `b.md` 描述对齐。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpinnerUpdatePayload {
    pub section: String,
    pub message: String,
    pub is_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// 单个阶段的结构化结果。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupPhaseResultPayload {
    pub section: String,
    pub phase: String,
    pub title: String,
    pub cleaned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_count: Option<u64>,
}

/// 向所有 Webview 广播当前清理阶段状态。
pub fn emit_cleanup_spinner_update(app: &AppHandle, payload: &SpinnerUpdatePayload) {
    let _ = app.emit(EVT_CLEANUP_SPINNER_UPDATE, payload);
}

/// 向所有 Webview 广播单阶段结果。
pub fn emit_cleanup_phase_result(app: &AppHandle, payload: &CleanupPhaseResultPayload) {
    let _ = app.emit(EVT_CLEANUP_PHASE_RESULT, payload);
}

/// 卸载进度 payload：当前处理的 app、累计进度、当前操作项。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallProgressPayload {
    /// 当前正在处理的 app 路径
    pub app_path: String,
    /// 当前 app 名称（显示用）
    pub app_name: String,
    /// 当前是第几个 app（从 1 开始）
    pub current_index: usize,
    /// 总共多少个 app
    pub total_count: usize,
    /// 当前操作描述（如 "正在清理残留文件..."）
    pub current_action: String,
}

/// 向所有 Webview 广播卸载进度。
pub fn emit_uninstall_progress(app: &AppHandle, payload: &UninstallProgressPayload) {
    let _ = app.emit(EVT_UNINSTALL_PROGRESS, payload);
}

/// 单个 app 卸载完成 payload。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallCompletePayload {
    /// app 路径
    pub app_path: String,
    /// app 名称
    pub app_name: String,
    /// 是否成功
    pub success: bool,
    /// 释放空间（字节）
    pub freed_bytes: u64,
    /// 失败原因（如果有）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 建议操作（如果有）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// 向所有 Webview 广播单个 app 卸载完成。
pub fn emit_uninstall_complete(app: &AppHandle, payload: &UninstallCompletePayload) {
    let _ = app.emit(EVT_UNINSTALL_COMPLETE, payload);
}

/// 单条扫描线索（label + 体积 + 路径）。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupHintItem {
    pub label: String,
    pub size_bytes: u64,
    pub size_human: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// 扫描线索集合（"review 性质的发现"，非清理统计）。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupHintsResultPayload {
    pub section: String,
    pub phase: String,
    pub title: String,
    pub detected: bool,
    pub review_hint: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<CleanupHintItem>,
}

/// 向所有 Webview 广播扫描线索结果。
pub fn emit_cleanup_hints_result(app: &AppHandle, payload: &CleanupHintsResultPayload) {
    let _ = app.emit(EVT_CLEANUP_HINTS_RESULT, payload);
}

// ── Analyze 扫描进度 ──

/// 扫描进度快照，每 ~200ms 从 scanner 内推送一次。
#[derive(Clone, serde::Serialize)]
pub struct ScanProgressPayload {
    pub files_scanned: i64,
    pub dirs_scanned: i64,
    pub bytes_scanned: i64,
    pub current_path: String,
    /// 扫描完成百分比 0–100；-1 表示暂无估算。
    pub percent: i64,
    /// 百分比分母（字节目标）：优先既往快照子树大小，其次卷已用空间；0 = 无。
    pub bytes_target: i64,
    /// 卷总容量（字节，展示用）。
    pub disk_total: i64,
}

/// 向所有 Webview 广播实时扫描进度。
pub fn emit_analyze_scan_progress(app: &AppHandle, payload: &ScanProgressPayload) {
    log::info!(
        "[emit_scan_progress] files={}, dirs={}, bytes={}, path={}, percent={}, target={}, total={}",
        payload.files_scanned,
        payload.dirs_scanned,
        payload.bytes_scanned,
        payload.current_path,
        payload.percent,
        payload.bytes_target,
        payload.disk_total
    );
    let _ = app.emit(EVT_ANALYZE_SCAN_PROGRESS, payload);
}

/// 移到废纸篓的实时进度 payload。
#[derive(Clone, serde::Serialize)]
pub struct TrashProgressPayload {
    /// 当前已处理的文件数。
    pub files_processed: i64,
    /// 当前正在处理的路径。
    pub current_path: String,
    /// 阶段："counting" — 统计文件数，"done" — 完成。
    pub phase: String,
    /// 已处理路径数（多选时）。
    pub paths_done: i64,
    /// 总路径数（多选时）。
    pub paths_total: i64,
}

/// 向所有 Webview 广播移到废纸篓的实时进度。
pub fn emit_analyze_trash_progress(app: &AppHandle, payload: &TrashProgressPayload) {
    log::info!(
        "[emit_trash_progress] files={}, path={}, phase={}, paths_done/total={}/{}",
        payload.files_processed,
        payload.current_path,
        payload.phase,
        payload.paths_done,
        payload.paths_total,
    );
    let _ = app.emit(EVT_ANALYZE_TRASH_PROGRESS, payload);
}

// ── Clean 执行进度 ──

/// Clean 执行进度的单帧快照。
/// phase 取值：
/// - `"start"`：执行开始（current_category 为空）
/// - `"category_start"`：开始清理某个分类
/// - `"category_done"`：该分类清理完成
/// - `"complete"`：全部完成
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanApplyProgressPayload {
    pub phase: String,
    /// 当前正在清理的分类 id（category_start / category_done 时有效）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_category: Option<String>,
    /// 当前正在处理的路径文案（展示用）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_path: Option<String>,
    /// 累计已清理字节数。
    pub cleaned_bytes: u64,
    /// 已完成的分类数。
    pub done_categories: u64,
    /// 总分类数。
    pub total_categories: u64,
    /// 失败/跳过数。
    pub failed_count: u64,
}

/// 向所有 Webview 广播 Clean 执行进度。
pub fn emit_clean_apply_progress(app: &AppHandle, payload: &CleanApplyProgressPayload) {
    let _ = app.emit(EVT_CLEAN_APPLY_PROGRESS, payload);
}

// ── Updates（应用更新）brew 流式进度 ──

/// Updates 模块：brew upgrade 流式进度短语（对齐 Burrow `UpdatesModel.brewPhrase`）。
pub const EVT_UPDATES_BREW_PROGRESS: &str = "updates::brew-progress";

/// brew 升级进度 payload。
#[derive(Clone, serde::Serialize)]
pub struct BrewProgressPayload {
    /// 升级目标 id：单包 = formula/cask name，全部 = "brew"。
    pub id: String,
    /// `==> ` 前缀行提取的进度短语（如 "Pouring foo…"）。
    pub phrase: String,
}

/// 向所有 Webview 广播 brew 升级进度短语。
pub fn emit_updates_brew_progress(app: &AppHandle, id: &str, phrase: &str) {
    let _ = app.emit(
        EVT_UPDATES_BREW_PROGRESS,
        BrewProgressPayload {
            id: id.to_string(),
            phrase: phrase.to_string(),
        },
    );
}
