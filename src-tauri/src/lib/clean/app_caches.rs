//! GUI 应用缓存清理 — 严格对齐 lib/clean/app_caches.sh
//!
//! 翻译策略:
//!   - SH 中所有 `safe_clean ~/path/* "label"` 调用 → Rust `safe_clean(&[&format!("{home}/path/*")], "label")`
//!   - 路径含通配符的部分由 `safe_clean` 内部 glob 展开
//!   - SH 中带 pgrep 守卫的(Xcode/Simulator/Spotify)在 Rust 同样要做,**否则会破坏正在使用的应用**
//!
//! 返回值约定:每个 sub-cleanup 返回 `(total_kb, total_count)`,以便 GUI 聚合展示。

use std::path::Path;
use std::process::Command;

use crate::core::base::{bytes_to_human, get_file_size, get_path_size_kb, home_dir, note_activity};
use crate::core::dry_run_registry::dry_run_register_cleanup_target;
use crate::core::file_ops::{safe_clean, safe_remove};
use crate::core::log::{debug_log, log_info, log_warning};
use crate::core::timeout::run_with_timeout_capture;

/// 主入口 — 对齐 SH `clean_user_gui_applications()` 第 406-428 行。
pub fn clean_user_gui_applications() -> (u64, u64) {
    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;
    for f in [
        clean_communication_apps,
        clean_dingtalk,
        clean_ai_apps,
        clean_design_tools,
        clean_video_tools,
        clean_3d_tools,
        clean_productivity_apps,
        clean_media_players,
        clean_video_players,
        clean_download_managers,
        clean_gaming_platforms,
        clean_translation_apps,
        clean_screenshot_tools,
        clean_email_clients,
        clean_task_apps,
        clean_shell_utils,
        clean_system_utils,
        clean_note_apps,
        clean_launcher_apps,
        clean_remote_desktop,
    ] as [fn() -> (u64, u64); 20]
    {
        let (kb, c) = f();
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
    }
    (total_kb, total_count)
}

// =============================================================================
// helper:统一处理 multi-pattern safe_clean
// =============================================================================

/// 接受多组 (paths, label),逐个调用 safe_clean,累加返回。
/// 所有 ~/X 模式会自动展开。
fn run_clean_jobs(jobs: &[(&[&str], &str)]) -> (u64, u64) {
    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;
    for (paths, label) in jobs {
        let (kb, c) = safe_clean(paths, label);
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
    }
    (total_kb, total_count)
}

// =============================================================================
// Xcode
// =============================================================================

fn pgrep_x_running(name: &str) -> bool {
    Command::new("pgrep")
        .args(["-x", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 对齐 SH `clean_xcode_derived_data()` 第 6-58 行。
/// 关键守卫:Xcode 在跑就不删,以免打断构建。
pub fn clean_xcode_derived_data() -> (u64, u64) {
    let dd_dir = format!("{}/Library/Developer/Xcode/DerivedData", home_dir());
    if !Path::new(&dd_dir).is_dir() {
        return (0, 0);
    }
    if pgrep_x_running("Xcode") {
        log_warning("Xcode is running, skipping DerivedData cleanup");
        return (0, 0);
    }

    let mut projects: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dd_dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                projects.push(p.to_string_lossy().to_string());
            }
        }
    }
    if projects.is_empty() {
        return (0, 0);
    }
    let total_size_kb = get_path_size_kb(&dd_dir);
    let dry_run = std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1"
        || std::env::var("DRY_RUN").unwrap_or_default() == "true";

    if dry_run {
        if !dry_run_register_cleanup_target(&dd_dir) {
            return (0, 0);
        }
        log_info(&format!(
            "[DRY RUN] Xcode DerivedData · {} projects, {}",
            projects.len(),
            bytes_to_human(total_size_kb.saturating_mul(1024))
        ));
        note_activity();
        return (total_size_kb, projects.len() as u64);
    }

    let mut removed: u64 = 0;
    for dir in &projects {
        if safe_remove(dir, true) {
            removed += 1;
        }
    }
    if removed > 0 {
        log_info(&format!(
            "Xcode DerivedData · {removed} projects, {}",
            bytes_to_human(total_size_kb.saturating_mul(1024))
        ));
        note_activity();
    }
    (total_size_kb, removed)
}

