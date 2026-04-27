//! 与 `Mole/cmd/analyze/json.go` 对齐的 JSON 模式（字段名 / `omitempty` 与 Go 一致）。

use super::cache;
use super::cleanable;
use super::constants::max_concurrent_overview;
use super::heap::{DirEntry, FileEntry};
use super::scanner;

use chrono::{DateTime, TimeZone, Utc};
use crossbeam_channel::bounded;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::AtomicI64;
use std::time::SystemTime;

/// Go `jsonOutput`
#[derive(Debug, Serialize)]
pub struct JsonOutput {
    pub path: String,
    pub overview: bool,
    pub entries: Vec<JsonEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty", rename = "large_files")]
    pub large_files: Vec<JsonFileEntry>,
    #[serde(rename = "total_size")]
    pub total_size: i64,
    #[serde(skip_serializing_if = "skip_zero_i64", rename = "total_files")]
    pub total_files: i64,
}

fn skip_zero_i64(v: &i64) -> bool {
    *v == 0
}

/// Go `jsonEntry`
#[derive(Debug, Serialize)]
pub struct JsonEntry {
    pub name: String,
    pub path: String,
    pub size: i64,
    #[serde(rename = "is_dir")]
    pub is_dir: bool,
    #[serde(skip_serializing_if = "is_false", rename = "insight")]
    pub insight: bool,
    #[serde(skip_serializing_if = "is_false", rename = "cleanable")]
    pub cleanable: bool,
    #[serde(skip_serializing_if = "is_false", rename = "protected")]
    pub protected: bool,
    #[serde(skip_serializing_if = "str::is_empty", rename = "last_access")]
    pub last_access: String,
    // ── GUI 扩展（V2 副标题数据源，Go CLI 契约无这些字段；零值省略保持输出紧凑）──
    /// symlink 标记 — Go CLI 靠 name 的 " →" 后缀隐式表达，GUI 用显式布尔
    #[serde(skip_serializing_if = "is_false", rename = "is_symlink")]
    pub is_symlink: bool,
    /// 目录直接子项统计（文件 / 目录 / 符号链接数），对标 lemon-cleaner 的 "X items"
    #[serde(skip_serializing_if = "skip_zero_i64", rename = "child_files")]
    pub child_files: i64,
    #[serde(skip_serializing_if = "skip_zero_i64", rename = "child_dirs")]
    pub child_dirs: i64,
    #[serde(skip_serializing_if = "skip_zero_i64", rename = "child_links")]
    pub child_links: i64,
    /// bundle 叶子捷径标记 — 首扫经 Spotlight 聚合大小叶子化（对齐柠檬
    /// specialFileExtensions），钻取时按需子树扫描；前端据此渲染 📦 badge 与骨架屏
    #[serde(skip_serializing_if = "is_false", rename = "is_bundle_leaf")]
    pub is_bundle_leaf: bool,
    #[serde(skip_serializing_if = "Option::is_none", rename = "bundle_id")]
    pub bundle_id: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "bundle_display_name"
    )]
    pub bundle_display_name: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Go `jsonFileEntry`
#[derive(Debug, Serialize, Clone)]
pub struct JsonFileEntry {
    pub name: String,
    pub path: String,
    pub size: i64,
}

// --- Go `main.go` createOverviewEntries 的 V2 精简版 ---
// V1/Go 原版返回 Home / User Library / Applications / System Library + insights 清理项（最多 18 项），
// 供 CLI 与旧 GUI 的「目录列表」使用；V2 GUI 的 LocationSelector 已固定为
// 「根目录 + 用户主目录 + 选择文件夹」三选项，这里仅返回前两项，让统计文案与 UI 对齐
// （第三项是前端交互动作，不属于数据项）。

fn create_overview_entries() -> Vec<DirEntry> {
    let mut entries = Vec::new();
    // 根目录：固定展示名 Macintosh HD（与 LocationSelector 根选项一致，对齐 CleanMyMac 标准显示）
    entries.push(DirEntry {
        name: "Macintosh HD".into(),
        path: "/".into(),
        is_dir: true,
        size: -1,
        last_access: None,
        is_symlink: false,
        child_files: 0,
        child_dirs: 0,
        child_links: 0,
        is_bundle_leaf: false,
        bundle_id: None,
        bundle_display_name: None,
    });
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        // 用户主目录：name 取文件夹名（如 liuy），与 LocationSelector 主目录选项一致
        let name = PathBuf::from(&home)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Home")
            .to_string();
        entries.push(DirEntry {
            name,
            path: home,
            is_dir: true,
            size: -1,
            last_access: None,
            is_symlink: false,
            child_files: 0,
            child_dirs: 0,
            child_links: 0,
            is_bundle_leaf: false,
            bundle_id: None,
            bundle_display_name: None,
        });
    }
    entries
}

