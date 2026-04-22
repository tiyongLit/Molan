use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

use crate::uninstall::brew::get_brew_cask_name;

#[derive(Serialize, Clone)]
pub struct AppListEntry {
    pub name: String,
    pub display_name: String,
    pub path: String,
    pub bundle_id: String,
    pub source: String,
    pub uninstall_name: String,
    pub size_bytes: u64,
    pub size_human: String,
    pub last_used_epoch: i64,
    pub last_used_relative: String,
    pub version: String,
    pub running: bool,
    /// 更新机制来源（对齐 Burrow `UpdateSources.detect`，零网络）：
    /// "sparkle" | "app_store" | "electron" | null（不可检测）。
    pub update_source: Option<crate::updates::detect::UpdateSource>,
}

/// 卸载应用元数据缓存：mtime 匹配 + 7 天 TTL 内复用 size / last_used，避免每次全量 mdls/du。
/// 对齐 SH `uninstall_app_metadata_v2`（只做同步读写的最小可行版，不做后台 refresh + 锁）。
const META_CACHE_TTL_SECS: i64 = 604800; // 7 天

#[derive(Serialize, Deserialize, Clone)]
struct AppMetaCacheEntry {
    path: String,
    mtime: u64,
    size_bytes: u64,
    last_used_epoch: i64,
    updated_epoch: i64,
}

fn meta_cache_file() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(format!("{home}/.cache/mole/uninstall_app_metadata_v2.json"))
}

fn load_meta_cache() -> HashMap<String, AppMetaCacheEntry> {
    let Ok(data) = std::fs::read_to_string(meta_cache_file()) else {
        return HashMap::new();
    };
    let Ok(entries) = serde_json::from_str::<Vec<AppMetaCacheEntry>>(&data) else {
        return HashMap::new();
    };
    entries.into_iter().map(|e| (e.path.clone(), e)).collect()
}

fn save_meta_cache(entries: &[AppMetaCacheEntry]) {
    let file = meta_cache_file();
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(data) = serde_json::to_string(entries) {
        let _ = std::fs::write(&file, data);
    }
}

// CLI scan_applications 对齐翻译
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_list_apps() -> Result<Vec<AppListEntry>, String> {
    // 扫描含目录遍历 + Spotlight 查询 + plist 解析（重 I/O）：spawn_blocking 隔离，
    // 避免占用 tokio worker 线程导致其他 IPC 排队。
    tauri::async_runtime::spawn_blocking(list_apps_blocking)
        .await
        .map_err(|e| format!("应用列表扫描任务失败: {e}"))?
}

