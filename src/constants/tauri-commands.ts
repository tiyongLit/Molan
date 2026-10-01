/**
 * Tauri 命令名常量：与 Rust 端 `invoke_handler` 注册名一致。
 * Day1 验证：递归扫描 + 废纸篓；后续恢复业务命令时在此追加即可。
 */

/** 默认：扫描当前用户主目录（官网版≈整盘透视；MAS 版受沙箱可见范围限制） */
export const CMD_SCAN_HOME = 'scan_home' as const
export const CMD_SCAN_DIRECTORY = 'scan_directory' as const
export const CMD_TRASH_PATHS = 'trash_paths' as const

/** Mole Go 二进制桥接命令 */
export const CMD_MOLE_ANALYZE = 'mole_analyze' as const
/** 磁盘分析：取消当前活跃扫描（切换目录/离开页面时调用，后端 jwalk 每批检查代际） */
export const CMD_MOLE_ANALYZE_CANCEL = 'mole_analyze_cancel' as const
/** 磁盘分析：轻量级目录导航（从内存会话树读取，微秒级返回） */
export const CMD_MOLE_ANALYZE_NAVIGATE = 'mole_analyze_navigate' as const
/** 磁盘分析：释放会话内存树（离开 Analyze 页面时调用） */
export const CMD_MOLE_ANALYZE_CLEAR_SESSION = 'mole_analyze_clear_session' as const
export const CMD_MOLE_STATUS_START_WATCH = 'mole_status_start_watch' as const
export const CMD_MOLE_STATUS_STOP_WATCH = 'mole_status_stop_watch' as const
/** 单次全量状态采集：不启动 watch 线程（静默设计，Home/Analyze 挂载时取首帧快照） */
export const CMD_MOLE_STATUS_ONCE = 'mole_status_once' as const
/** 关闭进程（对齐柠檬 killProcessByID：用户态 SIGTERM） */
export const CMD_MOLE_KILL_PROCESS = 'mole_kill_process' as const
export const CMD_MOLE_CLEAN = 'mole_clean' as const
export const CMD_MOLE_PURGE = 'mole_purge' as const
export const CMD_MOLE_CHECK = 'mole_check' as const
export const CMD_MOLE_OPTIMIZE = 'mole_optimize' as const
export const CMD_MOLE_LIST_APPS = 'mole_list_apps' as const
export const CMD_MOLE_UNINSTALL = 'mole_uninstall' as const
export const CMD_MOLE_UNINSTALL_BATCH = 'mole_uninstall_batch' as const
export const CMD_MOLE_GET_UNINSTALL_HISTORY = 'mole_get_uninstall_history' as const
export const CMD_MOLE_CLEAR_UNINSTALL_HISTORY = 'mole_clear_uninstall_history' as const
export const CMD_MOLE_REVEAL_IN_TRASH = 'mole_reveal_in_trash' as const

/** Orphan — 孤儿残留扫描（对齐 PureMac ReversePathsFetch） */
export const CMD_MOLE_ORPHAN_SCAN = 'mole_orphan_scan' as const
export const CMD_MOLE_ORPHAN_DELETE = 'mole_orphan_delete' as const
/** 卸载残留定向链路：点击系统通知 → 消费 pending 快照 → 跳卸载页定向扫描 */
export const CMD_MOLE_RESIDUAL_TAKE_PENDING = 'mole_residual_take_pending' as const
export const CMD_MOLE_ORPHAN_SCAN_FOR = 'mole_orphan_scan_for' as const

/** Updates — 应用更新（Updates 标签页后端） */
export const CMD_MOLE_UPDATES_BREW_OUTDATED = 'mole_updates_brew_outdated' as const
export const CMD_MOLE_UPDATES_CHECK = 'mole_updates_check' as const
export const CMD_MOLE_UPDATES_APPLY = 'mole_updates_apply' as const
export const CMD_MOLE_UPDATES_BREW_UPGRADE = 'mole_updates_brew_upgrade' as const