/// 根目录卷的容量快照 (used, total)（bytes）：`statfs('/')`，毫秒级。
/// 非 macOS 或 statfs 失败返回 `None`。供扫描进度字节估算使用
///（与 DiskStatus.free 单一事实来源同口径：used = total - free）。
#[cfg(target_os = "macos")]
pub(crate) fn root_disk_usage() -> Option<(i64, i64)> {
    use std::ffi::CString;
    let path = CString::new("/").ok()?;
    let mut s = unsafe { std::mem::zeroed::<libc::statfs>() };
    if unsafe { libc::statfs(path.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let total = (s.f_blocks as u64).saturating_mul(s.f_bsize as u64);
    let free = (s.f_bfree as u64).saturating_mul(s.f_bsize as u64);
    let used = total.saturating_sub(free);
    Some((used as i64, total as i64))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn root_disk_usage() -> Option<(i64, i64)> {
    None
}

/// 根目录卷的已用容量（bytes）：`statfs('/')` 取 `(f_blocks - f_bfree) × f_bsize`，毫秒级。
/// 非 macOS 或 statfs 失败返回 `None`，由调用方回退递归测量。
pub(crate) fn root_disk_used_bytes() -> Option<i64> {
    root_disk_usage().map(|(used, _)| used)
}

fn system_time_to_utc(st: SystemTime) -> DateTime<Utc> {
    let d = st.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    Utc.timestamp_opt(d.as_secs() as i64, d.subsec_nanos())
        .single()
        .unwrap_or_else(|| Utc::now())
}

fn system_time_rfc3339(t: SystemTime) -> String {
    system_time_to_utc(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Go `jsonEntriesFromDirEntries`
pub fn json_entries_from_dir_entries(entries: &[DirEntry]) -> Vec<JsonEntry> {
    let mut output = Vec::with_capacity(entries.len());
    for entry in entries {
        // 旧磁盘缓存兼容：scanner 曾给 symlink 的 name 拼接 " →" 后缀（Go CLI 展示约定），
        // 现已改为显式 is_symlink 标记；读到旧缓存时在此清洗，缓存过期重写后自然消失。
        let (name, is_symlink) = if entry.name.ends_with(" →") {
            (entry.name.trim_end_matches(" →").to_string(), true)
        } else {
            (entry.name.clone(), entry.is_symlink)
        };
        let mut item = JsonEntry {
            name,
            path: entry.path.clone(),
            size: entry.size,
            is_dir: entry.is_dir,
            insight: false,
            cleanable: entry.is_dir && cleanable::is_cleanable_dir(&entry.path),
            protected: entry.is_dir && super::protected::is_protected_entry_path(&entry.path),
            last_access: String::new(),
            is_symlink,
            child_files: entry.child_files,
            child_dirs: entry.child_dirs,
            child_links: entry.child_links,
            is_bundle_leaf: entry.is_bundle_leaf,
            bundle_id: entry.bundle_id.clone(),
            bundle_display_name: entry.bundle_display_name.clone(),
        };
        if let Some(ts) = entry.last_access {
            item.last_access = system_time_rfc3339(ts);
        }
        output.push(item);
    }
    output
}

/// Go `jsonFileEntriesFromFileEntries`
pub fn json_file_entries_from_file_entries(files: &[FileEntry]) -> Vec<JsonFileEntry> {
    files
        .iter()
        .map(|f| JsonFileEntry {
            name: f.name.clone(),
            path: f.path.clone(),
            size: f.size,
        })
        .collect()
}

/// Go `measureOverviewEntriesForJSON` — 并发测量 overview 各条目大小，
/// 用 `max_concurrent_overview` 限制并发度，结果保持原始顺序。
///
/// `/` 直取 statfs 已用容量（毫秒级），其余走「缓存 → 递归」二档。
fn measure_overview_entries_for_json(overview_entries: &[DirEntry]) -> Vec<DirEntry> {
    let n = overview_entries.len();
    if n == 0 {
        return Vec::new();
    }

    // Semaphore: 预填充 max_concurrent_overview 个 token，控制并发数
    let (sem_tx, sem_rx) = bounded::<()>(max_concurrent_overview);
    for _ in 0..max_concurrent_overview {
        sem_tx.send(()).unwrap();
    }

    let (result_tx, result_rx) = bounded::<(usize, DirEntry)>(n);

    std::thread::scope(|s| {
        for (i, entry) in overview_entries.iter().enumerate() {
            let sem_rx = sem_rx.clone();
            let sem_tx = sem_tx.clone();
            let result_tx = result_tx.clone();
            let mut item = entry.clone();

            s.spawn(move || {
                // Acquire semaphore token
                sem_rx.recv().unwrap();

                // 根目录（/）：整盘语义，statfs 毫秒级直取已用容量，避免首次全盘递归扫描（10-30s）；
                // statfs 失败时回退递归测量（保持健壮）。
                let size_result: Result<i64, String> = if item.path == "/" {
                    match root_disk_used_bytes() {
                        Some(used) => Ok(used),
                        None => scanner::measure_overview_size(&item.path),
                    }
                } else {
                    match cache::load_overview_cached_size(&item.path) {
                        Ok(cached) if cached > 0 => Ok(cached),
                        _ => scanner::measure_overview_size(&item.path),
                    }
                };

                if let Ok(size) = size_result {
                    item.size = size;
                }

                let _ = result_tx.send((i, item));
                // Release semaphore token
                let _ = sem_tx.send(());
            });
        }
    });

    // 所有线程结束后关闭 result channel，收集结果
    drop(result_tx);
    let mut measured: Vec<Option<DirEntry>> = (0..n).map(|_| None).collect();
    for (i, entry) in result_rx {
        measured[i] = Some(entry);
    }
    measured.into_iter().map(|o| o.unwrap()).collect()
}

/// Go `performDirectoryScanForJSON`
pub fn perform_directory_scan_for_json(path: &str) -> JsonOutput {
    let t0 = std::time::Instant::now();

    let files_scanned = AtomicI64::new(0);
    let dirs_scanned = AtomicI64::new(0);
    let bytes_scanned = AtomicI64::new(0);
    let current_path = Mutex::new(String::new());

    let result = match scanner::scan_path_concurrent_all_entries(
        path,
        &files_scanned,
        &dirs_scanned,
        &bytes_scanned,
        Some(&current_path),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("failed to scan directory: {e}");
            std::process::exit(1);
        }
    };

    log::info!(
        "[scan] perform_directory_scan done: {path} in {:.1}s, entries={}, total_size={}",
        t0.elapsed().as_secs_f64(),
        result.entries.len(),
        result.total_size
    );

    JsonOutput {
        path: path.to_string(),
        overview: false,
        entries: json_entries_from_dir_entries(&result.entries),
        large_files: json_file_entries_from_file_entries(&result.large_files),
        total_size: result.total_size,
        total_files: result.total_files,
    }
}

/// Go `performOverviewScanForJSON`
pub fn perform_overview_scan_for_json(path: &str) -> JsonOutput {
    let overview_entries = create_overview_entries();

    let mut total_size: i64 = 0;
    let mut entries: Vec<DirEntry> = Vec::with_capacity(overview_entries.len());

    for entry in measure_overview_entries_for_json(&overview_entries) {
        // Match the TUI: omit scanned insight/tool entries that ended up empty.
        if entry.size == 0 {
            continue;
        }
        total_size = total_size.saturating_add(entry.size);
        entries.push(entry);
    }

    entries.sort_by(|a, b| b.size.cmp(&a.size));

    JsonOutput {
        path: path.to_string(),
        overview: true,
        entries: json_entries_from_dir_entries(&entries),
        large_files: Vec::new(),
        total_size,
        total_files: 0,
    }
}

/// Go `performScanForJSON`
pub fn perform_scan_for_json(path: &str, is_overview: bool) -> JsonOutput {
    if is_overview {
        perform_overview_scan_for_json(path)
    } else {
        perform_directory_scan_for_json(path)
    }
}

/// Go `runJSONMode`
pub fn run_json_mode(path: &str, is_overview: bool) {
    let result = perform_scan_for_json(path, is_overview);
    match serde_json::to_string_pretty(&result) {
        Ok(s) => print!("{s}\n"),
        Err(e) => {
            eprintln!("failed to encode JSON: {e}");
            std::process::exit(1);
        }
    }
}

/// GUI 友好版：与 `perform_directory_scan_for_json` 一致，但失败返回 `Err`，不 `process::exit`。
///
/// **与 Go CLI `scanCmd` 对齐的缓存策略（V2 快照模型）**：
/// 1. 新鲜快照（向上逐级查找，节点级 mtime 校验）→ 直接返回，零延迟
/// 2. 过期快照（扫描时间仍在 stale 窗口内）→ 先用旧数据返回
/// 3. 无快照 → 全量 bulkwalk 扫描，完成后异步保存单根快照
pub fn try_perform_directory_scan_for_json(path: &str) -> Result<JsonOutput, String> {
    try_perform_directory_scan_for_json_impl(path, false)
}

/// `try_perform_directory_scan_for_json` 的 `skip_cache` 变体。
/// `skip_cache=true` 时跳过快照读取并失效目标快照，强制全量扫描。
pub fn try_perform_directory_scan_for_json_impl(
    path: &str,
    skip_cache: bool,
) -> Result<JsonOutput, String> {
    let t0 = std::time::Instant::now();

    // skip_cache 时先失效目标快照，确保走全量扫描
    if skip_cache {
        cache::invalidate_cache_tree(path);
    }

    if !skip_cache {
        // 与 Go `scanCmd`/`loadCacheFromDisk` 对齐：优先新鲜快照（祖先向上查找）
        if let Ok(Some(hit)) = cache::find_fresh_snapshot(path) {
            if let Some(output) = build_output_from_snapshot(&hit, path) {
                log::info!(
                    "[scan] SNAPSHOT HIT: {path} (root={}), entries={}, size={}",
                    hit.root,
                    output.entries.len(),
                    output.total_size
                );
                return Ok(output);
            }
        }

        // 与 Go `scanCmd`/`loadStaleCacheFromDisk` 对齐：过期但在 stale 窗口内
        if let Ok(Some(hit)) = cache::find_stale_snapshot(path) {
            if let Some(output) = build_output_from_snapshot(&hit, path) {
                log::info!(
                    "[scan] STALE SNAPSHOT used: {path} (root={}), entries={}, size={}",
                    hit.root,
                    output.entries.len(),
                    output.total_size
                );
                return Ok(output);
            }
        }
    }

    // 无快照 → 全量 bulkwalk 扫描
    log::info!("[scan] no snapshot for {path}, starting full scan");

    let files_scanned = AtomicI64::new(0);
    let dirs_scanned = AtomicI64::new(0);
    let bytes_scanned = AtomicI64::new(0);
    let current_path = Mutex::new(String::new());

    let outcome = scanner::scan_subtree(
        path,
        &files_scanned,
        &dirs_scanned,
        &bytes_scanned,
        Some(&current_path),
    )
    .map_err(|e| format!("scan failed: {e}"))?;

    log::info!(
        "[scan] full scan done: {path} in {:.1}s, entries={}, large_files={}, total_size={}, total_files={}",
        t0.elapsed().as_secs_f64(),
        outcome.result.entries.len(),
        outcome.result.large_files.len(),
        outcome.result.total_size,
        outcome.result.total_files
    );

    // 扫描完成后异步保存快照，后续访问秒级返回。
    // 单根快照内部自洽（硬链接去重在同一快照内一致），去重子树也照常缓存。
    if let Ok(root_mod) = std::fs::metadata(path).and_then(|m| m.modified()) {
        let mod_time_secs = root_mod
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let snapshot = cache::DirSnapshot {
            schema_version: cache::CACHE_SCHEMA_VERSION,
            root: path.to_string(),
            mod_time_secs,
            scan_time: chrono::Utc::now(),
            total_size: outcome.result.total_size,
            total_files: outcome.result.total_files,
            large_files: outcome.result.large_files.clone(),
            nodes: outcome.nodes,
        };
        let path_owned = path.to_string();
        std::thread::spawn(move || {
            if let Err(e) = cache::save_snapshot_to_disk(&path_owned, &snapshot) {
                log::warn!("[json] snapshot save failed for {path_owned}: {e}");
            }
        });
    }

    Ok(JsonOutput {
        path: path.to_string(),
        overview: false,
        entries: json_entries_from_dir_entries(&outcome.result.entries),
        large_files: json_file_entries_from_file_entries(&outcome.result.large_files),
        total_size: outcome.result.total_size,
        total_files: outcome.result.total_files,
    })
}

/// 从快照命中构建查询目录的 JSON 输出。
///
/// V2 对齐 Lemon 展示口径：快照内全量明细（无折叠目录），任意层级直接命中；
/// 仅当查询路径不在快照内（如被排除目录的下钻目标）时返回 `None` 视为 miss。
fn build_output_from_snapshot(hit: &cache::SnapshotHit, path: &str) -> Option<JsonOutput> {
    let node = hit.snapshot.nodes.get(path)?;
    // bundle 叶子捷径：命中节点首扫时被叶子化（内部无子节点），且快照根不是该 bundle
    // 自身（尚未按需扫描）→ 视为 miss，调用方以 bundle 路径为根按需子树扫描并落独立根快照。
    if node.bundle_leaf && node.children.is_empty() && node.files.is_empty() && hit.root != path {
        return None;
    }
    let entries = scanner::entries_for_dir(&hit.snapshot.nodes, path);
    Some(JsonOutput {
        path: path.to_string(),
        overview: false,
        entries: json_entries_from_dir_entries(&entries),
        large_files: json_file_entries_from_file_entries(&hit.snapshot.large_files),
        total_size: node.size,
        total_files: node.total_files,
    })
}

/// GUI 友好版：与 `perform_overview_scan_for_json` 一致，但永不 `process::exit`。
pub fn try_perform_overview_scan_for_json(path: &str) -> Result<JsonOutput, String> {
    let t0 = std::time::Instant::now();
    let overview_entries = create_overview_entries();

    let mut total_size: i64 = 0;
    let mut entries: Vec<DirEntry> = Vec::with_capacity(overview_entries.len());

    for entry in measure_overview_entries_for_json(&overview_entries) {
        if entry.size == 0 {
            continue;
        }
        total_size = total_size.saturating_add(entry.size);
        entries.push(entry);
    }

    entries.sort_by(|a, b| b.size.cmp(&a.size));

    log::info!(
        "[scan] overview done in {:.1}s: {} entries, total_size={}",
        t0.elapsed().as_secs_f64(),
        entries.len(),
        total_size
    );

    Ok(JsonOutput {
        path: path.to_string(),
        overview: true,
        entries: json_entries_from_dir_entries(&entries),
        large_files: Vec::new(),
        total_size,
        total_files: 0,
    })
}

/// GUI 友好版：与 `perform_scan_for_json` 同语义的 GUI 友好版本（失败返回 `Err`）。
pub fn try_perform_scan_for_json(path: &str, is_overview: bool) -> Result<JsonOutput, String> {
    try_perform_scan_for_json_impl(path, is_overview, false)
}

/// `try_perform_scan_for_json` 的 `skip_cache` 变体。
/// `skip_cache=true` 时跳过磁盘缓存，强制全量扫描（仅对 directory scan 有效）。
pub fn try_perform_scan_for_json_impl(
    path: &str,
    is_overview: bool,
    skip_cache: bool,
) -> Result<JsonOutput, String> {
    if is_overview {
        try_perform_overview_scan_for_json(path)
    } else {
        try_perform_directory_scan_for_json_impl(path, skip_cache)
    }
}

// ── tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::analyze::scanner::DirNode;

    fn snapshot_with_bundle_node(leaf: bool, root: &str) -> cache::DirSnapshot {
        let mut nodes = std::collections::HashMap::new();
        nodes.insert(
            "/Apps/Foo.app".to_string(),
            DirNode {
                name: "Foo.app".into(),
                bundle_leaf: leaf,
                ..DirNode::default()
            },
        );
        cache::DirSnapshot {
            schema_version: cache::CACHE_SCHEMA_VERSION,
            root: root.to_string(),
            mod_time_secs: 0,
            scan_time: chrono::Utc::now(),
            total_size: 0,
            total_files: 0,
            large_files: Vec::new(),
            nodes,
        }
    }

    #[test]
    fn bundle_leaf_ancestor_hit_is_miss_until_on_demand_scan() {
        // 首扫叶子化（根=/）后钻取 bundle → miss（触发按需子树扫描）
        let hit = cache::SnapshotHit {
            root: "/".into(),
            snapshot: snapshot_with_bundle_node(true, "/"),
        };
        assert!(build_output_from_snapshot(&hit, "/Apps/Foo.app").is_none());

        // 按需扫描已落独立根快照（root == path）→ 命中秒开
        let hit2 = cache::SnapshotHit {
            root: "/Apps/Foo.app".into(),
            snapshot: snapshot_with_bundle_node(true, "/Apps/Foo.app"),
        };
        assert!(build_output_from_snapshot(&hit2, "/Apps/Foo.app").is_some());

        // 非叶子节点不受影响
        let hit3 = cache::SnapshotHit {
            root: "/".into(),
            snapshot: snapshot_with_bundle_node(false, "/"),
        };
        assert!(build_output_from_snapshot(&hit3, "/Apps/Foo.app").is_some());
    }
}