/// 对齐 SH `clean_xcode_tools()` 第 60-132 行。
pub fn clean_xcode_tools() -> (u64, u64) {
    let home = home_dir();
    let xcode_running = pgrep_x_running("Xcode");
    let simulator_running = pgrep_x_running("Simulator");

    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;

    if !simulator_running {
        let jobs: Vec<(Vec<String>, String)> = vec![
            (
                vec![format!("{home}/Library/Developer/CoreSimulator/Caches/*")],
                "Simulator cache".to_string(),
            ),
            (
                vec![format!(
                    "{home}/Library/Developer/CoreSimulator/Devices/*/data/tmp/*"
                )],
                "Simulator temp files".to_string(),
            ),
            (
                vec![format!("{home}/Library/Logs/CoreSimulator/*")],
                "CoreSimulator logs".to_string(),
            ),
        ];
        for (paths, label) in &jobs {
            let p_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
            let (kb, c) = safe_clean(&p_refs, label);
            total_kb = total_kb.saturating_add(kb);
            total_count = total_count.saturating_add(c);
        }
        // xcrun simctl delete unavailable(对齐 SH 第 78-116 行,带 2s/5s 超时)
        if Command::new("which")
            .arg("xcrun")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            let dry_run = std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1"
                || std::env::var("DRY_RUN").unwrap_or_default() == "true";
            let listing = run_with_timeout_capture(
                2.0,
                "xcrun",
                &["simctl", "list", "devices", "unavailable"],
            )
            .unwrap_or_default();
            // 命中 UUID 形式 (XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX)
            let mut unavail = 0u64;
            for line in listing.lines() {
                if line.contains('(') && line.contains(')') && count_dashes(line) >= 4 {
                    unavail += 1;
                }
            }
            if unavail > 0 {
                if dry_run {
                    log_info(&format!(
                        "[DRY RUN] Unavailable simulators · would delete {unavail} devices"
                    ));
                    note_activity();
                } else {
                    let rc = crate::core::timeout::run_with_timeout(
                        5.0,
                        "xcrun",
                        &["simctl", "delete", "unavailable"],
                    );
                    if rc == 0 {
                        log_info(&format!(
                            "Unavailable simulators · deleted {unavail} devices"
                        ));
                        note_activity();
                    } else {
                        log_warning(&format!(
                            "Unavailable simulators · simctl delete failed (exit={rc})"
                        ));
                        debug_log(&format!("xcrun simctl delete unavailable returned {rc}"));
                    }
                }
            }
        }
    } else {
        log_warning("Simulator is running, skipping Simulator cache/temp/log cleanup");
    }

    // Xcode cache(无论 Xcode 跑没跑都可以清,因为是 DerivedData 之外的)
    let xcode_jobs: Vec<(Vec<String>, String)> = vec![
        (
            vec![format!("{home}/Library/Caches/com.apple.dt.Xcode/*")],
            "Xcode cache".to_string(),
        ),
        (
            vec![format!("{home}/Library/Developer/Xcode/iOS Device Logs/*")],
            "iOS device logs".to_string(),
        ),
        (
            vec![format!(
                "{home}/Library/Developer/Xcode/watchOS Device Logs/*"
            )],
            "watchOS device logs".to_string(),
        ),
        (
            vec![format!("{home}/Library/Developer/Xcode/Products/*")],
            "Xcode build products".to_string(),
        ),
    ];
    for (paths, label) in &xcode_jobs {
        let p_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        let (kb, c) = safe_clean(&p_refs, label);
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
    }

    // Xcode 没跑时再清 DerivedData / Archives / Documentation
    if !xcode_running {
        let (kb, c) = clean_xcode_derived_data();
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);

        let archive_jobs: Vec<(Vec<String>, String)> = vec![
            (
                vec![format!("{home}/Library/Developer/Xcode/Archives/*")],
                "Xcode archives".to_string(),
            ),
            (
                vec![format!(
                    "{home}/Library/Developer/Xcode/DocumentationCache/*"
                )],
                "Xcode documentation cache".to_string(),
            ),
            (
                vec![format!(
                    "{home}/Library/Developer/Xcode/DocumentationIndex/*"
                )],
                "Xcode documentation index".to_string(),
            ),
        ];
        for (paths, label) in &archive_jobs {
            let p_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
            let (kb, c) = safe_clean(&p_refs, label);
            total_kb = total_kb.saturating_add(kb);
            total_count = total_count.saturating_add(c);
        }
    } else {
        log_warning("Xcode is running, skipping DerivedData/Archives/Documentation cleanup");
    }

    (total_kb, total_count)
}

fn count_dashes(s: &str) -> u32 {
    s.chars().filter(|c| *c == '-').count() as u32
}

// =============================================================================
// 通信 / DingTalk / AI / Design / Video / 3D / Productivity / Media / Video Players /
// Download / Gaming / Translate / Screenshot / Email / Task / Shell / Sys / Note /
// Launcher / Remote
// =============================================================================

