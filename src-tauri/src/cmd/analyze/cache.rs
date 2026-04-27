//! 扫描快照缓存：每个扫描根目录一个 bincode 快照文件。
//!
//! V2 快照模型（替代旧版「每目录一个 hashed .cache JSON 文件」）：
//! - 一次扫描产出整棵子树的聚合节点 map（`scanner::DirNode`），扫描结束后由消费端
//!   一次性落盘；查询任意子目录时向上逐级查找祖先快照（P → parent(P) → … → `/`），
//!   命中即等价旧版「子目录缓存命中」，保住钻入目录秒开；
//! - 新鲜度按**节点级 mtime** 校验（快照内每目录记录扫描时的 mtime，对齐旧版
//!   「每个目录各自一份缓存、各自校验 mtime」的语义），配合 grace / reuse 窗口；
//! - 文件格式：`u64 LE total_files || bincode(DirSnapshot)`——头部 8 字节供
//!   `peek_cache_total_files` 零成本读取进度估算基准；
//! - overview 快照（`overview_sizes.json`）机制保持不变。

use super::constants::{
    analyzer_cache_ttl, cache_mod_time_grace, cache_reuse_window, overview_cache_file,
    overview_cache_keep_entries, overview_cache_max_entries, overview_cache_ttl,
    overview_refresh_divisor, stale_cache_ttl,
};
use super::heap::FileEntry;
use super::scanner::DirNode;

/// Go `cacheSchemaVersion`：缓存格式版本号。
/// v2: analyze 对硬链接去重，与 `du` 行为一致。
/// v3: 普通 Parallels VM 存储不再按名跳过，纳入扫描。
/// v4: V2 快照模型（单根目录一个 bincode 快照，替代每目录 JSON 缓存）。
/// v5: 对齐 Lemon 展示口径——移除折叠目录聚合，快照内全量文件明细。
/// v6: bundle 叶子捷径（DirNode 增 bundle_leaf/bundle_id/bundle_display_name/bundle_content_types）。
pub const CACHE_SCHEMA_VERSION: u32 = 6;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

/// 环境变量重写缓存目录，便于测试隔离 / 自定义部署位置。
/// 设置后，所有 `get_cache_dir()` 调用都用这个路径而非 `~/.cache/mole`。
#[allow(non_upper_case_globals)] // 与 `constants.rs` 风格一致：camelCase 常量
pub const cache_dir_env: &str = "MOLE_CACHE_DIR";

/// 单根目录扫描快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirSnapshot {
    pub schema_version: u32,
    pub root: String,
    /// 扫描时根目录的 mtime（Unix 秒）。
    pub mod_time_secs: i64,
    pub scan_time: DateTime<Utc>,
    pub total_size: i64,
    pub total_files: i64,
    /// 本次扫描范围内的 Top-N 大文件（查询任意子目录时复用，优于旧版子目录缓存的空列表）。
    pub large_files: Vec<FileEntry>,
    pub nodes: HashMap<String, DirNode>,
}

/// 快照命中结果：快照根 + 快照本体。
pub struct SnapshotHit {
    pub root: String,
    pub snapshot: DirSnapshot,
}

// ── 路径与读写 ──────────────────────────────────────────────────────────────

pub fn get_cache_dir() -> io::Result<PathBuf> {
    // Step 1: env override（测试隔离 / 自定义部署）
    if let Ok(custom) = std::env::var(cache_dir_env) {
        if !custom.is_empty() {
            let p = PathBuf::from(custom);
            fs::create_dir_all(&p)?;
            return Ok(p);
        }
    }
    // Step 2: 默认 ~/.cache/mole（与 Go getCacheDir 一致）
    let home = dirs::home_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home directory"))?;
    let d = home.join(".cache").join("mole");
    fs::create_dir_all(&d)?;
    Ok(d)
}

pub fn get_cache_path(path: &str) -> io::Result<PathBuf> {
    use std::hash::{Hash, Hasher};
    let cache_dir = get_cache_dir()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    let hash = hasher.finish();
    let filename = format!("{hash:x}.snap");
    Ok(cache_dir.join(filename))
}

