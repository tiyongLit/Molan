// 对标 Mole bin/clean.sh — 薄壳：调 lib/clean/* → 返回 JSON
// dry_run 通过 MOLE_DRY_RUN 环境变量控制（lib/clean/ 各函数内部读取）

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::clean::{app_caches, apps, caches, dev, hints, launch_services, system, user};
use crate::core::app_protection::is_path_whitelisted;
use crate::core::base::{
    FINDER_METADATA_SENTINEL, MOLE_ONE_GIB_KB, SpinnerAppGuard, current_spinner_app_handle,
    end_section, get_free_space, run_cmd, start_section,
};
use crate::core::dry_run_registry::clear_seen_cleanup_targets;
use crate::core::log::{log_operation_session_end, log_operation_session_start};
use crate::core::sudo;
use crate::events::{
    CleanApplyProgressPayload, CleanupHintItem, CleanupHintsResultPayload,
    CleanupPhaseResultPayload, PHASE_APP_CACHES, PHASE_APP_SUPPORT_LOGS,
    PHASE_APPLE_SILICON_CACHES, PHASE_APPLICATIONS, PHASE_BROWSERS, PHASE_CLOUD_STORAGE,
    PHASE_DEV_TOOLS, PHASE_DEVICE_FIRMWARE, PHASE_FINDER_METADATA, PHASE_LOCAL_SNAPSHOTS,
    PHASE_OFFICE_CACHES, PHASE_ORPHANED_CONTAINER_STUBS, PHASE_ORPHANED_DATA,
    PHASE_ORPHANED_SYSTEM_SERVICES, PHASE_PROJECT_ARTIFACTS, PHASE_SYSTEM, PHASE_SYSTEM_DATA_HINTS,
    PHASE_TIME_MACHINE, PHASE_USER_ESSENTIALS, PHASE_VIRTUALIZATION, emit_clean_apply_progress,
    emit_cleanup_hints_result, emit_cleanup_phase_result,
};
use crate::manage::whitelist;

static MOLE_CLEAN_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// 最近一次 `clean_scan` 成功完成的时间（用于 `clean_status` 展示数据时效）。
/// 纯内存态，重启后归零；前端据此判断是否提示「数据已过期，建议重新扫描」。
static LAST_SCAN_AT: std::sync::Mutex<Option<std::time::SystemTime>> = std::sync::Mutex::new(None);

struct CleanGuard;

impl Drop for CleanGuard {
    fn drop(&mut self) {
        MOLE_CLEAN_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}

// ============================================================
// 扫描快照注册表 — 防重放、防篡改、校验 scan_id
// ============================================================

/// 快照过期时间：30 分钟。超时后 apply 拒绝执行，要求用户重新扫描。
const SCAN_EXPIRY_SECONDS: u64 = 30 * 60;

/// 快照中的单个 item 记录（仅后端可见，不序列化给前端）。
#[derive(Clone)]
struct SnapshotItem {
    category_id: String,
    /// 是否命中白名单（apply 时二次校验）
    whitelist_matched: bool,
    /// 是否需要 sudo
    requires_sudo: bool,
    /// 扫描时的体积（用于日志/统计，apply 以实际删除为准）
    size: u64,
    /// 扫描时的状态：cleanable / info / locked / empty
    status: String,
}

/// 一次扫描的完整快照。apply 时据此验证前端请求的合法性。
struct ScanSnapshot {
    scan_id: String,
    created_at: std::time::Instant,
    /// "category_id::item_id" → SnapshotItem
    items: std::collections::HashMap<String, SnapshotItem>,
    size_metric: String,
}

impl ScanSnapshot {
    fn is_expired(&self) -> bool {
        self.created_at.elapsed().as_secs() > SCAN_EXPIRY_SECONDS
    }
}

/// 全局扫描注册表。容量上限 1（新扫描覆盖旧扫描）。
static SCAN_REGISTRY: std::sync::RwLock<Option<ScanSnapshot>> = std::sync::RwLock::new(None);

/// 存入快照。新扫描覆盖旧扫描。
fn store_scan_snapshot(snapshot: ScanSnapshot) {
    if let Ok(mut guard) = SCAN_REGISTRY.write() {
        *guard = Some(snapshot);
    }
}

/// 取出并清除快照（一次性使用，防重放）。
fn take_scan_snapshot(scan_id: &str) -> Result<ScanSnapshot, String> {
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
fn generate_scan_id() -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let pid = std::process::id();
    format!("scan-{ts:x}-{pid:x}")
}

/// 判断分类是否为「谨慎清理」（影响 default_selected）。
fn is_cautious_category(cat_id: &str) -> bool {
    matches!(
        cat_id,
        "office_cache" | "virtualization" | "device_firmware" | "large_files"
    )
}

/// 计算 item 的默认勾选状态（后端权威，前端只读）。
/// 规则对齐前端原逻辑：status!=info && size>0 && !whitelist_matched && recommend && !cautious
fn compute_default_selected(
    item_status: &str,
    item_size: u64,
    item_whitelist_matched: bool,
    cat_recommend: bool,
    cat_id: &str,
    cat_requires_sudo: bool,
    has_sudo: bool,
) -> bool {
    if item_status != "cleanable" {
        return false;
    }
    if item_size == 0 {
        return false;
    }
    if item_whitelist_matched {
        return false;
    }
    if !cat_recommend {
        return false;
    }
    if is_cautious_category(cat_id) {
        return false;
    }
    if cat_requires_sudo && !has_sudo {
        return false;
    }
    true
}

#[derive(Serialize)]
struct CleanOutput {
    mode: String,
    collected_at: String,
    /// 扫描唯一标识。前端 apply 时须回传此 ID，后端据此从 SCAN_REGISTRY 取快照验证。
    /// 仅 dry_run（扫描）模式返回；execute 模式为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    scan_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    whitelist: Option<WhitelistInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    categories: Option<Vec<CleanCategory>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    results: Option<Vec<CleanResult>>,
    summary: CleanSummary,
}

#[derive(Serialize)]
struct WhitelistInfo {
    /// 对齐 `perform_cleanup`：`WHITELIST_PATTERNS` 总数。
    active_patterns: usize,
    /// 对齐 `DEFAULT_WHITELIST_PATTERNS` 命中条数（Shell 文案里的 core）。
    core_pattern_count: usize,
    /// 非预设条目数（Shell 文案里的 custom）。
    custom_pattern_count: usize,
    /// 对齐 Shell：仅在 dry-run 下列出；执行清理时不序列化空数组。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    patterns: Vec<String>,
    /// 对齐 clean.sh:1098-1103：白名单加载时的验证警告。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct CleanCategory {
    id: String,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tips: Option<String>,
    recommend: bool,
    cautious: bool,
    requires_sudo: bool,
    whitelist_matched: bool,
    items: Vec<CleanItem>,
}

#[derive(Serialize)]
struct CleanItem {
    id: String,
    path: String,
    size: u64,
    size_human: String,
    file_count: u64,
    status: String,
    /// 该子项是否命中白名单（路径匹配 / item: 前缀）。
    /// 前端据此禁用勾选，实现二级勾选控制。
    whitelist_matched: bool,
    /// 后端权威计算的默认勾选状态。
    /// 规则：status==cleanable && size>0 && !whitelist_matched && cat.recommend && !cautious
    /// 前端据此初始化 selectedItemIds，用户可自由修改。
    default_selected: bool,
    /// 真实文件系统路径；Some(path) 时前端可渲染「在 Finder 中显示」按钮。
    /// 对齐 Lemon：只有背后对应真实、可定位路径的项才显示打开按钮。
    /// 聚合型 / 无单一路径的项为 None，前端不显示按钮。
    #[serde(skip_serializing_if = "Option::is_none")]
    real_path: Option<String>,
}

#[derive(Serialize)]
struct CleanResult {
    category_id: String,
    item_id: String,
    path: String,
    size_cleaned: u64,
    size_cleaned_human: String,
    file_count: u64,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct CleanSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    total_cleanable_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_cleanable_size_human: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_cleaned_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_cleaned_size_human: Option<String>,
    total_file_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    category_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    success_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skipped_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failed_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    final_free_space: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    final_free_space_human: Option<String>,
    /// 对齐 clean.sh `emit_free_space_summary` 的「Free space change」。
    /// 仅在 execute 模式且有初始值时可算。dry-run 时不输出。
    #[serde(skip_serializing_if = "Option::is_none")]
    free_space_change: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    free_space_change_human: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    movie_equivalent: Option<String>,
}

fn bytes_to_human(b: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let k = crate::constants::SIZE_BASE;
    if b == 0 {
        return "0B".into();
    }
    let kb = b / k;
    let mut val = kb as f64;
    let mut unit_idx = 1;
    while val >= k as f64 && unit_idx < 4 {
        val /= k as f64;
        unit_idx += 1;
    }
    if val.fract() < 0.05 {
        format!("{}{}", val as u64, UNITS[unit_idx])
    } else {
        format!("{:.1}{}", val, UNITS[unit_idx])
    }
}

fn kb_to_human(kb: u64) -> String {
    bytes_to_human(kb.saturating_mul(crate::constants::SIZE_BASE))
}

/// 检查子项是否命中白名单。
/// 支持两种模式：
///   - `item:{item_id}` 前缀匹配（如 `item:dev_go`）
///   - 文件系统路径 glob 匹配（沿用 `is_path_whitelisted`）
fn item_is_whitelisted(item_id: &str, item_path: Option<&str>, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let marker = format!("item:{item_id}");
    if patterns.iter().any(|p| p == &marker) {
        return true;
    }
    if let Some(p) = item_path {
        if !p.is_empty() && is_path_whitelisted(p, patterns) {
            return true;
        }
    }
    false
}

fn make_item(
    id: &str,
    path: &str,
    size_kb: u64,
    count: u64,
    whitelist_matched: bool,
    default_selected: bool,
    real_path: Option<String>,
) -> CleanItem {
    CleanItem {
        id: id.into(),
        path: path.into(),
        size: size_kb * 1024,
        size_human: kb_to_human(size_kb),
        file_count: count,
        status: "cleanable".into(),
        whitelist_matched,
        default_selected,
        real_path,
    }
}

fn make_info_item(
    id: &str,
    label: &str,
    size_kb: u64,
    count: u64,
    whitelist_matched: bool,
) -> CleanItem {
    CleanItem {
        id: id.into(),
        path: label.into(),
        size: size_kb * 1024,
        size_human: kb_to_human(size_kb),
        file_count: count,
        status: "info".into(),
        whitelist_matched,
        default_selected: false, // info 项永远不默认勾选
        real_path: None,         // info 项无真实路径
    }
}

/// macOS 隐藏文件判断：路径最后一段文件名以 `.` 开头即视为隐藏。
/// 只比较最后一段，避免误伤 `.cargo/registry` 这类路径中间含点、末段正常的项。
fn is_hidden_entry(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().starts_with('.'))
        .unwrap_or(false)
}

