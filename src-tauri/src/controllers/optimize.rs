use serde::Serialize;
use serde_json::Value;
use std::time::Instant;
use tauri::{AppHandle, Emitter};

use crate::check::health_json::{HealthOptimizationItem, collect_health_json};
use crate::core::sudo;
use crate::manage::autofix::perform_auto_fix;
use crate::optimize::diagnostics;
use crate::optimize::outcome::OptimizeOutcome;
use crate::optimize::tasks::execute_optimization;
use crate::optimize::{clear_failure, take_failure};
use crate::whitelist_optimize;

/// 与前端 `EVT_OPTIMIZE_PROGRESS` 字符串保持一致。
const EVT_OPTIMIZE_PROGRESS: &str = "optimize::progress";

#[derive(Serialize, Clone)]
struct OptimizeTask {
    id: String,
    name: String,
    description: String,
    safe: bool,
    status: String,
}

#[derive(Serialize, Clone)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum OptimizeProgress {
    /// 即将开始执行所有选中任务。
    Begin { total: usize, actions: Vec<String> },
    /// 当前任务开始。
    TaskStart {
        index: usize,
        total: usize,
        action: String,
        name: String,
        description: String,
    },
    /// 任务被跳过（白名单 / sudo 不可用）。
    TaskSkipped {
        index: usize,
        total: usize,
        action: String,
        name: String,
        reason: String,
    },
    /// 当前任务完成。
    TaskDone {
        index: usize,
        total: usize,
        action: String,
        name: String,
        ok: bool,
        /// 六态结局(applied/unchanged/skipped/unavailable/attention/failed)
        outcome: String,
        duration_ms: u128,
        error: Option<String>,
    },
    /// 全部完成。
    Complete {
        total: usize,
        success: usize,
        failed: usize,
        skipped: usize,
        duration_ms: u128,
        /// 六态统计:status → 数量
        outcomes: std::collections::HashMap<String, usize>,
    },
}

fn emit(app: &AppHandle, payload: &OptimizeProgress) {
    let _ = app.emit(EVT_OPTIMIZE_PROGRESS, payload);
}

fn build_task_list() -> Vec<HealthOptimizationItem> {
    match collect_health_json() {
        Ok(h) => h.optimizations,
        Err(e) => {
            log::warn!("[mole_optimize] collect_health_json failed: {e}");
            Vec::new()
        }
    }
}