pub fn clean_code_editors() -> (u64, u64) {
    let h = home_dir();
    let jobs: Vec<(Vec<String>, &'static str)> = vec![
        (
            vec![format!("{h}/Library/Application Support/Code/logs/*")],
            "VS Code logs",
        ),
        (
            vec![format!("{h}/Library/Application Support/Code/Cache/*")],
            "VS Code cache",
        ),
        (
            vec![format!(
                "{h}/Library/Application Support/Code/CachedExtensions/*"
            )],
            "VS Code extension cache",
        ),
        (
            vec![format!("{h}/Library/Application Support/Code/CachedData/*")],
            "VS Code data cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.sublimetext.*/*")],
            "Sublime Text cache",
        ),
        (vec![format!("{h}/Library/Caches/Zed/*")], "Zed cache"),
        (vec![format!("{h}/Library/Logs/Zed/*")], "Zed logs"),
    ];
    run_jobs(&jobs)
}

pub fn clean_communication_apps() -> (u64, u64) {
    let h = home_dir();
    let mut jobs: Vec<(Vec<String>, &'static str)> = vec![
        (
            vec![format!("{h}/Library/Application Support/discord/Cache/*")],
            "Discord cache",
        ),
        (
            vec![format!("{h}/Library/Application Support/legcord/Cache/*")],
            "Legcord cache",
        ),
        (
            vec![format!("{h}/Library/Application Support/Slack/Cache/*")],
            "Slack cache",
        ),
        (
            vec![format!("{h}/Library/Caches/us.zoom.xos/*")],
            "Zoom cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.tencent.xinWeChat/*")],
            "WeChat cache",
        ),
        (
            vec![format!("{h}/Library/Caches/ru.keepcoder.Telegram/*")],
            "Telegram cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.microsoft.teams2/*")],
            "Microsoft Teams cache",
        ),
        (
            vec![format!("{h}/Library/Caches/net.whatsapp.WhatsApp/*")],
            "WhatsApp cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.skype.skype/*")],
            "Skype cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.tencent.meeting/*")],
            "Tencent Meeting cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.tencent.WeWorkMac/*")],
            "WeCom cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.feishu.*/*")],
            "Feishu cache",
        ),
    ];
    if Path::new(&format!("{h}/Library/Application Support/Microsoft/Teams")).is_dir() {
        for (sub, label) in [
            ("Cache", "Microsoft Teams legacy cache"),
            (
                "Application Cache",
                "Microsoft Teams legacy application cache",
            ),
            ("Code Cache", "Microsoft Teams legacy code cache"),
            ("GPUCache", "Microsoft Teams legacy GPU cache"),
            ("logs", "Microsoft Teams legacy logs"),
            ("tmp", "Microsoft Teams legacy temp files"),
        ] {
            jobs.push((
                vec![format!(
                    "{h}/Library/Application Support/Microsoft/Teams/{sub}/*"
                )],
                label,
            ));
        }
    }
    run_jobs(&jobs)
}

pub fn clean_dingtalk() -> (u64, u64) {
    let h = home_dir();
    let mut jobs: Vec<(Vec<String>, &'static str)> = vec![
        (
            vec![format!("{h}/Library/Caches/dd.work.exclusive4aliding/*")],
            "DingTalk iDingTalk cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.alibaba.AliLang.osx/*")],
            "AliLang security component",
        ),
    ];
    if Path::new(&format!("{h}/Library/Application Support/iDingTalk")).is_dir() {
        jobs.push((
            vec![format!("{h}/Library/Application Support/iDingTalk/log/*")],
            "DingTalk logs",
        ));
        jobs.push((
            vec![format!(
                "{h}/Library/Application Support/iDingTalk/holmeslogs/*"
            )],
            "DingTalk holmes logs",
        ));
    }
    run_jobs(&jobs)
}

pub fn clean_ai_apps() -> (u64, u64) {
    let h = home_dir();
    let mut jobs: Vec<(Vec<String>, &'static str)> = vec![
        (
            vec![format!("{h}/Library/Caches/com.openai.chat/*")],
            "ChatGPT cache",
        ),
        (
            vec![format!(
                "{h}/Library/Caches/com.anthropic.claudefordesktop/*"
            )],
            "Claude desktop cache",
        ),
        (vec![format!("{h}/Library/Logs/Claude/*")], "Claude logs"),
        (
            vec![format!("{h}/Library/Logs/com.openai.codex/*")],
            "Codex CLI logs",
        ),
    ];
    if Path::new(&format!("{h}/Library/Application Support/Codex")).is_dir() {
        for (sub, label) in [
            ("Cache", "Codex cache"),
            ("Code Cache", "Codex code cache"),
            ("GPUCache", "Codex GPU cache"),
            ("DawnGraphiteCache", "Codex Dawn cache"),
            ("DawnWebGPUCache", "Codex WebGPU cache"),
        ] {
            jobs.push((
                vec![format!("{h}/Library/Application Support/Codex/{sub}/*")],
                label,
            ));
        }
    }
    run_jobs(&jobs)
}

