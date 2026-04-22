//! 调试埋点日志（JSONL）：scan / clean / verify 三阶段数值口径排查。
//!
//! 背景：排查「前端勾选量 vs 清理报告量 vs 磁盘实际释放」三者对不上的问题，
//! 用本地持久化日志做可复现的排除法定位。
//!
//! - 输出目录：`~/Library/Logs/mole/`
//! - 批次文件：`debug-{timestamp}.log`。dry-run 扫描完成时 [`begin_batch`] 新建；
//!   clean / verify 追加到同一批次；跨进程（无批次）时自建兜底文件。
//! - 格式：JSON Lines，每行一个对象；公共字段 `ts` / `op` / `session_id` / `event`。
//! - 轮转：单文件超过 5MB 时归档为 `.1`，保留最近 3 个备份（`.1` / `.2` / `.3`）。
//! - 默认开启；所有写入失败静默，不影响主流程。

use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 单批次文件大小上限：5MB（对齐 operations.log 口径）。
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
/// 轮转保留的备份数量。
const MAX_BACKUPS: u32 = 3;

/// 当前批次文件路径（进程内共享）。
static CURRENT_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);
/// 写锁：避免并发写入交错。
static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn log_dir() -> PathBuf {
    PathBuf::from(format!("{}/Library/Logs/mole", super::base::home_dir()))
}

fn compact_ts() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// 生成带前缀的会话 ID（`scan-...` / `clean-...`），策略对齐 `generate_scan_id`（时间戳+pid）。
pub fn new_session_id(prefix: &str) -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let pid = std::process::id();
    format!("{prefix}-{ts:x}-{pid:x}")
}

/// 开启新批次文件 `debug-{timestamp}.log`（dry-run 扫描完成时调用）。返回批次文件路径。
pub fn begin_batch() -> String {
    let path = log_dir().join(format!("debug-{}.log", compact_ts()));
    if let Ok(mut guard) = CURRENT_FILE.lock() {
        *guard = Some(path.clone());
    }
    path.to_string_lossy().into_owned()
}

/// 当前批次文件；不存在则新建（clean / verify 独立进程运行时的兜底）。
fn current_or_new_file() -> PathBuf {
    let mut guard = match CURRENT_FILE.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(path) = guard.as_ref() {
        return path.clone();
    }
    let path = log_dir().join(format!("debug-{}.log", compact_ts()));
    *guard = Some(path.clone());
    path
}

/// 超过 `max_bytes` 时轮转：`file` → `file.1` → `file.2` → ...，超出 `backups` 的最旧备份被丢弃。
fn rotate_path(path: &Path, max_bytes: u64, backups: u32) {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if backups == 0 || size <= max_bytes {
        return;
    }
    let base = path.to_string_lossy().to_string();
    let _ = std::fs::remove_file(format!("{base}.{backups}"));
    for i in (1..backups).rev() {
        let _ = std::fs::rename(format!("{base}.{i}"), format!("{base}.{}", i + 1));
    }
    let _ = std::fs::rename(&base, format!("{base}.1"));
}

/// 追加一行 JSONL；超过上限自动轮转。
fn append_jsonl(path: &Path, line: &str) {
    let _guard = match WRITE_LOCK.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    rotate_path(path, MAX_FILE_BYTES, MAX_BACKUPS);
    let path_str = path.to_string_lossy();
    super::base::ensure_user_file(&path_str);
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| {
            use std::io::Write;
            writeln!(f, "{line}")
        });
}

/// 组装一行记录（公共字段 + 业务字段）。
fn build_line(op: &str, session_id: &str, event: &str, data: Map<String, Value>) -> String {
    let mut obj = Map::new();
    obj.insert("ts".to_string(), json!(super::log::get_timestamp()));
    obj.insert("op".to_string(), json!(op));
    obj.insert("session_id".to_string(), json!(session_id));
    obj.insert("event".to_string(), json!(event));
    for (k, v) in data {
        obj.insert(k, v);
    }
    Value::Object(obj).to_string()
}

/// 写入一条记录（通用入口）；`op` ∈ scan / clean / verify。
pub fn write_record(op: &str, session_id: &str, event: &str, data: Map<String, Value>) {
    let line = build_line(op, session_id, event, data);
    append_jsonl(&current_or_new_file(), &line);
}