/// 根据分类和子项 ID 解析真实文件系统路径。
/// 对齐 Lemon：只有背后对应真实、可定位路径的项才返回 Some，前端据此显示「在 Finder 中显示」按钮。
/// 聚合型 / 无单一路径的项返回 None。
/// 隐藏文件（`.DS_Store` 等）在 macOS Finder 默认不可见，定位体验失效，统一返回 None 隐藏按钮；
/// 清理功能不受影响（scan / apply 仍按原逻辑执行）。
fn resolve_item_real_path(cat_id: &str, item_id: &str) -> Option<String> {
    let h = crate::core::base::home_dir();
    let resolved = match cat_id {
        // ── 用户基础文件清理（多 item）──
        "user_cache" => match item_id {
            "user_cache" => Some(format!("{h}/Library/Caches")),
            "user_logs" => Some(format!("{h}/Library/Logs")),
            "trash" => Some(format!("{h}/.Trash")),
            // 清理目标是 home 目录下所有 `.DS_Store` 等隐藏元数据文件，Finder 默认不可见，不提供定位
            "finder_metadata" => None,
            _ => None,
        },
        // ── 系统清理（多 item，路径均在 /Library 或 /var）──
        "system_caches" => match item_id {
            "system_crash_reports" | "system_memory_exception" => {
                Some("/Library/Logs/DiagnosticReports".into())
            }
            "system_logs" => Some("/var/log".into()),
            "system_diagnostic_logs" => Some("/Library/Logs/DiagnosticReports".into()),
            "system_power_logs" => Some("/var/log/powermanagement".into()),
            _ => None,
        },
        // ── 开发者工具（多 item，复用 dev 模块已有的路径映射）──
        "dev_tools" => dev::dev_item_primary_path(item_id),
        // ── 其余单 item / 聚合分类 ──
        "app_support_logs" => Some(format!("{h}/Library/Application Support")),
        "app_caches" | "browser_cache" | "cloud_storage" | "office_cache" | "applications"
        | "virtualization" | "orphaned_data" | "apple_silicon" | "device_firmware"
        | "time_machine" | "large_files" | "project_artifacts" => None,
        _ => None,
    };
    // 统一过滤隐藏文件/目录（如 `~/.Trash` 末段为 `.Trash`）：Finder 默认不可见，定位无意义
    resolved.filter(|p| !is_hidden_entry(p))
}

/// 对齐 `base.rs:get_free_space` 所选卷，用 `df -k` 解析可用空间（字节），供 JSON `summary.final_free_space`。
fn boot_volume_free_bytes() -> i64 {
    let target = if Path::new("/System/Volumes/Data").is_dir() {
        "/System/Volumes/Data"
    } else {
        "/"
    };
    let out = run_cmd("df", &["-k", target]).unwrap_or_default();
    out.lines()
        .nth(1)
        .and_then(|line| line.split_whitespace().nth(3))
        .and_then(|s| s.parse::<i64>().ok())
        .map(|kb| kb.saturating_mul(1024))
        .unwrap_or(0)
}

/// 对齐 `perform_cleanup` 白名单段：dry-run 下列 pattern 时跳过 `FINDER_METADATA` sentinel。
fn whitelist_patterns_for_response(patterns: &[String]) -> Vec<String> {
    patterns
        .iter()
        .filter(|p| p.as_str() != FINDER_METADATA_SENTINEL)
        .cloned()
        .collect()
}

/// 对齐 `clean.sh` `perform_cleanup` 1018–1037：`pattern` 是否与某条默认保护相同。
fn whitelist_core_custom_counts(patterns: &[String]) -> (usize, usize) {
    let defaults = whitelist::default_whitelist_patterns();
    let mut core = 0usize;
    let mut custom = 0usize;
    for p in patterns {
        let is_core = defaults
            .iter()
            .any(|d| whitelist::patterns_equivalent(p, d));
        if is_core {
            core += 1;
        } else {
            custom += 1;
        }
    }
    (core, custom)
}

