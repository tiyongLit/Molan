use serde::Serialize;
use std::path::PathBuf;
use walkdir::WalkDir;

// 定义返回给前端的数据结构
#[derive(Serialize, Clone)]
struct DirInfo {
    name: String,
    path: String,
    size: u64,
}

// 计算文件夹大小的辅助函数
fn get_dir_size(path: &PathBuf) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

// Tauri 命令：扫描文件夹
#[tauri::command]
fn scan_folder(path: String) -> Result<Vec<DirInfo>, String> {
    let mut results = Vec::new();
    let root = PathBuf::from(&path);

    // 检查路径是否存在且为目录
    if !root.exists() || !root.is_dir() {
        return Err("Invalid directory path".to_string());
    }

    // 遍历第一层子目录
    for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let entry_path = entry.path();

        if entry_path.is_dir() {
            let size = get_dir_size(&entry_path);
            results.push(DirInfo {
                name: entry_path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                path: entry_path.to_string_lossy().into_owned(),
                size,
            });
        }
    }

    // 按大小降序排列，让大文件排在前面
    results.sort_by(|a, b| b.size.cmp(&a.size));
    Ok(results)
}

// Tauri 命令：清理文件（移到废纸篓）
#[tauri::command]
fn clean_files(paths: Vec<String>) -> Result<(), String> {
    for path in paths {
        // 使用 trash crate，这符合 macOS 沙箱规范
        trash::delete(&path).map_err(|e| format!("Failed to delete {}: {}", path, e))?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init()) // 注册对话框插件
        .plugin(tauri_plugin_store::Builder::default().build())
        .invoke_handler(tauri::generate_handler![scan_folder, clean_files]) // 注册我们的命令
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
