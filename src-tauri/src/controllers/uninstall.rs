use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
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

    // CLI L522-575: for each app_dir, find -name *.app -maxdepth 3 -print0
    for dir in &app_dirs {
        if !Path::new(dir).is_dir() {
            continue;
        }
        let output = match Command::new("find")
            .arg(dir)
            .args(["-name", "*.app"])
            .args(["-maxdepth", "3"])
            .arg("-print0")
            .output()
        {
            Ok(o) => o,
            Err(_) => continue,
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        for raw_path in stdout.split('\0') {
            let app_path = raw_path.trim().to_string();
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

    // CLI Pass 2: process_app_metadata (L600-630)
    // filter by protect / bg_only, resolve display_name, then collect du/mdls
    // 并行处理每个 app 的 metadata 查询（defaults read、LSBackgroundOnly 等子进程调用）
    let qualified: Vec<(String, String, String, u64)> = app_data_tuples
        .par_iter()
        .filter_map(|(app_path, app_name, app_mtime)| {
            // CLI L609-615: defaults read CFBundleIdentifier
            let bundle_id = read_bundle_id_cli(app_path);

            // CLI L617-619: should_protect_from_uninstall
            if crate::core::app_protection::should_protect_from_uninstall(&bundle_id) {
                return None;
            }

            // CLI L621-628: LSBackgroundOnly check
            // OneDrive exemption (L629-643): top-level OneDrive.app is background-only
            // but the user explicitly installed it and should be able to uninstall it.
            let plist = format!("{}/Contents/Info.plist", app_path);
            if Path::new(&plist).is_file() {
                let bg_only = Command::new("defaults")
                    .args(["read", &plist, "LSBackgroundOnly"])
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();
                if bg_only == "1" || bg_only == "YES" || bg_only == "true" {
                    // New CLI: OneDrive exemption — skip LSBackgroundOnly filter
                    // for top-level OneDrive.app bundles.
                    let od = bundle_id.starts_with("com.microsoft.OneDrive")
                        && (app_path == "/Applications/OneDrive.app"
                            || app_path == &format!("{}/Applications/OneDrive.app", home));
                    if !od {
                        return None;
                    }
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

    // Batch get sizes for cold paths: mdls kMDItemLogicalSize → du fallback.
    if !cold_paths.is_empty() {
        let mut cold_sizes = batch_mdls_size_multi(&cold_paths);
        let du_needed: Vec<String> = cold_paths
            .iter()
            .filter(|p| cold_sizes.get(p.as_str()).copied().unwrap_or(0) == 0)
            .cloned()
            .collect();
        if !du_needed.is_empty() {
            for (k, v) in batch_du_sizes_multi(&du_needed) {
                cold_sizes.insert(k, v);
            }
        }
        for p in &cold_paths {
            sizes.insert(p.clone(), cold_sizes.get(p.as_str()).copied().unwrap_or(0));
        }
    }

    // CLI L237-245: batch mdls kMDItemLastUsedDate（仅冷行）
    if !cold_paths.is_empty() {
        if let Ok(out) = Command::new("mdls")
            .arg("-raw")
            .arg("-name")
            .arg("kMDItemLastUsedDate")
            .args(&cold_paths)
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            for (path, val) in cold_paths.iter().zip(text.split('\0')) {
                let v = val.trim();
                if v != "(null)" && !v.is_empty() {
                    if let Some(epoch) = parse_mdls_date(v) {
                        last_used_map.insert(path.clone(), epoch);
                    }
                }
            }
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
            let exec_name = read_bundle_executable_cli(path);
            let running = if !exec_name.is_empty() {
                pgrep_exact(&exec_name)
            } else {
                pgrep_exact(&name)
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

fn pgrep_exact(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    Command::new("pgrep")
        .args(["-x", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn read_bundle_executable_cli(app_path: &str) -> String {
    let plist = format!("{app_path}/Contents/Info.plist");
    if !Path::new(&plist).is_file() {
        return String::new();
    }
    Command::new("defaults")
        .args(["read", &plist, "CFBundleExecutable"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn same_file(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

/// Batch mdls -name kMDItemLogicalSize -raw for multiple .app paths.
/// Returns size in bytes (KB-aligned, matching CLI behavior).
fn batch_mdls_size_multi(paths: &[String]) -> HashMap<String, u64> {
    let mut result = HashMap::new();
    if paths.is_empty() {
        return result;
    }
    let output = match Command::new("mdls")
        .arg("-raw")
        .arg("-name")
        .arg("kMDItemLogicalSize")
        .args(paths)
        .output()
    {
        Ok(o) => o,
        Err(_) => return result,
    };
    if !output.status.success() {
        return result;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for (path, val) in paths.iter().zip(text.split('\0')) {
        let v = val.trim();
        if v == "(null)" || v.is_empty() {
            continue;
        }
        if let Ok(mdls_bytes) = v.parse::<u64>() {
            if mdls_bytes > 0 {
                let kb = mdls_bytes / 1024;
                result.insert(path.clone(), kb.saturating_mul(1024));
            }
        }
    }
    result
}

/// Batch du -skP for multiple paths. Returns size in bytes for each path.
fn batch_du_sizes_multi(paths: &[String]) -> HashMap<String, u64> {
    let mut result = HashMap::new();
    if paths.is_empty() {
        return result;
    }
    let output = match Command::new("du").args(["-skP"]).args(paths).output() {
        Ok(o) => o,
        Err(_) => return result,
    };
    // Best-effort: du may exit non-zero for some unreadable descendants but still
    // produce valid per-path output on the lines it could process.
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        // du 输出 "KB<TAB>path"，路径本身可能含空格（如 Application Support），
        // 只能按第一个空白拆一次：左边是 size，右边整体是路径。
        let line = line.trim_start();
        if line.is_empty() {
            continue;
        }
        let Some((kb_str, path)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        if let Ok(kb) = kb_str.parse::<u64>() {
            result.insert(path.to_string(), kb.saturating_mul(1024));
        }
    }
    result
}

/// Pre-compute file sizes for a newline-separated list of paths.
/// Files and symlinks use fast lstat, directories use one batch `du -skP`.
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
        let dir_sizes = batch_du_sizes_multi(&dir_paths);
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

fn read_bundle_id_cli(app_path: &str) -> String {
    let plist = format!("{}/Contents/Info.plist", app_path);
    if !Path::new(&plist).is_file() {
        return "unknown".to_string();
    }
    // CLI L612: defaults read
    Command::new("defaults")
        .args(["read", &plist, "CFBundleIdentifier"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn parse_mdls_date(date_str: &str) -> Option<i64> {
    Command::new("date")
        .args(["-j", "-f", "%Y-%m-%d %H:%M:%S %z", date_str, "+%s"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .to_string()
                .parse::<i64>()
                .ok()
        })
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
    Command::new("plutil")
        .args([
            "-extract",
            "CFBundleShortVersionString",
            "raw",
            "-o",
            "-",
            &plist,
        ])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default()
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

    let user_lc_all = std::env::var("LC_ALL").ok();
    let user_lang = std::env::var("LANG").ok();

    let mut md_cmd = Command::new("mdls");
    md_cmd.args(["-name", "kMDItemDisplayName", "-raw", app_path]);

    if let Some(ref lc_all) = user_lc_all {
        if !lc_all.is_empty() {
            md_cmd.env("LC_ALL", lc_all);
            if let Some(ref lang) = user_lang {
                if !lang.is_empty() {
                    md_cmd.env("LANG", lang);
                }
            }
        }
    } else if let Some(ref lang) = user_lang {
        if !lang.is_empty() {
            md_cmd.env("LANG", lang);
        }
    }

    let md_display_name = md_cmd
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let bundle_display_name = Command::new("plutil")
        .args(["-extract", "CFBundleDisplayName", "raw", "-o", "-", &plist])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let bundle_name = Command::new("plutil")
        .args(["-extract", "CFBundleName", "raw", "-o", "-", &plist])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
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

fn read_last_used_date(app_path: &str) -> i64 {
    Command::new("mdls")
        .args(["-name", "kMDItemLastUsedDate", "-raw", app_path])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() || s == "(null)" {
                return None;
            }
            let output = Command::new("date")
                .args(["-j", "-f", "%Y-%m-%d %H:%M:%S %z", &s, "+%s"])
                .output()
                .ok()
                .filter(|o| o.status.success())?;
            let ts = String::from_utf8_lossy(&output.stdout).trim().to_string();
            ts.parse::<i64>().ok()
        })
        .unwrap_or(0)
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
