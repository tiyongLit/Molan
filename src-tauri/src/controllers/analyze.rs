//! GUI 入口：对齐 Mole `analyze --json`。
//!
//! - 不再用 `WalkDir(max_depth=1)` 这种简化实现；
//! - 直接调用从 `Mole/cmd/analyze` 翻译而来的 `cmd::analyze::json::try_perform_scan_for_json`，
//!   语义与 `analyze-go --json [<path>]` 完全一致；
//! - 入参扁平：`path` + `overview`，与前端 `useTauri()` 的约定一致（不要再用 `args` 包一层）。

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use tauri::Manager;

use crate::analyze::cache;
use crate::analyze::delete::trash_path_with_progress;
use crate::analyze::json::try_perform_scan_for_json_impl;
use crate::analyze::scanner;
use crate::analyze::session;
use crate::core::base::set_analyze_app_handle;
use crate::events::{TrashProgressPayload, emit_analyze_trash_progress};

#[tauri::command(rename_all = "snake_case")]
pub fn mole_open_analyze_window(app: tauri::AppHandle) -> Result<(), String> {
    log::info!("[mole_open_analyze_window] called");
    if let Some(window) = app.get_webview_window("analyze") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    use tauri::WebviewUrl;
    use tauri::WebviewWindowBuilder;
    use tauri::webview::PageLoadEvent;

    let window = WebviewWindowBuilder::new(&app, "analyze", WebviewUrl::App("/analyze".into()))
        .title("磁盘分析")
        .inner_size(1056.0, 640.0)
        .resizable(true)
        .visible(false)
        // Reveal 门控：等前端渲染完成后再显示窗口，避免白屏闪烁
        .on_page_load(|webview, payload| {
            if let PageLoadEvent::Finished = payload.event() {
                let _ = webview.show();
                let _ = webview.set_focus();
                log::info!("[analyze] window revealed on page load");
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
            log::info!("[analyze] window revealed by 3s fallback");
        }
    });

    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_open_shell_window(app: tauri::AppHandle) -> Result<(), String> {
    log::info!("[mole_open_shell_window] called");

    // ── Find or create shell window ──
    let window = if let Some(window) = app.get_webview_window("shell") {
        // 窗口已存在（之前隐藏），直接显示
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    } else {
        use tauri::webview::PageLoadEvent;
        use tauri::{TitleBarStyle, WebviewUrl, WebviewWindowBuilder};

        let window = WebviewWindowBuilder::new(&app, "shell", WebviewUrl::App("/shell".into()))
            .title("Molan")
            .inner_size(940.0, 640.0)
            .resizable(true)
            .visible(false)
            .transparent(true)
            .title_bar_style(TitleBarStyle::Overlay)
            .hidden_title(true)
            // Reveal 门控：等前端渲染完成后再显示窗口，避免白屏闪烁
            .on_page_load(|webview, payload| {
                if let PageLoadEvent::Finished = payload.event() {
                    let _ = webview.show();
                    let _ = webview.set_focus();
                    // macOS: show() 后会重新启用原生阴影，需要再次关闭
                    #[cfg(target_os = "macos")]
                    {
                        use objc2::msg_send;
                        use objc2_foundation::NSObject as ObjcNSObject;
                        let _ = webview.ns_window().map(|ns_window| {
                            let ns: *mut ObjcNSObject = ns_window as *mut _;
                            unsafe {
                                let _: () = msg_send![ns, setHasShadow: false];
                            }
                        });
                    }
                    log::info!("[shell] window revealed on page load");
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
                // macOS: fallback reveal 同样需要关闭阴影
                #[cfg(target_os = "macos")]
                {
                    use objc2::msg_send;
                    use objc2_foundation::NSObject as ObjcNSObject;
                    let _ = fallback_win.ns_window().map(|ns_window| {
                        let ns: *mut ObjcNSObject = ns_window as *mut _;
                        unsafe {
                            let _: () = msg_send![ns, setHasShadow: false];
                        }
                    });
                }
                log::info!("[shell] window revealed by 3s fallback");
            }
        });

        window
    };

    Ok(())
}

/// 展开 `~/...`、`~`（当前用户）以及 IME 误输入的全角 `～`。
/// `~其他用户` 与 Shell 不一致且依赖系统账户库，这里明确报错。
fn expand_home_tilde(raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(PathBuf::new());
    }

    let normalized = if let Some(rest) = raw.strip_prefix('\u{FF5E}') {
        // 全角 ～ → 按 ASCII ~ 处理
        if rest.is_empty() {
            "~".to_string()
        } else if rest.starts_with('/') {
            format!("~{rest}")
        } else {
            format!("~/{}", rest.trim_start_matches('/'))
        }
    } else {
        raw.to_string()
    };

    if normalized == "~" {
        return crate::core::base::home_dir_opt().ok_or_else(|| "无法解析用户主目录".to_string());
    }
    if let Some(rest) = normalized.strip_prefix("~/") {
        let home =
            crate::core::base::home_dir_opt().ok_or_else(|| "无法解析用户主目录".to_string())?;
        return Ok(home.join(rest));
    }
    if normalized.starts_with('~') && normalized.len() > 1 {
        return Err(
            "不支持「~用户名」形式的路径，请使用绝对路径（例如 /Users/你的名字/...）".to_string(),
        );
    }

    Ok(PathBuf::from(normalized))
}