pub fn clean_design_tools() -> (u64, u64) {
    let h = home_dir();
    run_jobs(&[
        (
            vec![format!("{h}/Library/Caches/com.bohemiancoding.sketch3/*")],
            "Sketch cache",
        ),
        (
            vec![format!(
                "{h}/Library/Application Support/com.bohemiancoding.sketch3/cache/*"
            )],
            "Sketch app cache",
        ),
        (vec![format!("{h}/Library/Caches/Adobe/*")], "Adobe cache"),
        (
            vec![format!("{h}/Library/Caches/com.adobe.*/*")],
            "Adobe app caches",
        ),
        (
            vec![format!("{h}/Library/Caches/com.figma.Desktop/*")],
            "Figma cache",
        ),
        (
            vec![format!(
                "{h}/Library/Application Support/Adobe/Common/Media Cache Files/*"
            )],
            "Adobe media cache files",
        ),
    ])
}

pub fn clean_video_tools() -> (u64, u64) {
    let h = home_dir();
    let (kb, cnt) = run_jobs(&[
        (
            vec![format!("{h}/Library/Caches/net.telestream.screenflow10/*")],
            "ScreenFlow cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.FinalCut/*")],
            "Final Cut Pro cache",
        ),
        (
            vec![format!(
                "{h}/Library/Caches/com.blackmagic-design.DaVinciResolve/*"
            )],
            "DaVinci Resolve cache",
        ),
        (
            vec![format!("{h}/Movies/CacheClip/*")],
            "DaVinci Resolve CacheClip",
        ),
        (
            vec![format!("{h}/Library/Caches/com.adobe.PremierePro.*/*")],
            "Premiere Pro cache",
        ),
    ]);
    let (fcp_kb, fcp_cnt) = clean_final_cut_pro_generated_caches();
    (kb.saturating_add(fcp_kb), cnt.saturating_add(fcp_cnt))
}

