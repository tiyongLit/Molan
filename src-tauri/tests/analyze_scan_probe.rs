//! Analyze 扫描引擎探针测试：真实目录扫描冒烟 + 快照查询链路 + 手工基准计时。
//!
//! 覆盖 V2 重写后的核心链路：
//! - `scan_subtree`（getattrlistbulk + work-stealing 全并行遍历 + disjoint merge 聚合）对真实目录的端到端扫描；
//! - `DirSnapshot` 落盘 + 祖先快照查询（钻入子目录秒开的快照路径）；
//! - `measure_dir_size_native`（纯 Rust 替代 du 的 size-only 口径）。
//!
//! 扫描代际/取消标志是进程级全局状态：本文件内多个扫描测试用互斥锁串行化。

use mole_lib::analyze::{cache, json, scanner};
use std::sync::Mutex;
use std::sync::atomic::AtomicI64;

fn probe_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// 待扫描的真实目录：默认用本项目自身源码树（规模适中、无敏感内容）；
/// 手工基准可设置 `MOLE_PROBE_ROOT` 指向任意大目录（如项目根，含 node_modules）。
fn probe_root() -> String {
    if let Ok(custom) = std::env::var("MOLE_PROBE_ROOT") {
        if !custom.is_empty() {
            return custom;
        }
    }
    std::env::var("CARGO_MANIFEST_DIR")
        .map(|d| format!("{d}/src"))
        .unwrap_or_else(|_| "src".to_string())
}

fn zero_counters() -> (AtomicI64, AtomicI64, AtomicI64) {
    (AtomicI64::new(0), AtomicI64::new(0), AtomicI64::new(0))
}

#[test]
fn probe_scan_real_tree_and_query_snapshot() {
    let _lock = probe_lock();
    let root = probe_root();
    let (f, d, b) = zero_counters();

    let t0 = std::time::Instant::now();
    let outcome = scanner::scan_subtree(&root, &f, &d, &b, None).expect("scan real tree");
    let scan_elapsed = t0.elapsed();

    assert!(outcome.result.total_files > 0, "real tree must have files");
    assert!(outcome.result.total_size > 0, "real tree must have size");
    assert!(
        !outcome.result.entries.is_empty(),
        "root entries must be non-empty"
    );

    // 快照落盘 + 查询一个真实子目录（向上逐级命中根快照）
    let snapshot = cache::DirSnapshot {
        schema_version: cache::CACHE_SCHEMA_VERSION,
        root: root.clone(),
        mod_time_secs: std::fs::metadata(&root)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        scan_time: chrono::Utc::now(),
        total_size: outcome.result.total_size,
        total_files: outcome.result.total_files,
        large_files: outcome.result.large_files.clone(),
        nodes: outcome.nodes,
    };
    cache::save_snapshot_to_disk(&root, &snapshot).expect("save snapshot");

    let child = outcome.result.entries[0].path.clone();
    let hit = cache::find_fresh_snapshot(&child).expect("query snapshot");
    let hit = hit.expect("snapshot must hit via ancestor walk");
    let entries = scanner::entries_for_dir(&hit.snapshot.nodes, &child);
    assert!(!entries.is_empty(), "child entries must be non-empty");

    // 快照查询与全量扫描的条目数应一致（根目录口径）
    assert_eq!(hit.snapshot.nodes[&root].size, outcome.result.total_size);

    // 清理：失效探针快照，避免污染真实用户缓存目录
    cache::invalidate_cache(&root);

    println!(
        "[probe] scan {root}: {} files, {} bytes in {:.2}s; snapshot query OK",
        outcome.result.total_files,
        outcome.result.total_size,
        scan_elapsed.as_secs_f64()
    );
}

#[test]
fn probe_measure_dir_size_native() {
    let _lock = probe_lock();
    let root = probe_root();
    let t0 = std::time::Instant::now();
    let size = scanner::measure_dir_size_native(&root, "", &[]).expect("native measure");
    let elapsed = t0.elapsed();
    assert!(size > 0, "native measure must produce positive size");
    println!(
        "[probe] native size {root}: {size} bytes in {:.2}s",
        elapsed.as_secs_f64()
    );
}

#[test]
fn probe_json_scan_roundtrip() {
    let _lock = probe_lock();
    let root = probe_root();
    // 走 GUI 完整链路：快照 miss → 全量扫描 → 快照保存；二次查询 → 快照命中。
    let first = json::try_perform_scan_for_json(&root, false).expect("first scan");
    assert!(first.total_size > 0);
    let second = json::try_perform_scan_for_json(&root, false).expect("second scan");
    assert_eq!(
        first.total_size, second.total_size,
        "snapshot roundtrip must match"
    );

    cache::invalidate_cache(&root);
}