/** Self-Update — MoleStudio 自身更新 */
export const CMD_MOLE_APP_VERSION_CHECK = 'mole_app_version_check' as const
export const CMD_MOLE_APP_VERSION_INSTALL = 'mole_app_version_install' as const
export const CMD_MOLE_APP_VERSION_OPEN_APPSTORE = 'mole_app_version_open_appstore' as const

/** Startup — 启动项 */
export const CMD_MOLE_STARTUP_SCAN = 'mole_startup_scan' as const
export const CMD_MOLE_STARTUP_ACTION = 'mole_startup_action' as const

export const CMD_SET_WINDOW_SIZE = 'set_window_size' as const
export const CMD_GET_WINDOW_SIZE = 'get_window_size' as const

/** 打开/聚焦磁盘分析子窗口 */
export const CMD_MOLE_OPEN_ANALYZE_WINDOW = 'mole_open_analyze_window' as const

/** 打开/聚焦应用卸载子窗口 */
export const CMD_MOLE_OPEN_UNINSTALL_WINDOW = 'mole_open_uninstall_window' as const

/** 打开/聚焦新布局子窗口 */
export const CMD_MOLE_OPEN_SHELL_WINDOW = 'mole_open_shell_window' as const

/** 磁盘分析：删除选中路径到废纸篓 */
export const CMD_MOLE_ANALYZE_TRASH = 'mole_analyze_trash' as const

/** 磁盘分析：受保护目录名列表 */
export const CMD_MOLE_GET_PROTECTED_ANALYZE_PATHS = 'mole_get_protected_analyze_paths' as const

/** 原生图标注册表：内容寻址解析，返回 path→contentID + contentID→SVG data URI */
export const CMD_MOLE_NATIVE_ICONS_RESOLVE = 'mole_native_icons_resolve' as const

/** 原生 NSAlert 对话框（垂直三行流：app 图标 / 文案 / 原生按钮）：await 返回是否点击主按钮 */
export const CMD_MOLE_DIALOG = 'mole_dialog' as const

/** 用 Rust trash crate 直接移废纸篓（代替 Go 二进制的 execute 模式） */
export const CMD_MOLE_CLEAN_PATHS = 'mole_clean_paths' as const
export const CMD_MOLE_CLEAN_EXECUTE = 'mole_clean_execute' as const

/** Clean v2 — 扫描/执行分离 + size_metric + 取消（对齐 04_后端工作流.md 命名规范） */
export const CMD_CLEAN_STATUS = 'clean_status' as const
export const CMD_CLEAN_SCAN = 'clean_scan' as const
export const CMD_CLEAN_SCAN_CANCEL = 'clean_scan_cancel' as const
export const CMD_CLEAN_APPLY = 'clean_apply' as const
export const CMD_CLEAN_APPLY_CANCEL = 'clean_apply_cancel' as const
export const CMD_CLEAN_REVEAL_IN_FINDER = 'clean_reveal_in_finder' as const

/** Clean Job — 任务状态机（后端唯一事实来源）：受理/对账/取结果/取消 */
export const CMD_CLEAN_JOB_START = 'clean_job_start' as const
export const CMD_CLEAN_JOB_STATE = 'clean_job_state' as const
export const CMD_CLEAN_JOB_RESULT = 'clean_job_result' as const
export const CMD_CLEAN_JOB_CANCEL = 'clean_job_cancel' as const

/** Check --fix: 运行检查并自动修复可修复项 */
export const CMD_MOLE_CHECK_FIX = 'mole_check_fix' as const

/** Installer: 扫描已知目录中的安装器文件 */
export const CMD_MOLE_INSTALLER_SCAN = 'mole_installer_scan' as const
export const CMD_MOLE_INSTALLER_TRASH = 'mole_installer_trash' as const

/** Touch ID: 检查/配置 sudo Touch ID */
export const CMD_MOLE_TOUCHID_STATUS = 'mole_touchid_status' as const
export const CMD_MOLE_TOUCHID_ENABLE = 'mole_touchid_enable' as const