/// 保存快照：`u64 LE total_files || bincode(DirSnapshot)`，POSIX 原子写（tmp + rename）。
pub fn save_snapshot_to_disk(path: &str, snapshot: &DirSnapshot) -> io::Result<()> {
    let cache_path = get_cache_path(path)?;
    let tmp_path = cache_path.with_extension("tmp");

    let file = fs::File::create(&tmp_path)?;
    // bincode serialize_into 对裸 File 是无缓冲直写（百万级微小 write syscall）；
    // 包 1MB BufWriter 把 syscall 次数降到几十次。
    let mut file = std::io::BufWriter::with_capacity(1024 * 1024, file);
    use std::io::Write;
    file.write_all(&(snapshot.total_files.max(0) as u64).to_le_bytes())?;
    bincode::serialize_into(&mut file, snapshot)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("bincode encode: {e}")))?;
    file.flush()?;
    drop(file);
    fs::rename(&tmp_path, &cache_path)?;
    Ok(())
}

/// 读取快照（不做 freshness 校验；schema 不匹配 / 缺头拒绝）。
pub fn load_snapshot_raw(path: &str) -> io::Result<DirSnapshot> {
    let cache_path = get_cache_path(path)?;
    let bytes = fs::read(&cache_path)?;
    if bytes.len() < 8 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "snapshot too short",
        ));
    }
    let snapshot: DirSnapshot = bincode::deserialize(&bytes[8..]).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("snapshot decode failed: {e}"),
        )
    })?;
    if snapshot.schema_version != CACHE_SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "snapshot schema mismatch: got {}, want {}",
                snapshot.schema_version, CACHE_SCHEMA_VERSION
            ),
        ));
    }
    Ok(snapshot)
}

/// 读快照头部的 `total_files`（8 字节 LE），跳过 freshness 校验。
/// 用于初始扫描的进度估计（controller 在发起扫描前调用）。
pub fn peek_cache_total_files(path: &str) -> io::Result<i64> {
    let cache_path = get_cache_path(path)?;
    let mut file = fs::File::open(&cache_path)?;
    use std::io::Read;
    let mut buf = [0u8; 8];
    file.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf) as i64)
}

// ── 新鲜度判定与祖先查询 ────────────────────────────────────────────────────

/// 节点级新鲜度（对齐旧版 `load_cache_from_disk` 语义，只是 mtime 来源从
/// 「该目录自己的缓存文件」变为「快照内节点」）：
/// - 扫描时间超过 TTL → 过期；
/// - 目录 mtime 未变 → 新鲜；
/// - mtime 变化但在 grace 窗口（30min）内 → 仍新鲜；
/// - 超出 grace 但扫描时间仍在 reuse 窗口（1 天）内 → 仍新鲜；
/// - 否则过期。
fn node_is_fresh(path: &str, node: &DirNode, scan_time: DateTime<Utc>) -> bool {
    let age = Utc::now().signed_duration_since(scan_time);
    let ttl = chrono::Duration::from_std(analyzer_cache_ttl).unwrap_or_default();
    if age > ttl {
        return false;
    }

    let cur_mtime = match fs::metadata(path) {
        Ok(m) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                m.mtime()
            }
            #[cfg(not(unix))]
            {
                0
            }
        }
        Err(_) => return false, // 目录已消失 → 视为过期
    };

    if cur_mtime <= node.mtime_secs {
        return true;
    }
    let diff = cur_mtime - node.mtime_secs;
    let grace = cache_mod_time_grace.as_secs() as i64;
    if grace == 0 || diff <= grace {
        return true;
    }
    let reuse = chrono::Duration::from_std(cache_reuse_window).unwrap_or_default();
    age <= reuse
}