/// 对齐 SH `final_cut_pro_is_running()` 第 201-207 行。
fn fcp_is_running() -> bool {
    pgrep_x_running("Final Cut Pro")
        || Command::new("pgrep")
            .args(["-f", "/Final Cut Pro.app/"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// 对齐 SH `final_cut_pro_path_has_protected_component()` 第 209-220 行。
fn fcp_path_has_protected_component(path: &str) -> bool {
    let protected: &[&str] = &[
        "/Original Media",
        "/CurrentVersion.flexolibrary",
        "/CurrentVersion.plist",
        "/Settings.plist",
        "/Motion Templates",
        "/Final Cut Pro Backups",
    ];
    for comp in protected {
        if path.ends_with(comp) || path.contains(&format!("{comp}/")) {
            return true;
        }
    }
    false
}

/// 对齐 SH `is_final_cut_pro_generated_cache_target()` 第 222-245 行。
fn is_fcp_generated_cache_target(library: &str, target: &str) -> bool {
    if library.is_empty() || target.is_empty() {
        return false;
    }
    if !library.starts_with('/') || !target.starts_with('/') {
        return false;
    }
    let h = home_dir();
    let movies_prefix = format!("{h}/Movies/");
    if !(library.starts_with(&movies_prefix) && library.ends_with(".fcpbundle")) {
        return false;
    }
    if !target.starts_with(&format!("{library}/")) {
        return false;
    }
    let lib_p = Path::new(library);
    if !lib_p.is_dir() || lib_p.is_symlink() {
        return false;
    }
    let tgt_p = Path::new(target);
    if !tgt_p.is_dir() || tgt_p.is_symlink() {
        return false;
    }
    if fcp_path_has_protected_component(target) {
        return false;
    }
    let relative = match target.strip_prefix(&format!("{library}/")) {
        Some(r) => r,
        None => return false,
    };
    if relative.ends_with("/Render Files/High Quality Media")
        || relative.ends_with("/Transcoded Media/Proxy Media")
    {
        return true;
    }
    false
}

/// 对齐 SH `find_final_cut_pro_generated_cache_targets()` 第 247-273 行。
fn find_fcp_cache_targets() -> Vec<String> {
    let h = home_dir();
    let movies_dir = format!("{h}/Movies");
    if !Path::new(&movies_dir).is_dir() {
        return Vec::new();
    }
    let libs_out = Command::new("find")
        .args([
            &movies_dir,
            "-maxdepth",
            "4",
            "-type",
            "d",
            "-name",
            "*.fcpbundle",
            "-prune",
            "-print0",
        ])
        .output();
    let libs: Vec<String> = match libs_out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect(),
        _ => return Vec::new(),
    };
    let mut targets = Vec::new();
    for library in libs {
        let lib_p = Path::new(&library);
        if !lib_p.is_dir() || lib_p.is_symlink() {
            continue;
        }
        let cache_out = Command::new("find")
            .args([
                &library,
                "(",
                "-type",
                "d",
                "(",
                "-name",
                "Original Media",
                "-o",
                "-name",
                "Analysis Files",
                "-o",
                "-name",
                "Motion Templates",
                "-o",
                "-name",
                "Final Cut Pro Backups",
                ")",
                "-prune",
                ")",
                "-o",
                "(",
                "-type",
                "d",
                "(",
                "-path",
                "*/Render Files/High Quality Media",
                "-o",
                "-path",
                "*/Transcoded Media/Proxy Media",
                ")",
                "-print0",
                ")",
            ])
            .output();
        if let Ok(o) = cache_out {
            if o.status.success() {
                for target in String::from_utf8_lossy(&o.stdout)
                    .split('\0')
                    .filter(|s| !s.is_empty())
                {
                    if is_fcp_generated_cache_target(&library, target) {
                        targets.push(target.to_string());
                    }
                }
            }
        }
    }
    targets
}

/// 对齐 SH `clean_final_cut_pro_generated_caches()` 第 275-302 行。
fn clean_final_cut_pro_generated_caches() -> (u64, u64) {
    if fcp_is_running() {
        log_warning("Final Cut Pro is running, skipping generated cache cleanup");
        note_activity();
        return (0, 0);
    }
    let targets = find_fcp_cache_targets();
    if targets.is_empty() {
        return (0, 0);
    }
    let refs: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
    let (kb, c) = safe_clean(&refs, "Final Cut Pro generated cache");
    (kb, c)
}

pub fn clean_3d_tools() -> (u64, u64) {
    let h = home_dir();
    run_jobs(&[
        (
            vec![format!(
                "{h}/Library/Caches/org.blenderfoundation.blender/*"
            )],
            "Blender cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.maxon.cinema4d/*")],
            "Cinema 4D cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.autodesk.*/*")],
            "Autodesk cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.sketchup.*/*")],
            "SketchUp cache",
        ),
    ])
}