/** Whitelist: 白名单管理 */
export const CMD_MOLE_WHITELIST_READ = 'mole_whitelist_read' as const
export const CMD_MOLE_WHITELIST_WRITE = 'mole_whitelist_write' as const
export const CMD_MOLE_WHITELIST_PREDEFINED = 'mole_whitelist_predefined' as const

/** 权限：能力查询 + 管理员会话（对齐 clean.sh SYSTEM_CLEAN） */
export const CMD_MOLE_PRIVILEGE_CAPABILITIES = 'mole_privilege_capabilities' as const
export const CMD_MOLE_REQUEST_ADMIN_SESSION = 'mole_request_admin_session' as const
export const CMD_MOLE_REVOKE_ADMIN_SESSION = 'mole_revoke_admin_session' as const

/** Dock 退出拦截：用户确认退出（与 Rust `mole_confirm_dock_quit` 一致） */
export const CMD_MOLE_CONFIRM_DOCK_QUIT = 'mole_confirm_dock_quit' as const
/** Dock 退出拦截：查询当前是否有长任务在跑（与 Rust `mole_is_busy` 一致） */
export const CMD_MOLE_IS_BUSY = 'mole_is_busy' as const
/** 恢复 Dock 图标：打开主窗口时调用（与 Rust `mole_show_dock_icon` 一致） */
export const CMD_MOLE_SHOW_DOCK_ICON = 'mole_show_dock_icon' as const
/** 隐藏托盘气泡：走出场动画 + 严格配对 stop_status_watch（与 Rust `mole_dashboard_hide` 一致） */
export const CMD_MOLE_DASHBOARD_HIDE = 'mole_dashboard_hide' as const
/** 平台信息：获取安装形态/分发渠道（与 Rust `mole_get_platform_info` 一致） */
export const CMD_MOLE_GET_PLATFORM_INFO = 'mole_get_platform_info' as const

/** Settings — 设置功能 */
export const CMD_MOLE_OPEN_SETTINGS_WINDOW = 'mole_open_settings_window' as const
export const CMD_MOLE_AUTO_LAUNCH_STATUS = 'mole_auto_launch_status' as const
export const CMD_MOLE_AUTO_LAUNCH_TOGGLE = 'mole_auto_launch_toggle' as const

/** 废纸篓提醒「清空」：清空当前用户 ~/.Trash（独立于 Clean 缓存清理链路） */
export const CMD_MOLE_TRASH_EMPTY = 'mole_trash_empty' as const
export const CMD_MOLE_TRASH_REMINDER_GET_STATE = 'mole_trash_reminder_get_state' as const
export const CMD_MOLE_TRASH_REMINDER_ACTION = 'mole_trash_reminder_action' as const
export const CMD_MOLE_TRASH_REMINDER_UPDATE_SETTINGS = 'mole_trash_reminder_update_settings' as const

/** Purge Paths: 产物清理路径配置 */
export const CMD_MOLE_PURGE_PATHS_READ = 'mole_purge_paths_read' as const
export const CMD_MOLE_PURGE_PATHS_WRITE = 'mole_purge_paths_write' as const

/** UI 时序埋点转发（卡顿分析）：直接原生 invoke，绕开 useTauri Batcher 合并 */
export const CMD_MOLE_UI_LOG = 'mole_ui_log' as const