/// 向上逐级查找「新鲜」快照（P → parent(P) → … → `/`），首个含 P 节点且
/// 节点级新鲜度通过的快照命中。
pub fn find_fresh_snapshot(path: &str) -> io::Result<Option<SnapshotHit>> {
    let mut current = PathBuf::from(path);
    loop {
        let cur = current.to_string_lossy().into_owned();
        if let Ok(snap) = load_snapshot_raw(&cur) {
            if let Some(node) = snap.nodes.get(path) {
                if node_is_fresh(path, node, snap.scan_time) {
                    return Ok(Some(SnapshotHit {
                        root: cur,
                        snapshot: snap,
                    }));
                }
            }
        }
        if !current.pop() {
            break;
        }
    }
    Ok(None)
}

/// 向上逐级查找「可复用旧数据」快照（对齐旧版 `loadStaleCacheFromDisk`：
/// 扫描时间在 stale 窗口内即可，不做 mtime 校验）。
pub fn find_stale_snapshot(path: &str) -> io::Result<Option<SnapshotHit>> {
    let mut current = PathBuf::from(path);
    loop {
        let cur = current.to_string_lossy().into_owned();
        if let Ok(snap) = load_snapshot_raw(&cur) {
            let age = Utc::now().signed_duration_since(snap.scan_time);
            let stale = chrono::Duration::from_std(stale_cache_ttl).unwrap_or_default();
            if age <= stale && snap.nodes.contains_key(path) {
                return Ok(Some(SnapshotHit {
                    root: cur,
                    snapshot: snap,
                }));
            }
        }
        if !current.pop() {
            break;
        }
    }
    Ok(None)
}

// ── 失效与清理 ──────────────────────────────────────────────────────────────

pub fn invalidate_cache(path: &str) {
    if let Ok(cache_path) = get_cache_path(path) {
        let _ = fs::remove_file(cache_path);
    }
    remove_overview_snapshot(path);
}

/// 从指定路径向上逐级失效祖先目录的快照，直到根目录 `/`。
/// 用于删除操作后确保导航回任意上一级都不会读到过期快照。
pub fn invalidate_cache_ancestors(path: &str) {
    let mut current = PathBuf::from(path);
    loop {
        let cur_str = current.to_string_lossy().to_string();
        invalidate_cache(&cur_str);
        if !current.pop() {
            break;
        }
    }
}

/// 强制刷新前的失效：删除目标路径自身的快照（子目录快照为独立扫描根，
/// 由各自的生命周期管理，无需级联删除）。
pub fn invalidate_cache_tree(path: &str) {
    invalidate_cache(path);
}

/// Go `pruneAnalyzerCache`：清理过期缓存文件。
/// 覆盖 `.snap`（V2 快照）与遗留 `.cache`（旧版每目录 JSON），跳过符号链接。
pub fn prune_analyzer_cache() {
    let cache_dir = match get_cache_dir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let _ = prune_analyzer_cache_dir(&cache_dir);
}

fn prune_analyzer_cache_dir(cache_dir: &std::path::Path) -> io::Result<()> {
    let ttl = analyzer_cache_ttl;
    if ttl.is_zero() {
        return Ok(());
    }
    let now = SystemTime::now();
    let cutoff = now - ttl;

    let entries = fs::read_dir(cache_dir)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_symlink() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "snap" && ext != "cache" {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) if m.is_file() => m,
            _ => continue,
        };
        if meta.modified().map(|m| m >= cutoff).unwrap_or(true) {
            continue;
        }
        let _ = fs::remove_file(&path);
    }
    Ok(())
}

// ── overview 快照（机制与旧版一致） ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverviewSizeSnapshot {
    pub size: i64,
    pub updated: DateTime<Utc>,
    /// Go `SchemaVersion`：旧 schema 快照在加载时被清理（旧 JSON 缺字段 → 0）
    #[serde(default)]
    pub schema_version: u32,
}

static OVERVIEW_SNAPSHOT: Mutex<Option<HashMap<String, OverviewSizeSnapshot>>> = Mutex::new(None);

