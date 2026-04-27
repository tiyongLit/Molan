use crate::platform::macos_file_icon;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use tauri::Manager;

// ── 缓存路径 ──

/// 图标渲染语义版本：底层取图逻辑变化时递增（目录名随之变化）。
/// v2：symlink 不再 canonicalize，改用系统原生的「目标图标 + alias 角标」。
const ICON_CACHE_VERSION: u32 = 2;

/// 历史缓存目录名：升级版本后一次性清理，避免旧图标文件（单张可达数百 KB）成为孤儿。
const LEGACY_ICON_CACHE_DIRS: &[&str] = &["icon-cache"];

fn cache_root(app_handle: &tauri::AppHandle) -> PathBuf {
    app_handle.path().app_cache_dir().unwrap_or_default()
}

fn cache_dir(app_handle: &tauri::AppHandle) -> PathBuf {
    let root = cache_root(app_handle);
    let dir = root.join(format!("icon-cache-v{ICON_CACHE_VERSION}"));
    // 旧版本目录清理：仅删本模块自己创建的固定目录名，不含用户数据。
    for legacy in LEGACY_ICON_CACHE_DIRS {
        let old = root.join(legacy);
        if old.is_dir() {
            log::info!("[icon_cache] removing legacy cache dir {}", old.display());
            let _ = std::fs::remove_dir_all(&old);
        }
    }
    dir
}

fn cache_name(path: &str) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn read_mtime(path: &str) -> u64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn read_cached_mtime(dir: &PathBuf, name: &str) -> Option<u64> {
    std::fs::read_to_string(dir.join(format!("{}.mtime", name)))
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

fn write_cache(dir: &PathBuf, name: &str, base64: &str, mtime: u64) {
    let _ = std::fs::write(dir.join(format!("{}.icon", name)), base64);
    let _ = std::fs::write(dir.join(format!("{}.mtime", name)), mtime.to_string());
}

fn try_cache(dir: &PathBuf, name: &str, path: &str) -> Option<String> {
    let icon_path = dir.join(format!("{}.icon", name));
    if !icon_path.exists() {
        return None;
    }
    let cached_mtime = read_cached_mtime(dir, name)?;
    let current = read_mtime(path);
    if current == 0 || cached_mtime != current {
        return None;
    }
    std::fs::read_to_string(&icon_path).ok()
}

// ── commands ──

/// 单个路径图标：mtime 校验 + AppCache
#[tauri::command(rename_all = "snake_case")]
pub fn mole_get_icon_cached(
    path: String,
    app_handle: tauri::AppHandle,
) -> Result<Option<String>, String> {
    if path.is_empty() {
        return Ok(None);
    }
    let dir = cache_dir(&app_handle);
    let _ = std::fs::create_dir_all(&dir);
    let name = cache_name(&path);

    if let Some(b64) = try_cache(&dir, &name, &path) {
        return Ok(Some(b64));
    }

    let base64 = macos_file_icon::file_icon_png_base64(&path)?;
    if let Some(ref b64) = base64 {
        write_cache(&dir, &name, b64, read_mtime(&path));
    }
    Ok(base64)
}

/// 批量图标：单次 IPC，mtime 校验
#[tauri::command(rename_all = "snake_case")]
pub fn mole_get_icons_batch(
    paths: Vec<String>,
    app_handle: tauri::AppHandle,
) -> Result<HashMap<String, Option<String>>, String> {
    let dir = cache_dir(&app_handle);
    let _ = std::fs::create_dir_all(&dir);
    let mut result = HashMap::new();

    for path in &paths {
        if path.is_empty() {
            result.insert(path.clone(), None);
            continue;
        }
        let name = cache_name(path);
        let b64 = if let Some(cached) = try_cache(&dir, &name, path) {
            Some(cached)
        } else {
            match macos_file_icon::file_icon_png_base64(path) {
                Ok(Some(b64)) => {
                    write_cache(&dir, &name, &b64, read_mtime(path));
                    Some(b64)
                }
                _ => None,
            }
        };
        result.insert(path.clone(), b64);
    }

    Ok(result)
}