/// 对齐 SH `run_with_shell_timeout`：返回退出码模拟 Shell 行为。
/// - 0: 成功完成
/// - 124: 超时
/// - 130: 用户中断 (GUI 中通过取消按钮触发)
/// - 其他: 执行失败
///
/// 使用 signal_hook::flag 检测 SIGINT，确保 130 分支真正可达，
/// 而不仅仅是同步 Shell 的注释死代码。register_sudo_cleanup 也在
/// 独立线程监听同一信号（调用 process::exit(130)），flag 方式在
/// 进程退出前率先设置原子标志，recv_timeout 返回后即可判断。
fn run_cleanup_with_timeout<F>(timeout_ms: u64, label: &str, f: F) -> i32
where
    F: FnOnce() + Send + 'static,
{
    let interrupted = Arc::new(AtomicBool::new(false));
    let sigint_flag = Arc::clone(&interrupted);
    let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, sigint_flag);

    let (tx, rx) = mpsc::channel::<()>();
    thread::spawn(move || {
        f();
        let _ = tx.send(());
    });

    match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(_) => {
            if interrupted.load(Ordering::Relaxed) {
                crate::core::base::set_clean_cancelled();
                log::warn!("[clean] {label} interrupted by user");
                130
            } else {
                0
            }
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            crate::core::base::set_clean_cancelled();
            log::warn!("[clean] {label} timed out after 5 minutes, skipping remaining items");
            124
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            log::warn!("[clean] {label} failed (thread panic)");
            1
        }
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mole_clean(app: tauri::AppHandle, dry_run: bool) -> Result<Value, String> {
    let t0 = std::time::Instant::now();
    log::info!("[diagnose] mole_clean ENTRY dry_run={dry_run}");

    if MOLE_CLEAN_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return Err("A scan is already in progress. Please wait for it to complete.".into());
    }
    let _guard = CleanGuard;

    if dry_run {
        std::env::set_var("MOLE_DRY_RUN", "1");
    } else {
        std::env::remove_var("MOLE_DRY_RUN");
    }

    let app_clean = app.clone();
    log::info!("[diagnose] mole_clean about to spawn_blocking...");
    let output = tauri::async_runtime::spawn_blocking(move || {
        let t_block = std::time::Instant::now();
        log::info!("[diagnose] mole_clean spawn_blocking CLOSURE STARTED");
        let _spinner_guard = SpinnerAppGuard::enter(app_clean);
        log::info!("[diagnose] mole_clean SpinnerAppGuard entered");
        log::info!("[mole_clean] spawn_blocking started");
        crate::core::base::reset_clean_cancelled();
        clear_seen_cleanup_targets();
        log_operation_session_start("clean");

        // 对齐 clean.sh L1095-L1100：捕获初始剩余空间快照，供 summary 计算 Free space change。
        let initial_free_bytes = boot_volume_free_bytes();

        // 对齐 clean.sh `perform_cleanup` 核心清理之前：载入白名单（与 `mo clean` 同源）
        let whitelist_result = whitelist::load_whitelist("clean");
        // crate::core::set_whitelist(whitelist_result.patterns.clone());

        let (core_pattern_count, custom_pattern_count) =
            whitelist_core_custom_counts(&whitelist_result.patterns);
        log::info!(
            "[mole_clean] Whitelist: {} patterns active ({} core + {} custom)",
            whitelist_result.patterns.len(),
            core_pattern_count,
            custom_pattern_count,
        );
        if dry_run && !whitelist_result.patterns.is_empty() {
            for p in &whitelist_result.patterns {
                log::info!("[mole_clean]   whitelist pattern: {}", p);
            }
        }
        let whitelist_info = WhitelistInfo {
            active_patterns: whitelist_result.patterns.len(),
            core_pattern_count,
            custom_pattern_count,
            patterns: if dry_run {
                whitelist_patterns_for_response(&whitelist_result.patterns)
            } else {
                Vec::new()
            },
            warnings: whitelist_result.warnings,
        };

        let cat_whitelisted = |cat_id: &str| -> bool {
            whitelist::category_is_whitelisted(cat_id, &whitelist_result.patterns)
        };
        let item_whitelisted = |item_id: &str, item_path: Option<&str>| -> bool {
            item_is_whitelisted(item_id, item_path, &whitelist_result.patterns)
        };

        let mut categories: Vec<CleanCategory> = Vec::new();

        //===== 1. 系统清理 =====
        let system_clean = sudo::is_admin_authorized();
        let system_result = if system_clean {
            start_section("System");
            let result = system::clean_deep_system();
            system::clean_local_snapshots();
            end_section();
            if let Some(app) = current_spinner_app_handle() {
                emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                    section: "System".into(),
                    phase: PHASE_LOCAL_SNAPSHOTS.into(),
                    title: "Time Machine local snapshots".into(),
                    cleaned: false,
                    size_kb: None,
                    file_count: None,
                });
            }
            result
        } else {
            crate::clean::ModuleScanResult::empty()
        };
        let system_kb = system_result.total_kb();
        let system_count = system_result.total_count();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                section: "System".into(),
                phase: PHASE_SYSTEM.into(),
                title: "System caches & logs".into(),
                cleaned: system_clean,
                size_kb: Some(system_kb),
                file_count: Some(system_count),
            });
        }
        let sys_cat_wl = cat_whitelisted("system_caches");
        let system_items: Vec<CleanItem> = system_result.items.iter().map(|i| {
            let st = if system_clean { "cleanable" } else { "locked" };
            let wl = item_whitelisted(&i.id, i.path.as_deref());
            CleanItem {
                id: i.id.clone(),
                path: i.title.clone(),
                size: i.size_kb * 1024,
                size_human: kb_to_human(i.size_kb),
                file_count: i.file_count,
                status: st.into(),
                whitelist_matched: wl,
                default_selected: compute_default_selected(st, i.size_kb * 1024, wl, true, "system_caches", true, system_clean),
                real_path: resolve_item_real_path("system_caches", &i.id),
            }
        }).collect();
        categories.push(CleanCategory {
            id: "system_caches".into(),
            title: "System caches & logs".into(),
            tips: if system_clean {
                None
            } else {
                Some("Requires admin session for system scan/cleanup. Invoke mole_request_admin_session once, or run `sudo -v` before scanning.".into())
            },
            recommend: true,
            cautious: false,
            requires_sudo: true,
            whitelist_matched: sys_cat_wl,
            items: system_items,
        });

        // 对齐 clean.sh L1109-L1111：预检 TCC 权限，避免运行中弹窗。
        caches::check_tcc_permissions();

        // 对齐 clean.sh L1179-L1183：白名单验证警告在 System 阶段之后打印。
        if !whitelist_info.warnings.is_empty() {
            for w in &whitelist_info.warnings {
                log::info!("[mole_clean]   Whitelist warning: {}", w);
            }
        }

        //===== 2. 用户基础文件清理 =====
        start_section("User essentials");
        log::info!("[mole_clean] calling user::clean_user_essentials...");
        let ue_result = user::clean_user_essentials();
        let kb = ue_result.total_kb();
        let cnt = ue_result.total_count();
        log::info!("[mole_clean] user::clean_user_essentials done: {}KB, {} items", kb, cnt);
        let (finder_kb, finder_cnt) = user::clean_finder_metadata();
        log::info!("[mole_clean] user::clean_finder_metadata done: {}KB, {} items", finder_kb, finder_cnt);
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                section: "User essentials".into(),
                phase: PHASE_USER_ESSENTIALS.into(),
                title: "User app cache & logs".into(),
                cleaned: kb > 0 || finder_kb > 0,
                size_kb: Some(kb + finder_kb),
                file_count: Some(cnt + finder_cnt),
            });
            if finder_kb > 0 {
                emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                    section: "User essentials".into(),
                    phase: PHASE_FINDER_METADATA.into(),
                    title: "Finder metadata".into(),
                    cleaned: true,
                    size_kb: Some(finder_kb),
                    file_count: Some(finder_cnt),
                });
            }
        }
        let uc_cat_wl = cat_whitelisted("user_cache");
        let mut user_items: Vec<CleanItem> = ue_result.items.iter().map(|i| {
            let wl = item_whitelisted(&i.id, i.path.as_deref());
            CleanItem {
                id: i.id.clone(),
                path: i.title.clone(),
                size: i.size_kb * 1024,
                size_human: kb_to_human(i.size_kb),
                file_count: i.file_count,
                status: "cleanable".into(),
                whitelist_matched: wl,
                default_selected: compute_default_selected("cleanable", i.size_kb * 1024, wl, true, "user_cache", false, system_clean),
                real_path: resolve_item_real_path("user_cache", &i.id),
            }
        }).collect();
        if finder_kb > 0 || finder_cnt > 0 {
            let wl = item_whitelisted("finder_metadata", None);
            // real_path 统一走 resolve_item_real_path：finder_metadata 目标是隐藏的 .DS_Store，
            // Finder 默认不可见，返回 None，前端不显示「在 Finder 中显示」按钮
            user_items.push(make_item("finder_metadata", "Home directory, .DS_Store", finder_kb, finder_cnt, wl,
                compute_default_selected("cleanable", finder_kb * 1024, wl, true, "user_cache", false, system_clean),
                resolve_item_real_path("user_cache", "finder_metadata")));
        }
        categories.push(CleanCategory {
            id: "user_cache".into(),
            title: "User app cache & logs".into(),
            tips: None,
            recommend: true,
            cautious: false,
            requires_sudo: false,
            whitelist_matched: uc_cat_wl,
            items: user_items,
        });

        //===== 3. 应用缓存清理 =====
        start_section("App caches");
        let ac_result = user::clean_app_caches();
        let kb = ac_result.total_kb();
        let cnt = ac_result.total_count();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                section: "App caches".into(),
                phase: PHASE_APP_CACHES.into(),
                title: "App caches".into(),
                cleaned: kb > 0,
                size_kb: Some(kb),
                file_count: Some(cnt),
            });
        }
        let ac_cat_wl = cat_whitelisted("app_caches");
        let app_caches_items: Vec<CleanItem> = ac_result.items.iter().map(|i| {
            let wl = item_whitelisted(&i.id, i.path.as_deref());
            CleanItem {
                id: i.id.clone(),
                path: i.title.clone(),
                size: i.size_kb * 1024,
                size_human: kb_to_human(i.size_kb),
                file_count: i.file_count,
                status: "cleanable".into(),
                whitelist_matched: wl,
                default_selected: compute_default_selected("cleanable", i.size_kb * 1024, wl, true, "app_caches", false, system_clean),
                real_path: resolve_item_real_path("app_caches", &i.id),
            }
        }).collect();
        categories.push(CleanCategory {
            id: "app_caches".into(),
            title: "App caches".into(),
            tips: None,
            recommend: true,
            cautious: false,
            requires_sudo: false,
            whitelist_matched: ac_cat_wl,
            items: app_caches_items,
        });

        // ===== 4. 浏览器清理 =====
        start_section("Browsers");
        let (kb, cnt) = user::clean_browsers();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                section: "Browsers".into(),
                phase: PHASE_BROWSERS.into(),
                title: "Browser caches".into(),
                cleaned: kb > 0,
                size_kb: Some(kb),
                file_count: Some(cnt),
            });
        }
        let bc_wl = item_whitelisted("browser_cache_main", None);
        categories.push(CleanCategory {
            id: "browser_cache".into(),
            title: "Browser caches".into(),
            tips: None,
            recommend: true,
            cautious: false,
            requires_sudo: false,
            whitelist_matched: cat_whitelisted("browser_cache"),
            items: vec![make_item("browser_cache_main", "Browser cache data", kb, cnt, bc_wl,
                compute_default_selected("cleanable", kb * 1024, bc_wl, true, "browser_cache", false, system_clean),
                None)],
        });

        // ===== 5. 云服务和办公软件清理 =====
        start_section("Cloud & Office");
        let cloud_result = Arc::new(Mutex::new(None));
        let office_result = Arc::new(Mutex::new(None));
        let cr = Arc::clone(&cloud_result);
        let or = Arc::clone(&office_result);
        let exit_code = run_cleanup_with_timeout(300_000, "Cloud & Office cleanup", move || {
            *cr.lock().unwrap() = Some(user::clean_cloud_storage());
            *or.lock().unwrap() = Some(user::clean_office_applications());
        });
        // 对齐 Shell 的退出码处理逻辑
        if exit_code != 0 {
            if exit_code == 130 {
                log::warn!("[clean] Cloud & Office cleanup interrupted by user");
            } else if exit_code != 124 {
                log::warn!("[clean] Cloud & Office cleanup failed with exit code {}", exit_code);
            }
        }
        end_section();
        let (kb1, cnt1) = cloud_result.lock().unwrap().take().unwrap_or((0, 0));
        let (kb2, cnt2) = office_result.lock().unwrap().take().unwrap_or((0, 0));
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                section: "Cloud & Office".into(),
                phase: PHASE_CLOUD_STORAGE.into(),
                title: "Cloud storage cache".into(),
                cleaned: kb1 > 0,
                size_kb: Some(kb1),
                file_count: Some(cnt1),
            });
        }
        let cs_wl = item_whitelisted("cloud_storage_main", None);
        categories.push(CleanCategory {
            id: "cloud_storage".into(),
            title: "Cloud storage cache".into(),
            tips: None,
            recommend: true,
            cautious: false,
            requires_sudo: false,
            whitelist_matched: cat_whitelisted("cloud_storage"),
            items: vec![make_item("cloud_storage_main", "iCloud/Dropbox/OneDrive/Google Drive", kb1, cnt1, cs_wl,
                compute_default_selected("cleanable", kb1 * 1024, cs_wl, true, "cloud_storage", false, system_clean),
                None)],
        });
        let oc_wl = item_whitelisted("office_cache_main", None);
        categories.push(CleanCategory {
            id: "office_cache".into(),
            title: "Office caches".into(),
            tips: None,
            recommend: false,
            cautious: true,
            requires_sudo: false,
            whitelist_matched: cat_whitelisted("office_cache"),
            items: vec![make_item("office_cache_main", "Microsoft Office caches", kb2, cnt2, oc_wl,
                compute_default_selected("cleanable", kb2 * 1024, oc_wl, false, "office_cache", false, system_clean),
                None)],
        });
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                section: "Cloud & Office".into(),
                phase: PHASE_OFFICE_CACHES.into(),
                title: "Office caches".into(),
                cleaned: kb2 > 0,
                size_kb: Some(kb2),
                file_count: Some(cnt2),
            });
        }

        // ===== 6. 开发者工具清理 =====
        if !crate::core::base::is_clean_cancelled() {
        start_section("Developer tools");
        let dt_result = dev::clean_developer_tools();
        let kb = dt_result.total_kb();
        let cnt = dt_result.total_count();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                section: "Developer tools".into(),
                phase: PHASE_DEV_TOOLS.into(),
                title: "Developer tools cache".into(),
                cleaned: true,
                size_kb: Some(kb),
                file_count: Some(cnt),
            });
        }
        let dt_cat_wl = cat_whitelisted("dev_tools");
        let dev_items: Vec<CleanItem> = dt_result.items.iter().map(|i| {
            let wl = item_whitelisted(&i.id, i.path.as_deref());
            CleanItem {
                id: i.id.clone(),
                path: i.title.clone(),
                size: i.size_kb * 1024,
                size_human: kb_to_human(i.size_kb),
                file_count: i.file_count,
                status: "cleanable".into(),
                whitelist_matched: wl,
                default_selected: compute_default_selected("cleanable", i.size_kb * 1024, wl, true, "dev_tools", false, system_clean),
                real_path: resolve_item_real_path("dev_tools", &i.id),
            }
        }).collect();
        categories.push(CleanCategory {
            id: "dev_tools".into(),
            title: "Developer tools cache".into(),
            tips: None,
            recommend: true,
            cautious: false,
            requires_sudo: false,
            whitelist_matched: dt_cat_wl,
            items: dev_items,
        });

        // ===== 7. GUI应用程序清理 =====
        start_section("Applications");
        let (kb, cnt) = app_caches::clean_user_gui_applications();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                section: "Applications".into(),
                phase: PHASE_APPLICATIONS.into(),
                title: "Applications".into(),
                cleaned: kb > 0,
                size_kb: Some(kb),
                file_count: Some(cnt),
            });
        }
        let app_wl = item_whitelisted("applications_main", None);
        categories.push(CleanCategory {
            id: "applications".into(),
            title: "Applications".into(),
            tips: None,
            recommend: true,
            cautious: false,
            requires_sudo: false,
            whitelist_matched: cat_whitelisted("applications"),
            items: vec![make_item("applications_main", "App-specific caches & data", kb, cnt, app_wl,
                compute_default_selected("cleanable", kb * 1024, app_wl, true, "applications", false, system_clean),
                None)],
        });

        // ===== 8. 虚拟化工具清理 =====
        start_section("Virtualization");
        let (kb, cnt) = user::clean_virtualization_tools();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                section: "Virtualization".into(),
                phase: PHASE_VIRTUALIZATION.into(),
                title: "Virtualization caches".into(),
                cleaned: kb > 0,
                size_kb: Some(kb),
                file_count: Some(cnt),
            });
        }
        let virt_wl = item_whitelisted("virtualization_main", None);
        categories.push(CleanCategory {
            id: "virtualization".into(),
            title: "Virtualization caches".into(),
            tips: None,
            recommend: false,
            cautious: true,
            requires_sudo: false,
            whitelist_matched: cat_whitelisted("virtualization"),
            items: vec![make_item("virtualization_main", "Docker/Parallels/VMware caches", kb, cnt, virt_wl,
                compute_default_selected("cleanable", kb * 1024, virt_wl, false, "virtualization", false, system_clean),
                None)],
        });

        //===== 9. 应用支持日志清理 =====
        start_section("Application Support");
        let (kb, cnt) = user::clean_application_support_logs();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                section: "Application Support".into(),
                phase: PHASE_APP_SUPPORT_LOGS.into(),
                title: "Application Support logs/caches".into(),
                cleaned: kb > 0,
                size_kb: Some(kb),
                file_count: Some(cnt),
            });
        }
        let asl_wl = item_whitelisted("app_support_logs_main", None);
        categories.push(CleanCategory {
            id: "app_support_logs".into(),
            title: "Application Support logs/caches".into(),
            tips: None,
            recommend: true,
            cautious: false,
            requires_sudo: false,
            whitelist_matched: cat_whitelisted("app_support_logs"),
            items: vec![make_item("app_support_logs_main", "~/Library/Application Support logs & caches", kb, cnt, asl_wl,
                compute_default_selected("cleanable", kb * 1024, asl_wl, true, "app_support_logs", false, system_clean),
                Some(format!("{}/Library/Application Support", crate::core::base::home_dir())))],
        });

        // ===== 10. 应用残留文件清理 =====
        start_section("App leftovers");
        let (orphan_kb, orphan_cnt) = apps::clean_orphaned_app_data();
        let (svc_kb, svc_cnt) = apps::clean_orphaned_system_services();
        let (stub_kb, stub_cnt) = apps::clean_orphaned_container_stubs();
        let (ls_kb, ls_cnt) = launch_services::clean_stale_launch_services_registrations();
        let launch_agent_hints = hints::show_user_launch_agent_hint_notice();
        let dotdir_hints = hints::show_orphan_dotdir_hint_notice();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                section: "App leftovers".into(),
                phase: PHASE_ORPHANED_DATA.into(),
                title: "Orphaned app data".into(),
                cleaned: orphan_kb > 0,
                size_kb: Some(orphan_kb),
                file_count: Some(orphan_cnt),
            });
            if svc_kb > 0 || svc_cnt > 0 {
                emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                    section: "App leftovers".into(),
                    phase: PHASE_ORPHANED_SYSTEM_SERVICES.into(),
                    title: "Orphaned system services".into(),
                    cleaned: svc_kb > 0,
                    size_kb: Some(svc_kb),
                    file_count: Some(svc_cnt),
                });
            }
            if stub_kb > 0 || stub_cnt > 0 {
                emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                    section: "App leftovers".into(),
                    phase: PHASE_ORPHANED_CONTAINER_STUBS.into(),
                    title: "Orphaned container stubs".into(),
                    cleaned: stub_kb > 0,
                    size_kb: Some(stub_kb),
                    file_count: Some(stub_cnt),
                });
            }
            let has_la_hints = launch_agent_hints.detected;
            let has_dd_hints = dotdir_hints.detected;
            if has_la_hints {
                emit_cleanup_hints_result(&app, &launch_agent_hints);
            }
            if has_dd_hints {
                emit_cleanup_hints_result(&app, &dotdir_hints);
            }
        }
        let od_wl = item_whitelisted("orphaned_data_main", None);
        let od_total_kb = orphan_kb + svc_kb + stub_kb + ls_kb;
        let od_total_cnt = orphan_cnt + svc_cnt + stub_cnt + ls_cnt;
        categories.push(CleanCategory {
            id: "orphaned_data".into(),
            title: "Orphaned app data".into(),
            tips: None,
            recommend: true,
            cautious: false,
            requires_sudo: false,
            whitelist_matched: cat_whitelisted("orphaned_data"),
            items: vec![make_item("orphaned_data_main", "Data from removed apps, system services, container stubs, stale LaunchServices registrations", od_total_kb, od_total_cnt, od_wl,
                compute_default_selected("cleanable", od_total_kb * 1024, od_wl, true, "orphaned_data", false, system_clean),
                None)],
        });

        // ===== 11. Apple Silicon专用缓存清理 =====
        start_section("Apple Silicon");
        let (as_kb, as_cnt) = user::clean_apple_silicon_caches();
        end_section();
        if as_kb > 0 || as_cnt > 0 {
            if let Some(app) = current_spinner_app_handle() {
                emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                    section: "Apple Silicon".into(),
                    phase: PHASE_APPLE_SILICON_CACHES.into(),
                    title: "Apple Silicon caches".into(),
                    cleaned: as_kb > 0,
                    size_kb: Some(as_kb),
                    file_count: Some(as_cnt),
                });
            }
        }
        let as_wl = item_whitelisted("apple_silicon_main", None);
        categories.push(CleanCategory {
            id: "apple_silicon".into(),
            title: "Apple Silicon updates".into(),
            tips: None,
            recommend: true,
            cautious: false,
            requires_sudo: true,
            whitelist_matched: cat_whitelisted("apple_silicon"),
            items: vec![CleanItem {
                id: "apple_silicon_main".into(),
                path: "Rosetta 2 cache & media service cache".into(),
                size: as_kb * 1024,
                size_human: kb_to_human(as_kb),
                file_count: as_cnt,
                status: "cleanable".into(),
                whitelist_matched: as_wl,
                default_selected: compute_default_selected("cleanable", as_kb * 1024, as_wl, true, "apple_silicon", true, system_clean),
                real_path: None,
            }],
        });

        // ===== 12. 设备备份和固件清理 =====
        start_section("Device backups & firmware");
        let (kb, cnt) = user::clean_cached_device_firmware();
        let ios_backup_hints = user::check_ios_device_backups();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                section: "Device backups & firmware".into(),
                phase: PHASE_DEVICE_FIRMWARE.into(),
                title: "Device firmware & iOS backups".into(),
                cleaned: kb > 0,
                size_kb: Some(kb),
                file_count: Some(cnt),
            });
            if ios_backup_hints.detected {
                emit_cleanup_hints_result(&app, &ios_backup_hints);
            }
        }
        let df_wl = item_whitelisted("device_firmware_main", None);
        categories.push(CleanCategory {
            id: "device_firmware".into(),
            title: "Device firmware & iOS backups".into(),
            tips: None,
            recommend: false,
            cautious: true,
            requires_sudo: false,
            whitelist_matched: cat_whitelisted("device_firmware"),
            items: vec![make_item("device_firmware_main", "iOS device firmware & backups", kb, cnt, df_wl,
                compute_default_selected("cleanable", kb * 1024, df_wl, false, "device_firmware", false, system_clean),
                None)],
        });

        //===== 13. Time Machine清理 =====
        start_section("Time Machine");
        let (tm_kb, tm_cnt) = system::clean_time_machine_failed_backups();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(&app, &CleanupPhaseResultPayload {
                section: "Time Machine".into(),
                phase: PHASE_TIME_MACHINE.into(),
                title: "Failed Time Machine backups".into(),
                cleaned: tm_kb > 0,
                size_kb: Some(tm_kb),
                file_count: Some(tm_cnt),
            });
        }
        let tm_wl = item_whitelisted("time_machine_main", None);
        categories.push(CleanCategory {
            id: "time_machine".into(),
            title: "Time Machine failed backups".into(),
            tips: None,
            recommend: true,
            cautious: false,
            requires_sudo: false,
            whitelist_matched: cat_whitelisted("time_machine"),
            items: vec![make_item("time_machine_main", "Failed Time Machine backups", tm_kb, tm_cnt, tm_wl,
                compute_default_selected("cleanable", tm_kb * 1024, tm_wl, true, "time_machine", false, system_clean),
                None)],
        });

        //===== 14. 大文件检查 =====
        start_section("Large files");
        let large_files = user::check_large_file_candidates();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            if large_files.detected {
                emit_cleanup_hints_result(&app, &large_files);
            }
        }
        categories.push(CleanCategory {
            id: "large_files".into(),
            title: "Large file candidates".into(),
            tips: Some("Review files >1 GB in home directory; not directly cleaned by mole.".into()),
            recommend: false,
            cautious: true,
            requires_sudo: false,
            whitelist_matched: cat_whitelisted("large_files"),
            items: vec![make_info_item("large_files_info", "Large files >1 GB — review manually", 0, 0, item_whitelisted("large_files_info", None))],
        });

        // ===== 15. 系统数据线索提示 =====
        start_section("System Data clues");
        let clues = hints::show_system_data_hint_notice();
        end_section();
        let detected = !clues.is_empty();
        if let Some(app) = current_spinner_app_handle() {
            let items: Vec<CleanupHintItem> = clues
                .iter()
                .map(|(label, sz_kb, path)| CleanupHintItem {
                    label: label.clone(),
                    size_bytes: sz_kb.saturating_mul(1024),
                    size_human: bytes_to_human(sz_kb.saturating_mul(1024)),
                    path: path.clone(),
                    detail: None,
                })
                .collect();
            emit_cleanup_hints_result(
                &app,
                &CleanupHintsResultPayload {
                    section: "System Data clues".into(),
                    phase: PHASE_SYSTEM_DATA_HINTS.into(),
                    title: "System Data clues".into(),
                    detected,
                    review_hint: "Review: mo analyze, Device backups, docker system df".into(),
                    items,
                },
            );
        }
        let sysdata_items: Vec<CleanItem> = clues.iter().map(|(label, sz_kb, path)| {
            CleanItem {
                id: format!("sysdata_{}", label),
                path: format!("{} — {}", label, path),
                size: sz_kb * 1024,
                size_human: bytes_to_human(sz_kb * 1024),
                file_count: 0,
                status: "info".into(),
                whitelist_matched: false,
                default_selected: false, // info 项不默认勾选
                real_path: if path.starts_with('/') { Some(path.clone()) } else { None },
            }
        }).collect();
        categories.push(CleanCategory {
            id: "system_data_clues".into(),
            title: "System Data clues".into(),
            tips: Some("Review: mo analyze, Device backups, docker system df".into()),
            recommend: false,
            cautious: false,
            requires_sudo: false,
            whitelist_matched: cat_whitelisted("system_data_clues"),
            items: if sysdata_items.is_empty() {
                vec![make_info_item("sysdata_no_items", "No significant system data clues found", 0, 0, item_whitelisted("sysdata_no_items", None))]
            } else {
                sysdata_items
            },
        });

        // ===== 16. 项目产物提示 =====
        start_section("Project artifacts");
        let hints_data = hints::show_project_artifact_hint_notice();
        end_section();
        let detected = hints_data.detected;
        if let Some(app) = current_spinner_app_handle() {
            let mut items = Vec::new();
            if detected {
                let label = if hints_data.truncated {
                    format!("{}+ candidates", hints_data.count)
                } else {
                    format!("{} candidates", hints_data.count)
                };
                let path_str = if !hints_data.examples.is_empty() {
                    format!("Examples: {}", hints_data.examples.join(", "))
                } else {
                    String::new()
                };
                if hints_data.estimate_samples > 0 {
                    let partial = hints_data.estimate_partial
                        || hints_data.truncated
                        || hints_data.estimate_samples < hints_data.count;
                    let size_human = bytes_to_human(hints_data.estimated_kb.saturating_mul(1024));
                    let desc = if partial {
                        format!("at least {} sampled from {} items", size_human, hints_data.estimate_samples)
                    } else {
                        format!("sampled {}", size_human)
                    };
                    items.push(CleanupHintItem {
                        label: format!("{} — {}", label, desc),
                        size_bytes: hints_data.estimated_kb.saturating_mul(1024),
                        size_human,
                        path: path_str,
                        detail: None,
                    });
                } else {
                    items.push(CleanupHintItem {
                        label,
                        size_bytes: 0,
                        size_human: "—".into(),
                        path: path_str,
                        detail: None,
                    });
                }
            }
            emit_cleanup_hints_result(&app, &CleanupHintsResultPayload {
                section: "Project artifacts".into(),
                phase: PHASE_PROJECT_ARTIFACTS.into(),
                title: "Project artifacts".into(),
                detected,
                review_hint: "Review: mo purge".into(),
                items,
            });
        }
        let artifact_kb = if hints_data.detected { hints_data.estimated_kb } else { 0 };
        let artifact_count = if hints_data.detected { hints_data.count as u64 } else { 0 };
        let artifact_label = if hints_data.detected {
            if hints_data.truncated {
                format!("{}+ build artifact candidates", hints_data.count)
            } else {
                format!("{} build artifact candidates", hints_data.count)
            }
        } else {
            "No project artifacts detected".into()
        };
        let pa_wl = item_whitelisted("project_artifacts_main", None);
        categories.push(CleanCategory {
            id: "project_artifacts".into(),
            title: "Project artifacts".into(),
            tips: Some("Build artifacts from dev projects; use `mo purge` to review & clean.".into()),
            recommend: false,
            cautious: false,
            requires_sudo: false,
            whitelist_matched: cat_whitelisted("project_artifacts"),
            items: vec![make_item("project_artifacts_main", &artifact_label, artifact_kb, artifact_count, pa_wl,
                compute_default_selected("cleanable", artifact_kb * 1024, pa_wl, false, "project_artifacts", false, system_clean),
                None)],
        });

        } // is_clean_cancelled guard — 对齐 Shell：超时/中断后跳过后续 section

        // 诊断：打印每个 category 的详细信息
        for cat in &categories {
            let cat_items: Vec<String> = cat.items.iter().map(|i| {
                format!("  [{}] id={} size={} status={} wl={}", i.path, i.id, i.size, i.status, i.whitelist_matched)
            }).collect();
            log::info!("[diagnose] category id={} title={} items={} total_size_kb={}",
                cat.id, cat.title, cat.items.len(),
                cat.items.iter().map(|i| i.size).sum::<u64>() / 1024);
            for item_str in cat_items {
                log::info!("[diagnose]   {}", item_str);
            }
        }
        log::info!("[diagnose] total categories count={}", categories.len());

        // 计算总量
        let total_kb: u64 = categories.iter().flat_map(|c| c.items.iter()).map(|i| i.size / 1024).sum();
        let total_file_count: u64 = categories
            .iter()
            .flat_map(|c| c.items.iter())
            .map(|i| i.file_count)
            .sum();

        log::info!("[mole_clean] total cleanable: {}KB across {} categories", total_kb, categories.len());

        let is_execute = !dry_run;

        let (results, cleaned_kb, success_count, actual_file_count, skipped_count) = if is_execute {
            let mut r = Vec::new();
            let mut cleaned: u64 = 0;
            let mut success: u64 = 0;
            let mut actual_fc: u64 = 0;
            let mut skipped: u64 = 0;
            for cat in &categories {
                for item in &cat.items {
                    if item.size > 0 {
                        cleaned = cleaned.saturating_add(item.size / 1024);
                        success += 1;
                        actual_fc += item.file_count;
                        r.push(CleanResult {
                            category_id: cat.id.clone(),
                            item_id: item.id.clone(),
                            path: item.path.clone(),
                            size_cleaned: item.size,
                            size_cleaned_human: item.size_human.clone(),
                            file_count: item.file_count,
                            status: "cleaned".into(),
                            error: None,
                        });
                    } else {
                        skipped += 1;
                    }
                }
            }
            (Some(r), cleaned, success, actual_fc, skipped)
        } else {
            (None, 0, 0, 0, 0)
        };

        let final_free = if is_execute { Some(boot_volume_free_bytes()) } else { None };
        let final_free_human = if is_execute { Some(get_free_space()) } else { None };

        // 对齐 clean.sh `emit_free_space_summary`：仅在 execute 模式计算 Free space change。
        let (free_space_change, free_space_change_human) = if is_execute {
            let delta = final_free.zip(Some(initial_free_bytes)).map(|(f, i)| f - i);
            let delta_human = delta.map(|d| {
                let abs_delta = d.unsigned_abs();
                let human = bytes_to_human(abs_delta);
                if d >= 0 {
                    format!("+{human}")
                } else {
                    format!("-{human}")
                }
            });
            (delta, delta_human)
        } else {
            (None, None)
        };

        log_operation_session_end(
            "clean",
            if is_execute { actual_file_count } else { total_file_count },
            if is_execute { cleaned_kb } else { total_kb },
        );

        let status = if is_execute && cleaned_kb == 0 {
            Some("nothing_to_clean".into())
        } else if !is_execute && total_kb == 0 {
            Some("nothing_to_clean".into())
        } else {
            None
        };

        let movie_equivalent = if is_execute && cleaned_kb >= MOLE_ONE_GIB_KB {
            let freed_gb = cleaned_kb / MOLE_ONE_GIB_KB;
            let movies = freed_gb * 10 / 45;
            if movies > 0 {
                if movies == 1 {
                    Some(format!("Equivalent to ~1 4K movie of storage."))
                } else {
                    Some(format!("Equivalent to ~{movies} 4K movies of storage."))
                }
            } else {
                None
            }
        } else {
            None
        };

        log::info!("[diagnose] mole_clean spawn_blocking done, block_elapsed={:.1}s",
            t_block.elapsed().as_secs_f64());

        let summary = CleanSummary {
            total_cleanable_size: if is_execute { None } else { Some(total_kb * 1024) },
            total_cleanable_size_human: if is_execute { None } else { Some(kb_to_human(total_kb)) },
            total_cleaned_size: if is_execute { Some(cleaned_kb * 1024) } else { None },
            total_cleaned_size_human: if is_execute { Some(kb_to_human(cleaned_kb)) } else { None },
            total_file_count: if is_execute { actual_file_count } else { total_file_count },
            category_count: Some(categories.len()),
            success_count: if is_execute { Some(success_count) } else { None },
            skipped_count: if is_execute { Some(skipped_count) } else { None },
            failed_count: None,
            final_free_space: final_free,
            final_free_space_human: final_free_human,
            free_space_change,
            free_space_change_human,
            status,
            movie_equivalent,
        };

        // dry_run 模式下生成 scan_id 并存入快照注册表，供后续 clean_apply 验证
        let scan_id = if dry_run {
            let sid = generate_scan_id();
            let mut snapshot_items = std::collections::HashMap::new();
            for cat in &categories {
                for item in &cat.items {
                    let key = format!("{}::{}", cat.id, item.id);
                    snapshot_items.insert(key, SnapshotItem {
                        category_id: cat.id.clone(),
                        whitelist_matched: item.whitelist_matched || cat.whitelist_matched,
                        requires_sudo: cat.requires_sudo,
                        size: item.size,
                        status: item.status.clone(),
                    });
                }
            }
            store_scan_snapshot(ScanSnapshot {
                scan_id: sid.clone(),
                created_at: std::time::Instant::now(),
                items: snapshot_items,
                size_metric: "logical".into(),
            });
            log::info!("[mole_clean] scan snapshot stored: scan_id={}, items={}", sid, categories.iter().flat_map(|c| &c.items).count());
            Some(sid)
        } else {
            None
        };

        CleanOutput {
            mode: if dry_run { "dry_run".into() } else { "execute".into() },
            collected_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            scan_id,
            whitelist: Some(whitelist_info),
            categories: if is_execute { None } else { Some(categories) },
            results,
            summary,
        }
    })
    .await
    .map_err(|e| format!("clean task panicked: {}", e))?;

    log::info!(
        "[diagnose] mole_clean spawn_blocking AWAITED, total_elapsed={:.1}s",
        t0.elapsed().as_secs_f64()
    );

    serde_json::to_value(&output).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