/// 与 Go `cmd/analyze/main.go` 一致的入口语义：
/// - **空路径** ⇒ `path = "/"` 且 `overview = true`（与 CLI 默认行为相同）；
/// - **非空路径** ⇒ 解析为绝对路径，`overview` 由前端显式指定（默认 `false`）。
fn resolve_target(path: &str, overview: bool) -> Result<(String, bool), String> {
    if path.trim().is_empty() {
        return Ok(("/".to_string(), true));
    }

    let expanded = expand_home_tilde(path)?;
    let abs = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .map_err(|e| format!("cannot resolve current dir: {e}"))?
            .join(expanded)
    };

    // 与 Go `filepath.Abs` 行为接近：路径存在则 canonicalize，否则保留拼接结果。
    let abs_str = if abs.exists() {
        match abs.canonicalize() {
            Ok(c) => c.to_string_lossy().into_owned(),
            Err(_) => abs.to_string_lossy().into_owned(),
        }
    } else {
        abs.to_string_lossy().into_owned()
    };

    if !Path::new(&abs_str).exists() {
        log::error!("[resolve_target] path does not exist: {abs_str}");
        return Err(format!(
            "路径不存在: {}（已展开 ~/ 为主目录。若有笔误请核对，例如 m/Molan → mvp/Molan）",
            abs_str
        ));
    }

    Ok((abs_str, overview))
}

/// 获取指定路径所在卷的可用空间（bytes）。
///
/// V2：不再调用外部 `df`（对齐红线 1）。改用 `NSURLVolumeAvailableCapacityForImportantUsageKey`
/// —— 与 `metrics_disk::get_volume_capacity` 同一数据源（macOS「储存概述」口径），
/// 遵守全应用 `DiskStatus.free` 单一事实来源规范。
#[cfg(target_os = "macos")]
fn get_disk_free_bytes(path: &str) -> Option<i64> {
    crate::status::metrics_disk::available_capacity_for_path(path).map(|v| v as i64)
}

#[cfg(not(target_os = "macos"))]
fn get_disk_free_bytes(_path: &str) -> Option<i64> {
    None
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_analyze_cancel() -> Result<(), String> {
    log::info!("[mole_analyze_cancel] cancelling active scan");
    scanner::cancel_active_scan();
    Ok(())
}

/// 轻量级目录导航：从内存会话树中读取指定路径的子项（微秒级，零磁盘 I/O）。
///
/// 前端首次 scan 后所有 drillIn/goBack/breadcrumb 都走此命令，
/// 不再每次重新调用 mole_analyze 触发缓存/扫描管线。
///
/// async + spawn_blocking：大目录（数千条）的 entries 重建 + JSON 序列化可达数十 ms，
/// 同步命令会阻塞主线程（Tauri 同步命令在主线程执行）。
///
/// 返回体 large_files 恒为空 —— Top20 大文件是全树固定数据，前端 scanRoot 时已缓存，
/// navigate 时保留上一轮值，省去每次导航的 clone + 序列化。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_analyze_navigate(path: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let t0 = std::time::Instant::now();
        match session::navigate(&path) {
            Some(result) => {
                let json_entries =
                    crate::analyze::json::json_entries_from_dir_entries(&result.entries);
                let output = crate::analyze::json::JsonOutput {
                    path: path.clone(),
                    overview: false,
                    entries: json_entries,
                    large_files: Vec::new(),
                    total_size: result.total_size,
                    total_files: result.total_files,
                };
                let value = serde_json::to_value(&output)
                    .map_err(|e| format!("serialize navigate: {e}"))?;
                log::debug!(
                    "[mole_analyze_navigate] path={path} entries={} in {:.3}ms",
                    output.entries.len(),
                    t0.elapsed().as_secs_f64() * 1000.0
                );
                Ok(value)
            }
            None => {
                // 路径不在会话树中（bundle 叶子钻取 / 树未建立）→ 前端 fallback 到 mole_analyze
                Err("NAVIGATE_MISS".into())
            }
        }
    })
    .await
    .map_err(|e| format!("navigate task panicked: {e}"))?
}