pub fn clean_productivity_apps() -> (u64, u64) {
    let h = home_dir();
    run_jobs(&[
        (
            vec![format!("{h}/Library/Caches/com.tw93.MiaoYan/*")],
            "MiaoYan cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.klee.desktop/*")],
            "Klee cache",
        ),
        (
            vec![format!("{h}/Library/Caches/klee_desktop/*")],
            "Klee desktop cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.orabrowser.app/*")],
            "Ora browser cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.filo.client/*")],
            "Filo cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.flomoapp.mac/*")],
            "Flomo cache",
        ),
        (
            vec![format!(
                "{h}/Library/Application Support/Quark/Cache/videoCache/*"
            )],
            "Quark video cache",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.ranchero.NetNewsWire-Evergreen/Data/Library/Caches/*"
            )],
            "NetNewsWire cache",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.ideasoncanvas.mindnode/Data/Library/Caches/*"
            )],
            "MindNode cache",
        ),
        (vec![format!("{h}/.cache/kaku/*")], "Kaku cache"),
    ])
}

/// Spotify 离线音乐保护:对齐 SH 第 229-247 行。
/// `offline.bnk` 即便没下歌也会存在,>1KB 才算证据;`*.file` 为加密 track,确认下载。
pub fn clean_media_players() -> (u64, u64) {
    let h = home_dir();
    let spotify_data = format!("{h}/Library/Application Support/Spotify");
    let bnk_file = format!("{spotify_data}/PersistentCache/Storage/offline.bnk");
    let bnk_size = if Path::new(&bnk_file).is_file() {
        get_file_size(&bnk_file)
    } else {
        0
    };
    let storage_dir = format!("{spotify_data}/PersistentCache/Storage");
    let has_track_files =
        Path::new(&storage_dir).is_dir() && walk_first_match(&storage_dir, ".file").is_some();
    let has_offline_music = bnk_size > 1024 || has_track_files;

    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;

    if has_offline_music {
        log_warning("Spotify cache protected · offline music detected");
        note_activity();
    } else {
        let (kb, c) = safe_clean(
            &[&format!("{h}/Library/Caches/com.spotify.client/*")],
            "Spotify cache",
        );
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
    }

    let jobs: Vec<(Vec<String>, &'static str)> = vec![
        (
            vec![format!("{h}/Library/Caches/com.apple.Music")],
            "Apple Music cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.podcasts")],
            "Apple Podcasts cache",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.podcasts/Data/tmp/StreamedMedia"
            )],
            "Podcasts streamed media",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.podcasts/Data/tmp/*.heic"
            )],
            "Podcasts artwork cache",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.podcasts/Data/tmp/*.img"
            )],
            "Podcasts image cache",
        ),
        (
            vec![format!(
                "{h}/Library/Containers/com.apple.podcasts/Data/tmp/*CFNetworkDownload*.tmp"
            )],
            "Podcasts download temp",
        ),
        (
            vec![format!("{h}/Library/Caches/com.apple.TV/*")],
            "Apple TV cache",
        ),
        (
            vec![format!("{h}/Library/Caches/tv.plex.player.desktop")],
            "Plex cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.netease.163music")],
            "NetEase Music cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.tencent.QQMusic/*")],
            "QQ Music cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.kugou.mac/*")],
            "Kugou Music cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.kuwo.mac/*")],
            "Kuwo Music cache",
        ),
    ];
    let (kb, c) = run_jobs(&jobs);
    (total_kb.saturating_add(kb), total_count.saturating_add(c))
}

fn walk_first_match(root: &str, suffix: &str) -> Option<String> {
    let mut stack = vec![root.to_string()];
    let mut visited = 0;
    while let Some(dir) = stack.pop() {
        visited += 1;
        if visited > 500 {
            return None;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            let ps = p.to_string_lossy().to_string();
            if p.is_dir() {
                stack.push(ps);
            } else if ps.ends_with(suffix) {
                return Some(ps);
            }
        }
    }
    None
}

pub fn clean_video_players() -> (u64, u64) {
    let h = home_dir();
    let mut jobs: Vec<(Vec<String>, &'static str)> = vec![
        (
            vec![format!("{h}/Library/Caches/com.colliderli.iina")],
            "IINA cache",
        ),
        (
            vec![format!("{h}/Library/Caches/org.videolan.vlc")],
            "VLC cache",
        ),
        (vec![format!("{h}/Library/Caches/io.mpv")], "MPV cache"),
        (
            vec![format!("{h}/Library/Caches/com.iqiyi.player")],
            "iQIYI cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.tencent.tenvideo")],
            "Tencent Video cache",
        ),
        (
            vec![format!("{h}/Library/Caches/tv.danmaku.bili/*")],
            "Bilibili cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.douyu.*/*")],
            "Douyu cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.huya.*/*")],
            "Huya cache",
        ),
        (
            vec![format!("{h}/Library/Caches/smart.stremio*/*")],
            "Stremio cache",
        ),
    ];
    if Path::new(&format!("{h}/Library/Application Support/stremio")).is_dir() {
        jobs.push((
            vec![format!(
                "{h}/Library/Application Support/stremio/stremio-server/stremio-cache/*"
            )],
            "Stremio server cache",
        ));
    }
    run_jobs(&jobs)
}

pub fn clean_download_managers() -> (u64, u64) {
    let h = home_dir();
    run_jobs(&[
        (
            vec![format!("{h}/Library/Caches/net.xmac.aria2gui")],
            "Aria2 cache",
        ),
        (
            vec![format!("{h}/Library/Caches/org.m0k.transmission")],
            "Transmission cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.qbittorrent.qBittorrent")],
            "qBittorrent cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.downie.Downie-*")],
            "Downie cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.folx.*/*")],
            "Folx cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.charlessoft.pacifist/*")],
            "Pacifist cache",
        ),
    ])
}