/** 命令名数组，供 useTauri 按名生成 invoke api */
export const TAURI_COMMANDS = [
  CMD_SCAN_HOME,
  CMD_SCAN_DIRECTORY,
  CMD_TRASH_PATHS,
  CMD_MOLE_ANALYZE,
  CMD_MOLE_STATUS_START_WATCH,
  CMD_MOLE_STATUS_STOP_WATCH,
  CMD_MOLE_STATUS_ONCE,
  CMD_MOLE_KILL_PROCESS,
  CMD_MOLE_CLEAN,
  CMD_MOLE_PURGE,
  CMD_MOLE_CHECK,
  CMD_MOLE_OPTIMIZE,
  CMD_MOLE_LIST_APPS,
  CMD_MOLE_UNINSTALL,
  CMD_MOLE_UNINSTALL_BATCH,
  CMD_MOLE_GET_UNINSTALL_HISTORY,
  CMD_MOLE_CLEAR_UNINSTALL_HISTORY,
  CMD_MOLE_REVEAL_IN_TRASH,
  CMD_MOLE_ORPHAN_SCAN,
  CMD_MOLE_ORPHAN_DELETE,
  CMD_MOLE_RESIDUAL_TAKE_PENDING,
  CMD_MOLE_ORPHAN_SCAN_FOR,
  CMD_MOLE_UPDATES_BREW_OUTDATED,
  CMD_MOLE_UPDATES_CHECK,
  CMD_MOLE_UPDATES_APPLY,
  CMD_MOLE_UPDATES_BREW_UPGRADE,
  CMD_MOLE_APP_VERSION_CHECK,
  CMD_MOLE_APP_VERSION_INSTALL,
  CMD_MOLE_APP_VERSION_OPEN_APPSTORE,
  CMD_MOLE_STARTUP_SCAN,
  CMD_MOLE_STARTUP_ACTION,
  CMD_SET_WINDOW_SIZE,
  CMD_GET_WINDOW_SIZE,
  CMD_MOLE_NATIVE_ICONS_RESOLVE,
  CMD_MOLE_DIALOG,
  CMD_MOLE_CLEAN_PATHS,
  CMD_MOLE_CLEAN_EXECUTE,
  CMD_CLEAN_STATUS,
  CMD_CLEAN_SCAN,
  CMD_CLEAN_SCAN_CANCEL,
  CMD_CLEAN_APPLY,
  CMD_CLEAN_APPLY_CANCEL,
  CMD_CLEAN_REVEAL_IN_FINDER,
  CMD_CLEAN_JOB_START,
  CMD_CLEAN_JOB_STATE,
  CMD_CLEAN_JOB_RESULT,
  CMD_CLEAN_JOB_CANCEL,
  CMD_MOLE_CHECK_FIX,
  CMD_MOLE_INSTALLER_SCAN,
  CMD_MOLE_INSTALLER_TRASH,
  CMD_MOLE_TOUCHID_STATUS,
  CMD_MOLE_TOUCHID_ENABLE,
  CMD_MOLE_WHITELIST_READ,
  CMD_MOLE_WHITELIST_WRITE,
  CMD_MOLE_WHITELIST_PREDEFINED,
  CMD_MOLE_PRIVILEGE_CAPABILITIES,
  CMD_MOLE_REQUEST_ADMIN_SESSION,
  CMD_MOLE_REVOKE_ADMIN_SESSION,
  CMD_MOLE_PURGE_PATHS_READ,
  CMD_MOLE_PURGE_PATHS_WRITE,
  CMD_MOLE_OPEN_ANALYZE_WINDOW,
  CMD_MOLE_OPEN_UNINSTALL_WINDOW,
  CMD_MOLE_OPEN_SHELL_WINDOW,
  CMD_MOLE_ANALYZE_TRASH,
  CMD_MOLE_ANALYZE_CANCEL,
  CMD_MOLE_ANALYZE_NAVIGATE,
  CMD_MOLE_ANALYZE_CLEAR_SESSION,
  CMD_MOLE_GET_PROTECTED_ANALYZE_PATHS,
  CMD_MOLE_CONFIRM_DOCK_QUIT,
  CMD_MOLE_IS_BUSY,
  CMD_MOLE_DASHBOARD_HIDE,
  CMD_MOLE_GET_PLATFORM_INFO,
  CMD_MOLE_OPEN_SETTINGS_WINDOW,
  CMD_MOLE_AUTO_LAUNCH_STATUS,
  CMD_MOLE_AUTO_LAUNCH_TOGGLE,
  CMD_MOLE_TRASH_EMPTY,
  CMD_MOLE_TRASH_REMINDER_GET_STATE,
  CMD_MOLE_TRASH_REMINDER_ACTION,
  CMD_MOLE_TRASH_REMINDER_UPDATE_SETTINGS,
  CMD_MOLE_UI_LOG
] as const