/// 对齐 Shell `run_optimize_diagnostics`（diagnostics.sh:L415）。
/// 采集性能诊断数据并返回 JSON，前端可据此展示瓶颈提示。
fn collect_diagnostics_data() -> Value {
    let sample1 = diagnostics::opt_diag_get_ps_sample(1);
    let delay = diagnostics::opt_diag_sample_delay();
    let env1 = std::env::var("MOLE_OPTIMIZE_PS_SAMPLE_1").is_ok();
    let env2 = std::env::var("MOLE_OPTIMIZE_PS_SAMPLE_2").is_ok();
    if !env1 || !env2 {
        std::thread::sleep(std::time::Duration::from_secs_f64(delay.max(0.0)));
    }
    let sample2 = diagnostics::opt_diag_get_ps_sample(2);
    let totals1 = diagnostics::opt_diag_family_totals(&sample1);
    let totals2 = diagnostics::opt_diag_family_totals(&sample2);
    let threshold = diagnostics::opt_diag_cpu_threshold();

    let families = [
        "cloudshell",
        "syspolicyd",
        "windowserver",
        "spotlight",
        "coresim_disk_images",
    ];

    let mut sustained: Vec<Value> = Vec::new();
    let mut primary_family = "";
    let mut primary_avg: f64 = 0.0;

    for family in families.iter() {
        let cpu1 = diagnostics::opt_diag_family_total_for(&totals1, family);
        let cpu2 = diagnostics::opt_diag_family_total_for(&totals2, family);
        if cpu1 >= threshold && cpu2 >= threshold {
            let avg = (cpu1 + cpu2) / 2.0;
            let label = diagnostics::opt_diag_family_label(family);
            sustained.push(serde_json::json!({
                "family": family,
                "label": label,
                "avg_cpu": (avg * 10.0).round() / 10.0,
            }));
            if primary_family.is_empty() || avg > primary_avg {
                primary_family = family;
                primary_avg = avg;
            }
        }
    }

    let hdiutil_info = diagnostics::opt_diag_get_hdiutil_info();
    let image_pairs = diagnostics::opt_diag_parse_image_mount_pairs(&hdiutil_info);
    let detach_candidates = diagnostics::opt_diag_collect_detach_candidates(&image_pairs);
    let detach_list: Vec<Value> = detach_candidates
        .iter()
        .map(|(img, mnt)| serde_json::json!({"image": img, "mount": mnt}))
        .collect();

    // 对齐 SH L398-402：内存压力 / 空闲虚拟机 / 失控进程检测
    let mem_pressure = diagnostics::opt_diag_memory_pressure();
    let idle_vm = diagnostics::opt_diag_idle_vm();
    let runaway = diagnostics::opt_diag_runaway_process();
    let has_extra_findings = mem_pressure.is_some() || idle_vm.is_some() || !runaway.is_empty();

    let mut result = serde_json::json!({
        "has_bottleneck": !primary_family.is_empty() || has_extra_findings,
        "sustained": sustained,
        "detach_candidates": detach_list,
    });

    if !primary_family.is_empty() {
        let note = diagnostics::opt_diag_family_note(primary_family);
        let mut entry = serde_json::json!({
            "family": primary_family,
            "label": diagnostics::opt_diag_family_label(primary_family),
            "avg_cpu": (primary_avg * 10.0).round() / 10.0,
        });
        if !note.is_empty() {
            entry["note"] = serde_json::json!(note);
        }
        result["primary"] = entry;

        // syspolicyd extra info
        if primary_family == "syspolicyd" {
            let spctl_status = diagnostics::opt_diag_get_spctl_status();
            if !spctl_status.is_empty() {
                result["spctl_status"] = serde_json::json!(spctl_status);
            }
        }
    }

    // 追加 3 项新诊断发现
    if let Some(mem) = mem_pressure {
        result["memory_pressure"] = serde_json::json!(mem);
    }
    if let Some(vm) = idle_vm {
        result["idle_vm"] = serde_json::json!(vm);
    }
    if !runaway.is_empty() {
        result["runaway_processes"] = serde_json::json!(runaway);
    }

    result
}

fn make_dry_run_response(items: &[HealthOptimizationItem]) -> Value {
    let tasks: Vec<OptimizeTask> = items
        .iter()
        .map(|i| OptimizeTask {
            id: i.action.clone(),
            name: i.name.clone(),
            description: i.description.clone(),
            safe: i.safe,
            status: "pending".to_string(),
        })
        .collect();

    let safe_count = tasks.iter().filter(|t| t.safe).count();
    let total = tasks.len();

    let system_info = collect_health_json().ok().map(|h| {
        let used_pct = if h.memory_total_gb > 0.0 {
            (h.memory_used_gb / h.memory_total_gb * 100.0 * 10.0).round() / 10.0
        } else {
            0.0
        };
        serde_json::json!({
            "memory_used_gb": h.memory_used_gb,
            "memory_total_gb": h.memory_total_gb,
            "memory_used_percent": used_pct,
            "disk_used_gb": h.disk_used_gb,
            "disk_total_gb": h.disk_total_gb,
            "disk_used_percent": h.disk_used_percent,
            "uptime_days": h.uptime_days,
        })
    });

    let diagnostics_data = collect_diagnostics_data();

    serde_json::json!({
        "mode": "dry_run",
        "collected_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "system_info": system_info,
        "tasks": tasks,
        "diagnostics": diagnostics_data,
        "summary": {
            "total_tasks": total,
            "safe_count": safe_count,
            "would_apply_count": total,
        }
    })
}

