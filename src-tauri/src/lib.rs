//! MoleDesktop Tauri 后端。
//!
//! **构建矩阵**
//! - **官网完整版（默认 `full`）:** 一键 `scan_home` 解析真实用户主目录并扫描（分发时通常关闭 App Sandbox）。
//! - **MAS 精简版（`mas`）:** 同一 `scan_home` 入口；沙箱内仅能访问系统授予范围内的文件（常为容器目录），与完整版扫描范围可能不同。

#[cfg(all(feature = "mas", feature = "full"))]
compile_error!(
    "Cargo features `mas` 与 `full` 不能同时启用；MAS 构建请使用: --no-default-features --features mas"
);

pub mod embedded_rules;
mod scanner;

use serde::Deserialize;
use std::path::PathBuf;
use tauri::Emitter;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanDirectoryArgs {
    pub path: String,
    #[serde(default = "default_top_n")]
    pub top_n: usize,
    #[serde(default = "default_progress_every")]
    pub progress_every: u64,
    /// `logical`（默认，对标 Lemon/Finder）| `physical`（对标 Mole 物理占用）
    #[serde(default = "default_size_metric")]
    pub size_metric: String,
}

fn default_top_n() -> usize {
    50
}

fn default_progress_every() -> u64 {
    500
}

fn default_size_metric() -> String {
    "logical".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashPathsArgs {
    /// 本次扫描使用的根路径（通常与 `scan_home` / `scan_directory` 的 `root` 一致），所有 paths 必须在其下
    pub root: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanHomeArgs {
    #[serde(default = "default_top_n")]
    pub top_n: usize,
    #[serde(default = "default_progress_every")]
    pub progress_every: u64,
    #[serde(default = "default_size_metric")]
    pub size_metric: String,
}

/// 一键扫描用户主目录（Lemon / Mole 式入口，不弹文件夹选择框）。
/// - 官网完整版：通常为真实 `~`。
/// - MAS 沙箱：路径仍为系统解析的主目录，但可读范围受沙箱限制。
#[tauri::command]
async fn scan_home(
    app: tauri::AppHandle,
    args: ScanHomeArgs,
) -> Result<scanner::ScanResult, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法解析用户主目录（dirs::home_dir 为空）".to_string())?;
    let size_metric = scanner::SizeMetric::from_str(&args.size_metric)?;
    let app = app.clone();
    let top_n = args.top_n.max(1);
    let progress_every = args.progress_every;

    tauri::async_runtime::spawn_blocking(move || {
        scanner::scan_directory(&home, top_n, progress_every, size_metric, |prog| {
            let _ = app.emit("analyze::scan-progress", &prog);
        })
    })
    .await
    .map_err(|e| format!("扫描任务异常: {}", e))?
}

/// 按给定路径扫描（供调试或非主流程使用）
#[tauri::command]
async fn scan_directory(
    app: tauri::AppHandle,
    args: ScanDirectoryArgs,
) -> Result<scanner::ScanResult, String> {
    let size_metric = scanner::SizeMetric::from_str(&args.size_metric)?;
    let app = app.clone();
    let path = args.path.clone();
    let top_n = args.top_n.max(1);
    let progress_every = args.progress_every;

    tauri::async_runtime::spawn_blocking(move || {
        let root = PathBuf::from(path);
        scanner::scan_directory(&root, top_n, progress_every, size_metric, |prog| {
            let _ = app.emit("analyze::scan-progress", &prog);
        })
    })
    .await
    .map_err(|e| format!("扫描任务异常: {}", e))?
}

/// 将文件移到废纸篓；仅允许 `root` 目录树内的路径（MAS 友好）
#[tauri::command]
fn trash_paths(args: TrashPathsArgs) -> Result<(), String> {
    let root = PathBuf::from(&args.root);
    scanner::trash_paths_under_root(&root, &args.paths)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Debug)
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .invoke_handler(tauri::generate_handler![scan_home, scan_directory, trash_paths])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