fn map_of(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

fn human(bytes: u64) -> String {
    super::base::bytes_to_human(bytes)
}

// ─────────────────────────────── scan 阶段 ───────────────────────────────

/// 扫描前磁盘可用空间快照（`free_bytes` 与 `boot_volume_free_bytes()` 同源）。
pub fn scan_disk_before(session_id: &str, free_bytes: i64, free_human: &str) {
    write_record(
        "scan",
        session_id,
        "disk_before",
        map_of(vec![
            ("free_bytes", json!(free_bytes)),
            ("free_human", json!(free_human)),
        ]),
    );
}

/// 扫描结果中的单个 item 原始数据（id / path / size_kb / file_count / status / default_selected）。
#[allow(clippy::too_many_arguments)]
pub fn scan_item(
    session_id: &str,
    category_id: &str,
    item_id: &str,
    path: &str,
    real_path: Option<&str>,
    size_kb: u64,
    file_count: u64,
    status: &str,
    default_selected: bool,
) {
    write_record(
        "scan",
        session_id,
        "item",
        map_of(vec![
            ("category_id", json!(category_id)),
            ("item_id", json!(item_id)),
            ("item_key", json!(format!("{category_id}::{item_id}"))),
            ("path", json!(path)),
            ("real_path", json!(real_path)),
            ("size_kb", json!(size_kb)),
            ("size_human", json!(human(size_kb.saturating_mul(1024)))),
            ("file_count", json!(file_count)),
            ("status", json!(status)),
            ("default_selected", json!(default_selected)),
        ]),
    );
}

/// 默认勾选汇总（对照：用户最终勾选以 clean 阶段 `selection_summary` 为准）。
pub fn scan_default_selection_summary(
    session_id: &str,
    selected_items: u64,
    selected_size_bytes: u64,
    total_items: u64,
    total_size_bytes: u64,
) {
    write_record(
        "scan",
        session_id,
        "default_selection_summary",
        map_of(vec![
            ("selected_items", json!(selected_items)),
            ("selected_size_bytes", json!(selected_size_bytes)),
            ("selected_size_human", json!(human(selected_size_bytes))),
            ("total_items", json!(total_items)),
            ("total_size_bytes", json!(total_size_bytes)),
            ("total_size_human", json!(human(total_size_bytes))),
        ]),
    );
}

/// 扫描会话汇总。
pub fn scan_summary(
    session_id: &str,
    total_cleanable_size_bytes: u64,
    category_count: usize,
    file_count: u64,
    duration_ms: u64,
) {
    write_record(
        "scan",
        session_id,
        "summary",
        map_of(vec![
            (
                "total_cleanable_size_bytes",
                json!(total_cleanable_size_bytes),
            ),
            (
                "total_cleanable_size_human",
                json!(human(total_cleanable_size_bytes)),
            ),
            ("category_count", json!(category_count)),
            ("file_count", json!(file_count)),
            ("duration_ms", json!(duration_ms)),
        ]),
    );
}

// ─────────────────────────────── clean 阶段 ───────────────────────────────

/// 清理前磁盘可用空间快照。
pub fn clean_disk_before(session_id: &str, free_bytes: i64, free_human: &str) {
    write_record(
        "clean",
        session_id,
        "disk_before",
        map_of(vec![
            ("free_bytes", json!(free_bytes)),
            ("free_human", json!(free_human)),
        ]),
    );
}

/// 勾选快照中的单个 item（含未勾选项：`selected=false`；含被后端二次校验拦下项）。
pub fn clean_selection_item(
    session_id: &str,
    item_key: &str,
    category_id: &str,
    size_bytes: u64,
    selected: bool,
    status: &str,
) {
    write_record(
        "clean",
        session_id,
        "selection_item",
        map_of(vec![
            ("item_key", json!(item_key)),
            ("category_id", json!(category_id)),
            ("size_bytes", json!(size_bytes)),
            ("size_human", json!(human(size_bytes))),
            ("selected", json!(selected)),
            ("status", json!(status)),
        ]),
    );
}

/// 勾选汇总：最终选中量 + 后端二次校验拦下量 + 快照全量（对照）。
pub fn clean_selection_summary(
    session_id: &str,
    selected_items: u64,
    selected_size_bytes: u64,
    backend_skipped_items: u64,
    backend_skipped_size_bytes: u64,
    snapshot_total_items: u64,
    snapshot_total_size_bytes: u64,
) {
    write_record(
        "clean",
        session_id,
        "selection_summary",
        map_of(vec![
            ("selected_items", json!(selected_items)),
            ("selected_size_bytes", json!(selected_size_bytes)),
            ("selected_size_human", json!(human(selected_size_bytes))),
            ("backend_skipped_items", json!(backend_skipped_items)),
            (
                "backend_skipped_size_bytes",
                json!(backend_skipped_size_bytes),
            ),
            ("snapshot_total_items", json!(snapshot_total_items)),
            (
                "snapshot_total_size_bytes",
                json!(snapshot_total_size_bytes),
            ),
        ]),
    );
}

/// 单个分类的实际执行结果（循环内实时追加）。
pub fn clean_category_result(
    session_id: &str,
    category_id: &str,
    item_ids: &[String],
    size_cleaned: u64,
    status: &str,
    error: Option<&str>,
) {
    write_record(
        "clean",
        session_id,
        "category_result",
        map_of(vec![
            ("category_id", json!(category_id)),
            ("item_ids", json!(item_ids)),
            ("size_cleaned", json!(size_cleaned)),
            ("size_cleaned_human", json!(human(size_cleaned))),
            ("status", json!(status)),
            ("error", json!(error)),
        ]),
    );
}

/// 清理会话汇总（报告口径）。
pub fn clean_summary(
    session_id: &str,
    success_count: u64,
    failed_count: u64,
    total_cleaned_size: u64,
) {
    write_record(
        "clean",
        session_id,
        "summary",
        map_of(vec![
            ("success_count", json!(success_count)),
            ("failed_count", json!(failed_count)),
            ("total_cleaned_size", json!(total_cleaned_size)),
            ("total_cleaned_size_human", json!(human(total_cleaned_size))),
        ]),
    );
}

// ─────────────────────────────── verify 阶段 ───────────────────────────────

/// 清理后磁盘可用空间快照。
pub fn verify_disk_after(session_id: &str, free_bytes: i64, free_human: &str) {
    write_record(
        "verify",
        session_id,
        "disk_after",
        map_of(vec![
            ("free_bytes", json!(free_bytes)),
            ("free_human", json!(free_human)),
        ]),
    );
}

/// 验证结论：报告清理量 vs 磁盘实际变化。
///
/// `difference_bytes = free_space_change - total_cleaned_size`：
/// 负值表示「报告释放多于磁盘实际变化」（如文件移入废纸篓未清空），正值反之。
pub fn verify_verdict(
    session_id: &str,
    total_cleaned_size: u64,
    free_space_change: i64,
    free_space_change_human: &str,
    difference_bytes: i64,
) {
    let difference_human = if difference_bytes >= 0 {
        format!("+{}", human(difference_bytes.unsigned_abs()))
    } else {
        format!("-{}", human(difference_bytes.unsigned_abs()))
    };
    write_record(
        "verify",
        session_id,
        "verdict",
        map_of(vec![
            ("total_cleaned_size", json!(total_cleaned_size)),
            ("total_cleaned_size_human", json!(human(total_cleaned_size))),
            ("free_space_change", json!(free_space_change)),
            ("free_space_change_human", json!(free_space_change_human)),
            ("difference_bytes", json!(difference_bytes)),
            ("difference_human", json!(difference_human)),
        ]),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_has_prefix() {
        let id = new_session_id("clean");
        assert!(id.starts_with("clean-"), "unexpected id: {id}");
    }

    #[test]
    fn build_line_is_valid_json_with_meta() {
        let mut data = Map::new();
        data.insert("foo".to_string(), json!(1));
        let line = build_line("scan", "scan-1", "disk_before", data);
        let value: Value = serde_json::from_str(&line).expect("line must be valid json");
        assert_eq!(value["op"], "scan");
        assert_eq!(value["session_id"], "scan-1");
        assert_eq!(value["event"], "disk_before");
        assert_eq!(value["foo"], 1);
        assert!(
            value["ts"].as_str().map(|s| !s.is_empty()).unwrap_or(false),
            "ts must be non-empty"
        );
    }

    #[test]
    fn append_writes_valid_jsonl_lines() {
        let dir = std::env::temp_dir().join(format!("mole-dbg-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let file = dir.join("debug-write-test.log");

        let line = build_line(
            "scan",
            "scan-1",
            "summary",
            map_of(vec![("total_items", json!(3))]),
        );
        append_jsonl(&file, &line);
        append_jsonl(&file, &line);

        let content = std::fs::read_to_string(&file).expect("read back");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "two appended lines expected");
        for l in &lines {
            let v: Value = serde_json::from_str(l).expect("each line must be valid json");
            assert_eq!(v["event"], "summary");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotate_keeps_at_most_three_backups() {
        let dir = std::env::temp_dir().join(format!("mole-dbg-rotate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let file = dir.join("debug-test.log");

        // 连续轮转 4 次：.1/.2/.3 保留，最旧的被丢弃
        for byte in [b'a', b'b', b'c', b'd'] {
            std::fs::write(&file, vec![byte; 64]).expect("write payload");
            rotate_path(&file, 10, 3);
        }
        assert!(dir.join("debug-test.log.1").exists());
        assert!(dir.join("debug-test.log.2").exists());
        assert!(dir.join("debug-test.log.3").exists());
        assert!(!dir.join("debug-test.log.4").exists());
        assert!(!file.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
