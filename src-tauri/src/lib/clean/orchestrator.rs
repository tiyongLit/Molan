//! 扫描/执行内核 — run_clean_core 与分类构建辅助。
//! 自 `controllers/clean.rs` 逐字搬迁（行为不变，未做函数体拆解）。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tauri::Emitter;

use crate::clean::model::{
    CleanCategory, CleanItem, CleanOutput, CleanResult, CleanSummary, WhitelistInfo,
};
use crate::clean::scan_registry::{
    generate_scan_id, store_scan_snapshot, ScanSnapshot, SnapshotItem,
};
use crate::clean::selection::compute_default_selected;
use crate::clean::{app_caches, apps, caches, dev, hints, launch_services, system, user};
use crate::core::app_protection::is_path_whitelisted;
use crate::core::base::{
    FINDER_METADATA_SENTINEL, MOLE_ONE_GIB_KB, SpinnerAppGuard, current_spinner_app_handle,
    end_section, get_free_space, run_cmd, start_section,
};
use crate::core::debug_trace;
use crate::core::dry_run_registry::clear_seen_cleanup_targets;
use crate::core::log::{log_operation_session_end, log_operation_session_start};
use crate::core::sudo;
use crate::events::{
    CleanupHintItem, CleanupHintsResultPayload, CleanupPhaseResultPayload,
    EVT_CLEANUP_CATEGORY_RESULT, PHASE_APP_CACHES, PHASE_APP_SUPPORT_LOGS,
    PHASE_APPLE_SILICON_CACHES, PHASE_APPLICATIONS, PHASE_BROWSERS, PHASE_CLOUD_STORAGE,
    PHASE_DEV_TOOLS, PHASE_DEVICE_FIRMWARE, PHASE_FINDER_METADATA, PHASE_LOCAL_SNAPSHOTS,
    PHASE_OFFICE_CACHES, PHASE_ORPHANED_CONTAINER_STUBS, PHASE_ORPHANED_DATA,
    PHASE_ORPHANED_SYSTEM_SERVICES, PHASE_PROJECT_ARTIFACTS, PHASE_SYSTEM,
    PHASE_SYSTEM_DATA_HINTS, PHASE_TIME_MACHINE, PHASE_USER_ESSENTIALS, PHASE_VIRTUALIZATION,
    emit_cleanup_hints_result, emit_cleanup_phase_result,
};
use crate::manage::whitelist;

pub fn bytes_to_human(b: u64) -> String {
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
pub fn boot_volume_free_bytes() -> i64 {
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
    // 保留 JoinHandle：返回前必须等待子线程真正退出。否则游离线程会在父任务结束、
    // 下一次任务重置全局状态（CLEAN_CANCELLED / MOLE_DRY_RUN / 白名单）后继续读写，造成跨任务污染。
    let handle = thread::spawn(move || {
        f();
        let _ = tx.send(());
    });

    let code = match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
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
            log::warn!(
                "[clean] {label} timed out after {}ms, skipping remaining items",
                timeout_ms
            );
            124
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            log::warn!("[clean] {label} failed (thread panic)");
            1
        }
    };

    // 等待子线程退出后再返回：其内部外部命令均经 run_with_timeout_capture_lossy 加超时，
    // 故 join 有界。超时/取消分支下子线程可能仍在跑，join 确保它结束后父任务才继续，
    // 从而 MOLE_CLEAN_IN_PROGRESS 与全局模式状态不被游离线程污染。
    let _ = handle.join();

    code
}

/// 渐进式扫描：把一个已构建好的分类先经 `cleanup::category-result` 事件推给前端，再压入 `categories`。
/// 前端据此在扫描过程中逐段渲染真实列表（含 items 与 size），而非等 `clean_scan` 整体返回。
/// 不改变 15 段划分/段序/段 id；收尾返回体、快照、`scan_id` 仍由 `categories` 统一构建（契约不变）。
fn push_and_emit_category(categories: &mut Vec<CleanCategory>, category: CleanCategory) {
    if let Some(app) = current_spinner_app_handle() {
        // 时序埋点（卡顿分析）：事件负载量级 = items 数 × 序列化字节（前端逐段重渲染的成本源）
        let t_emit = std::time::Instant::now();
        let item_count = category.items.len();
        let payload_bytes = serde_json::to_string(&category)
            .map(|s| s.len())
            .unwrap_or(0);
        let _ = app.emit(EVT_CLEANUP_CATEGORY_RESULT, &category);
        log::info!(
            "[clean-job][emit] category id={} items={} bytes={} took={:.1}ms",
            category.id,
            item_count,
            payload_bytes,
            t_emit.elapsed().as_secs_f64() * 1000.0
        );
    }
    categories.push(category);
}

