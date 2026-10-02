//! 扫描快照注册表 — 防重放、防篡改、校验 scan_id。
//! 自 `controllers/clean.rs` 纯搬迁（行为不变）。容量上限 1（新扫描覆盖旧扫描）。

/// 最近一次 `clean_scan` 成功完成的时间（用于 `clean_status` 展示数据时效）。
/// 纯内存态，重启后归零；前端据此判断是否提示「数据已过期，建议重新扫描」。
pub static LAST_SCAN_AT: std::sync::Mutex<Option<std::time::SystemTime>> = std::sync::Mutex::new(None);

// ============================================================
// 扫描快照注册表 — 防重放、防篡改、校验 scan_id
// ============================================================

/// 快照过期时间：30 分钟。超时后 apply 拒绝执行，要求用户重新扫描。
const SCAN_EXPIRY_SECONDS: u64 = 30 * 60;

/// 快照中的单个 item 记录（仅后端可见，不序列化给前端）。
#[derive(Clone)]
pub struct SnapshotItem {
    pub category_id: String,
    /// 是否命中白名单（apply 时二次校验）
    pub whitelist_matched: bool,
    /// 是否需要 sudo
    pub requires_sudo: bool,
    /// 扫描时的体积（用于日志/统计，apply 以实际删除为准）
    pub size: u64,
    /// 扫描时的状态：cleanable / info / locked / empty
    pub status: String,
}

/// 一次扫描的完整快照。apply 时据此验证前端请求的合法性。
pub struct ScanSnapshot {
    pub scan_id: String,
    pub created_at: std::time::Instant,
    /// "category_id::item_id" → SnapshotItem
    pub items: std::collections::HashMap<String, SnapshotItem>,
    pub size_metric: String,
}

impl ScanSnapshot {
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed().as_secs() > SCAN_EXPIRY_SECONDS
    }
}

/// 全局扫描注册表。容量上限 1（新扫描覆盖旧扫描）。
static SCAN_REGISTRY: std::sync::RwLock<Option<ScanSnapshot>> = std::sync::RwLock::new(None);

/// 存入快照。新扫描覆盖旧扫描。
pub fn store_scan_snapshot(snapshot: ScanSnapshot) {
    if let Ok(mut guard) = SCAN_REGISTRY.write() {
        *guard = Some(snapshot);
    }
}

/// 取出并清除快照（一次性使用，防重放）。
pub fn take_scan_snapshot(scan_id: &str) -> Result<ScanSnapshot, String> {
    let mut guard = SCAN_REGISTRY
        .write()
        .map_err(|_| "Scan registry lock poisoned".to_string())?;
    match guard.take() {
        None => Err("No scan snapshot available, please rescan".into()),
        Some(s) if s.scan_id != scan_id => {
            // scan_id 不匹配，放回（不销毁）
            *guard = Some(s);
            Err("Scan ID mismatch, please rescan".into())
        }
        Some(s) => Ok(s),
    }
}

/// 基于时间戳+进程ID 生成简单唯一 scan_id（无需 uuid crate）。
pub fn generate_scan_id() -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let pid = std::process::id();
    format!("scan-{ts:x}-{pid:x}")
}
