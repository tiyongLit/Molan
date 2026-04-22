/**
 * Tauri 命令名常量：与 Rust 端 invoke_handler 注册名一致，便于与插件约定统一。
 * 命名约定：<域>_<操作>
 */

export const CMD_GET_APP_CONFIG = 'get_app_config' as const
export const CMD_UPDATE_APP_CONFIG = 'update_app_config' as const
export const CMD_OPEN_RULE_EDITOR_WINDOW = 'open_rule_editor_window' as const
export const CMD_ADD_BROWSER_RULE = 'add_browser_rule' as const
export const CMD_QUERY_RULE_PAGE = 'query_rule_page' as const
export const CMD_ENABLE_RULE = 'enable_rule' as const
export const CMD_DISABLE_RULE = 'disable_rule' as const
export const CMD_HIDE_RULE_EDITOR_WINDOW = 'hide_rule_editor_window' as const
export const CMD_SEND_MESSAGE_TO_MAIN_WINDOW = 'send_message_to_main_window' as const
export const CMD_MINIMIZE_WINDOW = 'minimize_window' as const
export const CMD_MAXIMIZE_WINDOW = 'maximize_window' as const
export const CMD_CLOSE_WINDOW = 'close_window' as const
export const CMD_SCAN_INSTALLED_PROCESS_WINDOW = 'scan_installed_process_window' as const
export const CMD_GET_APPLICATION_FINGERPRINT = 'get_application_fingerprint' as const

/** 命令名数组，供 useTauri 按名生成 invoke api */
export const TAURI_COMMANDS = [
  CMD_GET_APP_CONFIG,
  CMD_UPDATE_APP_CONFIG,
  CMD_OPEN_RULE_EDITOR_WINDOW,
  CMD_ADD_BROWSER_RULE,
  CMD_QUERY_RULE_PAGE,
  CMD_ENABLE_RULE,
  CMD_DISABLE_RULE,
  CMD_HIDE_RULE_EDITOR_WINDOW,
  CMD_SEND_MESSAGE_TO_MAIN_WINDOW,
  CMD_MINIMIZE_WINDOW,
  CMD_MAXIMIZE_WINDOW,
  CMD_CLOSE_WINDOW,
  CMD_SCAN_INSTALLED_PROCESS_WINDOW,
  CMD_GET_APPLICATION_FINGERPRINT
] as const
