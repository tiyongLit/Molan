//! 卸载执行编排 — 单应用 dry-run 评估与真实执行。
//! 自 `controllers/uninstall.rs` 纯搬迁（行为不变）。

use std::collections::HashMap;

use serde_json::Value;

use crate::uninstall::app_scan::{
    derive_file_type, is_path_sensitive, precompute_file_sizes, read_last_used_date,
    read_short_version, relative_time_from_epoch, resolve_display_name,
};

pub fn run_dry_run(app_path: &str, data_only: bool) -> Result<Value, String> {
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
        // mole dry-run 不按 size 过滤；前端对 size=0 显示 "—"。
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

pub fn run_execute(
    app: &tauri::AppHandle,
    app_path: &str,
    data_only: bool,
) -> Result<Value, String> {
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
