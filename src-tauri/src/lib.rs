//! MoleDesktop Tauri 后端。
//!
//! **构建矩阵**
//! - 完整版（默认）: `pnpm tauri dev` / `pnpm tauri build`
//! - MAS 精简: `pnpm tauri:dev:mas` / `pnpm tauri:build:mas`（无 `du` 折叠，整树 WalkDir）

#[cfg(all(feature = "mas", feature = "full"))]
compile_error!(
    "Cargo features `mas` 与 `full` 不能同时启用；MAS 构建请使用: --no-default-features --features mas"
);

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
}

fn default_top_n() -> usize {
    50
}

fn default_progress_every() -> u64 {
    500
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashPathsArgs {
    /// 用户通过对话框授权的根目录，所有 paths 必须在其下
    pub root: String,
    pub paths: Vec<String>,
}

/// 深度递归扫描：Top-N 最大文件 + 进度事件 `analyze::scan-progress`
#[tauri::command]
async fn scan_directory(
    app: tauri::AppHandle,
    args: ScanDirectoryArgs,
) -> Result<scanner::ScanResult, String> {
    let app = app.clone();
    let path = args.path.clone();
    let top_n = args.top_n.max(1);
    let progress_every = args.progress_every;

    tauri::async_runtime::spawn_blocking(move || {
        let root = PathBuf::from(path);
        scanner::scan_directory(&root, top_n, progress_every, |prog| {
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
        .invoke_handler(tauri::generate_handler![scan_directory, trash_paths])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
