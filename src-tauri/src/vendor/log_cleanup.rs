use std::fs;
use std::path::PathBuf;

use tauri::Manager;

/// 日志保留天数：只保留最近 N 天的日志文件
const LOG_RETENTION_DAYS: i64 = 7;

/// 定时清理间隔：每 24 小时执行一次
const CLEANUP_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// tauri-plugin-log 的日志目录：~/Library/Logs/<identifier>
///
/// 与 tauri-plugin-log 内部使用 PathResolver 解析的路径保持一致。
/// macOS 用户一般不关机，app 可能数周不重启，
/// 因此除了启动时清理，还需定时兜底（见 `start_periodic_cleanup`）。
pub fn resolve_logs_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_log_dir().ok()
}

/// 删除超过 LOG_RETENTION_DAYS 的日志文件。
///
/// 只处理 tauri-plugin-log 生成的 `.log` 文件，按文件修改时间判断。
/// 性能：仅 readdir + metadata，日志目录通常 <10 个文件，微秒级完成。
pub fn cleanup_old_logs(app: &tauri::AppHandle) {
    let logs_dir = match resolve_logs_dir(app) {
        Some(dir) => dir,
        None => return,
    };

    if !logs_dir.exists() {
        return;
    }

    let cutoff = match std::time::SystemTime::now().checked_sub(std::time::Duration::from_secs(
        (LOG_RETENTION_DAYS * 86400) as u64,
    )) {
        Some(t) => t,
        None => return,
    };

    let entries = match fs::read_dir(&logs_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // 只处理 .log 文件
        if path.extension().and_then(|e| e.to_str()) != Some("log") {
            continue;
        }

        // 按修改时间判断是否过期
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                if modified < cutoff {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }
}

/// 后台定时清理：每 24 小时执行一次 `cleanup_old_logs`。
///
/// 通过 Tauri AppHandle 的 runtime 调度，app 退出时自动取消。
/// 开销极低：每 24 小时一次 readdir + 几次 stat，不影响任何业务逻辑。
pub fn start_periodic_cleanup(app: &tauri::AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(CLEANUP_INTERVAL_SECS)).await;
            cleanup_old_logs(&handle);
        }
    });
}
