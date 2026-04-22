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

/** 清理扫描：模块扫描完成，带大小/文件数（与 Rust `cleanup::phase-result` 一致） */
export const EVT_CLEANUP_PHASE_RESULT = 'cleanup::phase-result' as const

/** 清理扫描：渐进式分类结果，每个 section 完成即推送该分类完整条目（与 Rust `cleanup::category-result` 一致） */
export const EVT_CLEANUP_CATEGORY_RESULT = 'cleanup::category-result' as const

/** 清理执行：移废纸篓阶段进度（与 Rust `clean::apply-progress` 一致） */
export const EVT_CLEAN_APPLY_PROGRESS = 'clean::apply-progress' as const

/** 应用更新：brew upgrade 流式进度短语（与 Rust `updates::brew-progress` 一致） */
export const EVT_UPDATES_BREW_PROGRESS = 'updates::brew-progress' as const

/** 卸载：单个 app 清理进度（与 Rust `uninstall::progress` 一致） */
export const EVT_UNINSTALL_PROGRESS = 'uninstall::progress' as const

/** 卸载：单个 app 清理完成（与 Rust `uninstall::complete` 一致） */
export const EVT_UNINSTALL_COMPLETE = 'uninstall::complete' as const

/** 应用版本：MoleStudio 自身更新下载/安装进度（与 Rust `app-version::progress` 一致） */
export const EVT_APP_VERSION_PROGRESS = 'app-version::progress' as const

/** Dock 退出拦截：有长任务在跑时 Rust emit 此事件，前端弹确认框（与 Rust `dock-quit-requested` 一致） */
export const EVT_DOCK_QUIT_REQUESTED = 'dock-quit-requested' as const

/** 卸载残留自动检测：检测到新 .app 进入废纸篓（与 Rust `uninstall::residual-detected` 一致） */
export const EVT_RESIDUAL_DETECTED = 'uninstall::residual-detected' as const

/** Clean 任务状态机快照：后端每次状态转换整体广播（与 Rust `clean::job-state` 一致），前端按 seq 单调应用 */
export const EVT_CLEAN_JOB_STATE = 'clean::job-state' as const

/** 提醒快照更新，只定向 trash-reminder；事件不是唯一事实源。 */
export const EVT_TRASH_REMINDER_STATE = 'trash::reminder-state' as const

/** 托盘气泡：即将离场（滑出动画前 emit，此刻窗口仍可见），前端据此立即复位瞬态 UI（如齿轮下拉） */
export const EVT_DASHBOARD_HIDE_REQUESTED = 'dashboard::hide-requested' as const

/** 卸载：单个 app 清理进度（与 Rust `uninstall::progress` 一致） */