pub struct MoleCleanPathItem {
    pub item_id: String,
    pub category_id: String,
    pub path: String,
    pub size: u64,
}

#[derive(Deserialize)]
pub struct MoleCleanPathsArgs {
    pub items: Vec<MoleCleanPathItem>,
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_clean_paths(args: MoleCleanPathsArgs) -> Result<Value, String> {
    log::info!("[mole_clean_paths] {} items", args.items.len());
    let mut results = Vec::new();
    let mut total_cleaned: u64 = 0;
    let mut success_count: u64 = 0;
    let mut failed_count: u64 = 0;

    for item in &args.items {
        let path = std::path::Path::new(&item.path);
        let meta = std::fs::metadata(path).ok();
        let file_size = meta.map(|m| m.len()).unwrap_or(0);

        match trash::delete(path) {
            Ok(()) => {
                total_cleaned += file_size;
                success_count += 1;
                results.push(serde_json::json!({
                    "category_id": item.category_id,
                    "item_id": item.item_id,
                    "path": item.path,
                    "size_cleaned": file_size,
                    "status": "cleaned",
                }));
            }
            Err(e) => {
                failed_count += 1;
                results.push(serde_json::json!({
                    "category_id": item.category_id,
                    "item_id": item.item_id,
                    "path": item.path,
                    "size_cleaned": 0,
                    "status": "failed",
                    "error": e.to_string(),
                }));
            }
        }
    }

    Ok(serde_json::json!({
        "results": results,
        "summary": {
            "total_cleaned_size": total_cleaned,
            "success_count": success_count,
            "failed_count": failed_count,
        }
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mole_clean_execute(category_ids: Vec<String>) -> Result<Value, String> {
    let output = tauri::async_runtime::spawn_blocking(move || {
        if MOLE_CLEAN_IN_PROGRESS.swap(true, Ordering::SeqCst) {
            return Err("A cleanup is already in progress. Please wait.".into());
        }
        let _guard = CleanGuard;

        log::info!(
            "[mole_clean_execute] executing {} categories: {:?}",
            category_ids.len(),
            category_ids
        );

        let whitelist_result = whitelist::load_whitelist("clean");
        crate::core::set_whitelist(whitelist_result.patterns);

        std::env::remove_var("MOLE_DRY_RUN");

        crate::core::base::reset_clean_cancelled();
        log_operation_session_start("clean");

        let initial_free_bytes = boot_volume_free_bytes();

        let has_sudo = sudo::is_admin_authorized();

        let mut results: Vec<serde_json::Value> = Vec::new();
        let mut total_cleaned: u64 = 0;
        let mut success_count: u64 = 0;
        let mut failed_count: u64 = 0;

        for cat_id in &category_ids {
            let cleaned_kb: u64 = match cat_id.as_str() {
                "system_caches" => {
                    if has_sudo {
                        let r = system::clean_deep_system();
                        system::clean_local_snapshots();
                        r.total_kb()
                    } else {
                        log::warn!("[mole_clean_execute] system_caches skipped: no sudo");
                        results.push(serde_json::json!({
                            "category_id": cat_id,
                            "item_id": "",
                            "path": "System caches",
                            "size_cleaned": 0,
                            "status": "skipped",
                            "error": "Requires admin session"
                        }));
                        failed_count += 1;
                        continue;
                    }
                }
                "user_cache" => {
                    let r = user::clean_user_essentials();
                    let (kb, _) = user::clean_finder_metadata();
                    r.total_kb().saturating_add(kb)
                }
                "app_caches" => user::clean_app_caches().total_kb(),
                "browser_cache" => user::clean_browsers().0,
                "cloud_storage" => user::clean_cloud_storage().0,
                "office_cache" => user::clean_office_applications().0,
                "dev_tools" => dev::clean_developer_tools().total_kb(),
                "applications" => app_caches::clean_user_gui_applications().0,
                "virtualization" => user::clean_virtualization_tools().0,
                "app_support_logs" => user::clean_application_support_logs().0,
                "orphaned_data" => {
                    let (k1, _) = apps::clean_orphaned_app_data();
                    let (k2, _) = apps::clean_orphaned_system_services();
                    let (k3, _) = apps::clean_orphaned_container_stubs();
                    let (k4, _) = launch_services::clean_stale_launch_services_registrations();
                    k1.saturating_add(k2).saturating_add(k3).saturating_add(k4)
                }
                "apple_silicon" => {
                    if has_sudo {
                        user::clean_apple_silicon_caches().0
                    } else {
                        log::warn!("[mole_clean_execute] apple_silicon skipped: no sudo");
                        results.push(serde_json::json!({
                            "category_id": cat_id,
                            "item_id": "",
                            "path": "Apple Silicon caches",
                            "size_cleaned": 0,
                            "status": "skipped",
                            "error": "Requires admin session"
                        }));
                        failed_count += 1;
                        continue;
                    }
                }
                "device_firmware" => user::clean_cached_device_firmware().0,
                "time_machine" => system::clean_time_machine_failed_backups().0,
                _ => {
                    log::info!(
                        "[mole_clean_execute] skipping non-cleanable category: {}",
                        cat_id
                    );
                    continue;
                }
            };

            total_cleaned = total_cleaned.saturating_add(cleaned_kb.saturating_mul(1024));
            success_count += 1;
            results.push(serde_json::json!({
                "category_id": cat_id,
                "item_id": "",
                "path": "",
                "size_cleaned": cleaned_kb.saturating_mul(1024),
                "status": "cleaned",
            }));
        }

        log_operation_session_end("clean", success_count, total_cleaned / 1024);

        let final_free = boot_volume_free_bytes();
        let free_space_change = (final_free as i64) - (initial_free_bytes as i64);
        let free_space_change_human = {
            let abs_delta = free_space_change.unsigned_abs();
            let human = bytes_to_human(abs_delta);
            if free_space_change >= 0 {
                format!("+{human}")
            } else {
                format!("-{human}")
            }
        };

        Ok(serde_json::json!({
            "results": results,
            "summary": {
                "total_cleaned_size": total_cleaned,
                "success_count": success_count,
                "failed_count": failed_count,
                "free_space_change": free_space_change,
                "free_space_change_human": free_space_change_human,
            }
        }))
    })
    .await
    .map_err(|e| format!("clean execute panicked: {e}"))?;

    output
}

// ============================================================
// v2 命令：clean_status / clean_scan / clean_apply / 取消
// ------------------------------------------------------------
// 设计要点（对齐 .trae/rules/04_后端工作流.md）：
// - 扫描与执行分离：`clean_scan` 仅 dry-run 预览，无副作用；`clean_apply` 才真删。
// - 参数显式化：不再用 `MOLE_DRY_RUN` 环境变量对外传模式（lib 内部仍读它，controller
//   据显式语义在内部设置）；新增 `size_metric` 参数贯通前后端。
// - 执行粒度：`clean_apply` 接收 `item_ids`（"categoryId::itemId"），后端按 category
//   去重执行。当前 lib 函数是「整类清理」，多项分类的部分勾选会整类清理——前端对
//   多项分类的部分勾选会提示用户。真 per-item 删除需后续重构 lib clean_* 函数。
// - 旧命令（mole_clean / mole_clean_execute / mole_clean_paths）保留向后兼容。
// ============================================================

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct CleanStatusInfo {
    /// 当前是否拥有管理员会话（影响 system_caches / apple_silicon 等需 sudo 的分类）。
    sudo_session_active: bool,
    /// 最近一次扫描完成时间（ISO8601）；进程重启后为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    last_scan_at: Option<String>,
}

/// 查询清理页进入时的状态：管理员会话是否有效、上次扫描时间。
/// 火绒式流程：进入页面先调此命令，再决定是否提示授权 / 数据过期。
#[tauri::command(rename_all = "snake_case")]
pub fn clean_status() -> Result<Value, String> {
    let sudo_active = sudo::is_admin_authorized();
    let last_iso = LAST_SCAN_AT
        .lock()
        .ok()
        .and_then(|g| *g)
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, 0))
        .map(|dt| dt.to_rfc3339());
    serde_json::to_value(CleanStatusInfo {
        sudo_session_active: sudo_active,
        last_scan_at: last_iso,
    })
    .map_err(|e| e.to_string())
}

/// 启动扫描（dry-run 预览，无副作用）。
/// 复用 `mole_clean(dry_run=true)` 的扫描逻辑，并在返回体中回填 `size_metric`。
/// 扫描进度通过既有 `cleanup::phase-result` / `cleanup::spinner-update` 事件推送。
#[tauri::command(rename_all = "snake_case")]
pub async fn clean_scan(
    app: tauri::AppHandle,
    size_metric: Option<String>,
) -> Result<Value, String> {
    let metric = size_metric.unwrap_or_else(|| "logical".into());
    let mut value = mole_clean(app, true).await?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "size_metric".into(),
            serde_json::Value::String(metric.clone()),
        );
    }
    // 记录扫描完成时间（仅成功路径：mole_clean 出错会提前 return Err）
    if let Ok(mut g) = LAST_SCAN_AT.lock() {
        *g = Some(std::time::SystemTime::now());
    }
    Ok(value)
}

/// 取消正在进行的扫描。
#[tauri::command(rename_all = "snake_case")]
pub fn clean_scan_cancel() -> Result<(), String> {
    crate::core::base::set_clean_cancelled();
    Ok(())
}

#[derive(Deserialize)]
pub struct CleanApplyArgs {
    /// 选中项的 id 数组，格式 `"categoryId::itemId"`。
    /// 后端按 category 去重后整类执行（当前 lib 限制，P4 阶段改造为 item 级执行）。
    pub item_ids: Vec<String>,
    /// 扫描唯一标识，必须与最近一次 clean_scan 返回的 scan_id 一致。
    /// 后端据此从 SCAN_REGISTRY 取快照验证，防重放、防篡改。
    pub scan_id: String,
}

/// 执行清理（移废纸篓）。接收前端勾选的 item_ids + scan_id，验证快照后按 category 去重逐类执行。
/// 执行进度通过 `clean::apply-progress` 事件推送；失败/跳过的分类计入 `failed_count`，
/// 不中断后续分类（对齐「清理失败继续删其他」的产品决策）。
#[tauri::command(rename_all = "snake_case")]
pub async fn clean_apply(app: tauri::AppHandle, args: CleanApplyArgs) -> Result<Value, String> {
    log::info!(
        "[clean_apply] scan_id={}, {} items",
        args.scan_id,
        args.item_ids.len(),
    );

    // ── 验证快照：scan_id 匹配 + 未过期 ──
    let snapshot = take_scan_snapshot(&args.scan_id)?;
    if snapshot.is_expired() {
        return Err("Scan data expired (>30 min). Please rescan before cleaning.".into());
    }

    // ── 验证每个 item_id 都在快照中，且未被白名单拦截 ──
    let mut valid_item_keys: Vec<String> = Vec::new();
    let mut skipped_items: Vec<String> = Vec::new();
    for item_key in &args.item_ids {
        match snapshot.items.get(item_key.as_str()) {
            None => {
                log::warn!("[clean_apply] item not in scan snapshot: {}", item_key);
                return Err(format!(
                    "Item '{}' not found in scan snapshot. Please rescan.",
                    item_key
                ));
            }
            Some(item) => {
                if item.whitelist_matched {
                    log::info!("[clean_apply] item skipped (whitelist): {}", item_key);
                    skipped_items.push(item_key.clone());
                } else if item.status != "cleanable" {
                    log::info!(
                        "[clean_apply] item skipped (not cleanable, status={}): {}",
                        item.status,
                        item_key
                    );
                    skipped_items.push(item_key.clone());
                } else if item.requires_sudo && !sudo::is_admin_authorized() {
                    log::info!("[clean_apply] item skipped (requires sudo): {}", item_key);
                    skipped_items.push(item_key.clone());
                } else {
                    valid_item_keys.push(item_key.clone());
                }
            }
        }
    }

    // ── 将有效 item_keys 按 category 分组（保留 item 维度，用于精确派发） ──
    let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for key in &valid_item_keys {
        let parts: Vec<&str> = key.splitn(2, "::").collect();
        if parts.len() != 2 {
            continue;
        }
        let cat = parts[0].to_string();
        let item_id = parts[1].to_string();
        if seen.insert(cat.clone()) {
            grouped.push((cat, vec![item_id]));
        } else if let Some(last) = grouped.last_mut() {
            last.1.push(item_id);
        }
    }
    let category_ids: Vec<String> = grouped.iter().map(|(c, _)| c.clone()).collect();
    log::info!(
        "[clean_apply] {} valid items → {} categories: {:?}, {} skipped",
        valid_item_keys.len(),
        grouped.len(),
        category_ids,
        skipped_items.len(),
    );

    let app_for_task = app.clone();
    let output = tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        if MOLE_CLEAN_IN_PROGRESS.swap(true, Ordering::SeqCst) {
            return Err("A cleanup is already in progress. Please wait.".into());
        }
        let _guard = CleanGuard;

        let whitelist_result = whitelist::load_whitelist("clean");
        crate::core::set_whitelist(whitelist_result.patterns);
        std::env::remove_var("MOLE_DRY_RUN");
        crate::core::base::reset_clean_cancelled();
        log_operation_session_start("clean");

        let initial_free_bytes = boot_volume_free_bytes();
        let has_sudo = sudo::is_admin_authorized();

        let total_categories = category_ids.len() as u64;
        emit_clean_apply_progress(
            &app_for_task,
            &CleanApplyProgressPayload {
                phase: "start".into(),
                current_category: None,
                current_path: None,
                cleaned_bytes: 0,
                done_categories: 0,
                total_categories,
                failed_count: 0,
            },
        );

        let mut results: Vec<serde_json::Value> = Vec::new();
        let mut total_cleaned: u64 = 0;
        let mut success_count: u64 = 0;
        let mut failed_count: u64 = 0;
        let mut done_categories: u64 = 0;

        for (cat_id, item_ids) in &grouped {
            if crate::core::base::is_clean_cancelled() {
                log::info!("[clean_apply] cancelled by user, stopping");
                break;
            }
            emit_clean_apply_progress(
                &app_for_task,
                &CleanApplyProgressPayload {
                    phase: "category_start".into(),
                    current_category: Some(cat_id.clone()),
                    current_path: Some(cat_id.clone()),
                    cleaned_bytes: total_cleaned,
                    done_categories,
                    total_categories,
                    failed_count,
                },
            );

            log::info!(
                "[clean_apply] executing category '{}', {} items: {:?}",
                cat_id,
                item_ids.len(),
                item_ids,
            );
            let (cleaned_bytes, status, error) = execute_category_items(cat_id, item_ids, has_sudo);
            done_categories += 1;

            if status == "cleaned" {
                total_cleaned = total_cleaned.saturating_add(cleaned_bytes);
                success_count += 1;
            } else {
                failed_count += 1;
            }
            results.push(serde_json::json!({
                "category_id": cat_id,
                "item_id": item_ids.join(","),
                "path": "",
                "size_cleaned": cleaned_bytes,
                "status": status,
                "error": error,
            }));

            emit_clean_apply_progress(
                &app_for_task,
                &CleanApplyProgressPayload {
                    phase: "category_done".into(),
                    current_category: Some(cat_id.clone()),
                    current_path: None,
                    cleaned_bytes: total_cleaned,
                    done_categories,
                    total_categories,
                    failed_count,
                },
            );
        }

        log_operation_session_end("clean", success_count, total_cleaned / 1024);

        let final_free = boot_volume_free_bytes();
        let free_space_change = (final_free as i64) - (initial_free_bytes as i64);
        let free_space_change_human = {
            let abs_delta = free_space_change.unsigned_abs();
            let human = bytes_to_human(abs_delta);
            if free_space_change >= 0 {
                format!("+{human}")
            } else {
                format!("-{human}")
            }
        };

        emit_clean_apply_progress(
            &app_for_task,
            &CleanApplyProgressPayload {
                phase: "complete".into(),
                current_category: None,
                current_path: None,
                cleaned_bytes: total_cleaned,
                done_categories,
                total_categories,
                failed_count,
            },
        );

        Ok(serde_json::json!({
            "results": results,
            "summary": {
                "total_cleaned_size": total_cleaned,
                "success_count": success_count,
                "failed_count": failed_count,
                "free_space_change": free_space_change,
                "free_space_change_human": free_space_change_human,
            }
        }))
    })
    .await
    .map_err(|e| format!("clean apply panicked: {e}"))?;

    output
}

