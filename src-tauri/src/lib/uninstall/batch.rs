//! 对齐 `lib/uninstall/batch.sh`(SH 第 1-993 行)。
//!
//! GUI 端不做 TUI 交互(spinner / `read_key` / 转义序列),其它业务逻辑、
//! 安全校验、白名单保护、超时控制全部按 SH 翻译。
//!
//! 之前 Rust 版本只有"壳子"(SH 690 行 → Rust 16 行),且把 `has_sensitive_data`
//! 写成 `false`、`decode_file_list` 缺 null/绝对路径校验、`brew_uninstall_cask`
//! 失败后 `sudo rm -rf` 兜底——本次重写一并修复,行为完全对齐 SH。

use std::path::Path;
use std::process::Command;

use serde::Serialize;

use crate::core::app_protection::{
    find_app_files, find_app_system_files, force_kill_app, get_diagnostic_report_paths_for_app,
};
use crate::core::base::{
    bytes_to_human, get_file_owner, get_lsregister_path, get_path_size_kb, home_dir,
};
use crate::core::common::remove_apps_from_dock;
use crate::core::file_ops::{
    _mole_delete_log, _mole_move_to_trash_batch, _mole_privileged_path_has_mutable_ancestor,
    MOLE_OK, calculate_total_size, diagnose_removal_failure, mole_delete, safe_remove,
    safe_sudo_remove, stat_path_identity, validate_path_for_deletion,
};
use crate::core::log::{debug_log, log_error, log_operation, log_warning};
use crate::core::sudo::ensure_admin_session;
use crate::core::timeout::run_with_timeout;

use super::brew::{
    CaskInstallState, brew_autoremove_silent, brew_uninstall_cask, get_brew_cask_name,
    is_brew_cask_installed,
};
use super::leftovers;

// ============================================================================
// Named timeouts — 对齐 lib/core/timeouts.sh
// ============================================================================

/// 对齐 MOLE_TIMEOUT_MEDIUM_PROBE_SEC=5: 单次 launchctl / bootout 探测超时。
const MOLE_TIMEOUT_MEDIUM_PROBE_SEC: f64 = 5.0;
/// 对齐 MOLE_TIMEOUT_PKG_LIST_SEC=10: pkg 列表 / lsregister gc 超时。
const MOLE_TIMEOUT_PKG_LIST_SEC: f64 = 10.0;

// ============================================================================
// Helpers
// ============================================================================

pub fn is_uninstall_dry_run() -> bool {
    std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1"
}

/// 对齐 SH 第 18-33 行 `app_declares_local_network_usage`。
pub fn app_declares_local_network_usage(app_path: &str) -> bool {
    let info_plist = format!("{app_path}/Contents/Info.plist");
    if !Path::new(&info_plist).is_file() {
        return false;
    }
    if Command::new("plutil")
        .args([
            "-extract",
            "NSLocalNetworkUsageDescription",
            "raw",
            &info_plist,
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return true;
    }
    Command::new("plutil")
        .args([
            "-extract",
            "NSBonjourServices",
            "xml1",
            "-o",
            "-",
            &info_plist,
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 对齐 SH 第 37-59 行 `has_sensitive_data`。
///
/// 之前 Rust 直接返回 `false`,GUI 永远不会警告用户敏感数据被卷入清理。
/// 这里完整翻译 SH 的 case 模式,逐行匹配 .ssh/.aws/.docker/Documents/Cookies 等。
pub fn has_sensitive_data(files: &str) -> bool {
    if files.is_empty() {
        return false;
    }
    for line in files.lines() {
        let path = line.trim();
        if path.is_empty() {
            continue;
        }
        if file_path_is_sensitive(path) {
            return true;
        }
    }
    false
}

/// SH 第 45-55 行的 case 模式。`*` 是 shell glob,翻译成 substring 匹配。
fn file_path_is_sensitive(path: &str) -> bool {
    // shell case 中的 `\ ` 表示字面空格,这里直接用 ASCII 空格
    const SUBSTRING_PATTERNS: &[&str] = &[
        "/.warp",
        "/.config/",
        "/themes/",
        "/settings/",
        "/User Data/",
        "/.ssh/",
        "/.gnupg/",
        "/Documents/",
        "/Desktop/",
        "/Downloads/",
        "/Movies/",
        "/Music/",
        "/Pictures/",
        "/.password",
        "/.token",
        "/.auth",
        "/keychain",
        "/Passwords/",
        "/Accounts/",
        "/Cookies/",
        "/.aws/",
        "/.kube/",
        "/credentials/",
        "/secrets/",
    ];
    for pat in SUBSTRING_PATTERNS {
        if path.contains(pat) {
            return true;
        }
    }
    // `*/Preferences/*.plist`
    if let Some(idx) = path.rfind("/Preferences/") {
        let tail = &path[idx + "/Preferences/".len()..];
        if !tail.is_empty() && !tail.contains('/') && tail.ends_with(".plist") {
            return true;
        }
    }
    // `*/.docker/config.json`
    if path.ends_with("/.docker/config.json") {
        return true;
    }
    false
}

/// 对齐 SH 第 62-92 行 `decode_file_list`。
///
/// 修复:之前 Rust 缺 null byte 校验和绝对路径校验。
/// SH 端解码后必须满足:
/// - 不含 `\0`(否则拒绝);
/// - 每一非空行必须以 `/` 开头(必须是绝对路径)。
pub fn decode_file_list(encoded: &str, app_name: &str) -> String {
    let decoded = match base64_decode(encoded, true) {
        Ok(s) => s,
        Err(_) => match base64_decode(encoded, false) {
            Ok(s) => s,
            Err(_) => {
                debug_log(&format!("Failed to decode file list for {app_name}"));
                return String::new();
            }
        },
    };
    if decoded.contains('\0') {
        debug_log(&format!(
            "File list for {app_name} contains null bytes, rejecting"
        ));
        return String::new();
    }
    for line in decoded.lines() {
        if !line.is_empty() && !line.starts_with('/') {
            debug_log(&format!("Invalid path in file list for {app_name}: {line}"));
            return String::new();
        }
    }
    decoded
}

fn base64_decode(s: &str, darwin: bool) -> Result<String, ()> {
    use std::io::Write;
    let flag = if darwin { "-D" } else { "-d" };
    let mut child = Command::new("base64")
        .arg(flag)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| ())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(s.as_bytes());
    }
    let output = child.wait_with_output().map_err(|_| ())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(())
    }
}

/// SH 第 433-439 行的 base64 编码(用于在 detail 行里保存多行字符串)。
/// macOS `base64` 编码默认就不换行,但 SH 用 `tr -d '\n'` 兜底,这里也保留。
pub fn encode_file_list(text: &str) -> String {
    use std::io::Write;
    let mut child = match Command::new("base64")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return String::new(),
    };
    if !output.status.success() {
        return String::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .replace('\n', "")
        .replace('\r', "")
}

/// 对齐 SH 第 110 行 reverse-DNS 校验:
///   `^[a-zA-Z0-9][-a-zA-Z0-9]*(\.[a-zA-Z0-9][-a-zA-Z0-9]*)+$`
/// 必须**至少有一个 dot**,首字符是字母数字。
fn is_valid_reverse_dns_bundle_id(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut segments = s.split('.');
    let mut total_segments = 0u32;
    while let Some(seg) = segments.next() {
        total_segments += 1;
        if seg.is_empty() {
            return false;
        }
        let mut chars = seg.chars();
        let first = match chars.next() {
            Some(c) => c,
            None => return false,
        };
        if !first.is_ascii_alphanumeric() {
            return false;
        }
        for c in chars {
            if !(c.is_ascii_alphanumeric() || c == '-') {
                return false;
            }
        }
    }
    // 至少 2 段(必须有一个点)
    total_segments >= 2
}

/// 对齐 SH 第 168-172 行 `can_unload_launch_plist`。
/// 验证 plist 路径是否在合法的 LaunchAgents/Daemons 目录内。
fn can_unload_launch_plist(plist: &str) -> bool {
    if !plist.ends_with(".plist") {
        return false;
    }
    let home = home_dir();
    match plist {
        _ if plist.starts_with(&format!("{home}/Library/LaunchAgents/")) => {}
        _ if plist.starts_with("/Library/LaunchAgents/") => {}
        _ if plist.starts_with("/Library/LaunchDaemons/") => {}
        _ => return false,
    }
    validate_path_for_deletion(plist)
}

/// 对齐 SH 第 173-189 行 `unload_launch_plist`。
/// 带超时的 launchctl unload，支持 sudo。
fn unload_launch_plist(plist: &str, needs_sudo: bool) {
    if !can_unload_launch_plist(plist) {
        return;
    }
    if needs_sudo {
        let _ = run_with_timeout(
            MOLE_TIMEOUT_MEDIUM_PROBE_SEC,
            "sudo",
            &["/bin/launchctl", "unload", plist],
        );
    } else {
        let _ = run_with_timeout(
            MOLE_TIMEOUT_MEDIUM_PROBE_SEC,
            "launchctl",
            &["unload", plist],
        );
    }
}

/// 对齐 SH 第 97-136 行 `stop_launch_services`(第 3 参数 app_path 对齐 SH 第 435-456 行)。
/// 修复点:bundle_id 必须是 reverse-DNS(SH regex);允许 `${bundle_id}*.plist` 通配;
/// bundle_id 降级为 unknown 时仍要扫描 ProgramArguments 引用 app 路径的 plist。
pub fn stop_launch_services(bundle_id: &str, has_system_files: bool, app_path: &str) {
    if is_uninstall_dry_run() {
        debug_log(&format!(
            "[DRY RUN] Would unload launch services for bundle: {bundle_id}"
        ));
        return;
    }

    // bundle-id-keyed 扫描需要合法 reverse-DNS;app-path 扫描不依赖它,
    // sibling guard 把 bundle_id 降级为 unknown 时也必须执行——名字 glob 的
    // agent plist 会被 remove_file_list 删除,不 unload 则 job 残留到注销(SH 第 403-417 行)。
    let mut bundle_id_usable = true;
    if bundle_id.is_empty() || bundle_id == "unknown" {
        bundle_id_usable = false;
    } else if !is_valid_reverse_dns_bundle_id(bundle_id) {
        debug_log(&format!(
            "Invalid bundle_id format for LaunchAgent search: {bundle_id}"
        ));
        bundle_id_usable = false;
    }

    let home = home_dir();
    let user_la = format!("{home}/Library/LaunchAgents");

    // User-level LaunchAgents — use find -print0 patterns matching SH
    if bundle_id_usable && Path::new(&user_la).is_dir() {
        let pattern1 = format!("{}.plist", bundle_id);
        let pattern2 = format!("{}.*.plist", bundle_id);
        if let Ok(out) = Command::new("find")
            .args([
                &user_la,
                "-maxdepth",
                "1",
                "(",
                "-name",
                &pattern1,
                "-o",
                "-name",
                &pattern2,
                ")",
                "-print0",
            ])
            .output()
        {
            for raw in out.stdout.split(|&b| b == 0) {
                let path = String::from_utf8_lossy(raw);
                let path = path.trim();
                if path.is_empty() || !Path::new(path).exists() {
                    continue;
                }
                unload_launch_plist(path, false);
                safe_remove(path, true);
            }
        }
    }

    // System LaunchAgents/Daemons — only when has_system_files
    if bundle_id_usable && has_system_files {
        for dir in ["/Library/LaunchAgents", "/Library/LaunchDaemons"] {
            if !Path::new(dir).is_dir() {
                continue;
            }
            let pattern1 = format!("{}.plist", bundle_id);
            let pattern2 = format!("{}.*.plist", bundle_id);
            if let Ok(out) = Command::new("find")
                .args([
                    dir,
                    "-maxdepth",
                    "1",
                    "(",
                    "-name",
                    &pattern1,
                    "-o",
                    "-name",
                    &pattern2,
                    ")",
                    "-print0",
                ])
                .output()
            {
                for raw in out.stdout.split(|&b| b == 0) {
                    let path = String::from_utf8_lossy(raw);
                    let path = path.trim();
                    if path.is_empty() || !Path::new(path).exists() {
                        continue;
                    }
                    unload_launch_plist(path, true);
                    let _ = safe_sudo_remove(path, None);
                }
            }
        }
    }

    // 扫描 ProgramArguments 引用 app 路径的 plist(SH 第 435-456 行)。
    // 只 unload 不删除:plist 删除由 remove_file_list 统一走验证路径。
    if !app_path.is_empty() {
        if Path::new(&user_la).is_dir() {
            unload_launch_plists_matching_app_path(&user_la, false, app_path);
        }
        if has_system_files {
            for dir in ["/Library/LaunchAgents", "/Library/LaunchDaemons"] {
                if Path::new(dir).is_dir() {
                    unload_launch_plists_matching_app_path(dir, true, app_path);
                }
            }
        }
    }
}

/// 对齐 SH 第 331-387 行 `_uninstall_unload_launch_plists` 的 app_path 形态:
/// bundle_id 为空时枚举全部 *.plist,逐个 grep -qF app_path 匹配则 unload。
fn unload_launch_plists_matching_app_path(root: &str, needs_sudo: bool, app_path: &str) {
    let Ok(out) = Command::new("find")
        .args([root, "-maxdepth", "1", "-name", "*.plist", "-print0"])
        .output()
    else {
        return;
    };
    for raw in out.stdout.split(|&b| b == 0) {
        let path = String::from_utf8_lossy(raw);
        let path = path.trim();
        if path.is_empty() || !Path::new(path).exists() {
            continue;
        }
        // grep -qF 等价:plist 内容按字节包含 app_path 即命中
        let Ok(content) = std::fs::read(path) else {
            continue;
        };
        if !plist_references_app_path(&content, app_path) {
            continue;
        }
        unload_launch_plist(path, needs_sudo);
    }
}

/// grep -qF "$app_path" 的字节级等价(支持二进制 plist)。
fn plist_references_app_path(content: &[u8], app_path: &str) -> bool {
    let needle = app_path.as_bytes();
    !needle.is_empty() && content.windows(needle.len()).any(|w| w == needle)
}

/// 对齐 SH 第 140-155 行。
pub fn unregister_app_bundle(app_path: &str) {
    if app_path.is_empty() || !Path::new(app_path).exists() {
        return;
    }
    if !app_path.ends_with(".app") {
        return;
    }
    let lsregister = get_lsregister_path();
    if lsregister.is_empty() || !Path::new(&lsregister).exists() {
        return;
    }
    if is_uninstall_dry_run() {
        return;
    }
    let _ = Command::new(&lsregister).args(["-u", app_path]).output();
}

/// 对齐 SH 第 158-185 行 `refresh_launch_services_after_uninstall`。
///
/// 修复点:加 `run_with_timeout 10/15` 超时;主路径含 `-domain system`;
/// 124(超时)或非 0 时回退到去掉 `system` 的轻量重建;124 也算成功。
pub fn refresh_launch_services_after_uninstall() -> bool {
    let lsregister = get_lsregister_path();
    if lsregister.is_empty() || !Path::new(&lsregister).exists() {
        return false;
    }
    if is_uninstall_dry_run() {
        return true;
    }
    let _ = run_with_timeout(MOLE_TIMEOUT_PKG_LIST_SEC, &lsregister, &["-gc"]);
    let primary = run_with_timeout(
        15.0,
        &lsregister,
        &[
            "-r", "-f", "-domain", "local", "-domain", "user", "-domain", "system",
        ],
    );
    if primary == 0 || primary == 124 {
        return true;
    }
    // 非超时但失败:降级到去掉 system 域
    let fallback = run_with_timeout(
        MOLE_TIMEOUT_PKG_LIST_SEC,
        &lsregister,
        &["-r", "-f", "-domain", "local", "-domain", "user"],
    );
    fallback == 0 || fallback == 124
}

/// 对齐 SH 第 188-229 行 `remove_login_item`。
pub fn remove_login_item(app_name: &str, bundle_id: &str) {
    if is_uninstall_dry_run() {
        debug_log(&format!(
            "[DRY RUN] Would remove login item: {}",
            if app_name.is_empty() {
                bundle_id
            } else {
                app_name
            }
        ));
        return;
    }
    if app_name.is_empty() && bundle_id.is_empty() {
        return;
    }
    let clean_name = app_name.trim_end_matches(".app");
    if clean_name.is_empty() {
        return;
    }
    if std::env::var("MOLE_TEST_MODE").unwrap_or_default() == "1"
        || std::env::var("MOLE_TEST_NO_AUTH").unwrap_or_default() == "1"
    {
        return;
    }
    let escaped = clean_name.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "tell application \"System Events\"\n\
         try\n\
         set itemCount to count of login items\n\
         repeat with i from itemCount to 1 by -1\n\
         try\n\
         set itemName to name of login item i\n\
         if itemName is \"{escaped}\" then\n\
         delete login item i\n\
         end if\n\
         end try\n\
         end repeat\n\
         end try\n\
         end tell"
    );
    let _ = Command::new("osascript").arg("-e").arg(&script).output();
}