/// `mole_list_apps` 的阻塞主体（原命令函数体）。
fn list_apps_blocking() -> Result<Vec<AppListEntry>, String> {
    let home = std::env::var("HOME").unwrap_or_default();

    // CLI L482-486: app_dirs
    let mut app_dirs: Vec<String> = vec![
        "/Applications".to_string(),
        format!("{}/Applications", home),
        "/Library/Input Methods".to_string(),
        format!("{}/Library/Input Methods", home),
    ];

    // CLI L487-495: /Volumes/*/Applications (-d and -r: only if we can list the dir)
    if let Ok(entries) = std::fs::read_dir("/Volumes") {
        for entry in entries.filter_map(|e| e.ok()) {
            let vol_app = format!("{}/Applications", entry.path().display());
            if !Path::new(&vol_app).is_dir() {
                continue;
            }
            if std::fs::read_dir(&vol_app).is_err() {
                continue;
            }
            if same_file(&vol_app, "/Applications")
                || same_file(&vol_app, &format!("{}/Applications", home))
            {
                continue;
            }
            app_dirs.push(vol_app);
        }
    }

    // CLI L498-520: pkg_receipt_nonstandard_app_paths
    let pkg_paths = crate::core::pkg_receipts::pkg_receipt_nonstandard_app_paths();

    // CLI Pass 1: collect app paths
    let mut app_data_tuples: Vec<(String, String, u64)> = Vec::new(); // (path, name, mtime)
    let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

    // CLI L499-520: pkg paths first (dedup against app_dirs)
    for pkg_path in &pkg_paths {
        if !Path::new(pkg_path).is_dir() || pkg_path.is_empty() {
            continue;
        }
        // CLI L504-508: [[ "$pkg_app_path" == "$app_dir"/*.app ]] (* does not match '/')
        let already_scanned = app_dirs
            .iter()
            .any(|dir| pkg_is_direct_child_app_bundle(pkg_path, dir));
        if already_scanned {
            continue;
        }
        if seen_paths.insert(pkg_path.clone()) {
            let app_name = Path::new(pkg_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let app_mtime = get_file_mtime(pkg_path);
            app_data_tuples.push((pkg_path.clone(), app_name, app_mtime));
        }
    }

    // CLI L522-575: for each app_dir, `find -name "*.app" -maxdepth 3` 的原生等价
    for dir in &app_dirs {
        if !Path::new(dir).is_dir() {
            continue;
        }
        for app_path in find_app_bundles_maxdepth3(dir) {
            if app_path.is_empty() || !Path::new(&app_path).exists() {
                continue;
            }

            // CLI L540-543: skip nested apps inside another .app bundle
            if let Some(parent) = Path::new(&app_path).parent() {
                let parent_str = parent.to_string_lossy();
                if parent_str.contains(".app/") || parent_str.ends_with(".app") {
                    continue;
                }
            }

            // CLI L546-563: symlink handling
            if Path::new(&app_path).is_symlink() {
                if let Ok(target) = std::fs::read_link(&app_path) {
                    let resolved = resolve_symlink(&app_path, &target);
                    // CLI L559-563: skip system paths
                    if resolved.starts_with("/System/")
                        || resolved.starts_with("/usr/bin/")
                        || resolved.starts_with("/usr/lib/")
                        || resolved.starts_with("/bin/")
                        || resolved.starts_with("/sbin/")
                        || resolved.starts_with("/private/etc/")
                    {
                        continue;
                    }
                }
            }

            if seen_paths.insert(app_path.clone()) {
                let app_name = Path::new(&app_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                let app_mtime = get_file_mtime(&app_path);
                app_data_tuples.push((app_path, app_name, app_mtime));
            }
        }
    }

    if app_data_tuples.is_empty() {
        return Ok(vec![]);
    }

    // 运行中应用名字集合：NSWorkspace 元信息索引（8s TTL、毫秒级、跨调用共享），
    // 替代原逐 app 的 `pgrep -x` 子进程。
    let running_index = crate::platform::macos_running_apps::running_apps_icon_index();

    // CLI Pass 2: process_app_metadata (L600-630)
    // filter by protect / bg_only, resolve display_name, then collect du/mdls
    // 并行处理每个 app 的 metadata 查询（原生 plist / Spotlight 读取，无子进程）
    let qualified: Vec<(String, String, String, u64)> = app_data_tuples
        .par_iter()
        .filter_map(|(app_path, app_name, app_mtime)| {
            // CLI L609-615: 读 Info.plist CFBundleIdentifier（原 defaults read）
            let bundle_id = read_bundle_id(app_path);

            // CLI L617-619: should_protect_from_uninstall
            if crate::core::app_protection::should_protect_from_uninstall(&bundle_id) {
                return None;
            }

            // CLI L621-628: LSBackgroundOnly check（原 defaults read）
            // OneDrive exemption (L629-643): top-level OneDrive.app is background-only
            // but the user explicitly installed it and should be able to uninstall it.
            let plist = format!("{}/Contents/Info.plist", app_path);
            if Path::new(&plist).is_file() && plist_bool_key(&plist, "LSBackgroundOnly") {
                // New CLI: OneDrive exemption — skip LSBackgroundOnly filter
                // for top-level OneDrive.app bundles.
                let od = bundle_id.starts_with("com.microsoft.OneDrive")
                    && (app_path == "/Applications/OneDrive.app"
                        || app_path == &format!("{}/Applications/OneDrive.app", home));
                if !od {
                    return None;
                }
            }

            // CLI L631-633: resolve display_name
            let mut display_name = resolve_display_name(app_path, app_name);

            // CLI L635-637: post-process sanitize
            if display_name.ends_with(".app") {
                display_name = display_name[..display_name.len() - 4].to_string();
            }
            display_name = display_name.replace('|', "-");
            display_name = display_name
                .replace('\t', "")
                .replace('\r', "")
                .replace('\n', "");

            Some((app_path.clone(), display_name, bundle_id, *app_mtime))
        })
        .collect();

    if qualified.is_empty() {
        return Ok(vec![]);
    }

    // 元数据缓存：mtime 匹配 + TTL 内复用 size / last_used（对齐 SH warm/cold 分区）。
    let meta_cache = load_meta_cache();
    let now_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // 缓存命中直接复用；未命中的冷行重新读 mdls/du。
    let mut sizes: HashMap<String, u64> = HashMap::new();
    let mut last_used_map: HashMap<String, i64> = HashMap::new();
    let cold_paths: Vec<String> = qualified
        .iter()
        .filter_map(
            |(path, _, _, app_mtime)| match meta_cache.get(path.as_str()) {
                Some(e)
                    if e.mtime == *app_mtime
                        && now_epoch - e.updated_epoch <= META_CACHE_TTL_SECS =>
                {
                    sizes.insert(path.clone(), e.size_bytes);
                    last_used_map.insert(path.clone(), e.last_used_epoch);
                    None
                }
                _ => Some(path.clone()),
            },
        )
        .collect();

    // Batch get sizes for cold paths: Spotlight 逻辑大小 → du 等价兜底（均原生）。
    if !cold_paths.is_empty() {
        let mut cold_sizes = batch_logical_sizes(&cold_paths);
        let du_needed: Vec<String> = cold_paths
            .iter()
            .filter(|p| cold_sizes.get(p.as_str()).copied().unwrap_or(0) == 0)
            .cloned()
            .collect();
        if !du_needed.is_empty() {
            for (k, v) in batch_physical_sizes(&du_needed) {
                cold_sizes.insert(k, v);
            }
        }
        for p in &cold_paths {
            sizes.insert(p.clone(), cold_sizes.get(p.as_str()).copied().unwrap_or(0));
        }
    }

    // CLI L237-245: batch 取 kMDItemLastUsedDate（仅冷行，原生 Spotlight 查询）
    if !cold_paths.is_empty() {
        let last_used: Vec<(String, i64)> = cold_paths
            .par_iter()
            .filter_map(|p| {
                crate::platform::macos_mditem::last_used_epoch(p).map(|epoch| (p.clone(), epoch))
            })
            .collect();
        for (path, epoch) in last_used {
            last_used_map.insert(path, epoch);
        }
    }

    let mut entries: Vec<AppListEntry> = qualified
        .par_iter()
        .map(|(path, display_name, bundle_id, app_mtime)| {
            let size_bytes = *sizes.get(path).unwrap_or(&0);
            // CLI L243-248: last_used fallback to app mtime
            let last_used_epoch = if let Some(&e) = last_used_map.get(path) {
                e
            } else {
                *app_mtime as i64
            };
            let last_used_relative = relative_time_from_epoch(last_used_epoch);

            let name = Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            let version = read_short_version(path);
            let exec_name = read_bundle_executable(path);
            // 运行中判定：可执行名（或 app 名）出现在 NSWorkspace 运行应用索引即视为运行中
            let running = if !exec_name.is_empty() {
                running_index.by_name.contains_key(&exec_name)
            } else {
                running_index.by_name.contains_key(&name)
            };

            // uninstall_name：brew cask token 优先，否则 display name（对齐 SH 第 1435-1439 行）。
            let cask_name = get_brew_cask_name(path);
            let source = if cask_name.is_some() {
                "Homebrew".to_string()
            } else {
                "App".to_string()
            };
            let uninstall_name = cask_name.unwrap_or_else(|| display_name.clone());

            AppListEntry {
                name,
                display_name: display_name.clone(),
                path: path.clone(),
                bundle_id: bundle_id.clone(),
                source,
                uninstall_name,
                size_bytes,
                size_human: crate::core::base::bytes_to_human(size_bytes),
                last_used_epoch,
                last_used_relative,
                version,
                running,
                update_source: crate::updates::detect::detect_update_source(path),
            }
        })
        .collect();

    // 写回元数据缓存（同步、最小可行版；Mole 的后台 refresh + 锁不在本次范围）。
    let new_cache: Vec<AppMetaCacheEntry> = qualified
        .iter()
        .map(|(path, _, _, app_mtime)| AppMetaCacheEntry {
            path: path.clone(),
            mtime: *app_mtime,
            size_bytes: *sizes.get(path).unwrap_or(&0),
            last_used_epoch: last_used_map
                .get(path)
                .copied()
                .unwrap_or(*app_mtime as i64),
            updated_epoch: now_epoch,
        })
        .collect();
    save_meta_cache(&new_cache);

    // New CLI Phase 6: deduplicate by bundle_id — keep the best-ranked path
    // when the same bundle appears on multiple volumes (e.g. backup drive).
    // Priority: /Applications > ~/Applications > other > /Volumes.
    dedupe_entries_by_bundle_id(&mut entries);

    // CLI L818: sort by epoch oldest-first
    entries.sort_by_key(|e| e.last_used_epoch);

    // ── 日志：扫描结果汇总 ──
    log::info!(
        "[uninstall.list_apps] scanned {} apps from {} dirs (pkg_paths={})",
        entries.len(),
        app_dirs.len(),
        pkg_paths.len()
    );
    for e in &entries {
        log::info!(
            "[uninstall.list_apps.entry] name={} display={} bundle={} path={} size={} last_used={} running={}",
            e.name,
            e.display_name,
            e.bundle_id,
            e.path,
            e.size_human,
            e.last_used_relative,
            e.running
        );
    }

    Ok(entries)
}

fn read_bundle_executable(app_path: &str) -> String {
    let plist = format!("{app_path}/Contents/Info.plist");
    if !Path::new(&plist).is_file() {
        return String::new();
    }
    // 原 CLI：`defaults read <plist> CFBundleExecutable`
    plist_string_key(&plist, "CFBundleExecutable").unwrap_or_default()
}

fn same_file(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

/// 读 Info.plist 字符串键（原 `defaults read` / `plutil -extract ... raw` 的原生替代）。
fn plist_string_key(plist_path: &str, key: &str) -> Option<String> {
    match plist::Value::from_file(plist_path)
        .ok()
        .and_then(|v| v.into_dictionary())
        .and_then(|mut d| d.remove(key))
    {
        Some(plist::Value::String(s)) => Some(s),
        _ => None,
    }
}

/// 读 Info.plist 布尔键（对齐 `defaults read` 输出 "1"/"YES"/"true" 的判定语义）。
fn plist_bool_key(plist_path: &str, key: &str) -> bool {
    match plist::Value::from_file(plist_path)
        .ok()
        .and_then(|v| v.into_dictionary())
        .and_then(|mut d| d.remove(key))
    {
        Some(plist::Value::Boolean(b)) => b,
        Some(plist::Value::Integer(i)) => i.as_signed() == Some(1),
        Some(plist::Value::String(s)) => matches!(s.as_str(), "1" | "YES" | "true"),
        _ => false,
    }
}

/// CLI L522-575 `find <dir> -name "*.app" -maxdepth 3 -print0` 的原生等价：
/// 返回深度 ≤3、名字以 `.app` 结尾的条目路径。
/// 不进入符号链接（对齐 find 默认不跟随）与 .app 包内部（嵌套 app 原逻辑随后被过滤，等价剪枝）。
fn find_app_bundles_maxdepth3(dir: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut stack: Vec<(std::path::PathBuf, usize)> = vec![(std::path::PathBuf::from(dir), 0)];
    while let Some((current, depth)) = stack.pop() {
        if depth >= 3 {
            continue; // 再深入将超过 maxdepth 3
        }
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue; // 不可读目录跳过（find 打印错误后继续）
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if entry.file_name().to_string_lossy().ends_with(".app") {
                out.push(path.to_string_lossy().to_string());
                continue; // 不进入 .app 包内部
            }
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                stack.push((path, depth + 1));
            }
        }
    }
    out
}

/// 批量取冷路径逻辑大小（原 `mdls -raw -name kMDItemLogicalSize` 的批量调用，
/// 改原生 Spotlight 查询）；未索引的路径不入表，调用侧按 0 处理并触发磁盘占用兜底。
fn batch_logical_sizes(paths: &[String]) -> HashMap<String, u64> {
    if paths.is_empty() {
        return HashMap::new();
    }
    paths
        .par_iter()
        .filter_map(|p| {
            crate::platform::macos_mditem::logical_size(p).map(|bytes| {
                // 与原 CLI 一致：按 KB 截断后回填字节
                (p.clone(), (bytes / 1024).saturating_mul(1024))
            })
        })
        .collect()
}

/// 批量取磁盘块占用（原 `du -skP` 批量调用的原生等价）；返回字节数（KB 对齐后 ×1024）。
fn batch_physical_sizes(paths: &[String]) -> HashMap<String, u64> {
    if paths.is_empty() {
        return HashMap::new();
    }
    paths
        .par_iter()
        .map(|p| (p.clone(), du_kb_native(p).saturating_mul(1024)))
        .collect()
}

/// `du -skP <path>` 的原生等价：512B 块求和（硬链接仅计一次，不跟随符号链接），
/// 返回 KB（与原 du -k 的 `(blocks + 1) / 2` 向上取整一致）。
fn du_kb_native(path: &str) -> u64 {
    let mut total_blocks: u64 = 0;
    let mut seen_links: std::collections::HashSet<(u64, u64)> = std::collections::HashSet::new();
    for entry in walkdir::WalkDir::new(path).follow_links(false) {
        let Ok(entry) = entry else {
            continue; // 不可读条目跳过（du 同样 best-effort）
        };
        // follow_links(false) 下 metadata() 即 lstat，与 du 不跟随符号链接的口径一致
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            // 硬链接去重：dev+ino 相同只计一次（对齐 du 语义）
            if meta.nlink() > 1 && !seen_links.insert((meta.dev(), meta.ino())) {
                continue;
            }
            total_blocks = total_blocks.saturating_add(meta.blocks());
        }
        #[cfg(not(unix))]
        {
            total_blocks = total_blocks.saturating_add(meta.len().div_ceil(512));
        }
    }
    total_blocks.div_ceil(2)
}

