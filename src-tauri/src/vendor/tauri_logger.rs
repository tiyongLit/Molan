use std::fs;
use std::path::PathBuf;

use chrono::Local;
use tauri::App;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Tauri 应用子目录名，写死为 flowshield（与 packages/flowshield 对齐）
const TAURI_APP_DIR: &str = "flowshield";

/// 日志保留天数：只保留最近 N 天的日志文件（按文件名中的日期判断）。1 = 只保留今天，2 = 今天+昨天，以此类推
const LOG_RETENTION_DAYS: i64 = 7;

/// 项目根目录名，与 Electron APP_NAME 对齐；改项目根目录名时同步改环境变量或默认值
fn workspace_root_name() -> String {
    std::env::var("APP_NAME").unwrap_or_else(|_| "kang".to_string())
}

/// 系统级 AppData 目录（不含 Tauri 的 com.tauri-app.xxx 子目录），与 Electron app.getPath("appData") 对齐
fn system_app_data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join("Library/Application Support")
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

/// 等价 Electron 的 workspace：appData/APP_NAME（如 AppData/Roaming/kang，不含 com.tauri-app.flowshield）
fn resolve_workspace(_app: &App) -> PathBuf {
    system_app_data_dir().join(workspace_root_name())
}

/// 日志目录：appData/kang/flowshield/logs
fn resolve_logs_dir(app: &App) -> PathBuf {
    resolve_workspace(app).join(TAURI_APP_DIR).join("logs")
}

/// 计算日志文件路径：appData/kang/flowshield/logs/YYYY-MM-DD-flowshield.log
fn resolve_log_path(app: &App) -> PathBuf {
    let logs_dir = resolve_logs_dir(app);
    let _ = fs::create_dir_all(&logs_dir);

    let date = Local::now().format("%Y-%m-%d").to_string();
    logs_dir.join(format!("{date}-{TAURI_APP_DIR}.log"))
}

/// 启动时执行一次：删除超过保留天数的旧日志文件（仅处理 YYYY-MM-DD-flowshield.log 格式）
fn cleanup_old_logs(app: &App) {
    let logs_dir = resolve_logs_dir(app);
    if !logs_dir.exists() {
        return;
    }

    let today = Local::now().date_naive();
    let cutoff = today - chrono::Duration::days(LOG_RETENTION_DAYS - 1);

    let entries = match fs::read_dir(&logs_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let suffix = format!("-{TAURI_APP_DIR}.log");
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.ends_with(&suffix) {
            continue;
        }
        let date_str = name.strip_suffix(&suffix).unwrap_or("");
        if date_str.len() != 10 {
            continue;
        }
        let file_date = match chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => continue,
        };
        if file_date < cutoff {
            let _ = fs::remove_file(&path);
        }
    }
}

/// 初始化全局日志：控制台 + 文件，info/warn/error/debug 统一走 tracing 宏
pub fn init_logger(app: &App) {
    // 避免重复初始化
    static INIT: std::sync::Once = std::sync::Once::new();

    INIT.call_once(|| {
        cleanup_old_logs(app);

        let log_path = resolve_log_path(app);

        let file_appender = match fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!("failed to open log file {:?}: {}", log_path, e);
                return;
            }
        };

        let (non_blocking_file, _guard) = tracing_appender::non_blocking(file_appender);

        // 环境变量控制日志级别，默认 info
        let env_filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

        let fmt_layer_stdout = fmt::layer()
            .with_target(true)
            .with_ansi(true)
            .with_thread_ids(false)
            .with_thread_names(false);

        let fmt_layer_file = fmt::layer()
            .with_target(true)
            .with_ansi(false)
            .with_writer(non_blocking_file);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer_stdout)
            .with(fmt_layer_file)
            .init();

        tracing::info!("logger initialized, log file: {:?}", log_path);
    });
}