/// 扫描/执行内核（阻塞调用；不含准入与 busy 守卫，由调用方负责持有）。
/// 由旧 `mole_clean` 阻塞闭包正文逐行提取，行为不变；`clean_job_start` 的 worker
/// 与旧 `mole_clean` 共用同一份实现，保证「扫描只有一条执行路径」。
pub fn run_clean_core(app_clean: tauri::AppHandle, dry_run: bool) -> CleanOutput {
    let t_block = std::time::Instant::now();
    log::info!("[diagnose] mole_clean spawn_blocking CLOSURE STARTED");
    let _spinner_guard = SpinnerAppGuard::enter(app_clean);
    log::info!("[diagnose] mole_clean SpinnerAppGuard entered");
    log::info!("[mole_clean] spawn_blocking started");
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

    // system_clean 在所有 guard 外定义，供 sections 1-16 共享
    let system_clean = sudo::is_admin_authorized();

    // 对齐 Mole `_run_cleanup_step`：每个 section 前独立检查取消标志，
    // 取消/超时后不再启动后续 section（Section 级 guard，粒度细于原分组 guard）。
    if !crate::core::base::is_clean_cancelled() {
        //===== 1. 系统清理 =====
        let _sec_t = std::time::Instant::now();
        let system_result = if system_clean {
            start_section("System");
            let result = system::clean_deep_system();
            system::clean_local_snapshots();
            end_section();
            if let Some(app) = current_spinner_app_handle() {
                emit_cleanup_phase_result(
                    &app,
                    &CleanupPhaseResultPayload {
                        section: "System".into(),
                        phase: PHASE_LOCAL_SNAPSHOTS.into(),
                        title: "Time Machine local snapshots".into(),
                        cleaned: false,
                        size_kb: None,
                        file_count: None,
                    },
                );
            }
            result
        } else {
            crate::clean::ModuleScanResult::empty()
        };
        let system_kb = system_result.total_kb();
        let system_count = system_result.total_count();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(
                &app,
                &CleanupPhaseResultPayload {
                    section: "System".into(),
                    phase: PHASE_SYSTEM.into(),
                    title: "System caches & logs".into(),
                    cleaned: system_clean,
                    size_kb: Some(system_kb),
                    file_count: Some(system_count),
                },
            );
        }
        let sys_cat_wl = cat_whitelisted("system_caches");
        let system_items: Vec<CleanItem> = system_result
            .items
            .iter()
            .map(|i| {
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
                    default_selected: compute_default_selected(
                        st,
                        i.size_kb * 1024,
                        wl,
                        true,
                        "system_caches",
                        true,
                        system_clean,
                    ),
                    real_path: resolve_item_real_path("system_caches", &i.id),
                }
            })
            .collect();
        push_and_emit_category(
            &mut categories,
            CleanCategory {
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
            },
        );

        log::info!(
            "[mole_clean] section System: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );

        // 对齐 clean.sh L1109-L1111：预检 TCC 权限，避免运行中弹窗。
        caches::check_tcc_permissions();

        // 对齐 clean.sh L1179-L1183：白名单验证警告在 System 阶段之后打印。
        if !whitelist_info.warnings.is_empty() {
            for w in &whitelist_info.warnings {
                log::info!("[mole_clean]   Whitelist warning: {}", w);
            }
        }
    } // section 1 (System) guard

    if !crate::core::base::is_clean_cancelled() {
        //===== 2. 用户基础文件清理 =====
        let _sec_t = std::time::Instant::now();
        start_section("User essentials");
        log::info!("[mole_clean] calling user::clean_user_essentials...");
        let ue_result = user::clean_user_essentials();
        let kb = ue_result.total_kb();
        let cnt = ue_result.total_count();
        log::info!(
            "[mole_clean] user::clean_user_essentials done: {}KB, {} items",
            kb,
            cnt
        );
        let (finder_kb, finder_cnt) = user::clean_finder_metadata();
        log::info!(
            "[mole_clean] user::clean_finder_metadata done: {}KB, {} items",
            finder_kb,
            finder_cnt
        );
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(
                &app,
                &CleanupPhaseResultPayload {
                    section: "User essentials".into(),
                    phase: PHASE_USER_ESSENTIALS.into(),
                    title: "User app cache & logs".into(),
                    cleaned: kb > 0 || finder_kb > 0,
                    size_kb: Some(kb + finder_kb),
                    file_count: Some(cnt + finder_cnt),
                },
            );
            if finder_kb > 0 {
                emit_cleanup_phase_result(
                    &app,
                    &CleanupPhaseResultPayload {
                        section: "User essentials".into(),
                        phase: PHASE_FINDER_METADATA.into(),
                        title: "Finder metadata".into(),
                        cleaned: true,
                        size_kb: Some(finder_kb),
                        file_count: Some(finder_cnt),
                    },
                );
            }
        }
        let uc_cat_wl = cat_whitelisted("user_cache");
        let mut user_items: Vec<CleanItem> = ue_result
            .items
            .iter()
            .map(|i| {
                let wl = item_whitelisted(&i.id, i.path.as_deref());
                CleanItem {
                    id: i.id.clone(),
                    path: i.title.clone(),
                    size: i.size_kb * 1024,
                    size_human: kb_to_human(i.size_kb),
                    file_count: i.file_count,
                    status: "cleanable".into(),
                    whitelist_matched: wl,
                    default_selected: compute_default_selected(
                        "cleanable",
                        i.size_kb * 1024,
                        wl,
                        true,
                        "user_cache",
                        false,
                        system_clean,
                    ),
                    real_path: resolve_item_real_path("user_cache", &i.id),
                }
            })
            .collect();
        if finder_kb > 0 || finder_cnt > 0 {
            let wl = item_whitelisted("finder_metadata", None);
            // real_path 统一走 resolve_item_real_path：finder_metadata 目标是隐藏的 .DS_Store，
            // Finder 默认不可见，返回 None，前端不显示「在 Finder 中显示」按钮
            user_items.push(make_item(
                "finder_metadata",
                "Home directory, .DS_Store",
                finder_kb,
                finder_cnt,
                wl,
                compute_default_selected(
                    "cleanable",
                    finder_kb * 1024,
                    wl,
                    true,
                    "user_cache",
                    false,
                    system_clean,
                ),
                resolve_item_real_path("user_cache", "finder_metadata"),
            ));
        }
        push_and_emit_category(
            &mut categories,
            CleanCategory {
                id: "user_cache".into(),
                title: "User app cache & logs".into(),
                tips: None,
                recommend: true,
                cautious: false,
                requires_sudo: false,
                whitelist_matched: uc_cat_wl,
                items: user_items,
            },
        );

        log::info!(
            "[mole_clean] section User essentials: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );
    } // section 2 (User essentials) guard

    if !crate::core::base::is_clean_cancelled() {
        //===== 3. 应用缓存清理 =====
        let _sec_t = std::time::Instant::now();
        start_section("App caches");
        let ac_result = user::clean_app_caches();
        let kb = ac_result.total_kb();
        let cnt = ac_result.total_count();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(
                &app,
                &CleanupPhaseResultPayload {
                    section: "App caches".into(),
                    phase: PHASE_APP_CACHES.into(),
                    title: "App caches".into(),
                    cleaned: kb > 0,
                    size_kb: Some(kb),
                    file_count: Some(cnt),
                },
            );
        }
        let ac_cat_wl = cat_whitelisted("app_caches");
        let app_caches_items: Vec<CleanItem> = ac_result
            .items
            .iter()
            .map(|i| {
                let wl = item_whitelisted(&i.id, i.path.as_deref());
                CleanItem {
                    id: i.id.clone(),
                    path: i.title.clone(),
                    size: i.size_kb * 1024,
                    size_human: kb_to_human(i.size_kb),
                    file_count: i.file_count,
                    status: "cleanable".into(),
                    whitelist_matched: wl,
                    default_selected: compute_default_selected(
                        "cleanable",
                        i.size_kb * 1024,
                        wl,
                        true,
                        "app_caches",
                        false,
                        system_clean,
                    ),
                    real_path: resolve_item_real_path("app_caches", &i.id),
                }
            })
            .collect();
        push_and_emit_category(
            &mut categories,
            CleanCategory {
                id: "app_caches".into(),
                title: "App caches".into(),
                tips: None,
                recommend: true,
                cautious: false,
                requires_sudo: false,
                whitelist_matched: ac_cat_wl,
                items: app_caches_items,
            },
        );

        log::info!(
            "[mole_clean] section App caches: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );
    } // section 3 (App caches) guard

    if !crate::core::base::is_clean_cancelled() {
        // ===== 4. 浏览器清理 =====
        let _sec_t = std::time::Instant::now();
        start_section("Browsers");
        let (kb, cnt) = user::clean_browsers();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(
                &app,
                &CleanupPhaseResultPayload {
                    section: "Browsers".into(),
                    phase: PHASE_BROWSERS.into(),
                    title: "Browser caches".into(),
                    cleaned: kb > 0,
                    size_kb: Some(kb),
                    file_count: Some(cnt),
                },
            );
        }
        let bc_wl = item_whitelisted("browser_cache_main", None);
        push_and_emit_category(
            &mut categories,
            CleanCategory {
                id: "browser_cache".into(),
                title: "Browser caches".into(),
                tips: None,
                recommend: true,
                cautious: false,
                requires_sudo: false,
                whitelist_matched: cat_whitelisted("browser_cache"),
                items: vec![make_item(
                    "browser_cache_main",
                    "Browser cache data",
                    kb,
                    cnt,
                    bc_wl,
                    compute_default_selected(
                        "cleanable",
                        kb * 1024,
                        bc_wl,
                        true,
                        "browser_cache",
                        false,
                        system_clean,
                    ),
                    None,
                )],
            },
        );

        log::info!(
            "[mole_clean] section Browsers: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );
    } // section 4 (Browsers) guard

    if !crate::core::base::is_clean_cancelled() {
        // ===== 5. 云服务和办公软件清理 =====
        let _sec_t = std::time::Instant::now();
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
                log::warn!(
                    "[clean] Cloud & Office cleanup failed with exit code {}",
                    exit_code
                );
            }
        }
        end_section();
        let (kb1, cnt1) = cloud_result.lock().unwrap().take().unwrap_or((0, 0));
        let (kb2, cnt2) = office_result.lock().unwrap().take().unwrap_or((0, 0));
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(
                &app,
                &CleanupPhaseResultPayload {
                    section: "Cloud & Office".into(),
                    phase: PHASE_CLOUD_STORAGE.into(),
                    title: "Cloud storage cache".into(),
                    cleaned: kb1 > 0,
                    size_kb: Some(kb1),
                    file_count: Some(cnt1),
                },
            );
        }
        let cs_wl = item_whitelisted("cloud_storage_main", None);
        push_and_emit_category(
            &mut categories,
            CleanCategory {
                id: "cloud_storage".into(),
                title: "Cloud storage cache".into(),
                tips: None,
                recommend: true,
                cautious: false,
                requires_sudo: false,
                whitelist_matched: cat_whitelisted("cloud_storage"),
                items: vec![make_item(
                    "cloud_storage_main",
                    "iCloud/Dropbox/OneDrive/Google Drive",
                    kb1,
                    cnt1,
                    cs_wl,
                    compute_default_selected(
                        "cleanable",
                        kb1 * 1024,
                        cs_wl,
                        true,
                        "cloud_storage",
                        false,
                        system_clean,
                    ),
                    None,
                )],
            },
        );
        let oc_wl = item_whitelisted("office_cache_main", None);
        push_and_emit_category(
            &mut categories,
            CleanCategory {
                id: "office_cache".into(),
                title: "Office caches".into(),
                tips: None,
                recommend: false,
                cautious: true,
                requires_sudo: false,
                whitelist_matched: cat_whitelisted("office_cache"),
                items: vec![make_item(
                    "office_cache_main",
                    "Microsoft Office caches",
                    kb2,
                    cnt2,
                    oc_wl,
                    compute_default_selected(
                        "cleanable",
                        kb2 * 1024,
                        oc_wl,
                        false,
                        "office_cache",
                        false,
                        system_clean,
                    ),
                    None,
                )],
            },
        );
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(
                &app,
                &CleanupPhaseResultPayload {
                    section: "Cloud & Office".into(),
                    phase: PHASE_OFFICE_CACHES.into(),
                    title: "Office caches".into(),
                    cleaned: kb2 > 0,
                    size_kb: Some(kb2),
                    file_count: Some(cnt2),
                },
            );
        }
        log::info!(
            "[mole_clean] section Cloud & Office: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );
    } // section 5 (Cloud & Office) guard

    // ===== 6. 开发者工具清理 =====
    if !crate::core::base::is_clean_cancelled() {
        let _sec_t = std::time::Instant::now();
        start_section("Developer tools");
        let dt_result = dev::clean_developer_tools();
        let kb = dt_result.total_kb();
        let cnt = dt_result.total_count();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(
                &app,
                &CleanupPhaseResultPayload {
                    section: "Developer tools".into(),
                    phase: PHASE_DEV_TOOLS.into(),
                    title: "Developer tools cache".into(),
                    cleaned: true,
                    size_kb: Some(kb),
                    file_count: Some(cnt),
                },
            );
        }
        let dt_cat_wl = cat_whitelisted("dev_tools");
        let dev_items: Vec<CleanItem> = dt_result
            .items
            .iter()
            .map(|i| {
                let wl = item_whitelisted(&i.id, i.path.as_deref());
                CleanItem {
                    id: i.id.clone(),
                    path: i.title.clone(),
                    size: i.size_kb * 1024,
                    size_human: kb_to_human(i.size_kb),
                    file_count: i.file_count,
                    status: "cleanable".into(),
                    whitelist_matched: wl,
                    default_selected: compute_default_selected(
                        "cleanable",
                        i.size_kb * 1024,
                        wl,
                        true,
                        "dev_tools",
                        false,
                        system_clean,
                    ),
                    real_path: resolve_item_real_path("dev_tools", &i.id),
                }
            })
            .collect();
        push_and_emit_category(
            &mut categories,
            CleanCategory {
                id: "dev_tools".into(),
                title: "Developer tools cache".into(),
                tips: None,
                recommend: true,
                cautious: false,
                requires_sudo: false,
                whitelist_matched: dt_cat_wl,
                items: dev_items,
            },
        );
        log::info!(
            "[mole_clean] section Developer tools: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );
    } // section 6 (Developer tools) guard

    if !crate::core::base::is_clean_cancelled() {
        // ===== 7. GUI应用程序清理 =====
        let _sec_t = std::time::Instant::now();
        start_section("Applications");
        let (kb, cnt) = app_caches::clean_user_gui_applications();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(
                &app,
                &CleanupPhaseResultPayload {
                    section: "Applications".into(),
                    phase: PHASE_APPLICATIONS.into(),
                    title: "Applications".into(),
                    cleaned: kb > 0,
                    size_kb: Some(kb),
                    file_count: Some(cnt),
                },
            );
        }
        let app_wl = item_whitelisted("applications_main", None);
        push_and_emit_category(
            &mut categories,
            CleanCategory {
                id: "applications".into(),
                title: "Applications".into(),
                tips: None,
                recommend: true,
                cautious: false,
                requires_sudo: false,
                whitelist_matched: cat_whitelisted("applications"),
                items: vec![make_item(
                    "applications_main",
                    "App-specific caches & data",
                    kb,
                    cnt,
                    app_wl,
                    compute_default_selected(
                        "cleanable",
                        kb * 1024,
                        app_wl,
                        true,
                        "applications",
                        false,
                        system_clean,
                    ),
                    None,
                )],
            },
        );
        log::info!(
            "[mole_clean] section Applications: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );
    } // section 7 (Applications) guard

    if !crate::core::base::is_clean_cancelled() {
        // ===== 8. 虚拟化工具清理 =====
        let _sec_t = std::time::Instant::now();
        start_section("Virtualization");
        let (kb, cnt) = user::clean_virtualization_tools();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(
                &app,
                &CleanupPhaseResultPayload {
                    section: "Virtualization".into(),
                    phase: PHASE_VIRTUALIZATION.into(),
                    title: "Virtualization caches".into(),
                    cleaned: kb > 0,
                    size_kb: Some(kb),
                    file_count: Some(cnt),
                },
            );
        }
        let virt_wl = item_whitelisted("virtualization_main", None);
        push_and_emit_category(
            &mut categories,
            CleanCategory {
                id: "virtualization".into(),
                title: "Virtualization caches".into(),
                tips: None,
                recommend: false,
                cautious: true,
                requires_sudo: false,
                whitelist_matched: cat_whitelisted("virtualization"),
                items: vec![make_item(
                    "virtualization_main",
                    "Docker/Parallels/VMware caches",
                    kb,
                    cnt,
                    virt_wl,
                    compute_default_selected(
                        "cleanable",
                        kb * 1024,
                        virt_wl,
                        false,
                        "virtualization",
                        false,
                        system_clean,
                    ),
                    None,
                )],
            },
        );
        log::info!(
            "[mole_clean] section Virtualization: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );
    } // section 8 (Virtualization) guard

    if !crate::core::base::is_clean_cancelled() {
        //===== 9. 应用支持日志清理 =====
        let _sec_t = std::time::Instant::now();
        start_section("Application Support");
        let (kb, cnt) = user::clean_application_support_logs();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(
                &app,
                &CleanupPhaseResultPayload {
                    section: "Application Support".into(),
                    phase: PHASE_APP_SUPPORT_LOGS.into(),
                    title: "Application Support logs/caches".into(),
                    cleaned: kb > 0,
                    size_kb: Some(kb),
                    file_count: Some(cnt),
                },
            );
        }
        let asl_wl = item_whitelisted("app_support_logs_main", None);
        push_and_emit_category(
            &mut categories,
            CleanCategory {
                id: "app_support_logs".into(),
                title: "Application Support logs/caches".into(),
                tips: None,
                recommend: true,
                cautious: false,
                requires_sudo: false,
                whitelist_matched: cat_whitelisted("app_support_logs"),
                items: vec![make_item(
                    "app_support_logs_main",
                    "~/Library/Application Support logs & caches",
                    kb,
                    cnt,
                    asl_wl,
                    compute_default_selected(
                        "cleanable",
                        kb * 1024,
                        asl_wl,
                        true,
                        "app_support_logs",
                        false,
                        system_clean,
                    ),
                    Some(format!(
                        "{}/Library/Application Support",
                        crate::core::base::home_dir()
                    )),
                )],
            },
        );
        log::info!(
            "[mole_clean] section Application Support: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );
    } // section 9 (Application Support) guard

    if !crate::core::base::is_clean_cancelled() {
        // ===== 10. 应用残留文件清理 =====
        let _sec_t = std::time::Instant::now();
        start_section("App leftovers");
        let (orphan_kb, orphan_cnt) = apps::clean_orphaned_app_data();
        let (svc_kb, svc_cnt) = apps::clean_orphaned_system_services();
        let (stub_kb, stub_cnt) = apps::clean_orphaned_container_stubs();
        let (ls_kb, ls_cnt) = launch_services::clean_stale_launch_services_registrations();
        let launch_agent_hints = hints::show_user_launch_agent_hint_notice();
        let dotdir_hints = hints::show_orphan_dotdir_hint_notice();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(
                &app,
                &CleanupPhaseResultPayload {
                    section: "App leftovers".into(),
                    phase: PHASE_ORPHANED_DATA.into(),
                    title: "Orphaned app data".into(),
                    cleaned: orphan_kb > 0,
                    size_kb: Some(orphan_kb),
                    file_count: Some(orphan_cnt),
                },
            );
            if svc_kb > 0 || svc_cnt > 0 {
                emit_cleanup_phase_result(
                    &app,
                    &CleanupPhaseResultPayload {
                        section: "App leftovers".into(),
                        phase: PHASE_ORPHANED_SYSTEM_SERVICES.into(),
                        title: "Orphaned system services".into(),
                        cleaned: svc_kb > 0,
                        size_kb: Some(svc_kb),
                        file_count: Some(svc_cnt),
                    },
                );
            }
            if stub_kb > 0 || stub_cnt > 0 {
                emit_cleanup_phase_result(
                    &app,
                    &CleanupPhaseResultPayload {
                        section: "App leftovers".into(),
                        phase: PHASE_ORPHANED_CONTAINER_STUBS.into(),
                        title: "Orphaned container stubs".into(),
                        cleaned: stub_kb > 0,
                        size_kb: Some(stub_kb),
                        file_count: Some(stub_cnt),
                    },
                );
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
        push_and_emit_category(
            &mut categories,
            CleanCategory {
                id: "orphaned_data".into(),
                title: "Orphaned app data".into(),
                tips: None,
                recommend: true,
                cautious: false,
                requires_sudo: false,
                whitelist_matched: cat_whitelisted("orphaned_data"),
                items: vec![make_item(
                    "orphaned_data_main",
                    "Data from removed apps, system services, container stubs, stale LaunchServices registrations",
                    od_total_kb,
                    od_total_cnt,
                    od_wl,
                    compute_default_selected(
                        "cleanable",
                        od_total_kb * 1024,
                        od_wl,
                        true,
                        "orphaned_data",
                        false,
                        system_clean,
                    ),
                    None,
                )],
            },
        );
        log::info!(
            "[mole_clean] section App leftovers: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );
    } // section 10 (App leftovers) guard

    // Sections 11-16：每个 section 前独立检查取消标志。
    if !crate::core::base::is_clean_cancelled() {
        // ===== 11. Apple Silicon专用缓存清理 =====
        let _sec_t = std::time::Instant::now();
        start_section("Apple Silicon");
        let (as_kb, as_cnt) = user::clean_apple_silicon_caches();
        end_section();
        if as_kb > 0 || as_cnt > 0 {
            if let Some(app) = current_spinner_app_handle() {
                emit_cleanup_phase_result(
                    &app,
                    &CleanupPhaseResultPayload {
                        section: "Apple Silicon".into(),
                        phase: PHASE_APPLE_SILICON_CACHES.into(),
                        title: "Apple Silicon caches".into(),
                        cleaned: as_kb > 0,
                        size_kb: Some(as_kb),
                        file_count: Some(as_cnt),
                    },
                );
            }
        }
        let as_wl = item_whitelisted("apple_silicon_main", None);
        push_and_emit_category(
            &mut categories,
            CleanCategory {
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
                    default_selected: compute_default_selected(
                        "cleanable",
                        as_kb * 1024,
                        as_wl,
                        true,
                        "apple_silicon",
                        true,
                        system_clean,
                    ),
                    real_path: None,
                }],
            },
        );
        log::info!(
            "[mole_clean] section Apple Silicon: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );
    } // section 11 (Apple Silicon) guard

    if !crate::core::base::is_clean_cancelled() {
        // ===== 12. 设备备份和固件清理 =====
        let _sec_t = std::time::Instant::now();
        start_section("Device backups & firmware");
        let (kb, cnt) = user::clean_cached_device_firmware();
        let ios_backup_hints = user::check_ios_device_backups();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(
                &app,
                &CleanupPhaseResultPayload {
                    section: "Device backups & firmware".into(),
                    phase: PHASE_DEVICE_FIRMWARE.into(),
                    title: "Device firmware & iOS backups".into(),
                    cleaned: kb > 0,
                    size_kb: Some(kb),
                    file_count: Some(cnt),
                },
            );
            if ios_backup_hints.detected {
                emit_cleanup_hints_result(&app, &ios_backup_hints);
            }
        }
        let df_wl = item_whitelisted("device_firmware_main", None);
        push_and_emit_category(
            &mut categories,
            CleanCategory {
                id: "device_firmware".into(),
                title: "Device firmware & iOS backups".into(),
                tips: None,
                recommend: false,
                cautious: true,
                requires_sudo: false,
                whitelist_matched: cat_whitelisted("device_firmware"),
                items: vec![make_item(
                    "device_firmware_main",
                    "iOS device firmware & backups",
                    kb,
                    cnt,
                    df_wl,
                    compute_default_selected(
                        "cleanable",
                        kb * 1024,
                        df_wl,
                        false,
                        "device_firmware",
                        false,
                        system_clean,
                    ),
                    None,
                )],
            },
        );
        log::info!(
            "[mole_clean] section Device firmware: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );
    } // section 12 (Device backups & firmware) guard

    if !crate::core::base::is_clean_cancelled() {
        //===== 13. Time Machine清理 =====
        let _sec_t = std::time::Instant::now();
        start_section("Time Machine");
        let (tm_kb, tm_cnt) = system::clean_time_machine_failed_backups();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            emit_cleanup_phase_result(
                &app,
                &CleanupPhaseResultPayload {
                    section: "Time Machine".into(),
                    phase: PHASE_TIME_MACHINE.into(),
                    title: "Failed Time Machine backups".into(),
                    cleaned: tm_kb > 0,
                    size_kb: Some(tm_kb),
                    file_count: Some(tm_cnt),
                },
            );
        }
        let tm_wl = item_whitelisted("time_machine_main", None);
        push_and_emit_category(
            &mut categories,
            CleanCategory {
                id: "time_machine".into(),
                title: "Time Machine failed backups".into(),
                tips: None,
                recommend: true,
                cautious: false,
                requires_sudo: false,
                whitelist_matched: cat_whitelisted("time_machine"),
                items: vec![make_item(
                    "time_machine_main",
                    "Failed Time Machine backups",
                    tm_kb,
                    tm_cnt,
                    tm_wl,
                    compute_default_selected(
                        "cleanable",
                        tm_kb * 1024,
                        tm_wl,
                        true,
                        "time_machine",
                        false,
                        system_clean,
                    ),
                    None,
                )],
            },
        );
        log::info!(
            "[mole_clean] section Time Machine: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );
    } // section 13 (Time Machine) guard

    if !crate::core::base::is_clean_cancelled() {
        //===== 14. 大文件检查 =====
        let _sec_t = std::time::Instant::now();
        start_section("Large files");
        let large_files = user::check_large_file_candidates();
        end_section();
        if let Some(app) = current_spinner_app_handle() {
            if large_files.detected {
                emit_cleanup_hints_result(&app, &large_files);
            }
        }
        push_and_emit_category(
            &mut categories,
            CleanCategory {
                id: "large_files".into(),
                title: "Large file candidates".into(),
                tips: Some(
                    "Review files >1 GB in home directory; not directly cleaned by mole.".into(),
                ),
                recommend: false,
                cautious: true,
                requires_sudo: false,
                whitelist_matched: cat_whitelisted("large_files"),
                items: vec![make_info_item(
                    "large_files_info",
                    "Large files >1 GB — review manually",
                    0,
                    0,
                    item_whitelisted("large_files_info", None),
                )],
            },
        );
        log::info!(
            "[mole_clean] section Large files: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );
    } // section 14 (Large files) guard

    if !crate::core::base::is_clean_cancelled() {
        // ===== 15. 系统数据线索提示 =====
        let _sec_t = std::time::Instant::now();
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
        let sysdata_items: Vec<CleanItem> = clues
            .iter()
            .map(|(label, sz_kb, path)| {
                CleanItem {
                    id: format!("sysdata_{}", label),
                    path: format!("{} — {}", label, path),
                    size: sz_kb * 1024,
                    size_human: bytes_to_human(sz_kb * 1024),
                    file_count: 0,
                    status: "info".into(),
                    whitelist_matched: false,
                    default_selected: false, // info 项不默认勾选
                    real_path: if path.starts_with('/') {
                        Some(path.clone())
                    } else {
                        None
                    },
                }
            })
            .collect();
        push_and_emit_category(
            &mut categories,
            CleanCategory {
                id: "system_data_clues".into(),
                title: "System Data clues".into(),
                tips: Some("Review: mo analyze, Device backups, docker system df".into()),
                recommend: false,
                cautious: false,
                requires_sudo: false,
                whitelist_matched: cat_whitelisted("system_data_clues"),
                items: if sysdata_items.is_empty() {
                    vec![make_info_item(
                        "sysdata_no_items",
                        "No significant system data clues found",
                        0,
                        0,
                        item_whitelisted("sysdata_no_items", None),
                    )]
                } else {
                    sysdata_items
                },
            },
        );
        log::info!(
            "[mole_clean] section System Data clues: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );
    } // section 15 (System Data clues) guard

    if !crate::core::base::is_clean_cancelled() {
        // ===== 16. 项目产物提示 =====
        let _sec_t = std::time::Instant::now();
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
                        format!(
                            "at least {} sampled from {} items",
                            size_human, hints_data.estimate_samples
                        )
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
            emit_cleanup_hints_result(
                &app,
                &CleanupHintsResultPayload {
                    section: "Project artifacts".into(),
                    phase: PHASE_PROJECT_ARTIFACTS.into(),
                    title: "Project artifacts".into(),
                    detected,
                    review_hint: "Review: mo purge".into(),
                    items,
                },
            );
        }
        let artifact_kb = if hints_data.detected {
            hints_data.estimated_kb
        } else {
            0
        };
        let artifact_count = if hints_data.detected {
            hints_data.count as u64
        } else {
            0
        };
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
        push_and_emit_category(
            &mut categories,
            CleanCategory {
                id: "project_artifacts".into(),
                title: "Project artifacts".into(),
                tips: Some(
                    "Build artifacts from dev projects; use `mo purge` to review & clean.".into(),
                ),
                recommend: false,
                cautious: false,
                requires_sudo: false,
                whitelist_matched: cat_whitelisted("project_artifacts"),
                items: vec![make_item(
                    "project_artifacts_main",
                    &artifact_label,
                    artifact_kb,
                    artifact_count,
                    pa_wl,
                    compute_default_selected(
                        "cleanable",
                        artifact_kb * 1024,
                        pa_wl,
                        false,
                        "project_artifacts",
                        false,
                        system_clean,
                    ),
                    None,
                )],
            },
        );
        log::info!(
            "[mole_clean] section Project artifacts: {:.1}s",
            _sec_t.elapsed().as_secs_f64()
        );
    } // section 16 (Project artifacts) guard

    // 诊断：打印每个 category 的详细信息
    for cat in &categories {
        let cat_items: Vec<String> = cat
            .items
            .iter()
            .map(|i| {
                format!(
                    "  [{}] id={} size={} status={} wl={}",
                    i.path, i.id, i.size, i.status, i.whitelist_matched
                )
            })
            .collect();
        log::info!(
            "[diagnose] category id={} title={} items={} total_size_kb={}",
            cat.id,
            cat.title,
            cat.items.len(),
            cat.items.iter().map(|i| i.size).sum::<u64>() / 1024
        );
        for item_str in cat_items {
            log::info!("[diagnose]   {}", item_str);
        }
    }
    log::info!("[diagnose] total categories count={}", categories.len());

    // 计算总量
    let total_kb: u64 = categories
        .iter()
        .flat_map(|c| c.items.iter())
        .map(|i| i.size / 1024)
        .sum();
    let total_file_count: u64 = categories
        .iter()
        .flat_map(|c| c.items.iter())
        .map(|i| i.file_count)
        .sum();

    log::info!(
        "[mole_clean] total cleanable: {}KB across {} categories",
        total_kb,
        categories.len()
    );

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

    let final_free = if is_execute {
        Some(boot_volume_free_bytes())
    } else {
        None
    };
    let final_free_human = if is_execute {
        Some(get_free_space())
    } else {
        None
    };

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
        if is_execute {
            actual_file_count
        } else {
            total_file_count
        },
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

    log::info!(
        "[diagnose] mole_clean spawn_blocking done, block_elapsed={:.1}s",
        t_block.elapsed().as_secs_f64()
    );

    let summary = CleanSummary {
        total_cleanable_size: if is_execute {
            None
        } else {
            Some(total_kb * 1024)
        },
        total_cleanable_size_human: if is_execute {
            None
        } else {
            Some(kb_to_human(total_kb))
        },
        total_cleaned_size: if is_execute {
            Some(cleaned_kb * 1024)
        } else {
            None
        },
        total_cleaned_size_human: if is_execute {
            Some(kb_to_human(cleaned_kb))
        } else {
            None
        },
        total_file_count: if is_execute {
            actual_file_count
        } else {
            total_file_count
        },
        category_count: Some(categories.len()),
        success_count: if is_execute {
            Some(success_count)
        } else {
            None
        },
        skipped_count: if is_execute {
            Some(skipped_count)
        } else {
            None
        },
        failed_count: None,
        final_free_space: final_free,
        final_free_space_human: final_free_human,
        free_space_change,
        free_space_change_human,
        status,
        movie_equivalent,
    };

    // 本次任务是否被取消：取消/超时的扫描不得发布可执行快照（scan_id），
    // 否则 clean_apply 可能基于「半份扫描结果」执行删除，违反快照防重放/防篡改约定。
    let was_cancelled = crate::core::base::is_clean_cancelled();
    // dry_run 模式下生成 scan_id 并存入快照注册表，供后续 clean_apply 验证
    let scan_id = if dry_run && !was_cancelled {
        let sid = generate_scan_id();
        let mut snapshot_items = std::collections::HashMap::new();
        for cat in &categories {
            for item in &cat.items {
                let key = format!("{}::{}", cat.id, item.id);
                snapshot_items.insert(
                    key,
                    SnapshotItem {
                        category_id: cat.id.clone(),
                        whitelist_matched: item.whitelist_matched || cat.whitelist_matched,
                        requires_sudo: cat.requires_sudo,
                        size: item.size,
                        status: item.status.clone(),
                    },
                );
            }
        }
        store_scan_snapshot(ScanSnapshot {
            scan_id: sid.clone(),
            created_at: std::time::Instant::now(),
            items: snapshot_items,
            size_metric: "logical".into(),
        });
        log::info!(
            "[mole_clean] scan snapshot stored: scan_id={}, items={}",
            sid,
            categories.iter().flat_map(|c| &c.items).count()
        );

        // 调试埋点：scan 阶段 JSONL 落盘（批次文件 debug-{timestamp}.log）
        debug_trace::begin_batch();
        debug_trace::scan_disk_before(
            &sid,
            initial_free_bytes,
            &bytes_to_human(initial_free_bytes.max(0) as u64),
        );
        let mut dbg_sel_items: u64 = 0;
        let mut dbg_sel_bytes: u64 = 0;
        let mut dbg_total_items: u64 = 0;
        let mut dbg_total_bytes: u64 = 0;
        for cat in &categories {
            for item in &cat.items {
                debug_trace::scan_item(
                    &sid,
                    &cat.id,
                    &item.id,
                    &item.path,
                    item.real_path.as_deref(),
                    item.size / 1024,
                    item.file_count,
                    &item.status,
                    item.default_selected,
                );
                dbg_total_items += 1;
                dbg_total_bytes += item.size;
                if item.default_selected {
                    dbg_sel_items += 1;
                    dbg_sel_bytes += item.size;
                }
            }
        }
        debug_trace::scan_default_selection_summary(
            &sid,
            dbg_sel_items,
            dbg_sel_bytes,
            dbg_total_items,
            dbg_total_bytes,
        );
        debug_trace::scan_summary(
            &sid,
            total_kb.saturating_mul(1024),
            categories.len(),
            total_file_count,
            t_block.elapsed().as_millis() as u64,
        );
        Some(sid)
    } else {
        None
    };

    CleanOutput {
        mode: if dry_run {
            "dry_run".into()
        } else {
            "execute".into()
        },
        collected_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        scan_id,
        // 复用 was_cancelled 单次读取：保证「是否发布 scan_id」与「上报 cancelled」一致，
        // 避免出现 scan_id 已发布却报告 cancelled=true 的矛盾终态。
        cancelled: was_cancelled,
        whitelist: Some(whitelist_info),
        categories: if is_execute { None } else { Some(categories) },
        results,
        summary,
    }
}