/// Pre-compute file sizes for a newline-separated list of paths.
/// Files and symlinks use fast lstat, directories use one native batch walk.
fn precompute_file_sizes(all_paths: &str) -> HashMap<String, u64> {
    let mut map = HashMap::new();
    let mut dir_paths: Vec<String> = Vec::new();

    for line in all_paths.lines() {
        let path = line.trim().to_string();
        if path.is_empty() {
            continue;
        }
        let p = Path::new(&path);
        // Use symlink_metadata (lstat) — fast, no fork.
        let meta = match p.symlink_metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() && !p.is_symlink() {
            dir_paths.push(path);
        } else {
            map.insert(path, meta.len());
        }
    }

    if !dir_paths.is_empty() {
        let dir_sizes = batch_physical_sizes(&dir_paths);
        for (k, v) in dir_sizes {
            map.insert(k, v);
        }
    }

    map
}

/// Bash `[[ "$pkg_path" == "$app_dir"/*.app ]]` where `*` does not cross `/`.
fn pkg_is_direct_child_app_bundle(pkg_path: &str, app_dir: &str) -> bool {
    let app_dir = app_dir.trim_end_matches('/');
    if app_dir.is_empty() || !pkg_path.ends_with(".app") {
        return false;
    }
    let Some(parent) = Path::new(pkg_path).parent() else {
        return false;
    };
    parent.to_string_lossy() == app_dir
}

