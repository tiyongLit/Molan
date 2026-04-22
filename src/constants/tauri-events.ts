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