pub fn clean_gaming_platforms() -> (u64, u64) {
    let h = home_dir();
    let mut jobs: Vec<(Vec<String>, &'static str)> = vec![(
        vec![format!("{h}/Library/Caches/com.valvesoftware.steam/*")],
        "Steam cache",
    )];
    if Path::new(&format!("{h}/Library/Application Support/Steam")).is_dir() {
        for (sub, label) in [
            ("htmlcache/*", "Steam web cache"),
            ("appcache/*", "Steam app cache"),
            ("depotcache/*", "Steam depot cache"),
            ("steamapps/shadercache/*", "Steam shader cache"),
            ("logs/*", "Steam logs"),
        ] {
            jobs.push((
                vec![format!("{h}/Library/Application Support/Steam/{sub}")],
                label,
            ));
        }
    }
    jobs.push((
        vec![format!(
            "{h}/Library/Caches/com.epicgames.EpicGamesLauncher/*"
        )],
        "Epic Games cache",
    ));
    jobs.push((
        vec![format!("{h}/Library/Caches/com.blizzard.Battle.net/*")],
        "Battle.net cache",
    ));
    if Path::new(&format!("{h}/Library/Application Support/Battle.net")).is_dir() {
        jobs.push((
            vec![format!(
                "{h}/Library/Application Support/Battle.net/Cache/*"
            )],
            "Battle.net app cache",
        ));
    }
    jobs.push((
        vec![format!("{h}/Library/Caches/com.ea.*/*")],
        "EA Origin cache",
    ));
    jobs.push((
        vec![format!("{h}/Library/Caches/com.gog.galaxy/*")],
        "GOG Galaxy cache",
    ));
    jobs.push((
        vec![format!("{h}/Library/Caches/com.riotgames.*/*")],
        "Riot Games cache",
    ));
    if Path::new(&format!("{h}/Library/Application Support/minecraft")).is_dir() {
        for (sub, label) in [
            ("logs/*", "Minecraft logs"),
            ("crash-reports/*", "Minecraft crash reports"),
            ("webcache/*", "Minecraft web cache"),
            ("webcache2/*", "Minecraft web cache 2"),
        ] {
            jobs.push((
                vec![format!("{h}/Library/Application Support/minecraft/{sub}")],
                label,
            ));
        }
    }
    if Path::new(&format!("{h}/.lunarclient")).is_dir() {
        for (sub, label) in [
            ("game-cache/*", "Lunar Client game cache"),
            ("launcher-cache/*", "Lunar Client launcher cache"),
            ("logs/*", "Lunar Client logs"),
            ("offline/*/logs/*", "Lunar Client offline logs"),
            ("offline/files/*/logs/*", "Lunar Client offline file logs"),
        ] {
            jobs.push((vec![format!("{h}/.lunarclient/{sub}")], label));
        }
    }
    jobs.push((
        vec![format!("{h}/Library/Caches/net.pcsx2.PCSX2/*")],
        "PCSX2 cache",
    ));
    if Path::new(&format!("{h}/Library/Application Support/PCSX2")).is_dir() {
        jobs.push((
            vec![format!("{h}/Library/Application Support/PCSX2/cache/*")],
            "PCSX2 shader cache",
        ));
        jobs.push((vec![format!("{h}/Library/Logs/PCSX2/*")], "PCSX2 logs"));
    }
    if Path::new(&format!("{h}/Library/Application Support/rpcs3")).is_dir() {
        jobs.push((
            vec![format!("{h}/Library/Caches/net.rpcs3.rpcs3/*")],
            "RPCS3 cache",
        ));
        jobs.push((
            vec![format!("{h}/Library/Application Support/rpcs3/logs/*")],
            "RPCS3 logs",
        ));
    }
    run_jobs(&jobs)
}

pub fn clean_translation_apps() -> (u64, u64) {
    let h = home_dir();
    run_jobs(&[
        (
            vec![format!("{h}/Library/Caches/com.youdao.YoudaoDict")],
            "Youdao Dictionary cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.eudic.*")],
            "Eudict cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.bob-build.Bob")],
            "Bob Translation cache",
        ),
    ])
}

pub fn clean_screenshot_tools() -> (u64, u64) {
    let h = home_dir();
    run_jobs(&[
        (
            vec![format!("{h}/Library/Caches/com.cleanshot.*")],
            "CleanShot cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.reincubate.camo")],
            "Camo cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.xnipapp.xnip")],
            "Xnip cache",
        ),
    ])
}

pub fn clean_email_clients() -> (u64, u64) {
    let h = home_dir();
    run_jobs(&[
        (
            vec![format!("{h}/Library/Caches/com.readdle.smartemail-Mac")],
            "Spark cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.airmail.*")],
            "Airmail cache",
        ),
    ])
}

pub fn clean_task_apps() -> (u64, u64) {
    let h = home_dir();
    run_jobs(&[
        (
            vec![format!("{h}/Library/Caches/com.todoist.mac.Todoist")],
            "Todoist cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.any.do.*")],
            "Any.do cache",
        ),
    ])
}