/// 取消正在进行的清理。
#[tauri::command(rename_all = "snake_case")]
pub fn clean_apply_cancel() -> Result<(), String> {
    crate::core::base::set_clean_cancelled();
    Ok(())
}

/// 在 Finder 中定位指定路径（对齐 Lemon 的「Show in Finder」）。
/// 仅用于有真实路径（`real_path` 非空）的清理子项，前端据此显示并触发。
#[tauri::command(rename_all = "snake_case")]
pub fn clean_reveal_in_finder(path: String) -> Result<(), String> {
    crate::platform::macos_reveal::reveal_in_finder(&path)
}

/// 按 item 精确执行分类清理（P4 改造：解决"勾 1 清全"问题）。
/// 对 `dev_tools` / `system_caches` / `user_cache` / `app_caches` 4 个多 item 分类，
/// 根据用户实际勾选的 item_ids 逐条派发对应的 lib 子函数；
/// 其余分类 fallback 到原 `execute_category`（它们只有 1 个聚合 item）。
fn execute_category_items(
    cat_id: &str,
    item_ids: &[String],
    has_sudo: bool,
) -> (u64, String, Option<String>) {
    // ── 4 个多 item 分类的 dispatch table ──
    let dispatch: Option<&[(&str, fn() -> (u64, u64))]> = match cat_id {
        "dev_tools" => Some(&[
            ("dev_sqlite", dev::clean_sqlite_temp_files),
            ("dev_npm", dev::clean_dev_npm),
            ("dev_python", dev::clean_dev_python),
            ("dev_go", dev::clean_dev_go),
            ("dev_mise", dev::clean_dev_mise),
            ("dev_rust", dev::clean_dev_rust),
            ("dev_ruby", dev::clean_dev_ruby),
            ("dev_perl", dev::clean_dev_perl),
            ("dev_docker", dev::clean_dev_docker),
            ("dev_cloud", dev::clean_dev_cloud),
            ("dev_nix", dev::clean_dev_nix),
            ("dev_shell", dev::clean_dev_shell),
            ("dev_frontend", dev::clean_dev_frontend),
            ("dev_project_caches", caches::clean_project_caches),
            ("dev_mobile", dev::clean_dev_mobile),
            ("dev_jvm", dev::clean_dev_jvm),
            ("dev_jetbrains_toolbox", dev::clean_dev_jetbrains_toolbox),
            ("dev_jetbrains_logs", dev::clean_dev_jetbrains_logs),
            ("dev_ai_agents", dev::clean_dev_ai_agents_extended),
            ("dev_xctest", dev::clean_xcode_xctest_devices),
            (
                "dev_coresimulator",
                dev::clean_xcode_system_coresimulator_caches,
            ),
            ("dev_codex_runtimes", dev::clean_codex_runtimes),
            ("dev_codex_cli", dev::clean_codex_cli),
            ("dev_antigravity", dev::clean_antigravity_caches),
            ("dev_chrome_mcp", dev::clean_chrome_devtools_mcp_caches),
            ("dev_agent_wt", dev::clean_dev_agent_worktrees),
            ("dev_other_langs", dev::clean_dev_other_langs),
            ("dev_cicd", dev::clean_dev_cicd),
            ("dev_database", dev::clean_dev_database),
            ("dev_api_tools", dev::clean_dev_api_tools),
            ("dev_network", dev::clean_dev_network),
            ("dev_misc", dev::clean_dev_misc),
            ("dev_elixir", dev::clean_dev_elixir),
            ("dev_haskell", dev::clean_dev_haskell),
            ("dev_ocaml", dev::clean_dev_ocaml),
            ("dev_xcode", app_caches::clean_xcode_tools),
            ("dev_code_editors", app_caches::clean_code_editors),
            (
                "dev_homebrew_cache",
                dev::clean_dev_homebrew_cache_with_locks,
            ),
            (
                "dev_homebrew_locks",
                dev::clean_dev_homebrew_cache_with_locks,
            ),
            ("dev_homebrew_cleanup", dev::clean_dev_homebrew_cleanup_item),
            (
                "dev_homebrew_autoremove",
                dev::clean_dev_homebrew_autoremove_item,
            ),
        ]),
        "system_caches" => {
            if !has_sudo {
                return (0, "skipped".into(), Some("Requires admin session".into()));
            }
            Some(&[
                ("system_caches", system::clean_system_caches),
                ("system_temp_files", system::clean_system_temp_files),
                ("system_crash_reports", system::clean_system_crash_reports),
                ("system_logs", system::clean_system_logs),
                (
                    "system_third_party_logs",
                    system::clean_third_party_system_logs,
                ),
                (
                    "system_library_updates",
                    system::clean_system_library_updates,
                ),
                (
                    "system_macos_installers",
                    system::clean_macos_installer_files,
                ),
                (
                    "system_browser_code_sign",
                    system::clean_browser_code_sign_caches,
                ),
                (
                    "system_rebuildable_service",
                    system::clean_rebuildable_system_service_caches,
                ),
                (
                    "system_rebuildable_gpu",
                    system::clean_accessible_rebuildable_gpu_caches,
                ),
                (
                    "system_diagnostic_logs",
                    system::clean_system_diagnostic_logs,
                ),
                ("system_power_logs", system::clean_power_logs),
                (
                    "system_memory_exception",
                    system::clean_memory_exception_reports,
                ),
            ])
        }
        "user_cache" => Some(&[
            ("user_cache", user::clean_user_app_cache),
            ("user_logs", user::clean_user_app_logs),
            ("darwin_runtime", user::clean_darwin_user_runtime_dirs),
            ("trash", user::clean_user_trash),
            ("recent_items", user::clean_recent_items),
            ("finder_metadata", user::clean_finder_metadata),
        ]),
        "app_caches" => Some(&[
            ("app_caches_system", user::clean_app_caches_system),
            ("app_caches_downloads", user::clean_app_caches_downloads),
            ("app_caches_identity", user::clean_app_caches_identity),
            ("app_caches_support", user::clean_app_caches_support),
            ("app_caches_sandbox", user::clean_app_caches_sandbox),
        ]),
        _ => None,
    };

    if let Some(table) = dispatch {
        let mut total_kb: u64 = 0;
        for item_id in item_ids {
            if let Some((_, func)) = table.iter().find(|(id, _)| *id == item_id.as_str()) {
                let (kb, _cnt) = func();
                total_kb = total_kb.saturating_add(kb);
            } else {
                log::info!(
                    "[clean_apply] item '{}' not in {} dispatch table, skipping",
                    item_id,
                    cat_id
                );
            }
        }
        // system_caches 清理完后额外执行 local snapshots 清理（与扫描流程对齐）
        if cat_id == "system_caches" {
            system::clean_local_snapshots();
        }
        (total_kb.saturating_mul(1024), "cleaned".into(), None)
    } else {
        // 其余分类（单 item 或 info 类）：fallback 到原整类执行
        execute_category(cat_id, has_sudo)
    }
}

