/**
 * Tauri 事件名常量：与 Rust 端 events.rs 约定一致，用于 listen/emit。
 * 命名约定：<域>::<事件名>
 */

/** 配置域：应用配置更新后发出，payload 为最新 AppConfig */
export const EVT_CONFIG_UPDATED = 'config::updated' as const

/** 数据域：Core 确认写成功后 Rust 广播，payload 见 `TableRefreshPayload` */
export const EVT_TABLE_REFRESH = 'data::table-refresh' as const

/** 磁盘分析：递归扫描进度（与 Rust `analyze::scan-progress` 一致） */
export const EVT_ANALYZE_SCAN_PROGRESS = 'analyze::scan-progress' as const

/** 磁盘分析：移到废纸篓的实时进度（与 Rust `analyze::trash-progress` 一致） */
export const EVT_ANALYZE_TRASH_PROGRESS = 'analyze::trash-progress' as const

/** 系统优化：执行阶段任务级进度（与 Rust `optimize::progress` 一致） */
export const EVT_OPTIMIZE_PROGRESS = 'optimize::progress' as const

/** 系统监控：每秒一帧的快照（与 Rust `status::snapshot` 一致），对齐 Mole TUI `refreshInterval = 1s` */
export const EVT_STATUS_SNAPSHOT = 'status::snapshot' as const

/** 清理扫描：当前扫描模块更新（与 Rust `cleanup::spinner-update` 一致） */
export const EVT_CLEANUP_SPINNER_UPDATE = 'cleanup::spinner-update' as const

/** 清理扫描：模块扫描完成，带大小/文件数（与 Rust `cleanup::phase-result` 一致） */
export const EVT_CLEANUP_PHASE_RESULT = 'cleanup::phase-result' as const

/** 清理扫描：发现需要审查的项（与 Rust `cleanup::hints-result` 一致） */
export const EVT_CLEANUP_HINTS_RESULT = 'cleanup::hints-result' as const

/** 清理执行：移废纸篓阶段进度（与 Rust `clean::apply-progress` 一致） */
export const EVT_CLEAN_APPLY_PROGRESS = 'clean::apply-progress' as const

/** 应用更新：brew upgrade 流式进度短语（与 Rust `updates::brew-progress` 一致） */
export const EVT_UPDATES_BREW_PROGRESS = 'updates::brew-progress' as const

/** 卸载：单个 app 清理进度（与 Rust `uninstall::progress` 一致） */
export const EVT_UNINSTALL_PROGRESS = 'uninstall::progress' as const

/** 卸载：单个 app 清理完成（与 Rust `uninstall::complete` 一致） */
export const EVT_UNINSTALL_COMPLETE = 'uninstall::complete' as const

/** 卸载：单个 app 清理进度（与 Rust `uninstall::progress` 一致） */