fn get_file_mtime(path: &str) -> u64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn resolve_symlink(link_path: &str, target: &Path) -> String {
    if target.is_relative() {
        if let Some(parent) = Path::new(link_path).parent() {
            let resolved = parent.join(target);
            return std::fs::canonicalize(&resolved)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
        }
        String::new()
    } else {
        target.to_string_lossy().to_string()
    }
}

fn read_bundle_id(app_path: &str) -> String {
    let plist = format!("{}/Contents/Info.plist", app_path);
    if !Path::new(&plist).is_file() {
        return "unknown".to_string();
    }
    // CLI L612: `defaults read <plist> CFBundleIdentifier` 的原生替代
    plist_string_key(&plist, "CFBundleIdentifier").unwrap_or_else(|| "unknown".to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mole_uninstall(
    app: tauri::AppHandle,
    app_path: String,
    dry_run: bool,
    data_only: Option<bool>,
) -> Result<Value, String> {
    let data_only = data_only.unwrap_or(false);
    if dry_run {
        run_dry_run(&app_path, data_only)
    } else {
        run_execute(&app, &app_path, data_only)
    }
}

fn run_dry_run(app_path: &str, data_only: bool) -> Result<Value, String> {
    let apps = crate::uninstall::batch::collect_app_details(&[app_path.to_string()])?;

    let detail = match apps.first() {
        Some(d) => d,
        None => {
            return Ok(serde_json::json!({
                "mode": "dry_run",
                "collected_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                "app": null,
                "related_files": [],
                "summary": {
                    "total_size": 0,
                    "total_size_human": "0B",
                    "file_count": 0,
                    "has_sensitive_data": false,
                    "sensitive_paths": [],
                    "launch_agents": []
                }
            }));
        }
    };

    // manual removal(对齐 SH `manual_removal_apps`):预扫描已拒绝(身份不可绑定 /
    // 特权删除路径祖先可变),不做预览,只报告原因。
    if !detail.manual_removal_reason.is_empty() {
        return Ok(serde_json::json!({
            "mode": "dry_run",
            "collected_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "app": null,
            "manual_removal": true,
            "reason": detail.manual_removal_reason,
            "related_files": [],
            "summary": {
                "total_size": 0,
                "total_size_human": "0B",
                "file_count": 0,
                "has_sensitive_data": false,
                "sensitive_paths": [],
                "launch_agents": []
            }
        }));
    }

    let version = read_short_version(app_path);
    let display_name = resolve_display_name(app_path, &detail.app_name);
    let last_used_epoch = read_last_used_date(app_path);
    let last_used_relative = relative_time_from_epoch(last_used_epoch);
    let app_size = detail.total_kb.saturating_mul(1024);

    // Build related-files path list (system files are now review-only — not deletable).
    let mut all_paths = String::new();
    if !detail.related_files.is_empty() {
        all_paths.push_str(&detail.related_files);
    }
    log::info!(
        "[uninstall.dry_run] app={} related_raw={:?}",
        app_path,
        detail
            .related_files
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect::<Vec<_>>()
    );

    // New CLI: review-only system files — precompute sizes separately for frontend preview.
    let review_size_map = if !detail.review_system_files.is_empty() {
        precompute_file_sizes(&detail.review_system_files)
    } else {
        HashMap::new()
    };

    // Pre-compute sizes in batch: files use fast lstat, dirs use one batch du -skP.
    // This replaces the old per-path file_or_dir_size() loop that spawned du once per directory.
    let all_size_map = precompute_file_sizes(&all_paths);

    let mut related_files: Vec<Value> = Vec::new();
    for line in all_paths.lines() {
        let path = line.trim();
        if path.is_empty() {
            continue;
        }

        // 空目录（du 报 0KB，如只有 0 字节日志的 Logs 目录）也照常列出，
        // 对齐 Burrow（mo dry-run 不按 size 过滤）；前端对 size=0 显示 "—"。
        let size = match all_size_map.get(path) {
            Some(s) => *s,
            None => continue, // path missing/inaccessible
        };

        let file_type = derive_file_type(path);
        let sensitive = is_path_sensitive(path);

        related_files.push(serde_json::json!({
            "path": path,
            "size": size,
            "size_human": crate::core::base::bytes_to_human(size),
            "type": file_type,
            "has_sensitive_data": sensitive
        }));
    }
    log::info!(
        "[uninstall.dry_run] app={} related_count={}",
        app_path,
        related_files.len()
    );

    let mut sensitive_paths: Vec<&str> = Vec::new();
    let mut launch_agents: Vec<&str> = Vec::new();
    for rf in &related_files {
        if rf["has_sensitive_data"].as_bool() == Some(true) {
            if let Some(p) = rf["path"].as_str() {
                sensitive_paths.push(p);
            }
        }
        if rf["type"].as_str() == Some("loginItem") {
            if let Some(p) = rf["path"].as_str() {
                launch_agents.push(p);
            }
        }
    }

    let file_count = (related_files.len() as i32) + 1;

    // Build review-only system files for frontend preview.
    let mut review_only_files: Vec<Value> = Vec::new();
    for line in detail.review_system_files.lines() {
        let path = line.trim();
        if path.is_empty() {
            continue;
        }
        // 与 related_files 同理：只跳过"不存在/不可读"，空目录照常列出。
        let Some(&size) = review_size_map.get(path) else {
            continue; // path missing/inaccessible
        };
        let file_type = derive_file_type(path);
        review_only_files.push(serde_json::json!({
            "path": path,
            "size": size,
            "size_human": crate::core::base::bytes_to_human(size),
            "type": file_type,
            "review_only": true
        }));
    }

    // ── 日志：干跑结果 ──
    log::info!(
        "[uninstall.dry_run] app={display_name} path={} bundle={} total_files={file_count} total_size={}",
        detail.app_path,
        detail.bundle_id,
        crate::core::base::bytes_to_human(app_size)
    );
    for line in all_paths.lines() {
        let p = line.trim();
        if !p.is_empty() {
            log::info!("[uninstall.dry_run.file] {display_name}: {p}");
        }
    }

    Ok(serde_json::json!({
        "mode": "dry_run",
        "data_only": data_only,
        "collected_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "app": {
            "name": display_name,
            "bundle_id": detail.bundle_id,
            "path": detail.app_path,
            "version": version,
            "size": app_size,
            "size_human": crate::core::base::bytes_to_human(app_size),
            "last_used_epoch": last_used_epoch,
            "last_used_relative": last_used_relative,
            "is_brew_cask": detail.is_brew_cask,
            "brew_cask_name": if detail.is_brew_cask && !detail.cask_name.is_empty() { Some(&detail.cask_name) } else { None },
            "is_official_uninstaller": detail.is_official_uninstaller,
            "official_vendor": detail.official_vendor,
        },
        "related_files": related_files,
        "review_only_files": review_only_files,
        "summary": {
            "total_size": app_size,
            "total_size_human": crate::core::base::bytes_to_human(app_size),
            "file_count": file_count,
            "has_sensitive_data": detail.has_sensitive_data,
            "sensitive_paths": sensitive_paths,
            "launch_agents": launch_agents
        }
    }))
}

fn run_execute(app: &tauri::AppHandle, app_path: &str, data_only: bool) -> Result<Value, String> {
    // Clear Data 模式：保留 app 本体，只清残留
    if data_only {
        unsafe { std::env::set_var("MOLE_UNINSTALL_DATA_ONLY", "1") };
    }
    let result =
        crate::uninstall::batch::batch_uninstall_applications(&[app_path.to_string()], Some(app));
    if data_only {
        unsafe { std::env::remove_var("MOLE_UNINSTALL_DATA_ONLY") };
    }

    let outcomes: Vec<Value> = result
        .outcomes
        .iter()
        .map(|o| {
            let cleaned = o.freed_kb.saturating_mul(1024);
            serde_json::json!({
                "path": o.app_path,
                "type": "app_bundle",
                "size_cleaned": cleaned,
                "size_cleaned_human": crate::core::base::bytes_to_human(cleaned),
                "status": if o.success { "removed" } else { "failed" },
                "error": if !o.success && !o.reason.is_empty() { Some(&o.reason) } else { None }
            })
        })
        .collect();

    let total_cleaned = result.total_size_freed_kb.saturating_mul(1024);

    Ok(serde_json::json!({
        "mode": "execute",
        "collected_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "app": {
            "name": "",
            "bundle_id": "",
            "path": app_path,
            "version": "",
            "size": 0,
            "size_human": "0B",
            "is_brew_cask": false,
            "brew_cask_name": null
        },
        "results": outcomes,
        "summary": {
            "total_cleaned_size": total_cleaned,
            "total_cleaned_size_human": crate::core::base::bytes_to_human(total_cleaned),
            "file_count": result.outcomes.len() as i32,
            "success_count": result.success_count,
            "skipped_count": 0,
            "failed_count": result.failed_count,
            "duration_seconds": 0
        }
    }))
}