pub fn clean_shell_utils() -> (u64, u64) {
    let h = home_dir();
    run_jobs(&[
        (vec![format!("{h}/.zcompdump*")], "Zsh completion cache"),
        (vec![format!("{h}/.lesshst")], "less history"),
        (vec![format!("{h}/.viminfo.tmp")], "Vim temporary files"),
        (vec![format!("{h}/.wget-hsts")], "wget HSTS cache"),
        (vec![format!("{h}/.cacher/logs/*")], "Cacher logs"),
        (vec![format!("{h}/.kite/logs/*")], "Kite logs"),
        (
            vec![format!("{h}/Library/Caches/dev.warp.Warp-Stable/*")],
            "Warp cache",
        ),
        (vec![format!("{h}/Library/Logs/warp.log")], "Warp log"),
        (
            vec![format!("{h}/Library/Caches/SentryCrash/Warp/*")],
            "Warp Sentry crash reports",
        ),
        (
            vec![format!("{h}/Library/Caches/com.mitchellh.ghostty/*")],
            "Ghostty cache",
        ),
    ])
}

pub fn clean_system_utils() -> (u64, u64) {
    let h = home_dir();
    let mut jobs: Vec<(Vec<String>, &'static str)> = vec![
        (
            vec![format!("{h}/Library/Caches/com.runjuu.Input-Source-Pro/*")],
            "Input Source Pro cache",
        ),
        (
            vec![format!("{h}/Library/Caches/macos-wakatime.WakaTime/*")],
            "WakaTime cache",
        ),
        (
            vec![format!(
                "{h}/Library/Application Support/WeType/com.onevcat.Kingfisher.ImageCache.WeType/*"
            )],
            "WeType image cache",
        ),
        (
            vec![format!(
                "{h}/Library/Application Support/WeType/DictUpdate/*"
            )],
            "WeType dict update cache",
        ),
    ];
    if Path::new(&format!("{h}/Library/Application Support/mihomo-party")).is_dir() {
        for (sub, label) in [
            ("Cache", "mihomo-party cache"),
            ("Code Cache", "mihomo-party code cache"),
            ("GPUCache", "mihomo-party GPU cache"),
            ("DawnGraphiteCache", "mihomo-party Dawn cache"),
            ("DawnWebGPUCache", "mihomo-party WebGPU cache"),
            ("logs", "mihomo-party logs"),
        ] {
            jobs.push((
                vec![format!(
                    "{h}/Library/Application Support/mihomo-party/{sub}/*"
                )],
                label,
            ));
        }
    }
    jobs.push((
        vec![format!("{h}/Library/Caches/ws.stash.app.mac/*")],
        "Stash cache",
    ));
    run_jobs(&jobs)
}

pub fn clean_note_apps() -> (u64, u64) {
    let h = home_dir();
    run_jobs(&[
        (
            vec![format!("{h}/Library/Caches/notion.id/*")],
            "Notion cache",
        ),
        (
            vec![format!("{h}/Library/Caches/md.obsidian/*")],
            "Obsidian cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.logseq.*/*")],
            "Logseq cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.bear-writer.*/*")],
            "Bear cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.evernote.*/*")],
            "Evernote cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.yinxiang.*/*")],
            "Yinxiang Note cache",
        ),
    ])
}

pub fn clean_launcher_apps() -> (u64, u64) {
    let h = home_dir();
    run_jobs(&[
        (
            vec![format!(
                "{h}/Library/Caches/com.runningwithcrayons.Alfred/*"
            )],
            "Alfred cache",
        ),
        (
            vec![format!("{h}/Library/Caches/cx.c3.theunarchiver/*")],
            "The Unarchiver cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.raycast.macos/urlcache/*")],
            "Raycast URL cache",
        ),
        (
            vec![format!(
                "{h}/Library/Caches/com.raycast.macos/fsCachedData/*"
            )],
            "Raycast FS cache",
        ),
    ])
}

pub fn clean_remote_desktop() -> (u64, u64) {
    let h = home_dir();
    run_jobs(&[
        (
            vec![format!("{h}/Library/Caches/com.teamviewer.*/*")],
            "TeamViewer cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.anydesk.*/*")],
            "AnyDesk cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.todesk.*/*")],
            "ToDesk cache",
        ),
        (
            vec![format!("{h}/Library/Caches/com.sunlogin.*/*")],
            "Sunlogin cache",
        ),
    ])
}

fn run_jobs(jobs: &[(Vec<String>, &str)]) -> (u64, u64) {
    let mut total_kb: u64 = 0;
    let mut total_count: u64 = 0;
    for (paths, label) in jobs {
        let p_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        let (kb, c) = safe_clean(&p_refs, label);
        total_kb = total_kb.saturating_add(kb);
        total_count = total_count.saturating_add(c);
    }
    (total_kb, total_count)
}

#[allow(dead_code)]
fn _unused() {
    // 保留 helper 占位,避免 run_clean_jobs/get_path_size_kb 死代码警告(后续 GUI 接入会用到)
    let _ = run_clean_jobs as fn(&[(&[&str], &str)]) -> (u64, u64);
    let _ = get_path_size_kb;
}