/// 前端扁平传参：`{ dry_run, selected_actions? }`（与 `useTauri` 透传一致；不要用 `args` 包裹）。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_optimize(
    app: AppHandle,
    dry_run: bool,
    selected_actions: Option<Vec<String>>,
) -> Result<Value, String> {
    let items = build_task_list();

    if dry_run {
        return Ok(make_dry_run_response(&items));
    }

    // 选中过滤：保留 health_json 中的顺序（与 CLI 一致），仅保留命中 selected 的 action。
    let selected_set = selected_actions
        .as_ref()
        .map(|v| v.iter().cloned().collect::<std::collections::HashSet<_>>());
    let plan: Vec<HealthOptimizationItem> = items
        .into_iter()
        .filter(|it| match &selected_set {
            Some(s) => s.contains(&it.action),
            None => true,
        })
        .collect();

    let total = plan.len();
    if total == 0 {
        return Ok(serde_json::json!({
            "mode": "execute",
            "collected_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "results": [],
            "summary": {
                "total_tasks": 0,
                "applied_count": 0,
                "failed_count": 0,
                "outcomes": {
                    "applied": 0,
                    "unchanged": 0,
                    "skipped": 0,
                    "unavailable": 0,
                    "attention": 0,
                    "failed": 0
                },
                "duration_seconds": 0,
            }
        }));
    }

    // 加载白名单（对齐 optimize.sh load_whitelist "optimize"）
    let home = std::path::PathBuf::from(crate::core::base::home_dir());
    let whitelist_patterns = whitelist_optimize::load_optimize_whitelist_patterns(&home);

    // 进入执行：在 spawn_blocking 里逐个调用 execute_optimization，并 emit 进度。
    let app_clone = app.clone();
    let actions: Vec<String> = plan.iter().map(|p| p.action.clone()).collect();

    let summary = tauri::async_runtime::spawn_blocking(move || {
        let _busy = crate::core::busy_state::enter_busy();
        let _awake = crate::core::keep_awake::KeepAwakeGuard::acquire("System optimization");
        // 关键：execute 模式下确保 lib 不读到 dry_run。
        std::env::remove_var("MOLE_DRY_RUN");

        // 建立管理员会话，设置 MOLE_OPTIMIZE_SUDO_AVAILABLE
        // 对齐 Shell optimize.sh:L297-307：sudo 被拒绝时任务可降级跳过
        let sudo_available = sudo::ensure_admin_session();
        unsafe {
            std::env::set_var(
                "MOLE_OPTIMIZE_SUDO_AVAILABLE",
                if sudo_available { "true" } else { "false" },
            );
        }

        emit(
            &app_clone,
            &OptimizeProgress::Begin {
                total,
                actions: actions.clone(),
            },
        );

        let started = Instant::now();
        let mut success = 0usize;
        let mut failed = 0usize;
        let mut skipped = 0usize;
        let mut outcome_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut results = Vec::with_capacity(total);

        for (idx, item) in plan.iter().enumerate() {
            let i = idx + 1;

            // 白名单过滤（对齐 optimize.sh is_whitelisted 检查）
            if whitelist_optimize::is_whitelisted_optimize(&item.action, &whitelist_patterns, &home)
            {
                emit(
                    &app_clone,
                    &OptimizeProgress::TaskSkipped {
                        index: i,
                        total,
                        action: item.action.clone(),
                        name: item.name.clone(),
                        reason: "已加入白名单".to_string(),
                    },
                );
                skipped += 1;
                *outcome_counts.entry("skipped".to_string()).or_insert(0) += 1;
                results.push(serde_json::json!({
                    "task_id": &item.action,
                    "task_name": &item.name,
                    "status": "skipped",
                    "duration_seconds": 0.0,
                    "error": "已加入白名单",
                }));
                continue;
            }

            emit(
                &app_clone,
                &OptimizeProgress::TaskStart {
                    index: i,
                    total,
                    action: item.action.clone(),
                    name: item.name.clone(),
                    description: item.description.clone(),
                },
            );

            let task_started = Instant::now();
            // 每个任务前清空失败槽，任务内 `note_failure` 写入的原因
            // 在失败时随 TaskDone.error 透传给前端展示失败明细
            clear_failure();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                execute_optimization(&item.action)
            }));
            let duration_ms = task_started.elapsed().as_millis();

            // 六态结局协议:handler 返回 OptimizeOutcome;未知 action / panic 折算 failed
            let (ok, status, error) = match outcome {
                Ok(Some(o)) => {
                    let is_failed = o == OptimizeOutcome::Failed;
                    // 无论是否失败都取走槽内消息，避免残留污染下一个任务
                    let reason = take_failure();
                    (
                        !is_failed,
                        o.as_str().to_string(),
                        if is_failed { reason } else { None },
                    )
                }
                Ok(None) => (
                    false,
                    "failed".to_string(),
                    Some("unknown action".to_string()),
                ),
                Err(_) => (
                    false,
                    "failed".to_string(),
                    Some("task panicked".to_string()),
                ),
            };

            if ok {
                success += 1;
            } else {
                failed += 1;
            }
            *outcome_counts.entry(status.clone()).or_insert(0) += 1;

            results.push(serde_json::json!({
                "task_id": &item.action,
                "task_name": &item.name,
                "status": status,
                "duration_seconds": (duration_ms as f64) / 1000.0,
                "error": error,
            }));

            emit(
                &app_clone,
                &OptimizeProgress::TaskDone {
                    index: i,
                    total,
                    action: item.action.clone(),
                    name: item.name.clone(),
                    ok,
                    outcome: status,
                    duration_ms,
                    error,
                },
            );
        }

        let duration_ms = started.elapsed().as_millis();

        // 收集系统健康快照（对齐 Shell show_system_health）
        let system_info = collect_health_json().ok().map(|h| {
            let used_pct = if h.memory_total_gb > 0.0 {
                (h.memory_used_gb / h.memory_total_gb * 100.0 * 10.0).round() / 10.0
            } else {
                0.0
            };
            serde_json::json!({
                "memory_used_gb": h.memory_used_gb,
                "memory_total_gb": h.memory_total_gb,
                "memory_used_percent": used_pct,
                "disk_used_gb": h.disk_used_gb,
                "disk_total_gb": h.disk_total_gb,
                "disk_used_percent": h.disk_used_percent,
                "uptime_days": h.uptime_days,
            })
        });

        // 收集优化统计（对齐 Shell show_optimization_summary）
        let cache_kb: f64 = std::env::var("OPTIMIZE_CACHE_CLEANED_KB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
        let db_count: u32 = std::env::var("OPTIMIZE_DATABASES_COUNT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let config_count: u32 = std::env::var("OPTIMIZE_CONFIGS_REPAIRED")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        emit(
            &app_clone,
            &OptimizeProgress::Complete {
                total,
                success,
                failed,
                skipped,
                duration_ms,
                outcomes: outcome_counts.clone(),
            },
        );

        serde_json::json!({
            "mode": "execute",
            "collected_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "results": results,
            "system_info": system_info,
            "stats": {
                "cache_cleaned_kb": cache_kb,
                "databases_optimized": db_count,
                "configs_repaired": config_count,
            },
            "summary": {
                "total_tasks": total,
                "applied_count": success,
                "failed_count": failed,
                "skipped_count": skipped,
                "outcomes": outcome_counts,
                "duration_seconds": (duration_ms as f64) / 1000.0,
            }
        })
    })
    .await
    .map_err(|e| format!("optimize task panicked: {e}"))?;

    Ok(summary)
}