fn read_short_version(app_path: &str) -> String {
    let plist = format!("{}/Contents/Info.plist", app_path);
    if !Path::new(&plist).is_file() {
        return String::new();
    }
    // 原 `plutil -extract CFBundleShortVersionString raw -o -`
    plist_string_key(&plist, "CFBundleShortVersionString").unwrap_or_default()
}

fn resolve_display_name(app_path: &str, app_name: &str) -> String {
    let plist = format!("{}/Contents/Info.plist", app_path);
    if !Path::new(&plist).is_file() {
        return app_name.trim_end_matches(".app").to_string();
    }

    let shell_clean = |s: &str| -> String {
        s.replace('|', "-")
            .replace('\t', " ")
            .replace('\r', " ")
            .replace('\n', " ")
    };

    // Spotlight 显示名（原 `mdls -name kMDItemDisplayName -raw`；未索引返回空）
    let md_display_name =
        crate::platform::macos_mditem::spotlight_display_name(app_path).unwrap_or_default();

    // Info.plist 显示名（原 `plutil -extract CFBundleDisplayName / CFBundleName raw`）
    let bundle_display_name = plist_string_key(&plist, "CFBundleDisplayName").unwrap_or_default();
    let bundle_name = plist_string_key(&plist, "CFBundleName").unwrap_or_default();

    let md_display_name = if md_display_name.starts_with('/') {
        String::new()
    } else {
        shell_clean(&md_display_name)
    };

    let bundle_display_name = shell_clean(&bundle_display_name);
    let bundle_name = shell_clean(&bundle_name);

    let raw_name = app_name.trim_end_matches(".app").to_string();

    let display_name = if !md_display_name.is_empty()
        && md_display_name != "(null)"
        && md_display_name != raw_name
    {
        md_display_name
    } else if !bundle_display_name.is_empty() && bundle_display_name != "(null)" {
        bundle_display_name
    } else if !bundle_name.is_empty() && bundle_name != "(null)" {
        bundle_name
    } else {
        raw_name.clone()
    };

    let display_name = if display_name.starts_with('/') {
        raw_name.clone()
    } else {
        shell_clean(&display_name)
    };

    if display_name != raw_name && raw_name.starts_with(&display_name) {
        let suffix = &raw_name[display_name.len()..];
        if suffix.chars().any(|c| c.is_ascii_digit()) {
            return shell_clean(&raw_name);
        }
    }

    shell_clean(&display_name)
}

fn read_last_used_date(app_path: &str) -> i64 {
    // 原 `mdls -name kMDItemLastUsedDate -raw` + `date -j -f ... +%s`，改原生 Spotlight 查询
    crate::platform::macos_mditem::last_used_epoch(app_path).unwrap_or(0)
}