/// 对齐 SH 第 238-299 行 `remove_file_list`。
///
/// 修复点:trash batch 成功后需要同时记 `_mole_delete_log` + `log_operation`(SH 第 277 行)。
pub fn remove_file_list(file_list: &str, use_sudo: bool) -> usize {
    let mut count = 0usize;
    let mode = std::env::var("MOLE_DELETE_MODE").unwrap_or_else(|_| "permanent".to_string());
    let mut trash_batch: Vec<String> = Vec::new();
    let mut fallback_paths: Vec<String> = Vec::new();

    for file in file_list.lines() {
        let file = file.trim();
        if file.is_empty() {
            continue;
        }
        // PureMac 安全加固：高风险 dotfile/dotdir 兜底拦截（如 ~/.claude、~/.ssh）
        let home = std::env::var("HOME").unwrap_or_default();
        if crate::core::high_risk_dotpaths::is_high_risk_dotpath(file, &home) {
            log::warn!("[uninstall.remove_file_list] blocked high-risk dotpath: {file}");
            continue;
        }
        let p = Path::new(file);
        if !p.exists() && !p.is_symlink() {
            continue;
        }
        if !validate_path_for_deletion(file) {
            continue;
        }
        if use_sudo && is_uninstall_dry_run() {
            debug_log(&format!("[DRY RUN] Would sudo remove: {file}"));
            count += 1;
            continue;
        }
        if mode == "trash" && !use_sudo && !p.is_symlink() && !is_uninstall_dry_run() {
            trash_batch.push(file.to_string());
        } else {
            fallback_paths.push(file.to_string());
        }
    }

    if !trash_batch.is_empty() {
        if _mole_move_to_trash_batch(&trash_batch) {
            for bp in &trash_batch {
                _mole_delete_log("trash", "unknown", "ok", bp);
                log_operation(
                    &std::env::var("MOLE_CURRENT_COMMAND")
                        .unwrap_or_else(|_| "uninstall".to_string()),
                    "TRASHED",
                    bp,
                    Some("batch"),
                );
            }
            count += trash_batch.len();
        } else {
            // batch 失败:每个路径转 fallback 走 mole_delete(它会继续尝试 trash 单挑 + permanent 兜底)
            fallback_paths.extend(trash_batch.drain(..));
        }
    }

    for fb in &fallback_paths {
        if mole_delete(fb, use_sudo, None) == MOLE_OK {
            count += 1;
        }
    }
    count
}

// ============================================================================
// Per-app metadata collection (SH 第 348-440 行的预扫描)
// ============================================================================

/// 单个待卸载 app 的元数据快照,SH 第 439 行 `app_details` 里的字段对齐。
///
/// New CLI: system_files / diag_system 改为 review-only（仅展示，不删除）。
/// 实际的 review 副本存在 review_system_files 中供前端预览。
#[derive(Debug, Clone)]
pub struct AppDetail {
    pub app_name: String,
    pub app_path: String,
    pub bundle_id: String,
    pub total_kb: u64,
    pub related_files: String,
    /// System-level files to show in preview only (NOT deleted).
    pub review_system_files: String,
    /// Always empty — system files are review-only in new CLI.
    pub system_files: String,
    /// Always empty — system files are review-only in new CLI.
    pub diag_system: String,
    pub has_sensitive_data: bool,
    pub needs_sudo: bool,
    pub is_brew_cask: bool,
    pub cask_name: String,
    pub has_local_network_usage: bool,
    /// Login Item Helper bundle IDs discovered inside the app.
    pub login_item_helpers: String,
    /// Whether the app's official vendor provides its own uninstaller.
    pub is_official_uninstaller: bool,
    pub official_vendor: String,
    /// sibling guard 状态：none / guard / guard_login（对齐 SH `sibling_guard`）。
    pub sibling_guard: String,
    /// 降级前的原始 bundle id（执行期 fingerprint 复查用）。
    pub original_bundle_id: String,
    /// live sibling 扫描的 fingerprint（预览期快照，执行期复查用）。
    pub live_sibling_fingerprint: String,
    /// 预览期记录的 app bundle 身份 `dev:ino:mode`（对齐 SH `expected_app_identity`）。
    /// 执行期重查不一致即拒绝，防预览后路径被替换。
    pub expected_app_identity: String,
    /// 预览期记录的 Info.plist 身份（对齐 SH `expected_info_identity`；缺失为 "missing"）。
    pub expected_info_identity: String,
    /// 非空表示该 app 被拒绝并转 manual removal（对齐 SH `manual_removal_apps`），
    /// 不进入预览/执行，只报告原因。
    pub manual_removal_reason: String,
}