// ============================================================
// Check — 系统检查（Mole optimize 的子功能）
// ============================================================

#[tauri::command(rename_all = "snake_case")]
pub fn mole_check() -> Result<Value, String> {
    crate::check::all::generate_check_report_json_value()
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_check_fix() -> Result<Value, String> {
    let result = perform_auto_fix();
    Ok(serde_json::json!({
        "applied": result.applied,
        "items": result.items,
        "summary": {
            "auto_fix_available": result.applied > 0,
            "auto_fix_items": result.items,
        }
    }))
}

// ============================================================
// Touch ID — sudo Touch ID 配置（Mole optimize 的子功能）
// ============================================================

fn touchid_configured() -> bool {
    std::fs::read_to_string("/etc/pam.d/sudo")
        .map(|s| s.contains("pam_tid.so"))
        .unwrap_or(false)
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_touchid_status() -> Result<Value, String> {
    let supported = crate::core::sudo::check_touchid_support();
    let enabled = touchid_configured();
    Ok(serde_json::json!({
        "supported": supported,
        "enabled": enabled,
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_touchid_enable() -> Result<Value, String> {
    log::info!(
        "[mole_touchid_enable] enabling Touch ID for sudo via autofix.TOUCHID_NOT_CONFIGURED"
    );
    if touchid_configured() {
        return Ok(serde_json::json!({
            "supported": true,
            "enabled": true,
            "applied": false,
        }));
    }
    // 复用 perform_auto_fix 中已有的 PAM 写入分支：通过环境变量驱动。
    std::env::set_var("TOUCHID_NOT_CONFIGURED", "true");
    let result = perform_auto_fix();
    std::env::remove_var("TOUCHID_NOT_CONFIGURED");
    let enabled_now = touchid_configured();
    Ok(serde_json::json!({
        "supported": true,
        "enabled": enabled_now,
        "applied": result.applied > 0,
    }))
}