fn relative_time_from_epoch(value_epoch: i64) -> String {
    if value_epoch <= 0 {
        return "Unknown".to_string();
    }

    let now_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let diff = now_epoch - value_epoch;
    if diff < 0 {
        return "Unknown".to_string();
    }

    let days_ago = diff / 86400;

    if days_ago == 0 {
        "Today".to_string()
    } else if days_ago == 1 {
        "Yesterday".to_string()
    } else if days_ago < 7 {
        format!("{} days ago", days_ago)
    } else if days_ago < 30 {
        let weeks_ago = days_ago / 7;
        if weeks_ago == 1 {
            "1 week ago".to_string()
        } else {
            format!("{} weeks ago", weeks_ago)
        }
    } else if days_ago < 365 {
        let months_ago = days_ago / 30;
        if months_ago == 1 {
            "1 month ago".to_string()
        } else {
            format!("{} months ago", months_ago)
        }
    } else {
        let years_ago = days_ago / 365;
        if years_ago == 1 {
            "1 year ago".to_string()
        } else {
            format!("{} years ago", years_ago)
        }
    }
}

/// New CLI Phase 6: Bundle ID 去重 — 当同一 bundle 出现在不同磁盘（如备份盘）时，
/// 只保留优先级最高的路径：/Applications > ~/Applications > 其他 > /Volumes。
fn path_rank(path: &str) -> u8 {
    let home = std::env::var("HOME").unwrap_or_default();
    if is_direct_app_under(path, "/Applications/") {
        return 1;
    }
    if is_direct_app_under(path, &format!("{}/Applications/", home)) {
        return 2;
    }
    if path.starts_with("/Volumes/") {
        return 4;
    }
    3
}

fn is_direct_app_under(path: &str, prefix: &str) -> bool {
    if !path.starts_with(prefix) {
        return false;
    }
    let rest = &path[prefix.len()..];
    !rest.contains('/') && rest.ends_with(".app")
}

fn dedupe_entries_by_bundle_id(entries: &mut Vec<AppListEntry>) {
    let mut best_idx: HashMap<String, usize> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut keep = vec![true; entries.len()];

    for (i, e) in entries.iter().enumerate() {
        let bid = &e.bundle_id;
        if bid.is_empty() || bid == "unknown" {
            let key = format!("__path__{}", i);
            order.push(key.clone());
            best_idx.insert(key, i);
            continue;
        }
        let rank = path_rank(&e.path);
        if let Some(&prev) = best_idx.get(bid) {
            if rank < path_rank(&entries[prev].path) {
                keep[prev] = false;
                best_idx.insert(bid.clone(), i);
                // update order entry
                for o in order.iter_mut() {
                    if o == bid {
                        *o = format!("__replaced__{}", i);
                        break;
                    }
                }
                order.push(bid.clone());
            } else {
                keep[i] = false;
            }
        } else {
            best_idx.insert(bid.clone(), i);
            order.push(bid.clone());
        }
    }

    // Filter in place, preserving order
    let mut write = 0;
    for read in 0..entries.len() {
        if keep[read] {
            if write != read {
                entries.swap(write, read);
            }
            write += 1;
        }
    }
    entries.truncate(write);
}

fn derive_file_type(path: &str) -> &str {
    if path.contains("DiagnosticReports") {
        "diagnostic_report"
    } else if path.contains("/Library/Application Scripts/") {
        "helper"
    } else if path.contains("/Library/LaunchAgents/") || path.contains("/Library/LaunchDaemons/") {
        "loginItem"
    } else if path.contains("/Library/Application Support/") {
        "application_support"
    } else if path.contains("/Library/Caches/") {
        "cache"
    } else if path.contains("/Library/Preferences/") {
        "preferences"
    } else if path.contains("/Library/Logs/") {
        "logs"
    } else if path.contains("/Library/Containers/") {
        "container"
    } else if path.contains("/Library/Group Containers/") {
        "group_container"
    } else if path.contains("/Library/Saved Application State/") {
        "saved_state"
    } else if path.contains("/Library/WebKit/") {
        "webkit"
    } else if path.contains("/Library/HTTPStorages/") {
        "http_storage"
    } else if path.contains("/Library/Frameworks/")
        || path.contains("/Library/Extensions/")
        || path.contains("/Library/Receipts/")
    {
        "system_file"
    } else {
        "other"
    }
}

fn is_path_sensitive(path: &str) -> bool {
    const SUBSTRING_PATTERNS: &[&str] = &[
        "/.warp",
        "/.config/",
        "/themes/",
        "/settings/",
        "/User Data/",
        "/.ssh/",
        "/.gnupg/",
        "/Documents/",
        "/Desktop/",
        "/Downloads/",
        "/Movies/",
        "/Music/",
        "/Pictures/",
        "/.password",
        "/.token",
        "/.auth",
        "/keychain",
        "/Passwords/",
        "/Accounts/",
        "/Cookies/",
        "/.aws/",
        "/.kube/",
        "/credentials/",
        "/secrets/",
    ];
    for pat in SUBSTRING_PATTERNS {
        if path.contains(pat) {
            return true;
        }
    }
    if let Some(idx) = path.rfind("/Preferences/") {
        let tail = &path[idx + "/Preferences/".len()..];
        if !tail.is_empty() && !tail.contains('/') && tail.ends_with(".plist") {
            return true;
        }
    }
    if path.ends_with("/.docker/config.json") {
        return true;
    }
    false
}

#[derive(Serialize, Clone)]
pub struct BatchUninstallOutcome {
    pub app_name: String,
    pub app_path: String,
    pub success: bool,
    pub freed_bytes: u64,
    pub freed_human: String,
    pub reason: String,
    pub suggestion: String,
}

