//! 应用清单扫描引擎 — App 发现、元数据缓存、体积/时间/展示辅助。
//! 自 `controllers/uninstall.rs` 纯搬迁（行为不变，含单元测试）。

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

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
    /// 更新机制来源（本地零网络检测）：
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

/// `mole_list_apps` 的阻塞主体（原命令函数体）。
pub fn list_apps_blocking() -> Result<Vec<AppListEntry>, String> {
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
    crate::core::bundle_id_anchor::plist_string_key(Path::new(&plist), "CFBundleExecutable")
        .unwrap_or_default()
}

fn same_file(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
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
pub fn precompute_file_sizes(all_paths: &str) -> HashMap<String, u64> {
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
    // CLI L612: `defaults read <plist> CFBundleIdentifier` 的原生替代
    crate::core::bundle_id_anchor::read_bundle_id_of_app(Path::new(app_path))
        .unwrap_or_else(|| "unknown".to_string())
}

pub fn read_short_version(app_path: &str) -> String {
    let plist = format!("{}/Contents/Info.plist", app_path);
    if !Path::new(&plist).is_file() {
        return String::new();
    }
    // 原 `plutil -extract CFBundleShortVersionString raw -o -`
    crate::core::bundle_id_anchor::plist_string_key(Path::new(&plist), "CFBundleShortVersionString")
        .unwrap_or_default()
}

pub fn resolve_display_name(app_path: &str, app_name: &str) -> String {
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
    let bundle_display_name =
        crate::core::bundle_id_anchor::plist_string_key(Path::new(&plist), "CFBundleDisplayName")
            .unwrap_or_default();
    let bundle_name =
        crate::core::bundle_id_anchor::plist_string_key(Path::new(&plist), "CFBundleName")
            .unwrap_or_default();

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

pub fn read_last_used_date(app_path: &str) -> i64 {
    // 原 `mdls -name kMDItemLastUsedDate -raw` + `date -j -f ... +%s`，改原生 Spotlight 查询
    crate::platform::macos_mditem::last_used_epoch(app_path).unwrap_or(0)
}

pub fn relative_time_from_epoch(value_epoch: i64) -> String {
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

pub fn derive_file_type(path: &str) -> &str {
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

pub fn is_path_sensitive(path: &str) -> bool {
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
            crate::core::bundle_id_anchor::plist_string_key(
                Path::new(&plist),
                "CFBundleIdentifier"
            )
            .as_deref(),
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