/// 执行单个分类的清理，返回 `(cleaned_bytes, status, error)`。
/// `status` ∈ `"cleaned" | "skipped"`；`skipped` 表示需 sudo 但未授权或非可清理分类。
fn execute_category(cat_id: &str, has_sudo: bool) -> (u64, String, Option<String>) {
    let cleaned_bytes: u64 = match cat_id {
        "system_caches" => {
            if has_sudo {
                let r = system::clean_deep_system();
                system::clean_local_snapshots();
                r.total_kb().saturating_mul(1024)
            } else {
                return (0, "skipped".into(), Some("Requires admin session".into()));
            }
        }
        "user_cache" => {
            let r = user::clean_user_essentials();
            let (kb, _) = user::clean_finder_metadata();
            r.total_kb().saturating_add(kb).saturating_mul(1024)
        }
        "app_caches" => user::clean_app_caches().total_kb().saturating_mul(1024),
        "browser_cache" => user::clean_browsers().0.saturating_mul(1024),
        "cloud_storage" => user::clean_cloud_storage().0.saturating_mul(1024),
        "office_cache" => user::clean_office_applications().0.saturating_mul(1024),
        "dev_tools" => dev::clean_developer_tools().total_kb().saturating_mul(1024),
        "applications" => app_caches::clean_user_gui_applications()
            .0
            .saturating_mul(1024),
        "virtualization" => user::clean_virtualization_tools().0.saturating_mul(1024),
        "app_support_logs" => user::clean_application_support_logs()
            .0
            .saturating_mul(1024),
        "orphaned_data" => {
            let (k1, _) = apps::clean_orphaned_app_data();
            let (k2, _) = apps::clean_orphaned_system_services();
            let (k3, _) = apps::clean_orphaned_container_stubs();
            let (k4, _) = launch_services::clean_stale_launch_services_registrations();
            k1.saturating_add(k2)
                .saturating_add(k3)
                .saturating_add(k4)
                .saturating_mul(1024)
        }
        "apple_silicon" => {
            if has_sudo {
                user::clean_apple_silicon_caches().0.saturating_mul(1024)
            } else {
                return (0, "skipped".into(), Some("Requires admin session".into()));
            }
        }
        "device_firmware" => user::clean_cached_device_firmware().0.saturating_mul(1024),
        "time_machine" => system::clean_time_machine_failed_backups()
            .0
            .saturating_mul(1024),
        // info / 提示类分类（large_files / system_data_clues / project_artifacts
        // 等）不在此执行，返回 skipped。
        _ => {
            log::info!("[clean_apply] skipping non-cleanable category: {}", cat_id);
            return (0, "skipped".into(), Some("Not a cleanable category".into()));
        }
    };
    (cleaned_bytes, "cleaned".into(), None)
}