fn read_bundle_executable(app_path: &str) -> String {
    let plist = format!("{app_path}/Contents/Info.plist");
    if !Path::new(&plist).is_file() {
        return String::new();
    }
    Command::new("defaults")
        .args(["read", &plist, "CFBundleExecutable"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn read_bundle_identifier(app_path: &str) -> String {
    let plist = format!("{app_path}/Contents/Info.plist");
    if !Path::new(&plist).is_file() {
        return String::new();
    }
    let v = Command::new("plutil")
        .args(["-extract", "CFBundleIdentifier", "raw", &plist])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if !v.is_empty() {
        return v;
    }
    Command::new("defaults")
        .args(["read", &plist, "CFBundleIdentifier"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn pgrep_exact(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    Command::new("pgrep")
        .args(["-x", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn current_user() -> String {
    std::env::var("USER").unwrap_or_else(|_| {
        Command::new("whoami")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    })
}

fn fast_cask_name_from_symlink(app_path: &str) -> Option<String> {
    let p = Path::new(app_path);
    if !p.is_symlink() {
        return None;
    }
    let resolved = std::fs::read_link(p).ok()?;
    let resolved_str = resolved.to_string_lossy().to_string();
    let marker = "/Caskroom/";
    let idx = resolved_str.find(marker)?;
    let after = &resolved_str[idx + marker.len()..];
    let token = after.split('/').next().unwrap_or("");
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

// ============================================================================
// Sibling guard（对齐 SH `uninstall_bundle_id_has_surviving_sibling` 轻量版）
// ============================================================================

/// bundle id 大小写不敏感归一（对齐 SH `uninstall_normalize_bundle_id`）。
fn normalize_bundle_id(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// 版本/渠道后缀剥离（对齐 SH `uninstall_strip_version_suffix`）。
fn strip_version_suffix(name: &str) -> String {
    for s in [
        "Nightly",
        "Beta",
        "Alpha",
        "Dev",
        "Canary",
        "Preview",
        "Insider",
        "Edge",
        "Stable",
        "Release",
        "RC",
        "LTS",
        "Developer Edition",
        "Technology Preview",
    ] {
        let marker = format!(" {s}");
        if let Some(base) = name.strip_suffix(&marker) {
            let base = base.trim();
            if !base.is_empty() {
                return base.to_string();
            }
        }
    }
    name.to_string()
}

/// live sibling 扫描返回码：有兄弟(0) / 无兄弟(1) / 不确定(2) / 部分扫描(3)。
const MOLE_UNINSTALL_SCAN_PARTIAL: i32 = 3;

/// live 扫描的 app 根（对齐 SH `_MOLE_UNINSTALL_LIVE_APP_ROOTS`）。
fn live_app_roots() -> Vec<String> {
    let home = home_dir();
    vec![
        "/Applications".to_string(),
        format!("{home}/Applications"),
        "/System/Applications".to_string(),
        "/Library/Input Methods".to_string(),
        format!("{home}/Library/Input Methods"),
        format!("{home}/Library/Application Support/Setapp/Applications"),
        "/opt/homebrew/Caskroom".to_string(),
        "/usr/local/Caskroom".to_string(),
    ]
}

/// path 的 hex 编码（避免 fingerprint 里 path 与分隔符冲突）。
fn hex_encode(s: &str) -> String {
    s.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// 文件身份 (dev, ino, mtime)，Rust 原生替代 SH 的 `stat -f%d:%i:%m`。
fn stat_identity(path: &str) -> Option<(u64, u64, i64)> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path).ok()?;
    Some((meta.dev(), meta.ino(), meta.mtime()))
}

/// 与 SH `[[ a -ef b ]]` 同义：跟随符号链接后同设备 + 同 inode。
fn dirs_same_file(a: &str, b: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

/// 候选是否就是选中的 app（含 inode 相同）。
fn live_candidate_is_selected(candidate: &str, selected_path: &str) -> bool {
    if candidate == selected_path {
        return true;
    }
    match (stat_identity(candidate), stat_identity(selected_path)) {
        (Some(a), Some(b)) => a.0 == b.0 && a.1 == b.1,
        _ => false,
    }
}

/// 候选是否是某个 .app 内嵌套的子 app（对齐 SH `_uninstall_live_candidate_is_nested_app`）。
fn live_candidate_is_nested_app(root: &str, candidate: &str) -> bool {
    if candidate == root {
        return false;
    }
    let Some(relative) = candidate.strip_prefix(&format!("{root}/")) else {
        return false;
    };
    let parent = match relative.rfind('/') {
        Some(i) => &relative[..i],
        None => return false,
    };
    let mut rest = parent;
    while !rest.is_empty() && rest != "." {
        let component = match rest.find('/') {
            Some(i) => &rest[..i],
            None => rest,
        };
        if component.ends_with(".app") {
            return true;
        }
        rest = match rest.find('/') {
            Some(i) => &rest[i + 1..],
            None => "",
        };
    }
    false
}

/// 候选是否已记录（路径或 inode 相同）。
fn live_sibling_path_is_duplicate(candidate: &str, live_paths: &[String]) -> bool {
    live_paths.iter().any(|existing| {
        if existing == candidate {
            return true;
        }
        match (stat_identity(candidate), stat_identity(existing)) {
            (Some(a), Some(b)) => a.0 == b.0 && a.1 == b.1,
            _ => false,
        }
    })
}

/// 生成单个幸存兄弟的 fingerprint record。
fn live_sibling_record(app: &str, info: &str) -> Result<String, ()> {
    let ai = stat_identity(app).ok_or(())?;
    let ii = stat_identity(info).ok_or(())?;
    Ok(format!(
        "{}|{}|{}|{}|{}|{}|{}",
        hex_encode(app),
        ai.0,
        ai.1,
        ai.2,
        ii.0,
        ii.1,
        ii.2,
    ))
}

/// plist 里 bundle id 的读取结果（对齐 SH `_uninstall_collect_live_sibling_candidate`）。
enum LiveBundleId {
    Present(String),
    Absent,
    Unreadable,
}

fn read_live_bundle_id(info: &str) -> LiveBundleId {
    if let Ok(out) = Command::new("plutil")
        .args(["-extract", "CFBundleIdentifier", "raw", info])
        .output()
    {
        if out.status.success() {
            let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !id.is_empty() && id != "(null)" {
                return LiveBundleId::Present(id);
            }
        }
    }
    // 无 id 或提取失败 → 用 lint 区分「能解析但无 id」与「损坏」。
    if let Ok(lint) = Command::new("plutil").args(["-lint", info]).output() {
        if lint.status.success() {
            return LiveBundleId::Absent;
        }
    }
    LiveBundleId::Unreadable
}

/// 收集一个 live 候选。返回 Ok(true)=是兄弟 / Ok(false)=不是 / Err=不确定。
fn collect_live_sibling_candidate(
    app: &str,
    selected_path: &str,
    bundle_id_lower: &str,
    live_paths: &mut Vec<String>,
    live_records: &mut Vec<String>,
) -> Result<bool, ()> {
    if live_candidate_is_selected(app, selected_path) {
        return Ok(false);
    }
    let mut info = format!("{app}/Contents/Info.plist");
    if !Path::new(&info).is_file() {
        // iOS/iPadOS：Wrapper/<name>.app/Info.plist
        let mut found = false;
        if let Ok(rd) = std::fs::read_dir(format!("{app}/Wrapper")) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() && p.extension().and_then(|s| s.to_str()) == Some("app") {
                    let c = format!("{}/Info.plist", p.display());
                    if Path::new(&c).is_file() {
                        info = c;
                        found = true;
                        break;
                    }
                }
            }
        }
        if !found {
            return Ok(false);
        }
    }
    match read_live_bundle_id(&info) {
        LiveBundleId::Present(id) => {
            if normalize_bundle_id(&id) != bundle_id_lower {
                return Ok(false);
            }
            if live_sibling_path_is_duplicate(app, live_paths) {
                return Ok(false);
            }
            let record = live_sibling_record(app, &info)?;
            live_paths.push(app.to_string());
            live_records.push(record);
            Ok(true)
        }
        LiveBundleId::Absent => Ok(false),
        LiveBundleId::Unreadable => Err(()),
    }
}

/// 扫描 root 下 maxdepth 层的 *.app（dir 或 symlink）。
/// indeterminate 只在 root 本身不可读时置 true；发现 .app 后不递归进其内部，
/// 避免因 app 内部受保护目录 read_dir 失败而误判「扫描不完整」（对齐 find 的静默跳过）。
fn find_app_bundles(root: &str, maxdepth: usize, indeterminate: &mut bool) -> Vec<String> {
    fn walk(
        dir: &str,
        depth: usize,
        maxdepth: usize,
        is_root: bool,
        out: &mut Vec<String>,
        indeterminate: &mut bool,
    ) {
        if depth > maxdepth {
            return;
        }
        let rd = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => {
                // 只有 root 本身读不了才算「可能漏判兄弟」；深层不可读静默跳过。
                if is_root {
                    *indeterminate = true;
                }
                return;
            }
        };
        for entry in rd.flatten() {
            let ft = entry.file_type();
            let is_dir = ft.as_ref().map(|t| t.is_dir()).unwrap_or(false);
            let is_symlink = ft.as_ref().map(|t| t.is_symlink()).unwrap_or(false);
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if name.ends_with(".app") && (is_dir || is_symlink) {
                out.push(path.to_string_lossy().to_string());
                // 不递归进入 .app 内部：live 扫描不需要嵌套 app，且内部受保护目录会误判 partial。
                continue;
            }
            if is_dir && depth < maxdepth {
                walk(
                    &path.to_string_lossy(),
                    depth + 1,
                    maxdepth,
                    false,
                    out,
                    indeterminate,
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 1, maxdepth, true, &mut out, indeterminate);
    out
}

/// 扫描 /Volumes 下的 app 根（Applications 目录 + 直接 .app）。返回 (roots, indeterminate)。
fn find_volume_app_roots() -> (Vec<String>, bool) {
    let mut out = Vec::new();
    let mut indeterminate = false;
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        for entry in rd.flatten() {
            let vol = entry.path();
            let apps = vol.join("Applications");
            if apps.is_dir() {
                out.push(apps.to_string_lossy().to_string());
            }
            if let Ok(sub) = std::fs::read_dir(&vol) {
                for e in sub.flatten() {
                    let p = e.path();
                    let name = e.file_name().to_string_lossy().to_string();
                    let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    let is_symlink = e.file_type().map(|t| t.is_symlink()).unwrap_or(false);
                    if name.ends_with(".app") && (is_dir || is_symlink) {
                        out.push(p.to_string_lossy().to_string());
                    }
                }
            } else {
                indeterminate = true;
            }
        }
    }
    (out, indeterminate)
}

/// 完整 live sibling 扫描（对齐 SH `uninstall_live_bundle_has_other_install`）。
/// 返回 (result_code, fingerprint)：0=有兄弟 / 1=无兄弟 / 2=不确定 / 3=部分扫描。
fn uninstall_live_bundle_has_other_install(bundle_id: &str, selected_path: &str) -> (i32, String) {
    if !is_valid_reverse_dns_bundle_id(bundle_id) {
        return (1, String::new());
    }
    let bundle_id_lower = normalize_bundle_id(bundle_id);
    let mut scan_indeterminate = false;

    let (pkg_apps, pkg_complete) =
        crate::core::pkg_receipts::pkg_receipt_nonstandard_app_paths_complete();
    if !pkg_complete {
        scan_indeterminate = true;
    }

    let mut live_roots = live_app_roots();
    if Path::new("/Volumes").is_dir() {
        let (vol_roots, vol_indeterminate) = find_volume_app_roots();
        if vol_indeterminate {
            scan_indeterminate = true;
        }
        live_roots.extend(vol_roots);
    }

    let mut live_paths: Vec<String> = Vec::new();
    let mut live_records: Vec<String> = Vec::new();
    let mut result: i32 = 1;

    for root in &live_roots {
        if !Path::new(root).exists() {
            continue;
        }
        if !Path::new(root).is_dir() {
            result = 2;
            break;
        }
        let mut find_indeterminate = false;
        let apps = find_app_bundles(root, 3, &mut find_indeterminate);
        if find_indeterminate {
            scan_indeterminate = true;
        }
        for app in &apps {
            // 跳过嵌套在 .app 内的子 app（对齐 SH `_uninstall_live_candidate_is_nested_app`）。
            if live_candidate_is_nested_app(root, app) {
                continue;
            }
            match collect_live_sibling_candidate(
                app,
                selected_path,
                &bundle_id_lower,
                &mut live_paths,
                &mut live_records,
            ) {
                Ok(true) => result = 0,
                Ok(false) => {}
                Err(()) => {
                    result = 2;
                    break;
                }
            }
        }
        if result == 2 {
            break;
        }
    }

    if result != 2 {
        for app in &pkg_apps {
            match collect_live_sibling_candidate(
                app,
                selected_path,
                &bundle_id_lower,
                &mut live_paths,
                &mut live_records,
            ) {
                Ok(true) => result = 0,
                Ok(false) => {}
                Err(()) => {
                    result = 2;
                    break;
                }
            }
        }
    }

    let fingerprint = if result == 0 {
        live_records.sort();
        live_records.dedup();
        live_records.join("\n")
    } else {
        String::new()
    };

    // 扫描不完整（部分不可读 / receipt 超时）且未找到兄弟 → 当作"可能有兄弟"。
    if scan_indeterminate && result == 1 {
        result = MOLE_UNINSTALL_SCAN_PARTIAL;
    }

    log::info!(
        "[uninstall.live_sibling] bundle={bundle_id} selected={selected_path} result={result} indeterminate={scan_indeterminate}"
    );

    (result, fingerprint)
}

/// 已安装应用清单：(path, bundle_id, basename 去 .app)。
/// sibling guard 第二道（清单级）只关心同 bundle id 幸存兄弟，不读 size。
fn installed_app_inventory() -> Vec<(String, String, String)> {
    let home = home_dir();
    let mut app_dirs: Vec<String> = vec![
        "/Applications".to_string(),
        format!("{home}/Applications"),
        "/Library/Input Methods".to_string(),
        format!("{home}/Library/Input Methods"),
    ];
    if let Ok(entries) = std::fs::read_dir("/Volumes") {
        for entry in entries.flatten() {
            let vol_app = format!("{}/Applications", entry.path().display());
            if !Path::new(&vol_app).is_dir() || std::fs::read_dir(&vol_app).is_err() {
                continue;
            }
            // 对齐 SH bin/uninstall.sh L377-381（-ef 检查）：跳过与 /Applications
            // 或 ~/Applications 同 inode 的卷镜像目录（firmlink / DMG symlink）。
            // 否则镜像里的同 bundle app 会被 sibling guard 误判为「幸存兄弟」，
            // 名字碰撞后清空 discovery_app_name → 残留发现整体被跳过。
            if dirs_same_file(&vol_app, "/Applications")
                || dirs_same_file(&vol_app, &format!("{home}/Applications"))
            {
                continue;
            }
            app_dirs.push(vol_app);
        }
    }

    let mut out: Vec<(String, String, String)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // pkg receipts 非标准安装位置
    for pkg_path in crate::core::pkg_receipts::pkg_receipt_nonstandard_app_paths() {
        if !Path::new(&pkg_path).is_dir() || !seen.insert(pkg_path.clone()) {
            continue;
        }
        let app_name = Path::new(&pkg_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .strip_suffix(".app")
            .unwrap_or("")
            .to_string();
        out.push((
            pkg_path.clone(),
            read_bundle_identifier(&pkg_path),
            app_name,
        ));
    }

    // 标准 app_dirs 下的 *.app（maxdepth 3）
    for dir in &app_dirs {
        if !Path::new(dir).is_dir() {
            continue;
        }
        let Ok(output) = Command::new("find")
            .arg(dir)
            .args(["-name", "*.app"])
            .args(["-maxdepth", "3"])
            .arg("-print0")
            .output()
        else {
            continue;
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        for raw in stdout.split('\0') {
            let app_path = raw.trim();
            if app_path.is_empty() || !Path::new(app_path).exists() {
                continue;
            }
            // 跳过嵌套 .app
            if let Some(parent) = Path::new(app_path).parent() {
                let ps = parent.to_string_lossy();
                if ps.contains(".app/") || ps.ends_with(".app") {
                    continue;
                }
            }
            if !seen.insert(app_path.to_string()) {
                continue;
            }
            let app_name = Path::new(app_path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .strip_suffix(".app")
                .unwrap_or("")
                .to_string();
            out.push((
                app_path.to_string(),
                read_bundle_identifier(app_path),
                app_name,
            ));
        }
    }
    out
}

/// 轻量读取 display name（plist CFBundleDisplayName → CFBundleName → 回退 basename）。
/// 只在需要名字碰撞判断时按需调用，不读 mdls（避免唤醒 Spotlight）。
fn read_display_name_light(app_path: &str, base_name: &str) -> String {
    let plist = format!("{app_path}/Contents/Info.plist");
    if !Path::new(&plist).is_file() {
        return base_name.to_string();
    }
    for key in ["CFBundleDisplayName", "CFBundleName"] {
        if let Ok(out) = Command::new("plutil")
            .args(["-extract", key, "raw", "-o", "-", &plist])
            .output()
        {
            if out.status.success() {
                let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !v.is_empty() && v != "(null)" {
                    return v;
                }
            }
        }
    }
    base_name.to_string()
}

/// 同 bundle id 且不在本次选中集合里的"幸存兄弟"的名字集合（lowercase，含 strip 版本）。
fn surviving_sibling_names(
    bundle_id: &str,
    app_path: &str,
    inventory: &[(String, String, String)],
    selected_paths: &[String],
) -> Vec<String> {
    if bundle_id.is_empty() || bundle_id == "unknown" {
        return Vec::new();
    }
    let bid_lower = normalize_bundle_id(bundle_id);
    let mut names: Vec<String> = Vec::new();
    for (other_path, other_bundle, other_base) in inventory {
        if normalize_bundle_id(other_bundle) != bid_lower {
            continue;
        }
        if other_path == app_path || !Path::new(other_path).is_dir() {
            continue;
        }
        if selected_paths.iter().any(|sp| sp == other_path) {
            continue;
        }
        let other_name = read_display_name_light(other_path, other_base);
        for candidate in [other_name.as_str(), other_base.as_str()] {
            if candidate.is_empty() {
                continue;
            }
            names.push(candidate.to_ascii_lowercase());
            names.push(strip_version_suffix(candidate).to_ascii_lowercase());
        }
    }
    names
}

/// 是否还有同 bundle id 的幸存兄弟（对齐 SH `uninstall_bundle_id_has_surviving_sibling`）。
fn has_surviving_sibling(
    bundle_id: &str,
    app_path: &str,
    inventory: &[(String, String, String)],
    selected_paths: &[String],
) -> bool {
    if bundle_id.is_empty() || bundle_id == "unknown" {
        return false;
    }
    let bid_lower = normalize_bundle_id(bundle_id);
    inventory.iter().any(|(other_path, other_bundle, _)| {
        normalize_bundle_id(other_bundle) == bid_lower
            && other_path != app_path
            && Path::new(other_path).is_dir()
            && !selected_paths.iter().any(|sp| sp == other_path)
    })
}

/// 选中 app 的名字是否与幸存兄弟碰撞（对齐 SH 第 1449-1458 行）。
fn discovery_name_collides(discovery_app_name: &str, survivor_names: &[String]) -> bool {
    let discovery_lower = discovery_app_name.to_ascii_lowercase();
    let discovery_base_lower = strip_version_suffix(discovery_app_name).to_ascii_lowercase();
    survivor_names.iter().any(|s| {
        discovery_lower == *s
            || discovery_base_lower == *s
            || s.contains(&discovery_lower)
            || s.contains(&discovery_base_lower)
    })
}

/// 对齐 SH 第 1280-1295 行 `_batch_selected_app_info_identity`:
/// Info.plist 的 `dev:ino:mode`;不存在且非符号链接 → "missing"。
fn selected_app_info_identity(app_path: &str) -> Option<String> {
    let info = format!("{app_path}/Contents/Info.plist");
    if !Path::new(&info).exists() && !Path::new(&info).is_symlink() {
        return Some("missing".to_string());
    }
    stat_path_identity(&info)
}

/// 对齐 SH 第 1297-1314 行 `_batch_selected_app_plan_matches`:
/// 两个预期身份都非空,且当前 app bundle 与 Info.plist 的 `dev:ino:mode` 均与预览一致。
fn selected_app_plan_matches(
    app_path: &str,
    expected_app_identity: &str,
    expected_info_identity: &str,
) -> bool {
    if expected_app_identity.is_empty() || expected_info_identity.is_empty() {
        return false;
    }
    let Some(current_app) = stat_path_identity(app_path) else {
        return false;
    };
    let Some(current_info) = selected_app_info_identity(app_path) else {
        return false;
    };
    current_app == expected_app_identity && current_info == expected_info_identity
}

/// 构造一个 manual-removal 占位 detail(对齐 SH `manual_removal_apps` 语义:
/// 不进入预览/执行,只报告)。
fn manual_removal_detail(app_name: &str, app_path: &str, reason: &str) -> AppDetail {
    AppDetail {
        app_name: app_name.to_string(),
        app_path: app_path.to_string(),
        bundle_id: String::new(),
        total_kb: 0,
        related_files: String::new(),
        review_system_files: String::new(),
        system_files: String::new(),
        diag_system: String::new(),
        has_sensitive_data: false,
        needs_sudo: false,
        is_brew_cask: false,
        cask_name: String::new(),
        has_local_network_usage: false,
        login_item_helpers: String::new(),
        is_official_uninstaller: false,
        official_vendor: String::new(),
        sibling_guard: String::new(),
        original_bundle_id: String::new(),
        live_sibling_fingerprint: String::new(),
        expected_app_identity: String::new(),
        expected_info_identity: String::new(),
        manual_removal_reason: reason.to_string(),
    }
}

/// 对齐 SH 第 348-440 行预扫描。`selected_apps` 是 `.app` 路径列表。
///
/// 注意:SH 输入是 `id|app_path|app_name|bundle_id|...` 6 字段管道串,
/// 这里 GUI 简化为只接收路径,所有 app_name / bundle_id / 关联文件由 Rust 重新计算。
pub fn collect_app_details(selected_apps: &[String]) -> Result<Vec<AppDetail>, String> {
    let mut details = Vec::new();
    let user = current_user();
    let home = home_dir();
    // sibling guard 需要一次全量清单（只关心 path + bundle_id + basename）。
    let inventory = installed_app_inventory();

    for app_path in selected_apps {
        if app_path.is_empty() || !Path::new(app_path).exists() {
            continue;
        }
        let app_name_raw = Path::new(app_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        // 对齐 SH：app_name 不带 .app 后缀，否则 find_app_files 的路径候选匹配不上
        let app_name = app_name_raw
            .strip_suffix(".app")
            .unwrap_or(&app_name_raw)
            .to_string();
        let mut bundle_id = read_bundle_identifier(app_path);
        // 记录降级前的原始 bundle id，执行期 fingerprint 复查用。
        let original_bundle_id = bundle_id.clone();

        // 对齐 SH 第 1342-1364 行:把确认记录绑定到被检查的确切 bundle 对象。
        // 预览期间路径可能被替换,执行期必须拒绝新 inode 而不是把同一路径名
        // 当作用户确认;identity 获取失败即转 manual removal。
        let Some(app_identity) = stat_path_identity(app_path) else {
            log_warning(&format!(
                "{app_name}: could not bind selected app identity, manual removal"
            ));
            details.push(manual_removal_detail(
                &app_name,
                app_path,
                "selected app identity unavailable",
            ));
            continue;
        };
        let Some(info_identity) = selected_app_info_identity(app_path) else {
            log_warning(&format!(
                "{app_name}: could not bind selected app Info.plist identity, manual removal"
            ));
            details.push(manual_removal_detail(
                &app_name,
                app_path,
                "selected app Info.plist identity unavailable",
            ));
            continue;
        };

        // 完整版 sibling guard（对齐 SH 第 1383-1471 行）：
        // 1) live 全盘扫描（权威）判断实时是否有同 bundle id 幸存兄弟；
        // 2) live 证明无兄弟后，再用已安装清单做第二道核对。
        let mut discovery_app_name = app_name.clone();
        let mut sibling_guard = "none".to_string();
        let mut live_sibling_fingerprint = String::new();

        let (live_rc, fingerprint) = uninstall_live_bundle_has_other_install(&bundle_id, app_path);
        let mut live_sibling_present = false;
        match live_rc {
            0 => {
                live_sibling_present = true;
                live_sibling_fingerprint = fingerprint;
            }
            1 => {} // 完整证明无兄弟
            i if i == MOLE_UNINSTALL_SCAN_PARTIAL => {
                // 扫描部分不可读 / receipt 超时 → 无法排除兄弟，保守当作有。
                live_sibling_present = true;
                log_warning(&format!(
                    "{app_name}: some paths could not be read, so shared leftovers are left in place"
                ));
            }
            i if i >= 128 => {
                // 信号：本实现不会产生，保留分支语义（保守当作有）。
                live_sibling_present = true;
            }
            _ => {
                // 2 = 无法确定（plist 损坏等）→ 整个批次失败（对齐 SH return 1）。
                log_error(&format!(
                    "Could not verify whether other installs share {app_name}'s bundle id; nothing was removed"
                ));
                return Err(format!(
                    "Could not complete the live same-bundle scan for {app_name}"
                ));
            }
        }

        if live_sibling_present {
            sibling_guard = "guard_login".to_string();
            discovery_app_name = String::new();
            bundle_id = "unknown".to_string();
        } else if has_surviving_sibling(&bundle_id, app_path, &inventory, selected_apps) {
            let survivor_names =
                surviving_sibling_names(&bundle_id, app_path, &inventory, selected_apps);
            if discovery_name_collides(&discovery_app_name, &survivor_names) {
                sibling_guard = "guard_login".to_string();
                discovery_app_name = String::new();
            } else {
                sibling_guard = "guard".to_string();
            }
            bundle_id = "unknown".to_string();
        }

        log::info!(
            "[uninstall.sibling] app={app_name} original_bundle={original_bundle_id} live_rc={live_rc} guard={sibling_guard} discovery={:?} bundle_after={bundle_id}",
            discovery_app_name
        );

        // running app 检测
        let exec_name = read_bundle_executable(app_path);
        let _running = pgrep_exact(if exec_name.is_empty() {
            &app_name
        } else {
            &exec_name
        });

        // brew cask 检测:先 fast 路径 readlink,然后再 fallback 到完整 get_brew_cask_name
        let mut cask_name = String::new();
        let mut is_brew_cask = false;
        if let Some(fast) = fast_cask_name_from_symlink(app_path) {
            cask_name = fast;
            is_brew_cask = true;
        } else if let Some(detected) = get_brew_cask_name(app_path) {
            cask_name = detected;
            is_brew_cask = true;
        }

        // sudo 需求
        let mut needs_sudo = false;
        let parent = Path::new(app_path).parent();
        let parent_writable = parent
            .map(|p| {
                use std::ffi::CString;
                CString::new(p.to_string_lossy().as_bytes())
                    .map(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 })
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !parent_writable {
            needs_sudo = true;
        }
        let owner = get_file_owner(app_path);
        if owner == "root" || (!owner.is_empty() && owner != user) {
            needs_sudo = true;
        }

        // 对齐 SH 第 1518-1527 行:特权删除的路径若存在调用用户可写祖先,
        // 预览绑定的路径无法绑定到 root 最终删除的对象(TOCTOU)。在任何
        // 发现/sudo 授权/进程 teardown 之前拒绝并转 manual removal。
        // Homebrew casks 走包管理器路径,不做此 direct-app preflight。
        if needs_sudo && !is_brew_cask && _mole_privileged_path_has_mutable_ancestor(app_path) {
            log_warning(&format!(
                "{app_name}: privileged removal below a mutable parent, manual removal"
            ));
            details.push(manual_removal_detail(
                &app_name,
                app_path,
                "privileged removal below a mutable parent",
            ));
            continue;
        }

        // 关联文件 / 系统文件 / 诊断报告
        let app_size_kb = get_path_size_kb(app_path);

        // 工具链启发式在 sibling guard 下整体关闭（对齐 SH `MOLE_UNINSTALL_SIBLING_SURVIVES=1`）。
        if sibling_guard != "none" {
            unsafe { std::env::set_var("MOLE_UNINSTALL_SIBLING_SURVIVES", "1") };
        }

        // discovery_app_name 为空（guard_login 名字碰撞）→ 只删 bundle，跳过发现。
        let (related_user, diag_user) = if discovery_app_name.is_empty() {
            (Vec::new(), Vec::new())
        } else {
            let related = find_app_files(&bundle_id, &discovery_app_name, app_path);
            let diag = if sibling_guard == "none" {
                get_diagnostic_report_paths_for_app(
                    app_path,
                    &discovery_app_name,
                    &format!("{home}/Library/Logs/DiagnosticReports"),
                )
            } else {
                Vec::new()
            };
            (related, diag)
        };
        if sibling_guard != "none" {
            unsafe { std::env::remove_var("MOLE_UNINSTALL_SIBLING_SURVIVES") };
        }

        // 新一代残留深度扫描（leftovers.rs）：UUID 容器 / Group Container（entitlements）/
        // base bundle id 派生 / Library depth-2 厂商目录。sibling guard 时 bundle_id 已降级
        // 为 unknown，引擎会自动关闭所有 bundle id 派生匹配；guard_login（名字碰撞）
        // 时与 find_app_files 一样整体跳过。
        let deep = if discovery_app_name.is_empty() {
            leftovers::DeepLeftovers::default()
        } else {
            leftovers::scan_deep_leftovers(&bundle_id, &discovery_app_name, app_path)
        };

        // 合并后做路径父子去重（Lemon filepathExistsArray 语义）：
        // 父目录在集内时子路径不重复列出，展示与大小统计都不重复。
        let related_vec = leftovers::dedupe_parents_child(
            related_user
                .iter()
                .chain(diag_user.iter())
                .chain(deep.deletable.iter())
                .cloned()
                .collect(),
        );

        let mut related_combined = String::new();
        for f in &related_vec {
            if !related_combined.is_empty() {
                related_combined.push('\n');
            }
            related_combined.push_str(f);
        }
        let related_size_kb = calculate_total_size(&related_combined);

        let system_user = if discovery_app_name.is_empty() {
            Vec::new()
        } else {
            find_app_system_files(&bundle_id, &discovery_app_name)
        };
        let mut system_files_str = String::new();
        for f in &system_user {
            if !system_files_str.is_empty() {
                system_files_str.push('\n');
            }
            system_files_str.push_str(f);
        }
        let diag_sys_paths = if discovery_app_name.is_empty() || sibling_guard != "none" {
            Vec::new()
        } else {
            get_diagnostic_report_paths_for_app(
                app_path,
                &discovery_app_name,
                "/Library/Logs/DiagnosticReports",
            )
        };
        let mut diag_system_str = String::new();
        for f in &diag_sys_paths {
            if !diag_system_str.is_empty() {
                diag_system_str.push('\n');
            }
            diag_system_str.push_str(f);
        }
        // New CLI: system files are review-only — show in preview but don't delete.
        // Save a copy for the frontend preview, then blank system_files/diag_system
        // so that _batch_execute_removals skips them entirely.
        // 深度扫描的系统域命中（/Library LaunchDaemon、PrivilegedHelperTools 等）
        // 同样只进审阅集；一并做父子去重。
        let review_vec = leftovers::dedupe_parents_child(
            system_user
                .iter()
                .chain(diag_sys_paths.iter())
                .chain(deep.review_only.iter())
                .cloned()
                .collect(),
        );
        let mut review_system = String::new();
        for f in &review_vec {
            if !review_system.is_empty() {
                review_system.push('\n');
            }
            review_system.push_str(f);
        }
        // Blank — do NOT delete system files.
        let system_files_str = String::new();
        let diag_system_str = String::new();

        // total_kb recalculated to exclude system files (review-only)。
        let total_kb = app_size_kb + related_size_kb;

        if !system_files_str.is_empty() || !diag_system_str.is_empty() {
            needs_sudo = true;
        }

        let has_sens = has_sensitive_data(&related_combined);
        let has_local_network_usage = app_declares_local_network_usage(app_path);

        // ── 日志：记录每个 app 扫描到的文件清单 ──
        log::info!(
            "[uninstall.collect] app={app_name} bundle={bundle_id} total_kb={total_kb} \
             related_cnt={} system_cnt={} diag_sys_cnt={} deep_review_cnt={} needs_sudo={needs_sudo} \
             is_brew={is_brew_cask}",
            related_vec.len(),
            system_user.len(),
            diag_sys_paths.len(),
            deep.review_only.len(),
        );
        if !related_vec.is_empty() {
            log::info!("[uninstall.collect.related] {app_name}: {:?}", related_vec);
        }
        if !system_user.is_empty() {
            log::info!(
                "[uninstall.collect.system_files] {app_name}: {:?}",
                system_user
            );
        }
        if !diag_sys_paths.is_empty() {
            log::info!(
                "[uninstall.collect.diag_system] {app_name}: {:?}",
                diag_sys_paths
            );
        }

        // New CLI: discover Login Item Helper bundle IDs.
        let login_helpers = discover_login_item_helper_bundle_ids(app_path);

        // New CLI: check for official uninstaller.
        let (is_official, official_vendor) =
            check_official_uninstaller(&bundle_id, &app_name, app_path);

        details.push(AppDetail {
            app_name,
            app_path: app_path.clone(),
            bundle_id,
            total_kb,
            related_files: related_combined,
            review_system_files: review_system,
            system_files: system_files_str,
            diag_system: diag_system_str,
            has_sensitive_data: has_sens,
            needs_sudo,
            is_brew_cask,
            cask_name,
            has_local_network_usage,
            login_item_helpers: login_helpers,
            is_official_uninstaller: is_official,
            official_vendor,
            sibling_guard,
            original_bundle_id,
            live_sibling_fingerprint,
            expected_app_identity: app_identity,
            expected_info_identity: info_identity,
            manual_removal_reason: String::new(),
        });
    }
    Ok(details)
}

// ============================================================================
// Per-app uninstall driver (SH 第 558-822 行)
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct AppUninstallOutcome {
    pub app_name: String,
    pub app_path: String,
    pub success: bool,
    pub freed_kb: u64,
    pub used_brew_successfully: bool,
    pub leftover_paths: Vec<String>,
    /// 删除的路径列表（用于历史记录）
    pub deleted_paths: Vec<String>,
    pub reason: String,
    pub suggestion: String,
    pub has_local_network_usage: bool,
    pub has_system_extension: bool,
    /// App was still running at uninstall time — success, but warn the user.
    pub running_at_uninstall: bool,
}

/// 对齐新 CLI: discover_login_item_helper_bundle_ids.
/// 扫描 app 包内的 Contents/Library/LoginItems/*.app,提取 bundle ID.
fn discover_login_item_helper_bundle_ids(app_path: &str) -> String {
    let login_items_root = format!("{app_path}/Contents/Library/LoginItems");
    let rd = match std::fs::read_dir(&login_items_root) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };
    let mut ids = String::new();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".app") {
            continue;
        }
        let info = format!("{}/Contents/Info.plist", entry.path().display());
        if !Path::new(&info).is_file() {
            continue;
        }
        let bid = read_bundle_identifier_simple(&info);
        if is_valid_reverse_dns_bundle_id(&bid) {
            if !ids.is_empty() {
                ids.push('\n');
            }
            ids.push_str(&bid);
        }
    }
    ids
}

/// 对齐新 CLI: bootout_login_item_helpers — 用 launchctl bootout gui/$uid/ 注销.
fn bootout_login_item_helpers(helper_ids: &str) {
    if helper_ids.is_empty() || is_uninstall_dry_run() {
        return;
    }
    let uid = unsafe { libc::getuid() };
    for hid in helper_ids.lines() {
        let hid = hid.trim();
        if hid.is_empty() || !is_valid_reverse_dns_bundle_id(hid) {
            continue;
        }
        let _ = run_with_timeout(
            MOLE_TIMEOUT_MEDIUM_PROBE_SEC,
            "launchctl",
            &["bootout", &format!("gui/{uid}/{hid}")],
        );
    }
}

/// 对齐新 CLI: official_uninstaller_vendor — 检查 app 是否自带卸载器或需要官方卸载器.
/// 完整覆盖 app_protection_data.sh 中的 OFFICIAL_UNINSTALLER_RULES:
///   ESET / Jamf / CrowdStrike / SentinelOne / GlobalProtect / Cisco
/// 返回 (is_official, vendor_name).
fn check_official_uninstaller(bundle_id: &str, app_name: &str, app_path: &str) -> (bool, String) {
    // Check for bundled uninstaller applications inside the .app package.
    let candidates = [
        "Uninstall.app",
        "Uninstaller.app",
        "Uninstall Assistant.app",
    ];
    for c in &candidates {
        let p = format!("{app_path}/Contents/Resources/{c}");
        if Path::new(&p).exists() {
            return (true, "bundled uninstaller".to_string());
        }
    }

    let normalized_bid = bundle_id.to_lowercase();
    let normalized_name = app_name.to_lowercase();

    // 对齐 app_protection_data.sh OFFICIAL_UNINSTALLER_RULES
    const OFFICIAL_RULES: &[(&[&str], &[&str], &str)] = &[
        // (bundle_prefixes, name_fragments, vendor_name)
        (
            &["com.eset."],
            &[
                "eset management agent",
                "eset remote administrator agent",
                "eset endpoint security",
                "eset endpoint antivirus",
            ],
            "ESET",
        ),
        (
            &["com.jamf.", "com.jamfsoftware."],
            &["jamf connect", "jamf protect", "jamf self service"],
            "Jamf",
        ),
        (
            &["com.crowdstrike."],
            &["crowdstrike", "falcon"],
            "CrowdStrike",
        ),
        (
            &["com.sentinelone.", "com.sentinel-labs."],
            &["sentinelone", "sentinel agent"],
            "SentinelOne",
        ),
        (
            &["com.paloaltonetworks."],
            &["globalprotect"],
            "GlobalProtect",
        ),
        (
            &["com.cisco.anyconnect", "com.cisco.secureclient"],
            &["cisco secure client", "cisco anyconnect"],
            "Cisco",
        ),
        (&["com.adobe"], &[], "Adobe"),
        (
            &[
                "com.microsoft.office",
                "com.microsoft.word",
                "com.microsoft.excel",
                "com.microsoft.powerpoint",
                "com.microsoft.outlook",
            ],
            &["microsoft office"],
            "Microsoft Office",
        ),
    ];

    for (prefixes, fragments, vendor) in OFFICIAL_RULES {
        // Match by bundle id prefix
        for prefix in *prefixes {
            if normalized_bid.starts_with(prefix) {
                return (true, vendor.to_string());
            }
        }
        // Match by name fragment
        for fragment in *fragments {
            if normalized_name.contains(fragment) {
                return (true, vendor.to_string());
            }
        }
    }

    (false, String::new())
}

/// 对齐 SH `_uninstall_match_loaded_background_items`(第 145-191 行)与
/// `_uninstall_background_job_loaded`(第 116-143 行):检查成功卸载的 app 的
/// bundle id / login-item helper 是否仍 loaded 在 launchd(bootout 被漏过或
/// 失败),命中则警告。SH 刻意不用 sfltool dumpbtm:非特权 dumpbtm 每次批处理
/// 都会弹 "sfltool wants to make changes" 管理员密码对话框,且 registered-
/// but-unloaded 的 BTM 记录本就是 macOS 下次登录时清理的设计残留。
fn check_btm_leftovers(success_paths: &[String], details: &[AppDetail]) -> Vec<String> {
    if success_paths.is_empty() || details.is_empty() {
        return Vec::new();
    }
    // 对齐 SH 第 120-122 行:test 模式报告 "not loaded",summary 保持安静。
    if std::env::var("MOLE_TEST_MODE").unwrap_or_default() == "1"
        || std::env::var("MOLE_TEST_NO_AUTH").unwrap_or_default() == "1"
    {
        return Vec::new();
    }
    let uid = unsafe { libc::getuid() };
    let mut leftovers = Vec::new();
    for d in details {
        if !success_paths.iter().any(|sp| sp == &d.app_path) {
            continue;
        }
        // sibling guard 可能把 bundle_id 降级为 unknown,而 helper id 仍然有效;
        // 逐 label 校验 reverse-DNS 后探测,无需显式跳过 unknown。
        let mut loaded = false;
        let mut labels: Vec<&str> = d.login_item_helpers.lines().map(str::trim).collect();
        labels.push(d.bundle_id.as_str());
        for label in labels {
            if label.is_empty() || !is_valid_reverse_dns_bundle_id(label) {
                continue;
            }
            if Command::new("launchctl")
                .args(["print", &format!("gui/{uid}/{label}")])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
            {
                loaded = true;
                break;
            }
        }
        if loaded {
            leftovers.push(d.app_name.clone());
        }
    }
    leftovers
}

fn read_bundle_identifier_simple(plist: &str) -> String {
    Command::new("plutil")
        .args(["-extract", "CFBundleIdentifier", "raw", plist])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// 容器 stub:macOS `containermanagerd` 通过 `com.apple.provenance` xattr 强保护,
/// `rm -rf` 必败。这类目录跳过,**不**计入 leftover。对齐 SH 第 711-716 行。
fn is_container_stub(path: &str) -> bool {
    path.contains("/Library/Containers/")
        && Path::new(&format!(
            "{path}/.com.apple.containermanagerd.metadata.plist"
        ))
        .is_file()
}

fn protected_symlink_target(target: &str) -> bool {
    matches!(
        target,
        s if s.starts_with("/System/")
            || s.starts_with("/usr/bin/")
            || s.starts_with("/usr/lib/")
            || s.starts_with("/bin/")
            || s.starts_with("/sbin/")
            || s.starts_with("/private/etc/")
    )
}

fn resolve_symlink_target_abs(app_path: &str) -> Option<String> {
    let target = std::fs::read_link(app_path).ok()?;
    let target_str = target.to_string_lossy().to_string();
    if target_str.starts_with('/') {
        return Some(target_str);
    }
    // 相对 → 拼父目录后 canonicalize
    let parent = Path::new(app_path).parent()?;
    let combined = parent.join(&target_str);
    Some(
        std::fs::canonicalize(&combined)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| combined.to_string_lossy().to_string()),
    )
}

/// 对齐 SH 第 705-726 行:计算 leftover 总大小(`du -skcP`)。
fn du_leftover_total_kb(paths: &[String]) -> u64 {
    if paths.is_empty() {
        return 0;
    }
    let mut args: Vec<&str> = vec!["-skcP"];
    for p in paths {
        args.push(p.as_str());
    }
    let out = Command::new("du").args(&args).output().ok();
    let Some(out) = out else { return 0 };
    let s = String::from_utf8_lossy(&out.stdout);
    // `du -skc` 总行 last line 起始数字
    let mut total: u64 = 0;
    for line in s.lines() {
        let first = line.split_whitespace().next().unwrap_or("");
        if let Ok(v) = first.parse::<u64>() {
            total = v; // 持续覆盖,最终取最后一行(`total`)
        }
    }
    total
}

/// 单个 app 的卸载流程,对齐 SH 第 558-822 行。
fn uninstall_one_app(detail: &AppDetail) -> AppUninstallOutcome {
    let mut outcome = AppUninstallOutcome {
        app_name: detail.app_name.clone(),
        app_path: detail.app_path.clone(),
        success: false,
        freed_kb: 0,
        used_brew_successfully: false,
        leftover_paths: Vec::new(),
        deleted_paths: Vec::new(),
        reason: String::new(),
        suggestion: String::new(),
        has_local_network_usage: detail.has_local_network_usage,
        has_system_extension: false,
        running_at_uninstall: false,
    };

    // manual removal(对齐 SH `manual_removal_apps` 第 1680-1686 行):
    // 预扫描已拒绝,不做任何 side effect,只报告。
    if !detail.manual_removal_reason.is_empty() {
        outcome.reason = "cannot be removed safely by Mole from this location".into();
        outcome.suggestion =
            "Move it to Trash in Finder; Mole left protected containers and app data untouched"
                .into();
        return outcome;
    }

    // 检查点 A(对齐 SH 第 1868-1876 行):pre-teardown 身份复查,
    // 在任何 side effect 之前拒绝预览后被替换的路径。
    if !selected_app_plan_matches(
        &detail.app_path,
        &detail.expected_app_identity,
        &detail.expected_info_identity,
    ) {
        outcome.reason = "selected app changed after preview".into();
        outcome.suggestion = "Select the app again and review the new removal plan".into();
        return outcome;
    }

    // 执行期 fingerprint 复查（对齐 SH 第 1883-1928 行）：
    // 预览与执行之间 app 集合可能变化（新装 / 复制 / 挂载），需重新 live 扫描并比对，
    // 不一致就 fail-closed，避免按过期的删除计划删掉幸存兄弟的共享数据。
    if is_valid_reverse_dns_bundle_id(&detail.original_bundle_id) {
        let (live_rc, current_fingerprint) =
            uninstall_live_bundle_has_other_install(&detail.original_bundle_id, &detail.app_path);
        match live_rc {
            0 | 1 => {
                if detail.live_sibling_fingerprint != current_fingerprint {
                    outcome.reason = "the app installation set changed after preview".into();
                    outcome.suggestion =
                        "Select the app again and review the new removal plan".into();
                }
            }
            i if i == MOLE_UNINSTALL_SCAN_PARTIAL
                && detail.sibling_guard == "guard_login"
                && detail.related_files.is_empty() =>
            {
                // 只删 bundle、无共享 teardown → 部分扫描的疑虑不影响，继续。
            }
            i if i >= 128 => {
                outcome.reason = "uninstall interrupted".into();
            }
            _ => {
                outcome.reason = "unable to verify other apps with the same bundle id".into();
                outcome.suggestion =
                    "Check mounted volumes and application folders, then try again".into();
            }
        }
    }

    if !outcome.reason.is_empty() {
        return outcome;
    }

    // 检查点 B(对齐 SH 第 1934-1944 行):teardown(bootout/unload/unregister/kill)前复查。
    if !selected_app_plan_matches(
        &detail.app_path,
        &detail.expected_app_identity,
        &detail.expected_info_identity,
    ) {
        outcome.reason = "selected app changed after preview".into();
        outcome.suggestion = "Select the app again and review the new removal plan".into();
        return outcome;
    }

    // Bootout Login Item Helpers before stopping other launch services.
    // sibling guard 时跳过：helper id 与幸存兄弟共享，bootout 会停掉幸存安装的 helper。
    if detail.sibling_guard == "none" {
        bootout_login_item_helpers(&detail.login_item_helpers);
    }

    let has_system_files = !detail.system_files.is_empty();
    stop_launch_services(&detail.bundle_id, has_system_files, &detail.app_path);
    unregister_app_bundle(&detail.app_path);

    // Login Item 按 display name 匹配；guard_login（名字碰撞）时删除会连累幸存兄弟，跳过。
    if detail.sibling_guard != "guard_login" {
        remove_login_item(&detail.app_name, &detail.bundle_id);
    }

    // New CLI: running app does NOT block uninstall.
    // macOS allows removing a running app bundle (the process keeps using mmap'd code).
    // Track it for a warning at the end.
    // sibling guard 时跳过：force_kill_app 按 bundle id / CFBundleExecutable 匹配，
    // 两者都可能属于幸存安装，kill 会误杀幸存者的进程。
    if detail.sibling_guard == "none" {
        if !force_kill_app(&detail.app_name, &detail.app_path) {
            outcome.running_at_uninstall = true;
        }
    }

    // 检查点 C(对齐 SH 第 2004-2014 行):删除/brew 判定前复查。
    if !selected_app_plan_matches(
        &detail.app_path,
        &detail.expected_app_identity,
        &detail.expected_info_identity,
    ) {
        outcome.reason = "selected app changed after preview".into();
        outcome.suggestion = "Select the app again and review the new removal plan".into();
        return outcome;
    }

    // Clear Data 模式：保留 app 本体，只清残留
    let data_only = std::env::var("MOLE_UNINSTALL_DATA_ONLY").unwrap_or_default() == "1";

    // 在 data_only 模式下，只计算残留文件的大小（不包含 app bundle）
    let app_bundle_size_kb = if data_only {
        get_path_size_kb(&detail.app_path)
    } else {
        0
    };

    // 主 bundle 删除
    if data_only {
        // 跳过 app bundle 删除，只清残留
        log::info!(
            "[uninstall.data_only] skipping app bundle deletion for {}",
            detail.app_path
        );
    } else if detail.is_brew_cask && !detail.cask_name.is_empty() {
        // sibling guard 时用 nozap：--zap 会删 bundle-id-keyed 的共享 config，幸存兄弟还在用。
        let zap_mode = if detail.sibling_guard == "none" {
            "zap"
        } else {
            "nozap"
        };
        if brew_uninstall_cask(&detail.cask_name, Some(&detail.app_path), zap_mode) {
            outcome.used_brew_successfully = true;
        } else {
            // 三态码决定兜底路径(对齐 SH 第 618-637 行)
            let cask_state = is_brew_cask_installed(&detail.cask_name);
            match cask_state {
                CaskInstallState::NotInstalled => {
                    // 检查点 D(对齐 SH 第 2046-2054 行):brew 卸载后 manual 兜底前复查。
                    if !selected_app_plan_matches(
                        &detail.app_path,
                        &detail.expected_app_identity,
                        &detail.expected_info_identity,
                    ) {
                        outcome.reason = "selected app changed after preview".into();
                        outcome.suggestion =
                            "Select the app again and review the new removal plan".into();
                        return outcome;
                    }
                    if mole_delete(
                        &detail.app_path,
                        detail.needs_sudo,
                        Some(&detail.expected_app_identity),
                    ) != MOLE_OK
                    {
                        outcome.reason = "brew cleanup incomplete, manual removal failed".into();
                    }
                }
                CaskInstallState::Installed => {
                    outcome.reason = "brew uninstall failed, package still installed".into();
                    outcome.suggestion = if zap_mode == "nozap" {
                        format!("Run brew uninstall --cask {}", detail.cask_name)
                    } else {
                        format!("Run brew uninstall --cask --zap {}", detail.cask_name)
                    };
                }
                CaskInstallState::Unknown => {
                    outcome.reason = "brew uninstall failed, package state unknown".into();
                    outcome.suggestion =
                        format!("Run brew uninstall --cask --zap {}", detail.cask_name);
                }
            }
        }
    } else if detail.needs_sudo {
        let p = Path::new(&detail.app_path);
        if p.is_symlink() {
            // SH 第 640-664 行:解析 symlink 目标,/System/* 等保护路径直接拒绝
            match resolve_symlink_target_abs(&detail.app_path) {
                Some(target) if protected_symlink_target(&target) => {
                    outcome.reason = "protected system symlink, cannot remove".into();
                }
                _ => {
                    if mole_delete(&detail.app_path, true, Some(&detail.expected_app_identity))
                        != MOLE_OK
                    {
                        outcome.reason = "failed to remove symlink".into();
                    }
                }
            }
        } else if is_uninstall_dry_run() {
            // SH 第 2118 行:dry-run 传 needs_sudo=false
            if mole_delete(&detail.app_path, false, Some(&detail.expected_app_identity)) != MOLE_OK
            {
                outcome.reason = "dry-run path validation failed".into();
            }
        } else {
            // 非 symlink + sudo:对齐 SH 走 mole_delete(内部做 mutable-ancestor 与
            // identity 检查),拿 i32 退出码做诊断。
            let rc = mole_delete(&detail.app_path, true, Some(&detail.expected_app_identity));
            if rc != MOLE_OK {
                let (reason, suggestion) = diagnose_removal_failure(rc, &detail.app_name);
                outcome.reason = reason;
                outcome.suggestion = suggestion;
            }
        }
    } else if mole_delete(&detail.app_path, false, Some(&detail.expected_app_identity)) != MOLE_OK {
        // SH 第 681-687 行:看是父目录不可写还是其它原因
        use std::ffi::CString;
        let parent_writable = Path::new(&detail.app_path)
            .parent()
            .and_then(|p| CString::new(p.to_string_lossy().as_bytes()).ok())
            .map(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 })
            .unwrap_or(false);
        outcome.reason = if !parent_writable {
            "parent directory not writable".into()
        } else {
            "remove failed, check permissions".into()
        };
    }

    if !outcome.reason.is_empty() {
        return outcome;
    }

    // 关联文件 + 诊断报告(用户域)
    remove_file_list(&detail.related_files, false);

    // leftover 收集:跳过容器 stub
    let mut leftovers: Vec<String> = Vec::new();
    for raw in detail.related_files.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let p = Path::new(line);
        if !(p.exists() || p.is_symlink()) {
            continue;
        }
        if is_container_stub(line) {
            continue;
        }
        leftovers.push(line.to_string());
    }
    let leftover_kb = du_leftover_total_kb(&leftovers);
    outcome.leftover_paths = leftovers;

    // 收集删除的路径（用于历史记录）
    // deleted_paths = related_files 中的所有路径 - leftover_paths（未成功删除的）
    let mut all_related: Vec<String> = detail
        .related_files
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .collect();

    // 移除 leftover_paths 中的路径（这些是未成功删除的）
    all_related.retain(|path| !outcome.leftover_paths.contains(path));
    outcome.deleted_paths = all_related;

    // 收集删除的路径（用于历史记录）
    // deleted_paths = related_files 中的所有路径 - leftover_paths（未成功删除的）
    let mut all_related: Vec<String> = detail
        .related_files
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .collect();

    // 移除 leftover_paths 中的路径（这些是未成功删除的）
    all_related.retain(|path| !outcome.leftover_paths.contains(path));
    outcome.deleted_paths = all_related;

    // 系统文件 + 诊断报告(系统域)
    if outcome.used_brew_successfully {
        remove_file_list(&detail.diag_system, true);
    } else {
        let mut sys_all = detail.system_files.clone();
        if !detail.diag_system.is_empty() {
            if !sys_all.is_empty() {
                sys_all.push('\n');
            }
            sys_all.push_str(&detail.diag_system);
        }
        remove_file_list(&sys_all, true);
    }

    // defaults / ByHost(SH 第 745-763 行)
    let bid = detail.bundle_id.clone();
    if !bid.is_empty() && bid != "unknown" {
        if is_uninstall_dry_run() {
            debug_log(&format!("[DRY RUN] Would clear defaults domain: {bid}"));
        } else if Command::new("defaults")
            .args(["read", &bid])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            let _ = Command::new("defaults").args(["delete", &bid]).output();
        }

        // ByHost
        let byhost_dir = format!("{}/Library/Preferences/ByHost", home_dir());
        if Path::new(&byhost_dir).is_dir() {
            // SH 第 756 行:必须是 `[A-Za-z0-9._-]+`
            let valid = !bid.is_empty()
                && bid
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
            if valid {
                if let Ok(rd) = std::fs::read_dir(&byhost_dir) {
                    let prefix = format!("{bid}.");
                    for e in rd.flatten() {
                        let name = e.file_name().to_string_lossy().to_string();
                        if !name.starts_with(&prefix) || !name.ends_with(".plist") {
                            continue;
                        }
                        let path = e.path().to_string_lossy().to_string();
                        // New CLI: ByHost plists are user-owned — use user-mode deletion.
                        let _ = mole_delete(&path, false, None);
                    }
                }
            } else {
                debug_log(&format!(
                    "Skipping ByHost cleanup, invalid bundle id: {bid}"
                ));
            }
        }
    }

    // 释放空间统计:扣除 leftover
    // 在 data_only 模式下，需要额外扣除 app bundle 的大小（因为 bundle 没有被删除）
    let mut total_kb_signed =
        detail.total_kb as i128 - leftover_kb as i128 - app_bundle_size_kb as i128;
    if total_kb_signed < 0 {
        total_kb_signed = 0;
    }
    outcome.freed_kb = total_kb_signed as u64;
    outcome.success = true;

    // 系统扩展检测(SH 第 798-803 行)
    if !bid.is_empty()
        && bid != "unknown"
        && bid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
        && Path::new("/Library/SystemExtensions").is_dir()
    {
        // 走 find,需要 -path "*${bundle_id}*"
        let pattern = format!("*{bid}*");
        let out = Command::new("find")
            .args([
                "/Library/SystemExtensions",
                "-maxdepth",
                "3",
                "-name",
                "*.systemextension",
                "-path",
                pattern.as_str(),
                "-print",
                "-quit",
            ])
            .output();
        if let Ok(o) = out {
            if !String::from_utf8_lossy(&o.stdout).trim().is_empty() {
                outcome.has_system_extension = true;
            }
        }
    }

    outcome
}

// ============================================================================
// Top-level batch entry
// ============================================================================

/// 整个批量卸载结果的可序列化汇总,前端可直接拿来渲染。
#[derive(Debug, Clone, Serialize)]
pub struct BatchUninstallSummary {
    pub success_count: u32,
    pub failed_count: u32,
    pub total_size_freed_kb: u64,
    pub size_display: String,
    pub outcomes: Vec<AppUninstallOutcome>,
    pub local_network_warning_apps: Vec<String>,
    pub system_extension_warning_apps: Vec<String>,
    /// Apps that were running at uninstall time but were successfully removed.
    pub running_at_uninstall_apps: Vec<String>,
    pub running_apps: Vec<String>,
    pub sudo_apps: Vec<String>,
    pub brew_cask_apps: Vec<String>,
    /// Apps blocked because they require an official uninstaller.
    pub blocked_apps: Vec<String>,
    /// Apps rejected at scan time (identity unavailable / privileged path below a
    /// mutable parent) — user must remove them manually in Finder.
    pub manual_removal_apps: Vec<String>,
    /// Apps with Background Items leftovers detected after uninstall.
    pub background_item_leftovers: Vec<String>,
    /// `success | warn | info`(对齐 SH `summary_status`)
    pub status: String,
    pub title: String,
}

/// 对齐 SH 第 302-993 行 `batch_uninstall_applications`。
///
/// GUI 端跳过 spinner/`read_key`/转义序列等 TUI 部分,其它流程严格翻译:
/// 1. 预扫描收集 app_details + running/sudo/brew 标记;
/// 2. 单点 sudo 会话(仅当真的需要 sudo 时);
/// 3. 启用 `MOLE_UNINSTALL_MODE`,逐个 app 跑卸载;
/// 4. 收尾:`remove_apps_from_dock` + `refresh_launch_services_after_uninstall` + `brew autoremove`;
/// 5. 还原 sudo 会话与 env。
///
/// 返回 [`BatchUninstallSummary`] 供前端渲染(SH 端是直接 `print_summary_block`)。
pub fn batch_uninstall_applications(
    selected_apps: &[String],
    app_handle: Option<&tauri::AppHandle>,
) -> BatchUninstallSummary {
    let mut summary = BatchUninstallSummary {
        success_count: 0,
        failed_count: 0,
        total_size_freed_kb: 0,
        size_display: String::new(),
        outcomes: Vec::new(),
        local_network_warning_apps: Vec::new(),
        system_extension_warning_apps: Vec::new(),
        running_at_uninstall_apps: Vec::new(),
        running_apps: Vec::new(),
        sudo_apps: Vec::new(),
        brew_cask_apps: Vec::new(),
        blocked_apps: Vec::new(),
        manual_removal_apps: Vec::new(),
        background_item_leftovers: Vec::new(),
        status: "info".to_string(),
        title: "Uninstall complete".to_string(),
    };
    if selected_apps.is_empty() {
        summary.title = "No applications were uninstalled.".into();
        return summary;
    }

    // 1. 预扫描
    let details = match collect_app_details(selected_apps) {
        Ok(d) => d,
        Err(e) => {
            summary.status = "warn".into();
            summary.title = "Uninstall incomplete".into();
            log_error(&e);
            return summary;
        }
    };
    if details.is_empty() {
        summary.title = "No applications were uninstalled.".into();
        return summary;
    }

    // ── 日志：批量卸载预扫描汇总 ──
    log::info!(
        "[uninstall.batch] selected_apps={} scanned_details={}",
        selected_apps.len(),
        details.len()
    );
    for d in &details {
        log::info!(
            "[uninstall.batch.detail] app={} bundle={} total_kb={} related_len={} sys_len={} diag_len={} \
             needs_sudo={} brew={}",
            d.app_name,
            d.bundle_id,
            d.total_kb,
            d.related_files
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count(),
            d.system_files
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count(),
            d.diag_system
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count(),
            d.needs_sudo,
            d.is_brew_cask,
        );
    }

    // 重新计算 running/sudo/brew lists(给前端展示用,与 SH 第 339-441 行对齐)
    for d in &details {
        let exec = read_bundle_executable(&d.app_path);
        if pgrep_exact(if exec.is_empty() { &d.app_name } else { &exec }) {
            summary.running_apps.push(d.app_name.clone());
        }
        if d.is_brew_cask {
            summary.brew_cask_apps.push(d.app_name.clone());
        }
        if d.needs_sudo {
            summary.sudo_apps.push(d.app_name.clone());
        }
        if d.is_official_uninstaller {
            summary
                .blocked_apps
                .push(format!("{}|{}", d.app_name, d.official_vendor));
        }
    }
    let total_estimated_kb: u64 = details.iter().map(|d| d.total_kb).sum();
    summary.size_display = bytes_to_human(total_estimated_kb.saturating_mul(1024));

    // 2. 提前授 sudo(SH 第 530-548 行)
    if !is_uninstall_dry_run()
        && (!summary.sudo_apps.is_empty() || !summary.brew_cask_apps.is_empty())
    {
        if !ensure_admin_session() {
            summary.status = "warn".into();
            summary.title = "Uninstall incomplete".into();
            return summary;
        }
    }

    // 3. 启用 uninstall 模式(让 app_protection 知道当前是用户主动卸载)
    let prev_mode = std::env::var("MOLE_UNINSTALL_MODE").ok();
    unsafe {
        std::env::set_var("MOLE_UNINSTALL_MODE", "1");
    }

    // 4. 逐个跑
    let mut total_freed_kb: u64 = 0;
    let mut brew_apps_removed: u32 = 0;
    let mut success_paths: Vec<String> = Vec::new();
    for (index, d) in details.iter().enumerate() {
        // 发送进度事件：开始处理当前 app
        if let Some(handle) = app_handle {
            crate::events::emit_uninstall_progress(
                handle,
                &crate::events::UninstallProgressPayload {
                    app_path: d.app_path.clone(),
                    app_name: d.app_name.clone(),
                    current_index: index + 1,
                    total_count: details.len(),
                    current_action: "正在清理残留文件...".to_string(),
                },
            );
        }
        // manual removal(对齐 SH 第 1680-1686 行):预扫描拒绝的 app 记录报告列表,
        // uninstall_one_app 内部会把它们转成失败 outcome(不做任何 side effect)。
        if !d.manual_removal_reason.is_empty() {
            summary.manual_removal_apps.push(d.app_name.clone());
        }
        // New CLI: skip apps that require an official uninstaller.
        if d.is_official_uninstaller {
            summary.outcomes.push(AppUninstallOutcome {
                app_name: d.app_name.clone(),
                app_path: d.app_path.clone(),
                success: false,
                freed_kb: 0,
                used_brew_successfully: false,
                leftover_paths: Vec::new(),
                deleted_paths: Vec::new(),
                reason: format!("requires the official {} uninstaller", d.official_vendor),
                suggestion: String::new(),
                has_local_network_usage: false,
                has_system_extension: false,
                running_at_uninstall: false,
            });
            summary.failed_count += 1;
            continue;
        }
        let outcome = uninstall_one_app(d);
        if outcome.success {
            summary.success_count += 1;
            total_freed_kb = total_freed_kb.saturating_add(outcome.freed_kb);
            success_paths.push(outcome.app_path.clone());
            if outcome.used_brew_successfully {
                brew_apps_removed += 1;
            }
            if outcome.has_local_network_usage {
                summary
                    .local_network_warning_apps
                    .push(outcome.app_name.clone());
            }
            if outcome.has_system_extension {
                summary
                    .system_extension_warning_apps
                    .push(outcome.app_name.clone());
            }
            if outcome.running_at_uninstall {
                summary
                    .running_at_uninstall_apps
                    .push(outcome.app_name.clone());
            }
        } else {
            summary.failed_count += 1;
        }

        // 发送完成事件：当前 app 处理完成
        if let Some(handle) = app_handle {
            crate::events::emit_uninstall_complete(
                handle,
                &crate::events::UninstallCompletePayload {
                    app_path: outcome.app_path.clone(),
                    app_name: outcome.app_name.clone(),
                    success: outcome.success,
                    freed_bytes: outcome.freed_kb.saturating_mul(1024),
                    reason: if outcome.reason.is_empty() {
                        None
                    } else {
                        Some(outcome.reason.clone())
                    },
                    suggestion: if outcome.suggestion.is_empty() {
                        None
                    } else {
                        Some(outcome.suggestion.clone())
                    },
                },
            );
        }

        summary.outcomes.push(outcome);
    }
    summary.total_size_freed_kb = total_freed_kb;

    // New CLI: Background Items 残留检测.
    if !is_uninstall_dry_run() {
        summary.background_item_leftovers = check_btm_leftovers(&success_paths, &details);
    }

    // 5. 状态/标题(SH 第 949-955 行)
    if summary.failed_count > 0 {
        summary.status = "warn".into();
        summary.title = "Uninstall incomplete".into();
    } else if summary.success_count > 0 {
        summary.status = "success".into();
        summary.title = "Uninstall complete".into();
    }
    if is_uninstall_dry_run() {
        summary.title = "Uninstall dry run complete".into();
    }

    // 6. brew autoremove + remove_apps_from_dock + refresh_launch_services
    //    SH 端把它们 `&` 到后台,这里 GUI 后端是同步执行,前端无感(几秒级)
    if brew_apps_removed > 0 && !is_uninstall_dry_run() {
        brew_autoremove_silent();
    }
    if summary.success_count > 0 && !success_paths.is_empty() {
        if !is_uninstall_dry_run() {
            let _ = remove_apps_from_dock(&success_paths);
            let _ = refresh_launch_services_after_uninstall();
        }
    }

    // 7. 收尾（不撤销授权，让它在整个应用生命周期内保持有效）
    unsafe {
        match prev_mode {
            Some(v) => std::env::set_var("MOLE_UNINSTALL_MODE", v),
            None => std::env::remove_var("MOLE_UNINSTALL_MODE"),
        }
    }

    // 8. 记录卸载历史（新增）
    if !is_uninstall_dry_run() && summary.success_count > 0 {
        let data_only = std::env::var("MOLE_UNINSTALL_DATA_ONLY").unwrap_or_default() == "1";

        for outcome in &summary.outcomes {
            if !outcome.success {
                continue;
            }

            // 统计 trashed_count 和 sudo_removed_count
            // 简化实现：假设所有 deleted_paths 都进了废纸篓
            // TODO: 后续可以从 remove_file_list 返回统计信息
            let trashed_count = outcome.deleted_paths.len();
            let sudo_removed_count = 0;

            let record = crate::uninstall::history::UninstallHistoryRecord {
                id: chrono::Utc::now()
                    .timestamp_nanos_opt()
                    .unwrap_or(0)
                    .to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                app_name: outcome.app_name.clone(),
                app_path: outcome.app_path.clone(),
                data_only,
                deleted_paths: outcome.deleted_paths.clone(),
                trashed_count,
                sudo_removed_count,
                total_size_bytes: outcome.freed_kb.saturating_mul(1024),
                file_count: outcome.deleted_paths.len(),
            };

            if let Err(e) = crate::uninstall::history::add_record(record) {
                log::error!("[uninstall.history] 记录历史失败: {}", e);
            }
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_data_matches_common_paths() {
        assert!(has_sensitive_data("/Users/me/.ssh/id_rsa"));
        assert!(has_sensitive_data("/Users/me/.aws/credentials"));
        assert!(has_sensitive_data("/Users/me/Documents/private.txt"));
        assert!(has_sensitive_data(
            "/Users/me/Library/Cookies/Cookies.binarycookies"
        ));
        assert!(has_sensitive_data(
            "/Users/me/Library/Application Support/Foo/.docker/config.json"
        ));
        assert!(has_sensitive_data(
            "/Users/me/Library/Preferences/com.apple.Safari.plist"
        ));
    }

    #[test]
    fn sensitive_data_ignores_neutral_paths() {
        assert!(!has_sensitive_data("/Applications/Foo.app"));
        assert!(!has_sensitive_data(
            "/Users/me/Library/Application Support/Foo/Cache.db"
        ));
        assert!(!has_sensitive_data(""));
    }

    #[test]
    fn reverse_dns_validator() {
        assert!(is_valid_reverse_dns_bundle_id("com.example.app"));
        assert!(is_valid_reverse_dns_bundle_id("com.foo-bar.baz"));
        // 无 dot
        assert!(!is_valid_reverse_dns_bundle_id("foo"));
        // 以 - 起头
        assert!(!is_valid_reverse_dns_bundle_id("-com.foo"));
        // 空段
        assert!(!is_valid_reverse_dns_bundle_id("com..foo"));
        // 段以 - 起头
        assert!(!is_valid_reverse_dns_bundle_id("com.-foo"));
    }

    #[test]
    fn protected_symlink_target_recognises_system_paths() {
        assert!(protected_symlink_target("/System/Library/Foo"));
        assert!(protected_symlink_target("/usr/bin/git"));
        assert!(protected_symlink_target("/usr/lib/dyld"));
        assert!(protected_symlink_target("/private/etc/passwd"));
        assert!(!protected_symlink_target("/Applications/Foo.app"));
    }

    #[test]
    fn decode_file_list_rejects_relative_paths() {
        // 编码 "relative/path\n/abs"
        let encoded = encode_file_list("relative/path\n/abs");
        let out = decode_file_list(&encoded, "test");
        // SH 端直接整段拒,Rust 也是
        assert!(out.is_empty());
    }

    #[test]
    fn decode_file_list_accepts_abs_only() {
        let encoded = encode_file_list("/Users/me/.ssh\n/Users/me/Documents/foo");
        let out = decode_file_list(&encoded, "test");
        assert_eq!(out, "/Users/me/.ssh\n/Users/me/Documents/foo");
    }

    #[test]
    fn plist_app_path_reference_detection_matches_grep_f() {
        // grep -qF 的字节级等价:XML plist 内嵌 app 路径
        let plist = br#"<?xml version="1.0"?><dict><key>ProgramArguments</key><array><string>/Applications/Foo Bar.app/Contents/MacOS/helper</string></array></dict>"#;
        assert!(plist_references_app_path(
            plist,
            "/Applications/Foo Bar.app"
        ));
        assert!(!plist_references_app_path(plist, "/Applications/Other.app"));
        assert!(!plist_references_app_path(plist, ""));
        // 二进制 plist 同样按字节命中
        let mut binary = vec![0u8; 16];
        binary.extend_from_slice(b"/Applications/Foo.app");
        assert!(plist_references_app_path(&binary, "/Applications/Foo.app"));
        // 前缀相同但不等长不命中(非子串即全串匹配)
        assert!(!plist_references_app_path(
            b"/Applications/Foo",
            "/Applications/Foo.app"
        ));
    }

    #[test]
    fn selected_app_plan_matches_binds_preview_identity() {
        let dir = std::env::temp_dir().join(format!(
            "mole-id-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let app = format!("{}/Foo.app", dir.display());
        std::fs::create_dir_all(format!("{}/Contents", app)).unwrap();
        std::fs::write(format!("{}/Contents/Info.plist", app), b"x").unwrap();

        let app_id = stat_path_identity(&app).unwrap();
        let info_id = selected_app_info_identity(&app).unwrap();
        assert!(selected_app_plan_matches(&app, &app_id, &info_id));

        // 替换 bundle → inode 变化,必须拒绝(对齐 SH:路径相同≠用户确认过这个对象)
        std::fs::remove_dir_all(&app).unwrap();
        std::fs::create_dir_all(format!("{}/Contents", app)).unwrap();
        std::fs::write(format!("{}/Contents/Info.plist", app), b"y").unwrap();
        assert!(!selected_app_plan_matches(&app, &app_id, &info_id));

        // 空 expected → 拒绝(SH 第 1301 行)
        assert!(!selected_app_plan_matches(&app, "", &info_id));
        assert!(!selected_app_plan_matches(&app, &app_id, ""));

        // 路径不存在 → 拒绝
        assert!(!selected_app_plan_matches(
            &format!("{}/Gone.app", dir.display()),
            &app_id,
            &info_id
        ));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stat_identity_format_matches_sh_stat_permissions() {
        // SH `stat -f%d:%i:%m`:三段十进制,末段是八进制权限位字符串(如 755)。
        // Rust {:o} 保证 493(0o755) 输出为 "755" 而不是 "493"。
        let id = stat_path_identity("/").expect("stat / must succeed");
        let parts: Vec<&str> = id.split(':').collect();
        assert_eq!(parts.len(), 3);
        assert!(
            parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        );
        // 权限位段只允许八进制数字
        assert!(parts[2].chars().all(|c| ('0'..='7').contains(&c)));
    }

    #[test]
    fn btm_leftovers_quiet_in_test_mode_and_skips_unmatched_paths() {
        // SH 第 120-122 行:TEST_MODE 时报告 "not loaded",summary 不产生后台项警告;
        // SH 第 168-173 行:app_path 不在 success_paths 里时跳过(不探测 launchd)。
        let d = manual_removal_detail("Foo", "/Applications/Foo.app", "x");
        // 空输入提前返回
        assert!(check_btm_leftovers(&[], &[]).is_empty());
        // success_paths 不匹配 → 跳过,不触碰 launchctl
        assert!(check_btm_leftovers(&["/Applications/Bar.app".into()], &[d.clone()]).is_empty());
        // test 模式:即使命中也不探测 launchd
        std::env::set_var("MOLE_TEST_MODE", "1");
        assert!(check_btm_leftovers(&[d.app_path.clone()], &[d]).is_empty());
        std::env::remove_var("MOLE_TEST_MODE");
    }
}