/// 释放会话内存树（用户离开 Analyze 页面时前端调用）。
#[tauri::command(rename_all = "snake_case")]
pub fn mole_analyze_clear_session() -> Result<(), String> {
    session::clear();
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mole_analyze(
    app: tauri::AppHandle,
    path: String,
    overview: Option<bool>,
    skip_cache: Option<bool>,
) -> Result<Value, String> {
    let is_overview = overview.unwrap_or(false);
    let skip = skip_cache.unwrap_or(false);
    let (target, is_overview) = resolve_target(&path, is_overview)?;

    log::info!(
        "[mole_analyze] target={target} overview={is_overview} skip_cache={skip} (raw_path={path:?})"
    );

    // 注入 AppHandle，让 scanner 内部能 emit scan-progress
    let app_clone = app.clone();
    set_analyze_app_handle(Some(app_clone));

    // 后台清理过期缓存（best-effort，不阻塞扫描）
    std::thread::spawn(|| {
        cache::prune_analyzer_cache();
    });

    // 对非 overview 扫描：注入字节目标估算（进度百分比分母），全链路统一。
    // 优先：目标路径在既往快照（新鲜或 stale）中的子树大小 —— 重扫/钻取秒级精准；
    // 兜底：根卷已用空间 —— 首次全盘扫描（对齐柠檬 hadScanDiskSize/usedDiskSize 字节口径）。
    if !is_overview {
        let mut bytes_target: i64 = 0;
        if let Ok(Some(hit)) = cache::find_fresh_snapshot(&target) {
            bytes_target = hit.snapshot.nodes.get(&target).map(|n| n.size).unwrap_or(0);
        }
        if bytes_target <= 0 {
            if let Ok(Some(hit)) = cache::find_stale_snapshot(&target) {
                bytes_target = hit.snapshot.nodes.get(&target).map(|n| n.size).unwrap_or(0);
            }
        }
        let (used, total) = crate::analyze::json::root_disk_usage().unwrap_or((0, 0));
        if bytes_target <= 0 {
            bytes_target = used;
        }
        scanner::set_scan_bytes_estimate(bytes_target, total);
    }

    let disk_free = get_disk_free_bytes(&target);

    let scan_start = std::time::Instant::now();
    let target_for_closure = target.clone();

    // 与 CLI 对齐：不设全局超时。scanner 内部对 du 等外部命令有独立的 5 分钟 deadline，
    // 对递归扫描目录有 fast-walk deadline。大目录（如 ~/Library）可能需要超过 2 分钟，
    // 但第一次扫描后结果会缓存到磁盘，后续访问瞬间返回。
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _busy = crate::core::busy_state::enter_busy();
        let _awake = crate::core::keep_awake::KeepAwakeGuard::acquire("Disk analysis");
        try_perform_scan_for_json_impl(&target_for_closure, is_overview, skip)
    })
    .await
    .map_err(|e| format!("analyze task panicked: {e}"))?;

    // 清理扫描预估（避免下一轮扫描读到过期值）
    scanner::set_scan_bytes_estimate(0, 0);

    let scan_elapsed = scan_start.elapsed();
    log::info!(
        "[mole_analyze] scan done in {:.1}s",
        scan_elapsed.as_secs_f64()
    );
    if scan_elapsed.as_secs() > 120 {
        log::warn!(
            "[mole_analyze] LONG SCAN: {:.0}s, target={}",
            scan_elapsed.as_secs_f64(),
            target
        );
    }

    // 清理 AppHandle
    set_analyze_app_handle(None);

    // 用户取消：转成稳定错误码（SCAN_CANCELLED），前端据此静默收尾，
    // 不把主动取消当成「扫描失败」弹错（对齐 Clean 的 cancelled 语义）。
    let output = match result {
        Ok(v) => v,
        Err(e) => {
            if e.contains(scanner::SCAN_CANCELLED_MARK) {
                log::info!("[mole_analyze] scan cancelled by user, target={target}");
                return Err(scanner::SCAN_CANCELLED_CODE.into());
            }
            return Err(e);
        }
    };
    let ser_start = std::time::Instant::now();
    let mut value =
        serde_json::to_value(&output).map_err(|e| format!("encode JsonOutput failed: {e}"))?;
    let ser_elapsed = ser_start.elapsed();
    log::info!(
        "[mole_analyze] serialize in {:.1}s, entries={}, large_files={}, total_size={}",
        ser_elapsed.as_secs_f64(),
        output.entries.len(),
        output.large_files.len(),
        output.total_size
    );
    if ser_elapsed.as_secs() > 10 {
        log::warn!(
            "[mole_analyze] SLOW SERIALIZE: {:.1}s, entries={}, large_files={}",
            ser_elapsed.as_secs_f64(),
            output.entries.len(),
            output.large_files.len()
        );
    }

    // 注入 disk_free 到返回的 JSON 对象
    if let Some(obj) = value.as_object_mut() {
        if let Some(df) = disk_free {
            obj.insert("diskFree".into(), serde_json::json!(df));
        }
    }

    Ok(value)
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeTrashArgs {
    paths: Vec<String>,
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_analyze_trash(app: tauri::AppHandle, args: AnalyzeTrashArgs) -> Result<Value, String> {
    log::info!(
        "[mole_analyze_trash] called with {} paths: {:?} | raw args JSON: {}",
        args.paths.len(),
        args.paths,
        serde_json::to_string(&args).unwrap_or_default()
    );

    // ── 1. 受保护路径兜底检查 ──
    let mut allowed: Vec<String> = Vec::new();
    let mut protected_names: Vec<String> = Vec::new();
    for path in &args.paths {
        if crate::analyze::protected::is_protected_entry_path(path) {
            if let Some(name) = Path::new(path).file_name().and_then(|n| n.to_str()) {
                protected_names.push(name.to_string());
            } else {
                protected_names.push(path.clone());
            }
            log::warn!("[mole_analyze_trash] blocked protected path: {path}");
        } else {
            allowed.push(path.clone());
        }
    }

    if !protected_names.is_empty() {
        return Err(format!(
            "以下目录受系统保护，不可删除: {}",
            protected_names.join(", ")
        ));
    }

    if allowed.is_empty() {
        return Err("没有可删除的路径".to_string());
    }

    // ── 2. 逐个处理，实时推送进度 ──
    let counter = AtomicI64::new(0);
    let total_paths = allowed.len() as i64;
    let mut total_files: i64 = 0;
    let mut errors: Vec<String> = Vec::new();

    for (idx, path) in allowed.iter().enumerate() {
        let paths_done = idx as i64;

        // 推送「开始统计」事件
        emit_analyze_trash_progress(
            &app,
            &TrashProgressPayload {
                files_processed: counter.load(Ordering::SeqCst),
                current_path: path.clone(),
                phase: "counting".into(),
                paths_done,
                paths_total: total_paths,
            },
        );

        match trash_path_with_progress(path, Some(&counter)) {
            Ok(c) => {
                total_files += c;
                // 推送「完成」事件
                emit_analyze_trash_progress(
                    &app,
                    &TrashProgressPayload {
                        files_processed: total_files,
                        current_path: path.clone(),
                        phase: "done".into(),
                        paths_done: paths_done + 1,
                        paths_total: total_paths,
                    },
                );
            }
            Err(e) => {
                if Path::new(path).symlink_metadata().is_err() {
                    log::warn!("[mole_analyze_trash] path already gone, skip: {path}");
                    continue;
                }
                log::error!("[mole_analyze_trash] failed: {path}, err={e}");
                errors.push(e);
            }
        }
    }

    // ── 3. 就地修补内存会话树（对齐 Lemon：删除后树立即反映变化，无需重扫）──
    session::remove_paths(&allowed);

    // 兼容过渡：同时失效磁盘快照缓存祖先（后续删除 cache 层后可移除此块）
    for path in &allowed {
        if let Some(parent) = std::path::Path::new(path).parent() {
            cache::invalidate_cache_ancestors(&parent.to_string_lossy());
        }
    }

    log::info!(
        "[mole_analyze_trash] done: total_files={total_files}, errors={}",
        errors.len()
    );
    Ok(serde_json::json!({
        "totalFiles": total_files,
        "errors": errors,
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_get_protected_analyze_paths() -> Vec<String> {
    crate::analyze::protected::get_protected_dir_names()
}