/// 批量卸载多个应用。
/// 后端调用 `batch_uninstall_applications`，返回每个应用的卸载结果。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_uninstall_batch(
    app: tauri::AppHandle,
    app_paths: Vec<String>,
    data_only: Option<bool>,
) -> Result<Value, String> {
    let data_only = data_only.unwrap_or(false);
    // Clear Data 模式：保留 app 本体，只清残留
    if data_only {
        unsafe { std::env::set_var("MOLE_UNINSTALL_DATA_ONLY", "1") };
    }
    // 卸载是耗时操作（多轮 du/trash/进程检查）：spawn_blocking 避免阻塞主线程
    // （Tauri 同步命令跑在主线程，几十个 app 的批量卸载会冻结整个事件循环）。
    let app_clone = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _busy = crate::core::busy_state::enter_busy();
        let _awake = crate::core::keep_awake::KeepAwakeGuard::acquire("Uninstalling apps");
        crate::uninstall::batch::batch_uninstall_applications(&app_paths, Some(&app_clone))
    })
    .await
    .map_err(|e| format!("卸载任务失败: {e}"))?;
    if data_only {
        unsafe { std::env::remove_var("MOLE_UNINSTALL_DATA_ONLY") };
    }

    let total_cleaned = result.total_size_freed_kb.saturating_mul(1024);
    let outcomes: Vec<Value> = result
        .outcomes
        .iter()
        .map(|o| {
            serde_json::json!({
                "app_name": o.app_name,
                "app_path": o.app_path,
                "success": o.success,
                "freed_bytes": o.freed_kb.saturating_mul(1024),
                "freed_human": crate::core::base::bytes_to_human(o.freed_kb.saturating_mul(1024)),
                "reason": o.reason,
                "suggestion": o.suggestion
            })
        })
        .collect();

    Ok(serde_json::json!({
        "mode": "batch",
        "success_count": result.success_count,
        "failed_count": result.failed_count,
        "total_cleaned_size": total_cleaned,
        "total_cleaned_size_human": crate::core::base::bytes_to_human(total_cleaned),
        "outcomes": outcomes,
        "running_apps": result.running_apps,
        "running_at_uninstall_apps": result.running_at_uninstall_apps,
        "sudo_apps": result.sudo_apps,
        "brew_cask_apps": result.brew_cask_apps,
        "blocked_apps": result.blocked_apps,
        "manual_removal_apps": result.manual_removal_apps,
        "background_item_leftovers": result.background_item_leftovers,
        "local_network_warning_apps": result.local_network_warning_apps,
        "system_extension_warning_apps": result.system_extension_warning_apps,
        "status_title": result.title,
        "status": result.status
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_open_uninstall_window(app: tauri::AppHandle) -> Result<(), String> {
    log::info!("[mole_open_uninstall_window] called");
    if let Some(window) = app.get_webview_window("uninstall") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    use tauri::WebviewUrl;
    use tauri::WebviewWindowBuilder;
    use tauri::webview::PageLoadEvent;

    let window = WebviewWindowBuilder::new(&app, "uninstall", WebviewUrl::App("/uninstall".into()))
        .title("应用卸载")
        .inner_size(1056.0, 640.0)
        .resizable(true)
        .visible(false)
        // Reveal 门控：等前端渲染完成后再显示窗口，避免白屏闪烁
        .on_page_load(|webview, payload| {
            if let PageLoadEvent::Finished = payload.event() {
                let _ = webview.show();
                let _ = webview.set_focus();
                log::info!("[uninstall] window revealed on page load");
            }
        })
        .build()
        .map_err(|e| e.to_string())?;

    let w = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = w.hide();
        }
    });

    // 3s fallback：防止 page load 事件延迟时窗口一直不可见
    let fallback_win = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(3));
        if !fallback_win.is_visible().unwrap_or(false) {
            let _ = fallback_win.show();
            let _ = fallback_win.set_focus();
            log::info!("[uninstall] window revealed by 3s fallback");
        }
    });

    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_get_uninstall_history() -> Result<Value, String> {
    let records = crate::uninstall::history::get_history();

    Ok(serde_json::json!({
        "records": records
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_clear_uninstall_history() -> Result<(), String> {
    crate::uninstall::history::clear_history()
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_reveal_in_trash() -> Result<(), String> {
    let home = dirs::home_dir().ok_or("无法获取用户目录")?;
    let trash_path = home.join(".Trash");

    // 使用 open 命令打开废纸篓
    std::process::Command::new("open")
        .arg(&trash_path)
        .output()
        .map_err(|e| format!("打开废纸篓失败: {}", e))?;

    Ok(())
}

// ============================================================================
// 孤儿残留扫描（对齐 PureMac AppState.findOrphans + ReversePathsFetch）
// ============================================================================

use crate::uninstall::orphan_safety::{OrphanEntry, is_safe_orphan_candidate, scan_orphans};

/// 孤儿残留扫描：反向扫描已安装 app 列表之外的残留文件。
///
/// 用户在用 MoleStudio 之前通过拖到废纸篓等方式卸载的 app，
/// 其残留数据靠正向扫描（find_app_files）抓不到。本命令实现反向扫描：
/// 遍历一组固定路径，过滤掉属于已安装 app 的条目，剩下的就是孤儿候选。
///
/// 安全策略（对齐 PureMac OrphanSafetyPolicy）：
/// - 白名单 root：只有 Caches/Logs/HTTPStorages 等易失数据目录下的孤儿可删除
/// - 黑名单 fragment：Preferences/Containers 等持久状态目录只展示不删
/// - 高风险 dotpath：~/.ssh、~/.claude 等一律拦截
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_orphan_scan(_app: tauri::AppHandle) -> Result<Vec<OrphanEntry>, String> {
    // 1. 复用 mole_list_apps 拿到已安装 app 列表
    let apps = mole_list_apps().await?;
    let installed: Vec<(String, String)> = apps
        .iter()
        .map(|a| (a.bundle_id.clone(), a.name.clone()))
        .collect();

    let home = crate::core::base::home_dir();

    // 2. spawn_blocking 执行扫描（重 I/O）
    let orphans = tauri::async_runtime::spawn_blocking(move || scan_orphans(&installed, &home))
        .await
        .map_err(|e| format!("孤儿扫描任务失败: {}", e))?;

    Ok(orphans)
}

/// 孤儿残留删除：走 trash crate + 用户确认。
///
/// 对每条路径再次校验 `is_safe_orphan_candidate`，
/// 然后调 `file_ops::mole_delete`（内部已有 validate_path_for_deletion + TOCTOU 防护）。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_orphan_delete(
    _app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<Value, String> {
    if paths.is_empty() {
        return Ok(serde_json::json!({
            "success_count": 0,
            "failed_count": 0,
            "total_freed_bytes": 0
        }));
    }

    let home = crate::core::base::home_dir();
    let mut success_count = 0usize;
    let mut failed_count = 0usize;
    let mut total_freed: u64 = 0;

    for path in &paths {
        // 二次安全校验（防前端传入非法路径）
        if !is_safe_orphan_candidate(path, &home) {
            log::warn!("[orphan_delete] rejected unsafe path: {}", path);
            failed_count += 1;
            continue;
        }

        // 计算删除前大小
        let size_before = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

        // 走统一删除管线（trash crate + validate_path_for_deletion）
        let exit_code = crate::core::file_ops::mole_delete(path, false, None);
        if exit_code == crate::core::file_ops::MOLE_OK {
            success_count += 1;
            total_freed += size_before;
            log::info!("[orphan_delete] trashed: {}", path);
        } else {
            failed_count += 1;
            log::warn!("[orphan_delete] failed (exit={}): {}", exit_code, path);
        }
    }

    Ok(serde_json::json!({
        "success_count": success_count,
        "failed_count": failed_count,
        "total_freed_bytes": total_freed
    }))
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// 原生 `*.app` 发现与系统 `find -name "*.app" -maxdepth 3` 的等价性：
    /// 对系统 find 结果应用生产同款「嵌套 .app 过滤」后，集合应完全一致。
    #[test]
    fn find_app_bundles_matches_system_find() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // ── 深度/类型各异的样本树 ──
        std::fs::create_dir_all(root.join("A.app/Contents/MacOS")).unwrap();
        std::fs::create_dir_all(root.join("Utilities/B.app")).unwrap();
        std::fs::create_dir_all(root.join("Utilities/plain")).unwrap();
        std::fs::create_dir_all(root.join("Nested/Deep/C.app")).unwrap();
        std::fs::create_dir_all(root.join("Nested/Deep/Deeper/D.app")).unwrap(); // depth4 超界
        std::fs::create_dir_all(root.join("A.app/Contents/Nested.app")).unwrap(); // .app 内嵌套
        std::fs::write(root.join("notes.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(root.join("A.app"), root.join("Link.app")).unwrap();
        std::os::unix::fs::symlink(root, root.join("Utilities/loop")).unwrap();

        let root_str = root.to_string_lossy().to_string();

        // 系统 find 基准（仅测试用；生产路径不调用外部二进制）
        let out = std::process::Command::new("find")
            .arg(&root_str)
            .args(["-name", "*.app", "-maxdepth", "3", "-print0"])
            .output()
            .expect("系统 find 不可用");
        let expected: BTreeSet<String> = String::from_utf8_lossy(&out.stdout)
            .split('\0')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .filter(|p| {
                // 生产同款过滤：跳过嵌套在其它 .app 内部的条目
                match Path::new(p).parent() {
                    Some(parent) => {
                        let s = parent.to_string_lossy();
                        !(s.contains(".app/") || s.ends_with(".app"))
                    }
                    None => true,
                }
            })
            .map(String::from)
            .collect();

        let actual: BTreeSet<String> = find_app_bundles_maxdepth3(&root_str).into_iter().collect();
        assert_eq!(actual, expected);
    }

    /// 原生磁盘占用与系统 `du -skP` 的 KB 口径一致（含硬链接去重与符号链接不跟随）。
    #[test]
    fn du_kb_matches_system_du() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("sub/deep")).unwrap();
        std::fs::write(root.join("a.bin"), vec![7u8; 4096]).unwrap();
        std::fs::write(root.join("sub/b.bin"), vec![1u8; 10_000]).unwrap();
        std::fs::write(root.join("sub/deep/c.bin"), vec![2u8; 777]).unwrap();
        // 硬链接：du 只计一次
        std::fs::hard_link(root.join("a.bin"), root.join("a-link.bin")).unwrap();
        // 符号链接：du 不跟随，只计链接自身
        std::os::unix::fs::symlink(root.join("sub"), root.join("sub-link")).unwrap();

        let root_str = root.to_string_lossy().to_string();
        let out = std::process::Command::new("du")
            .args(["-skP", &root_str])
            .output()
            .expect("系统 du 不可用");
        let expected: u64 = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(du_kb_native(&root_str), expected);
    }

    /// plist 读取（bundle id / 可执行名 / 版本 / 布尔键）对齐真实系统应用。
    #[test]
    fn plist_readers_on_real_bundle() {
        let terminal = "/System/Applications/Utilities/Terminal.app";
        if !Path::new(terminal).exists() {
            return; // 非标准系统布局环境跳过
        }
        let plist = format!("{terminal}/Contents/Info.plist");
        assert_eq!(
            plist_string_key(&plist, "CFBundleIdentifier").as_deref(),
            Some("com.apple.Terminal")
        );
        assert_eq!(read_bundle_id(terminal), "com.apple.Terminal");
        assert_eq!(read_bundle_executable(terminal), "Terminal");
        assert!(!read_short_version(terminal).is_empty());
        assert!(!plist_bool_key(&plist, "LSBackgroundOnly"));
    }

    /// 缺失 plist：bundle id 回退 "unknown"，可执行名/版本为空，布尔键为 false。
    #[test]
    fn plist_readers_on_missing_bundle() {
        let missing = "/nonexistent-mole-probe-9f3a/Nowhere.app";
        assert_eq!(read_bundle_id(missing), "unknown");
        assert_eq!(read_bundle_executable(missing), "");
        assert_eq!(read_short_version(missing), "");
        assert!(!plist_bool_key(
            &format!("{missing}/Contents/Info.plist"),
            "LSBackgroundOnly"
        ));
    }

    /// 真实机器冒烟：原生管线（find 等价物 + plist + Spotlight 大小 + du 兜底）
    /// 对前几个真实 .app 能跑通且不 panic；数值不做逐项硬断言。
    /// 注：不调 mole_list_apps——其运行中索引走主线程 dispatch，测试 harness 不排空主队列。
    #[test]
    fn native_pipeline_smoke_on_real_apps() {
        let apps = find_app_bundles_maxdepth3("/Applications");
        if apps.is_empty() {
            return; // 无 /Applications 内容的环境跳过
        }
        let sample: Vec<String> = apps.iter().take(5).cloned().collect();
        let logical = batch_logical_sizes(&sample);
        let physical = batch_physical_sizes(&sample);

        let mut with_bundle = 0usize;
        let mut with_size = 0usize;
        for app in &sample {
            let bid = read_bundle_id(app);
            let exec = read_bundle_executable(app);
            let ver = read_short_version(app);
            let logical_b = logical.get(app).copied().unwrap_or(0);
            let physical_b = physical.get(app).copied().unwrap_or(0);
            eprintln!(
                "[probe] {app} bundle={bid} exec={exec} ver={ver} logical={logical_b} physical={physical_b}"
            );
            if bid != "unknown" && !bid.is_empty() {
                with_bundle += 1;
            }
            if logical_b > 0 || physical_b > 0 {
                with_size += 1;
            }
        }
        assert!(with_bundle > 0, "真实应用应至少有一个可读到 bundle id");
        assert!(with_size > 0, "真实应用应至少有一个可算出大小");
    }
}
