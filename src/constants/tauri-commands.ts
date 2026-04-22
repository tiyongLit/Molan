/**
 * Tauri 命令名常量：与 Rust 端 `invoke_handler` 注册名一致。
 * Day1 验证：递归扫描 + 废纸篓；后续恢复业务命令时在此追加即可。
 */

export const CMD_SCAN_DIRECTORY = 'scan_directory' as const
export const CMD_TRASH_PATHS = 'trash_paths' as const

/** 命令名数组，供 useTauri 按名生成 invoke api */
export const TAURI_COMMANDS = [CMD_SCAN_DIRECTORY, CMD_TRASH_PATHS] as const