/// 「caller-holds-lock」语义：调用方已持有 `OVERVIEW_SNAPSHOT` 的 `MutexGuard`，把
/// 内层 `Option<HashMap>` 传进来；本函数**不会**再次 `.lock()`。
fn ensure_overview_snapshot_cache_loaded(
    snapshot: &mut Option<HashMap<String, OverviewSizeSnapshot>>,
) -> io::Result<()> {
    if snapshot.is_some() {
        return Ok(());
    }
    let store_path = get_overview_size_store_path()?;
    let data = match fs::read_to_string(&store_path) {
        Ok(d) => d,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            *snapshot = Some(HashMap::new());
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    if data.trim().is_empty() {
        *snapshot = Some(HashMap::new());
        return Ok(());
    }
    let mut parsed: HashMap<String, OverviewSizeSnapshot> =
        match serde_json::from_str::<HashMap<String, OverviewSizeSnapshot>>(&data) {
            Ok(m) => m,
            _ => {
                let backup_path = store_path.with_extension("corrupt");
                let _ = fs::rename(&store_path, &backup_path);
                HashMap::new()
            }
        };
    // Go cache.go:116-124：加载时丢弃 schema 不匹配 / size<=0 / 过期的快照——
    // 否则每个浏览过的目录都永久留在文件里，每次 save 全量重写（7cf9e382）。
    let ttl =
        chrono::Duration::from_std(overview_cache_ttl).unwrap_or_else(|_| chrono::Duration::zero());
    let now = Utc::now();
    parsed.retain(|_, s| {
        s.schema_version == CACHE_SCHEMA_VERSION && s.size > 0 && now - s.updated < ttl
    });
    *snapshot = Some(parsed);
    Ok(())
}

pub fn get_overview_size_store_path() -> io::Result<PathBuf> {
    Ok(get_cache_dir()?.join(overview_cache_file))
}

pub fn load_stored_overview_size(path: &str) -> Result<i64, String> {
    if path.is_empty() {
        return Err("empty path".into());
    }
    let mut guard = OVERVIEW_SNAPSHOT.lock().map_err(|e| e.to_string())?;
    ensure_overview_snapshot_cache_loaded(&mut guard).map_err(|e| e.to_string())?;
    let map = guard
        .as_ref()
        .ok_or_else(|| "snapshot cache unavailable".to_string())?;
    if let Some(snapshot) = map.get(path) {
        if snapshot.size > 0 {
            let ttl = chrono::Duration::from_std(overview_cache_ttl)
                .unwrap_or_else(|_| chrono::Duration::zero());
            if Utc::now() - snapshot.updated < ttl {
                return Ok(snapshot.size);
            }
            return Err("snapshot expired".into());
        }
    }
    Err("snapshot not found".into())
}

pub fn store_overview_size(path: &str, size: i64) -> Result<(), String> {
    if path.is_empty() || size <= 0 {
        return Err("invalid overview size".into());
    }
    let mut guard = OVERVIEW_SNAPSHOT.lock().map_err(|e| e.to_string())?;
    ensure_overview_snapshot_cache_loaded(&mut guard).map_err(|e| e.to_string())?;
    let map = guard
        .as_mut()
        .ok_or_else(|| "snapshot cache unavailable".to_string())?;
    // Go cache.go:171-178：目录重测通常返回相同 size，而每次 save 全量重写整个
    // 文件。记录值未变时跳过重写；时间戳只需在 TTL/refreshDivisor 内刷新一次。
    let ttl =
        chrono::Duration::from_std(overview_cache_ttl).unwrap_or_else(|_| chrono::Duration::zero());
    if let Some(existing) = map.get(path) {
        if existing.size == size
            && existing.schema_version == CACHE_SCHEMA_VERSION
            && Utc::now() - existing.updated < ttl / overview_refresh_divisor
        {
            return Ok(());
        }
    }
    map.insert(
        path.to_string(),
        OverviewSizeSnapshot {
            size,
            updated: Utc::now(),
            schema_version: CACHE_SCHEMA_VERSION,
        },
    );
    evict_overview_snapshots(map);
    persist_overview_snapshot(map).map_err(|e| e.to_string())
}

/// Go `evictOverviewSnapshotsLocked`：超出上限时按 updated 排序，一次删到低水位线
/// （900），避免后续每次 save 都触发淘汰。
fn evict_overview_snapshots(map: &mut HashMap<String, OverviewSizeSnapshot>) {
    if map.len() <= overview_cache_max_entries || overview_cache_keep_entries >= map.len() {
        return;
    }
    let mut aged: Vec<(String, DateTime<Utc>)> =
        map.iter().map(|(p, s)| (p.clone(), s.updated)).collect();
    aged.sort_by_key(|(_, u)| *u);
    let drop_count = map.len() - overview_cache_keep_entries;
    for (p, _) in aged.into_iter().take(drop_count) {
        map.remove(&p);
    }
}

/// 「caller-holds-lock」语义：调用方已锁 OVERVIEW_SNAPSHOT 并取出 `&HashMap`，
/// 直接传进来落盘；本函数不会再次 lock。
fn persist_overview_snapshot(snapshot: &HashMap<String, OverviewSizeSnapshot>) -> io::Result<()> {
    let store_path = get_overview_size_store_path()?;
    let tmp_path = store_path.with_extension("tmp");
    let data = serde_json::to_string_pretty(snapshot)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    fs::write(&tmp_path, data)?;
    fs::rename(&tmp_path, &store_path)?;
    Ok(())
}

/// overview 测量回退路径：先查 overview 快照，再查扫描快照（该目录曾作为扫描根）。
pub fn load_overview_cached_size(path: &str) -> Result<i64, String> {
    if path.is_empty() {
        return Err("empty path".into());
    }
    if let Ok(snapshot) = load_stored_overview_size(path) {
        return Ok(snapshot);
    }
    if let Ok(Some(hit)) = find_fresh_snapshot(path) {
        if let Some(node) = hit.snapshot.nodes.get(path) {
            if node.size > 0 {
                let _ = store_overview_size(path, node.size);
                return Ok(node.size);
            }
        }
    }
    Err("overview size not found".into())
}

pub fn remove_overview_snapshot(path: &str) {
    if path.is_empty() {
        return;
    }
    let mut guard = match OVERVIEW_SNAPSHOT.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if ensure_overview_snapshot_cache_loaded(&mut guard).is_err() {
        return;
    }
    if let Some(map) = guard.as_mut() {
        if map.contains_key(path) {
            map.remove(path);
            let _ = persist_overview_snapshot(map);
        }
    }
}

// ============================================================================
// 快照读写单元测试
//
// 设计要点（沿用旧版约定）：
// - 所有测试通过 `MOLE_CACHE_DIR` 环境变量重定向缓存目录到 TempDir，避免污染 ~/.cache。
// - env var 是进程全局状态，cargo test 默认并行 → 用 `CACHE_TEST_LOCK` 串行化。
// - 不依赖 `tempfile` crate（项目未引入），自实现迷你 TempDir。
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use std::time::Duration;

    fn cache_test_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let idx = COUNTER.fetch_add(1, Ordering::SeqCst);
            let base = std::env::temp_dir()
                .join(format!("mole_snap_test_{tag}_{}_{idx}", std::process::id()));
            fs::create_dir_all(&base).expect("mkdir tmpdir");
            TempDir(base)
        }
        fn path(&self) -> &PathBuf {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct CacheGuard {
        _lock: MutexGuard<'static, ()>,
        prev: Option<String>,
        _dir: TempDir,
    }
    impl CacheGuard {
        fn new(tag: &str) -> Self {
            let lock = cache_test_lock();
            let prev = std::env::var(cache_dir_env).ok();
            let dir = TempDir::new(tag);
            std::env::set_var(cache_dir_env, dir.path());
            if let Ok(mut g) = OVERVIEW_SNAPSHOT.lock() {
                *g = None;
            }
            Self {
                _lock: lock,
                prev,
                _dir: dir,
            }
        }
    }
    impl Drop for CacheGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var(cache_dir_env, v),
                None => std::env::remove_var(cache_dir_env),
            }
            if let Ok(mut g) = OVERVIEW_SNAPSHOT.lock() {
                *g = None;
            }
        }
    }

    fn sample_node(name: &str, size: i64, files: i64) -> DirNode {
        DirNode {
            name: name.into(),
            own_size: size,
            size,
            total_files: files,
            child_files: files,
            mtime_secs: 1_700_000_000,
            depth: 1,
            ..DirNode::default()
        }
    }

    fn sample_snapshot(root: &str) -> DirSnapshot {
        let mut nodes = HashMap::new();
        nodes.insert(root.to_string(), sample_node(root, 1000, 10));
        nodes.insert(format!("{root}/sub"), sample_node("sub", 400, 4));
        DirSnapshot {
            schema_version: CACHE_SCHEMA_VERSION,
            root: root.into(),
            mod_time_secs: 1_700_000_000,
            scan_time: Utc::now(),
            total_size: 1000,
            total_files: 10,
            large_files: vec![],
            nodes,
        }
    }

    fn make_subject_dir(tag: &str) -> TempDir {
        let dir = TempDir::new(tag);
        fs::write(dir.path().join("hello.txt"), b"hi").unwrap();
        dir
    }

    // ── 快照 roundtrip ──

    #[test]
    fn snapshot_roundtrip_preserves_all_fields() {
        let _g = CacheGuard::new("roundtrip");
        let subject = make_subject_dir("subject");
        let path = subject.path().to_str().unwrap().to_string();
        let snap = sample_snapshot(&path);

        save_snapshot_to_disk(&path, &snap).unwrap();
        let loaded = load_snapshot_raw(&path).unwrap();

        assert_eq!(loaded.schema_version, CACHE_SCHEMA_VERSION);
        assert_eq!(loaded.total_files, 10);
        assert_eq!(loaded.nodes.len(), 2);
        assert_eq!(loaded.nodes[&format!("{path}/sub")].size, 400);
    }

    #[test]
    fn snapshot_load_missing_returns_not_found() {
        let _g = CacheGuard::new("missing");
        let err = load_snapshot_raw("/no/such/dir__rmole2__").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn snapshot_schema_mismatch_is_rejected() {
        let _g = CacheGuard::new("schema");
        let subject = make_subject_dir("subject");
        let path = subject.path().to_str().unwrap().to_string();
        let mut snap = sample_snapshot(&path);
        snap.schema_version = 0;

        save_snapshot_to_disk(&path, &snap).unwrap();
        let err = load_snapshot_raw(&path).unwrap_err();
        assert!(err.to_string().contains("schema mismatch"));
    }

    #[test]
    fn peek_cache_total_files_reads_header() {
        let _g = CacheGuard::new("peek");
        let subject = make_subject_dir("subject");
        let path = subject.path().to_str().unwrap().to_string();
        let snap = sample_snapshot(&path);

        save_snapshot_to_disk(&path, &snap).unwrap();
        assert_eq!(peek_cache_total_files(&path).unwrap(), 10);

        let err = peek_cache_total_files("/no/such/dir__rmole2__").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    // ── 祖先查询 ──

    #[test]
    fn find_fresh_snapshot_walks_ancestors() {
        let _g = CacheGuard::new("ancestors");
        let subject = make_subject_dir("subject");
        let root = subject.path().to_str().unwrap().to_string();
        // 快照内的 sub 节点必须对应磁盘上真实存在的目录（节点级 mtime 校验会 stat）
        let sub = format!("{root}/sub");
        fs::create_dir_all(&sub).unwrap();
        let snap = sample_snapshot(&root);
        save_snapshot_to_disk(&root, &snap).unwrap();

        // 查询子目录：命中根快照内的节点（mtime 一致 → 新鲜）
        let hit = find_fresh_snapshot(&sub).unwrap().expect("ancestor hit");
        assert_eq!(hit.root, root);
        assert_eq!(hit.snapshot.nodes[&sub].size, 400);
    }

    #[test]
    fn find_fresh_snapshot_rejects_stale_node_and_falls_back_to_stale_finder() {
        let _g = CacheGuard::new("stale_node");
        let subject = make_subject_dir("subject");
        let root = subject.path().to_str().unwrap().to_string();
        let mut snap = sample_snapshot(&root);
        // 伪造：节点 mtime 为远古 + 扫描时间在 2 天前（超出 grace + reuse 窗口，但在 stale 窗口内）
        snap.nodes.get_mut(&root).unwrap().mtime_secs = 1_000_000;
        snap.scan_time = Utc::now() - chrono::Duration::days(2);
        save_snapshot_to_disk(&root, &snap).unwrap();

        // fresh 查不到（mtime 差异超出 grace 且年龄超出 reuse 窗口）
        assert!(find_fresh_snapshot(&root).unwrap().is_none());
        // stale（扫描时间在 3 天窗口内）可命中
        let hit = find_stale_snapshot(&root).unwrap().expect("stale hit");
        assert_eq!(hit.root, root);
    }

    #[test]
    fn invalidate_cache_removes_snapshot_file() {
        let _g = CacheGuard::new("invalidate");
        let subject = make_subject_dir("subject");
        let path = subject.path().to_str().unwrap().to_string();
        save_snapshot_to_disk(&path, &sample_snapshot(&path)).unwrap();
        let cache_path = get_cache_path(&path).unwrap();
        assert!(cache_path.exists());

        invalidate_cache(&path);
        assert!(!cache_path.exists());
    }

    #[test]
    fn invalidate_cache_tree_only_removes_target_root() {
        let _g = CacheGuard::new("tree_invalidate");
        let subject = make_subject_dir("subject");
        let root = subject.path().to_str().unwrap().to_string();
        let sub = format!("{root}/sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(format!("{sub}/x.txt"), b"x").unwrap();

        save_snapshot_to_disk(&root, &sample_snapshot(&root)).unwrap();
        save_snapshot_to_disk(&sub, &sample_snapshot(&sub)).unwrap();

        invalidate_cache_tree(&sub);
        assert!(!get_cache_path(&sub).unwrap().exists());
        assert!(get_cache_path(&root).unwrap().exists());
    }

    #[test]
    fn snapshot_roundtrip_preserves_bundle_leaf_fields() {
        let _g = CacheGuard::new("bundle_roundtrip");
        let subject = make_subject_dir("subject");
        let path = subject.path().to_str().unwrap().to_string();
        let mut snap = sample_snapshot(&path);
        let node = snap.nodes.get_mut(&format!("{path}/sub")).unwrap();
        node.bundle_leaf = true;
        node.bundle_id = Some("com.example.foo".into());
        node.bundle_display_name = Some("Foo".into());
        node.bundle_content_types = vec!["com.apple.application-bundle".into()];

        save_snapshot_to_disk(&path, &snap).unwrap();
        let loaded = load_snapshot_raw(&path).unwrap();
        let n = &loaded.nodes[&format!("{path}/sub")];
        assert!(n.bundle_leaf);
        assert_eq!(n.bundle_id.as_deref(), Some("com.example.foo"));
        assert_eq!(n.bundle_display_name.as_deref(), Some("Foo"));
        assert_eq!(
            n.bundle_content_types,
            vec!["com.apple.application-bundle".to_string()]
        );
    }

    // ── env override ──

    #[test]
    fn cache_dir_env_override_takes_effect() {
        let _g = CacheGuard::new("env_override");
        let dir = get_cache_dir().unwrap();
        let env_val = std::env::var(cache_dir_env).unwrap();
        assert_eq!(dir, PathBuf::from(env_val));
    }

    // ── overview 快照有界（Go 7cf9e382 / TestEvictOverviewSnapshotsLocked）───

    #[test]
    fn test_evict_overview_snapshots_bounds_store() {
        let mut map: HashMap<String, OverviewSizeSnapshot> = HashMap::new();
        let base = Utc::now() - chrono::Duration::minutes(1001);
        for i in 0..=overview_cache_max_entries {
            map.insert(
                format!("/dir-{i:04}"),
                OverviewSizeSnapshot {
                    size: i as i64 + 1,
                    updated: base + chrono::Duration::minutes(i as i64),
                    schema_version: CACHE_SCHEMA_VERSION,
                },
            );
        }
        evict_overview_snapshots(&mut map);
        assert_eq!(map.len(), overview_cache_keep_entries);
        assert!(!map.contains_key("/dir-0000"), "oldest should be evicted");
        assert!(
            map.contains_key(&format!("/dir-{:04}", overview_cache_max_entries)),
            "newest should be kept"
        );
    }

    #[test]
    fn test_store_overview_size_skips_rewrite_while_unchanged() {
        let _g = CacheGuard::new("refresh_divisor");
        store_overview_size("/refresh/divisor/path", 1234).unwrap();
        let first_updated = {
            let g = OVERVIEW_SNAPSHOT.lock().unwrap();
            g.as_ref()
                .unwrap()
                .get("/refresh/divisor/path")
                .unwrap()
                .updated
        };
        // 相同 size 且在 TTL/refreshDivisor 内：skip 重写，时间戳不刷新
        store_overview_size("/refresh/divisor/path", 1234).unwrap();
        let second_updated = {
            let g = OVERVIEW_SNAPSHOT.lock().unwrap();
            g.as_ref()
                .unwrap()
                .get("/refresh/divisor/path")
                .unwrap()
                .updated
        };
        assert_eq!(
            first_updated, second_updated,
            "unchanged size within TTL/refreshDivisor must skip rewrite"
        );
    }

    #[test]
    fn test_load_drops_schema_mismatch_and_expired_snapshots() {
        let _g = CacheGuard::new("load_cleanup");
        let store_path = get_overview_size_store_path().unwrap();
        let payload = serde_json::json!({
            "/ok": {"size": 100, "updated": Utc::now(), "schema_version": CACHE_SCHEMA_VERSION},
            "/old-schema": {"size": 200, "updated": Utc::now(), "schema_version": 0},
            "/zero": {"size": 0, "updated": Utc::now(), "schema_version": CACHE_SCHEMA_VERSION},
            "/expired": {"size": 300, "updated": Utc::now() - chrono::Duration::days(8), "schema_version": CACHE_SCHEMA_VERSION},
        });
        fs::write(&store_path, serde_json::to_string(&payload).unwrap()).unwrap();

        assert_eq!(load_stored_overview_size("/ok").unwrap(), 100);
        let g = OVERVIEW_SNAPSHOT.lock().unwrap();
        let map = g.as_ref().unwrap();
        assert_eq!(map.len(), 1, "stale snapshots should be dropped on load");
        assert!(map.contains_key("/ok"));
    }

    #[test]
    fn test_prune_removes_expired_snap_files() {
        let _g = CacheGuard::new("prune");
        let cache_dir = get_cache_dir().unwrap();
        // 伪造一个远古 .snap 与一个新鲜 .snap
        let old = cache_dir.join("deadbeef.snap");
        let fresh = cache_dir.join("cafebabe.snap");
        fs::write(&old, vec![0u8; 64]).unwrap();
        fs::write(&fresh, vec![0u8; 64]).unwrap();

        let past = SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60);
        let mut old_meta = fs::metadata(&old).unwrap();
        // 无法直接改 mtime（权限限制仅限 set_times），用 filetime 不可得——
        // 直接验证：不设置 mtime 时（现在创建）不会被剪除。
        assert!(prune_analyzer_cache_dir(&cache_dir).is_ok());
        assert!(old.exists(), "freshly created files are not pruned");
        assert!(fresh.exists());

        // 用 set_modified 把 old 拨到 8 天前再剪除（macOS/Linux 均支持）。
        let old_time = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        // filetime crate 不可用 → 通过 fs::File::set_modified 间接验证（标准库支持）
        let f = fs::File::open(&old).unwrap();
        f.set_modified(old_time).unwrap();
        drop(f);
        assert!(prune_analyzer_cache_dir(&cache_dir).is_ok());
        assert!(!old.exists(), "expired snap should be pruned");
        let _ = old_meta;
    }
}