// ============================================================
// Purge — 产物清理（Mole clean 的子功能）
// ============================================================

#[derive(Deserialize)]
pub struct MolePurgeArgs {
    pub dry_run: bool,
}

#[derive(Deserialize)]
pub struct MolePurgePathsWriteArgs {
    pub paths: Vec<String>,
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_purge(args: MolePurgeArgs) -> Result<Value, String> {
    log::info!("[mole_purge] called with dry_run={}", args.dry_run);
    Ok(serde_json::json!({
        "mode": if args.dry_run { "dry_run" } else { "execute" },
        "search_paths": [],
        "projects": [],
        "summary": {
            "total_artifact_count": 0,
            "total_artifact_size": 0
        }
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_purge_paths_read() -> Result<Value, String> {
    Ok(serde_json::json!({ "paths": [] }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_purge_paths_write(args: MolePurgePathsWriteArgs) -> Result<(), String> {
    log::info!("[mole_purge_paths_write] {} paths", args.paths.len());
    Ok(())
}

// ============================================================
// Installer — 安装器清理（Mole clean 的子功能）
// ============================================================

#[tauri::command(rename_all = "snake_case")]
pub fn mole_installer_scan() -> Result<Value, String> {
    Ok(serde_json::json!({
        "files": [],
        "total_size": 0,
        "total_files": 0
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_installer_trash(trash_root: String) -> Result<(), String> {
    log::info!("[mole_installer_trash] trash_root={}", trash_root);
    Ok(())
}

// ============================================================
// Whitelist — 白名单管理（Mole clean/optimize 的共用功能）
// ============================================================

#[derive(Deserialize)]
pub struct MoleWhitelistArgs {
    pub mode: String,
    pub patterns: Vec<String>,
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_whitelist_read(args: MoleWhitelistArgs) -> Result<Value, String> {
    let result = crate::manage::whitelist::load_whitelist(&args.mode);
    Ok(serde_json::json!({
        "patterns": result.patterns,
        "warnings": result.warnings,
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_whitelist_write(args: MoleWhitelistArgs) -> Result<(), String> {
    crate::manage::whitelist::save_whitelist_patterns(&args.mode, &args.patterns);
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_whitelist_predefined(args: MoleWhitelistArgs) -> Result<Value, String> {
    use std::path::Path;
    let home = dirs::home_dir().unwrap_or_else(|| Path::new("/").to_path_buf());
    let items = if args.mode == "optimize" {
        crate::whitelist_optimize::load_optimize_whitelist_patterns(&home)
            .into_iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.replace(['/', '.'], "_"),
                    "title": &p,
                    "pattern": &p,
                    "category": "optimize"
                })
            })
            .collect::<Vec<_>>()
    } else {
        let cache_items = crate::manage::whitelist::get_all_cache_items();
        if cache_items.is_empty() {
            let default_patterns = crate::manage::whitelist::default_whitelist_patterns();
            default_patterns
                .into_iter()
                .enumerate()
                .map(|(i, p)| {
                    serde_json::json!({
                        "id": format!("whitelist_{}", i),
                        "title": &p,
                        "pattern": &p,
                        "category": "system_cache"
                    })
                })
                .collect::<Vec<_>>()
        } else {
            cache_items
                .into_iter()
                .map(|(id, title, pattern)| {
                    serde_json::json!({
                        "id": id,
                        "title": title,
                        "pattern": pattern,
                        "category": "system_cache"
                    })
                })
                .collect::<Vec<_>>()
        }
    };
    Ok(serde_json::json!(items))
}
