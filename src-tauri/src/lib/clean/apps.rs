//! 应用数据清理 — 严格对齐 lib/clean/apps.sh
//!
//! 关键约束(违反任意一项都可能误删用户数据,务必严格执行):
//!   - `is_bundle_orphaned` 必须依次过 should_protect_data → 敏感关键词 → installed → 30 天 → mdfind
//!   - `clean_orphaned_app_data` 处理 8 类资源,**绝不**自动删 LaunchAgents/LaunchDaemons/Containers
//!   - `clean_orphaned_launch_agents` 在 SH 中是 no-op,Rust 必须保持 no-op
//!   - 所有删除入口必须走 safe_clean / safe_sudo_remove(内部已包白名单 + 保护表)
//!
//! 与 SH 的差异(刻意保留):
//!   - mdfind / launchctl 等慢调用统一加 5s 超时
//!   - mdfind 缓存改为内存 HashMap(GUI 后端不需要落盘)

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use crate::core::app_protection::{
    bundle_matches_pattern, is_path_whitelisted_from_global, should_protect_data,
    should_protect_path,
};
use crate::core::base::{
    MOLE_MAX_DS_STORE_FILES, MOLE_MAX_ORPHAN_ITERATIONS, MOLE_ORPHAN_AGE_DAYS, bytes_to_human,
    get_epoch_seconds, get_file_mtime, get_file_size, get_path_size_kb, home_dir, note_activity,
};
use crate::core::bundle_resolver::bundle_has_installed_app;
use crate::core::dry_run_registry::dry_run_register_cleanup_target;
use crate::core::file_ops::{MOLE_OK, safe_clean, safe_remove, safe_sudo_remove};
use crate::core::log::{debug_log, log_info, log_operation, log_warning};
use crate::core::timeout::run_with_timeout_capture;

pub const ORPHAN_AGE_THRESHOLD: u32 = MOLE_ORPHAN_AGE_DAYS;
pub const CLAUDE_VM_ORPHAN_AGE_THRESHOLD: u32 = 7;

/// 敏感数据关键字。命中(case-insensitive 通配符匹配)即视为受保护,绝不当作 orphan。
/// 严格对齐 SH `ORPHAN_NEVER_DELETE_PATTERNS`(第 162-172 行)。
const ORPHAN_NEVER_DELETE_PATTERNS: &[&str] = &[
    "*1password*",
    "*1Password*",
    "*keychain*",
    "*Keychain*",
    "*bitwarden*",
    "*Bitwarden*",
    "*lastpass*",
    "*LastPass*",
    "*keepass*",
    "*KeePass*",
    "*dashlane*",
    "*Dashlane*",
    "*enpass*",
    "*Enpass*",
    "*ssh*",
    "*gpg*",
    "*gnupg*",
    "com.apple.keychain*",
];

pub fn clean_ds_store_tree(target: &str, label: &str) -> (u64, u64) {
    let home = home_dir();
    let full_target = if target == "~" {
        home.clone()
    } else if let Some(rest) = target.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else {
        target.to_string()
    };

    if !Path::new(&full_target).is_dir() {
        return (0, 0);
    }

    let mut args: Vec<String> = vec!["find".to_string(), full_target.clone()];
    if full_target == home {
        args.push("-maxdepth".to_string());
        args.push("5".to_string());
    }
    // -path "X" -prune -o
    let exclude_paths = [
        "*/Library/Application Support/MobileSync",
        "*/Library/Developer",
        "*/.Trash",
        "*/node_modules",
        "*/.git",
        "*/Library/Caches",
    ];
    for ep in &exclude_paths {
        args.push("-path".to_string());
        args.push(ep.to_string());
        args.push("-prune".to_string());
        args.push("-o".to_string());
    }
    args.extend(
        ["-type", "f", "-name", ".DS_Store", "-print0"]
            .iter()
            .map(|s| s.to_string()),
    );

    let str_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let stdout = match Command::new(str_args[0]).args(&str_args[1..]).output() {
        Ok(o) => o.stdout,
        Err(_) => return (0, 0),
    };

    let dry_run = std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1"
        || std::env::var("DRY_RUN").unwrap_or_default() == "true";

    let mut file_count: u64 = 0;
    let mut total_bytes: u64 = 0;
    for entry in stdout.split(|&b| b == 0) {
        if entry.is_empty() {
            continue;
        }
        let path = String::from_utf8_lossy(entry).to_string();
        let size = get_file_size(&path);
        total_bytes = total_bytes.saturating_add(size);
        file_count = file_count.saturating_add(1);
        if !dry_run {
            // 已经过 find 的 prune,内部 should_protect_path 会再做一道把关
            let _ = safe_remove(&path, true);
        } else {
            dry_run_register_cleanup_target(&path);
        }
        if file_count >= MOLE_MAX_DS_STORE_FILES as u64 {
            break;
        }
    }
    if file_count > 0 {
        log_info(&format!(
            "{label}, {file_count} files, {}",
            bytes_to_human(total_bytes)
        ));
        note_activity();
    }
    let total_kb = (total_bytes + 1023) / 1024;
    (total_kb, file_count)
}

// =============================================================================
// scan_installed_apps
// =============================================================================

/// 5 分钟缓存条目
struct InstalledAppsCache {
    expires_at: u64,
    bundles: Vec<String>,
}

static INSTALLED_APPS_CACHE: OnceLock<Mutex<Option<InstalledAppsCache>>> = OnceLock::new();

/// 收集已安装 app 的 bundle id + 正在运行的 bundle id + 已注册 LaunchAgents,
/// 写入 `out_file`(对齐 SH 第 67-159 行)。
///
/// 重要差异(已对齐):
///   - 5 分钟内存缓存(SH 用文件,GUI 用 Mutex<Option> 即可)
///   - 并行扫描多个 app 目录
///   - osascript 系统进程列表 + lsappinfo fallback
///   - LaunchAgents 文件名作为额外 bundle 输入
pub fn scan_installed_apps(out_file: &str) {
    let cell = INSTALLED_APPS_CACHE.get_or_init(|| Mutex::new(None));
    let now = get_epoch_seconds();

    if let Ok(g) = cell.lock() {
        if let Some(entry) = g.as_ref() {
            if now < entry.expires_at {
                let _ = std::fs::write(out_file, entry.bundles.join("\n"));
                debug_log("Using cached installed_apps list");
                return;
            }
        }
    }

    let bundles = scan_installed_apps_uncached();
    let _ = std::fs::write(out_file, bundles.join("\n"));

    if let Ok(mut g) = cell.lock() {
        *g = Some(InstalledAppsCache {
            expires_at: now + 300,
            bundles,
        });
    }
}

fn scan_installed_apps_uncached() -> Vec<String> {
    let home = home_dir();
    let app_dirs = [
        "/Applications".to_string(),
        "/System/Applications".to_string(),
        format!("{home}/Applications"),
        "/opt/homebrew/Caskroom".to_string(),
        "/usr/local/Caskroom".to_string(),
        format!("{home}/Library/Application Support/Setapp/Applications"),
    ];

    let mut bundles: Vec<String> = Vec::new();

    for app_dir in &app_dirs {
        if !Path::new(app_dir).is_dir() {
            continue;
        }
        // 用 find -name '*.app' -maxdepth 3
        let out = Command::new("find")
            .args([app_dir, "-maxdepth", "3", "-name", "*.app", "-type", "d"])
            .output();
        let Ok(out) = out else { continue };
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let app_path = line.trim();
            if app_path.is_empty() {
                continue;
            }
            if let Some(bid) = read_bundle_id(app_path) {
                bundles.push(bid);
            }
        }
    }

    // 正在运行的 bundle id(SH 第 127-137 行)
    let test_mode = std::env::var("MOLE_TEST_MODE").unwrap_or_default() == "1"
        || std::env::var("MOLE_TEST_NO_AUTH").unwrap_or_default() == "1";
    if !test_mode {
        if let Some(out) = run_with_timeout_capture(
            5.0,
            "osascript",
            &[
                "-e",
                "tell application \"System Events\" to get bundle identifier of every application process",
            ],
        ) {
            for chunk in out.split(',') {
                let id = chunk.trim();
                if !id.is_empty() && id != "missing value" {
                    bundles.push(id.to_string());
                }
            }
        }
    }
    if let Some(out) = run_with_timeout_capture(3.0, "lsappinfo", &["list"]) {
        for line in out.lines() {
            // 形如 "CFBundleIdentifier"="com.example.app"
            if let Some(start) = line.find("\"CFBundleIdentifier\"=\"") {
                let rest = &line[start + "\"CFBundleIdentifier\"=\"".len()..];
                if let Some(end) = rest.find('"') {
                    bundles.push(rest[..end].to_string());
                }
            }
        }
    }

    // LaunchAgents 文件名(去掉 .plist 当 bundle id 用)。SH 第 140-143 行。
    if let Some(out) = run_with_timeout_capture(
        5.0,
        "find",
        &[
            &format!("{home}/Library/LaunchAgents"),
            "/Library/LaunchAgents",
            "-name",
            "*.plist",
            "-type",
            "f",
        ],
    ) {
        for line in out.lines() {
            let p = Path::new(line.trim());
            if let Some(name) = p.file_stem().and_then(|s| s.to_str()) {
                bundles.push(name.to_string());
            }
        }
    }

    bundles.sort();
    bundles.dedup();
    debug_log(&format!("Scanned {} unique applications", bundles.len()));
    bundles
}

fn read_bundle_id(app_path: &str) -> Option<String> {
    let plist = format!("{app_path}/Contents/Info.plist");
    if !Path::new(&plist).is_file() {
        return None;
    }
    Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleIdentifier", &plist])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty() && s != "missing value")
}

// =============================================================================
// is_bundle_orphaned
// =============================================================================

/// mdfind 内存缓存:bundle_id → exists?(对齐 SH 第 175 / 224-247 行的文件级缓存)
static MDFIND_CACHE: OnceLock<Mutex<std::collections::HashMap<String, bool>>> = OnceLock::new();

fn mdfind_bundle_exists(bundle_id: &str) -> bool {
    let cell = MDFIND_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Ok(g) = cell.lock() {
        if let Some(v) = g.get(bundle_id) {
            return *v;
        }
    }
    let exists = bundle_has_installed_app(bundle_id);
    if let Ok(mut g) = cell.lock() {
        g.insert(bundle_id.to_string(), exists);
    }
    exists
}

/// 对齐 SH `is_bundle_orphaned()` 第 178-253 行。
/// 任意一步认定为非 orphan 就立刻返回 false,绝对**不能**省略任何一步。
pub fn is_bundle_orphaned(
    bundle_id: &str,
    directory_path: &str,
    installed_bundles: &HashSet<String>,
) -> bool {
    if bundle_id.is_empty() {
        return false;
    }

    // 1. 保护表:数据敏感(密码管理器/IDE 等)直接放过
    if should_protect_data(bundle_id) {
        return false;
    }

    // 2. 敏感关键词(通配符 case-insensitive)
    let bundle_lower = bundle_id.to_ascii_lowercase();
    for pat in ORPHAN_NEVER_DELETE_PATTERNS {
        let pat_lower = pat.to_ascii_lowercase();
        if bundle_matches_pattern(&bundle_lower, &pat_lower) {
            return false;
        }
    }

    // 3. 已安装列表
    if installed_bundles.contains(bundle_id) {
        return false;
    }

    // 4. 硬编码系统组件
    if matches!(
        bundle_id,
        "loginwindow"
            | "dock"
            | "systempreferences"
            | "systemsettings"
            | "settings"
            | "controlcenter"
            | "finder"
            | "safari"
    ) {
        return false;
    }

    // 5. 30 天内修改过就不算 orphan
    if Path::new(directory_path).exists() || Path::new(directory_path).is_symlink() {
        let mtime = get_file_mtime(directory_path);
        let now = get_epoch_seconds();
        let days = now.saturating_sub(mtime) / 86400;
        if (days as u32) < ORPHAN_AGE_THRESHOLD {
            return false;
        }
    }

    // 6. mdfind 慢路径(覆盖非标准位置安装的 app)
    if bundle_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
        && bundle_id.len() >= 5
        && mdfind_bundle_exists(bundle_id)
    {
        return false;
    }

    true
}

/// 对齐 SH `is_claude_vm_bundle_orphaned()` 第 255-302 行。
pub fn is_claude_vm_bundle_orphaned(
    vm_bundle_path: &str,
    installed_bundles: &HashSet<String>,
) -> bool {
    let claude_bundle_id = "com.anthropic.claudefordesktop";
    if !Path::new(vm_bundle_path).is_dir() {
        return false;
    }
    // Claude 进程在跑就不算 orphan
    let running = Command::new("pgrep")
        .args(["-x", "Claude"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if running {
        return false;
    }
    if installed_bundles.contains(claude_bundle_id) {
        return false;
    }
    let mtime = get_file_mtime(vm_bundle_path);
    let now = get_epoch_seconds();
    let days = now.saturating_sub(mtime) / 86400;
    if (days as u32) < CLAUDE_VM_ORPHAN_AGE_THRESHOLD {
        return false;
    }
    if mdfind_bundle_exists(claude_bundle_id) {
        return false;
    }
    true
}

// =============================================================================
// clean_orphaned_app_data
// =============================================================================

struct ResourceType {
    base_path: String,
    label: &'static str,
    /// 子项 glob 模式(相对于 base_path)
    patterns: &'static [&'static str],
}

pub fn clean_orphaned_app_data() -> (u64, u64) {
    let home = home_dir();
    if !Path::new(&format!("{home}/Library/Caches")).exists() {
        log_warning("Skipped: No permission to access Library folders");
        return (0, 0);
    }

    // 1. 收集 installed bundles 到 HashSet(SH 用文件 + grep,我们用内存)
    let cache_file = format!("{home}/.cache/mole/installed_bundles");
    if let Some(p) = Path::new(&cache_file).parent() {
        let _ = std::fs::create_dir_all(p);
    }
    scan_installed_apps(&cache_file);
    let content = std::fs::read_to_string(&cache_file).unwrap_or_default();
    let installed_bundles: HashSet<String> = content
        .lines()
        .filter(|l| !l.is_empty())
        .map(|s| s.to_string())
        .collect();
    log_info(&format!(
        "Found {} active/installed apps",
        installed_bundles.len()
    ));

    let mut orphaned_count: u64 = 0;
    let mut total_kb: u64 = 0;

    // 2. Claude VM bundles
    let claude_dir = format!("{home}/Library/Application Support/Claude");
    if Path::new(&claude_dir).is_dir() {
        if let Some(out) = run_with_timeout_capture(
            10.0,
            "find",
            &[
                &claude_dir,
                "-maxdepth",
                "3",
                "-name",
                "*.bundle",
                "-type",
                "d",
            ],
        ) {
            for line in out.lines() {
                let bundle_path = line.trim();
                if bundle_path.is_empty() {
                    continue;
                }
                if !is_claude_vm_bundle_orphaned(bundle_path, &installed_bundles) {
                    continue;
                }
                if is_path_whitelisted_from_global(bundle_path) {
                    debug_log(&format!("Skipping whitelisted orphan: {bundle_path}"));
                    continue;
                }
                let size_kb = get_path_size_kb(bundle_path);
                if size_kb > 0 {
                    let (kb, count) = safe_clean(&[bundle_path], "Orphaned Claude workspace VM");
                    if count > 0 {
                        orphaned_count += count;
                        total_kb = total_kb.saturating_add(kb);
                    }
                }
            }
        }
    }

    // 3. 3 类资源 (严格对齐 SH 第 346-355 行)
    // **绝对不能**加 LaunchAgents/LaunchDaemons/Containers/Application Scripts/Group Containers
    let resource_types: Vec<ResourceType> = vec![
        ResourceType {
            base_path: format!("{home}/Library/Caches"),
            label: "Caches",
            patterns: &["com.*", "org.*", "net.*", "io.*"],
        },
        ResourceType {
            base_path: format!("{home}/Library/Logs"),
            label: "Logs",
            patterns: &["com.*", "org.*", "net.*", "io.*"],
        },
        ResourceType {
            base_path: format!("{home}/Library/Saved Application State"),
            label: "States",
            patterns: &["*.savedState"],
        },
    ];

    for rt in &resource_types {
        if !Path::new(&rt.base_path).is_dir() {
            continue;
        }
        let mut iter_count: u64 = 0;
        for pat in rt.patterns {
            let glob_pattern = format!("{}/{pat}", rt.base_path);
            for match_path in crate::core::file_ops::expand_glob_paths(&glob_pattern) {
                iter_count += 1;
                if iter_count > MOLE_MAX_ORPHAN_ITERATIONS as u64 {
                    break;
                }
                let basename = Path::new(&match_path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                let bundle_id = basename
                    .trim_end_matches(".savedState")
                    .trim_end_matches(".binarycookies")
                    .trim_end_matches(".plist")
                    .to_string();
                if !is_bundle_orphaned(&bundle_id, &match_path, &installed_bundles) {
                    continue;
                }
                if is_path_whitelisted_from_global(&match_path) {
                    debug_log(&format!("Skipping whitelisted orphan: {match_path}"));
                    continue;
                }
                let size_kb = get_path_size_kb(&match_path);
                if size_kb == 0 {
                    continue;
                }
                let label = format!("Orphaned {}: {bundle_id}", rt.label);
                let (kb, count) = safe_clean(&[&match_path], &label);
                if count > 0 {
                    orphaned_count += count;
                    total_kb = total_kb.saturating_add(kb);
                }
            }
        }
    }

    if orphaned_count > 0 {
        log_info(&format!(
            "Cleaned {orphaned_count} items, about {}",
            bytes_to_human(total_kb.saturating_mul(1024))
        ));
        note_activity();
    }

    let _ = std::fs::remove_file(&cache_file);
    (total_kb, orphaned_count)
}

// =============================================================================
// clean_orphaned_system_services
// =============================================================================
pub fn clean_orphaned_system_services() -> (u64, u64) {
    if !crate::core::sudo::is_admin_authorized() {
        return (0, 0);
    }

    // 保护模式列表：bundle_id 匹配这些模式的，如果对应 app 还存在，则跳过
    // 对齐 SH known_protect_patterns（apps.sh L438-462）
    let known_protect_patterns: &[(&str, &str)] = &[
        ("com.sogou.*", "/Library/Input Methods/SogouInput.app"),
        ("com.west2online.ClashX.*", "/Applications/ClashX.app"),
        ("com.clashmac.*", "/Applications/ClashMac.app"),
        (
            "com.nektony.AC*",
            "/Applications/App Cleaner & Uninstaller.app",
        ),
        ("cn.i4tools.*", "/Applications/i4Tools.app"),
        ("com.macpaw.CleanMyMac*", "/Applications/CleanMyMac X.app"),
        ("org.wireshark.ChmodBPF", "/Applications/Wireshark.app"),
        ("us.zoom.*", "/Applications/zoom.us.app"),
        ("it.remote.cli", "/Applications/Remote.It.app"),
        ("com.docker.*", "/Applications/Docker.app"),
        ("netbird", "/usr/local/bin/netbird"),
        ("homebrew.mxcl.*", ""),
    ];

    let mut orphaned: Vec<String> = Vec::new();

    let plist_binary_path = |plist: &str| -> Option<String> {
        let out = Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", "Print :ProgramArguments:0", plist])
            .output()
            .ok()?;
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
        let out = Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", "Print :Program", plist])
            .output()
            .ok()?;
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
        None
    };

    let is_package_managed_binary = |binary: &str| -> bool {
        binary.starts_with("/usr/local/bin/")
            || binary.starts_with("/usr/local/sbin/")
            || binary.starts_with("/opt/homebrew/bin/")
            || binary.starts_with("/opt/homebrew/sbin/")
            || binary.starts_with("/opt/homebrew/opt/")
            || binary.starts_with("/usr/bin/")
            || binary.starts_with("/usr/sbin/")
            || binary.starts_with("/bin/")
            || binary.starts_with("/sbin/")
            || binary.starts_with("/usr/libexec/")
    };

    let plist_is_orphaned = |plist: &str, bundle_id: &str| -> bool {
        let Some(binary) = plist_binary_path(plist) else {
            return false;
        };
        if Path::new(&binary).exists() {
            return false;
        }
        if is_package_managed_binary(&binary) {
            return false;
        }
        for &(file_pattern, app_path) in known_protect_patterns {
            let match_id = bundle_matches_pattern(bundle_id, file_pattern);
            let match_name = bundle_matches_pattern(&format!("{}.plist", bundle_id), file_pattern);
            if match_id || match_name {
                if app_path.is_empty() {
                    return false;
                }
                if Path::new(app_path).is_dir() {
                    return false;
                }
                if system_service_app_exists(bundle_id, app_path) {
                    return false;
                }
                break;
            }
        }
        true
    };

    let scan_dir = |dir: &str, only_plist: bool| -> Vec<String> {
        let mut out = Vec::new();
        if !Path::new(dir).is_dir() {
            return out;
        }
        let find_args: Vec<&str> = if only_plist {
            vec![
                "/usr/bin/find",
                dir,
                "-maxdepth",
                "1",
                "-name",
                "*.plist",
                "-print0",
            ]
        } else {
            vec![
                "/usr/bin/find",
                dir,
                "-maxdepth",
                "1",
                "-type",
                "f",
                "-print0",
            ]
        };
        let o = crate::core::sudo::sudo_output(&find_args);
        for entry in o.stdout.split(|&b| b == 0) {
            if entry.is_empty() {
                continue;
            }
            out.push(String::from_utf8_lossy(entry).to_string());
        }
        out
    };

    // LaunchDaemons + LaunchAgents: 通用检测
    for dir in ["/Library/LaunchDaemons", "/Library/LaunchAgents"] {
        for plist in scan_dir(dir, true) {
            let filename = Path::new(&plist)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if filename.starts_with("com.apple.") {
                continue;
            }
            let bundle_id = filename.trim_end_matches(".plist").to_string();
            if plist_is_orphaned(&plist, &bundle_id) {
                orphaned.push(plist);
            }
        }
    }

    // PrivilegedHelperTools: bundle 级检测（对齐 SH 第 692-702 行）
    for helper in scan_dir("/Library/PrivilegedHelperTools", false) {
        let filename = Path::new(&helper)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let skip_exts = [
            ".json",
            ".cfg",
            ".conf",
            ".me2me_enabled",
            ".log",
            ".dat",
            ".db",
            ".xml",
            ".yml",
            ".yaml",
            ".ini",
            ".txt",
            ".pid",
            ".sock",
            ".lock",
        ];
        if skip_exts.iter().any(|e| filename.ends_with(e)) {
            continue;
        }
        let bundle_id = filename.trim_end_matches(".plist").to_string();
        if bundle_id.starts_with("com.apple.") {
            continue;
        }

        let is_protected = known_protect_patterns
            .iter()
            .any(|&(file_pattern, app_path)| {
                let match_name = bundle_matches_pattern(&filename, file_pattern);
                let match_id = bundle_matches_pattern(&bundle_id, file_pattern);
                if match_name || match_id {
                    if app_path.is_empty() {
                        return true;
                    }
                    if Path::new(app_path).is_dir() {
                        return true;
                    }
                    system_service_app_exists(&bundle_id, app_path)
                } else {
                    false
                }
            });
        if is_protected {
            continue;
        }

        let prefixes = ["com.", "org.", "net.", "io."];
        if prefixes.iter().any(|p| bundle_id.starts_with(p))
            && !bundle_has_installed_app(&bundle_id)
        {
            orphaned.push(helper);
        }
    }

    if orphaned.is_empty() {
        return (0, 0);
    }

    orphaned.retain(|p| {
        if is_path_whitelisted_from_global(p) {
            debug_log(&format!("Skipping whitelisted orphan service: {p}"));
            false
        } else {
            true
        }
    });

    let dry_run = std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1"
        || std::env::var("DRY_RUN").unwrap_or_default() == "true";

    let mut removed_count: u64 = 0;
    let mut removed_kb: u64 = 0;

    for orphan_file in &orphaned {
        if dry_run {
            debug_log(&format!(
                "[DRY RUN] Would remove orphaned service: {orphan_file}"
            ));
            continue;
        }
        if should_protect_path(orphan_file) {
            debug_log(&format!(
                "Skipping protected orphaned service: {orphan_file}"
            ));
            continue;
        }
        let size_kb = String::from_utf8_lossy(
            &crate::core::sudo::sudo_output(&["/usr/bin/du", "-skP", orphan_file]).stdout,
        )
        .split_whitespace()
        .next()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
        if orphan_file.ends_with(".plist") {
            let _ = crate::core::sudo::sudo_output(&["/bin/launchctl", "unload", orphan_file]);
        }
        if safe_sudo_remove(orphan_file, Some(size_kb)) == MOLE_OK {
            removed_count += 1;
            removed_kb = removed_kb.saturating_add(size_kb);
            log_operation("clean", "REMOVED", orphan_file, Some("orphaned service"));
        }
    }

    if removed_count > 0 {
        log_info(&format!(
            "Cleaned {removed_count} orphaned services, about {}",
            bytes_to_human(removed_kb.saturating_mul(1024))
        ));
        note_activity();
    }
    (removed_kb, removed_count)
}

fn system_service_app_exists(bundle_id: &str, app_path: &str) -> bool {
    if !app_path.is_empty() && Path::new(app_path).exists() {
        return true;
    }
    if !app_path.is_empty() {
        let app_name = Path::new(app_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if app_path.starts_with("/Applications/") {
            let home = home_dir();
            if Path::new(&format!("{home}/Applications/{app_name}")).is_dir() {
                return true;
            }
            if Path::new(&format!("/Applications/Setapp/{app_name}")).is_dir() {
                return true;
            }
        }
        if app_path.starts_with("/Library/Input Methods/") {
            let home = home_dir();
            if Path::new(&format!("{home}/Library/Input Methods/{app_name}")).is_dir() {
                return true;
            }
        }
    }
    if bundle_id.is_empty() {
        return false;
    }
    bundle_has_installed_app(bundle_id)
}

// =============================================================================
// clean_orphaned_launch_agents
// =============================================================================

/// **重要**:user-level LaunchAgents 是用户拥有的自动化/配置,不是清理目标。
/// 与 SH 第 685-687 行 一致,本函数永远是 no-op。
/// 调用方:无需打印任何东西,即便用户在某个 UI 入口选了"清理 LaunchAgents",
/// 也应当只记录一行 debug 日志即可。
pub fn clean_orphaned_launch_agents() {
    debug_log("clean_orphaned_launch_agents is a no-op by design (user-owned automation)");
}

// =============================================================================
// clean_orphaned_container_stubs
// =============================================================================

/// 检测并移除由已卸载应用留下的仅含 metadata plist 的孤立容器存根。
/// 严格对齐 SH `clean_orphaned_container_stubs` 第 770-873 行。
///
/// 安全策略:
///   - 仅匹配硬编码允许列表中的 bundle ID glob
///   - 仅移除 stub-only 容器(只有 metadata plist,无 Data/ 或其他内容)
///   - 若 app 仍存在于已知路径或 mdfind 可查到时跳过
///   - 白名单路径跳过
pub fn clean_orphaned_container_stubs() -> (u64, u64) {
    let home = home_dir();
    let containers_dir = format!("{home}/Library/Containers");
    if !Path::new(&containers_dir).is_dir() {
        return (0, 0);
    }

    let dry_run = std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1"
        || std::env::var("DRY_RUN").unwrap_or_default() == "true";

    let stub_patterns: &[(&str, &str)] = &[
        ("com.macpaw.CleanMyMac*", "/Applications/CleanMyMac X.app"),
        ("*.com.macpaw.CleanMyMac*", "/Applications/CleanMyMac X.app"),
    ];

    let mut removed_count: u64 = 0;
    let mut removed_kb: u64 = 0;
    let mut failed_count: u64 = 0;

    let container_stub_app_exists = |bundle_id: &str, app_path: &str| -> bool {
        if Path::new(app_path).exists() {
            return true;
        }

        let app_name = Path::new(app_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let user_app = format!("{home}/Applications/{app_name}");
        if Path::new(&user_app).exists() {
            return true;
        }
        let setapp = format!("/Applications/Setapp/{app_name}");
        if Path::new(&setapp).exists() {
            return true;
        }
        let setapp_lib =
            format!("{home}/Library/Application Support/Setapp/Applications/{app_name}");
        if Path::new(&setapp_lib).exists() {
            return true;
        }

        let is_valid_id = bundle_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
            && bundle_id.len() >= 5;
        if is_valid_id {
            let query = format!("kMDItemCFBundleIdentifier == '{}'", bundle_id);
            if let Some(out) = run_with_timeout_capture(5.0, "mdfind", &[&query]) {
                if !out.trim().is_empty() {
                    return true;
                }
            }
        }

        false
    };

    for &(bundle_glob, app_path) in stub_patterns {
        let find_cmd = format!(
            "find \"{}\" -maxdepth 1 -name '{}' -type d -not -type l 2>/dev/null",
            containers_dir, bundle_glob
        );

        let args: Vec<&str> = vec!["-c", &find_cmd];
        let output = Command::new("sh")
            .args(&args)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();

        for container_dir in output.lines() {
            let container_dir = container_dir.trim();
            if container_dir.is_empty() || !Path::new(container_dir).is_dir() {
                continue;
            }
            if std::fs::symlink_metadata(container_dir)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(true)
            {
                continue;
            }

            let metadata_plist =
                format!("{container_dir}/.com.apple.containermanagerd.metadata.plist");
            if !Path::new(&metadata_plist).is_file() {
                continue;
            }

            let has_other_content = match std::fs::read_dir(container_dir) {
                Ok(rd) => rd
                    .flatten()
                    .any(|e| e.file_name() != ".com.apple.containermanagerd.metadata.plist"),
                Err(_) => continue,
            };
            if has_other_content {
                continue;
            }

            let bundle_id = Path::new(container_dir)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");

            if container_stub_app_exists(bundle_id, app_path) {
                continue;
            }

            if is_path_whitelisted_from_global(container_dir) {
                log_operation(
                    "clean",
                    "SKIPPED",
                    container_dir,
                    Some("whitelisted-stub-container"),
                );
                continue;
            }

            if dry_run {
                let sz = get_path_size_kb(container_dir);
                removed_kb = removed_kb.saturating_add(sz);
                removed_count += 1;
                log_operation(
                    "clean",
                    "SKIPPED",
                    container_dir,
                    Some("dry-run stub-container"),
                );
                continue;
            }

            match std::fs::remove_dir_all(container_dir) {
                Ok(()) => {
                    let sz = get_path_size_kb(container_dir);
                    removed_kb = removed_kb.saturating_add(sz);
                    removed_count += 1;
                    log_operation("clean", "REMOVED", container_dir, Some("stub-container"));
                }
                Err(e) => {
                    failed_count += 1;
                    let msg = format!("Failed to remove stub container {}: {}", container_dir, e);
                    log_warning(&msg);
                    log_operation("clean", "FAILED", container_dir, Some("stub-container"));
                }
            }
        }
    }

    if removed_count > 0 {
        if dry_run {
            println!(
                "  Orphaned app container stubs, {} stubs dry",
                removed_count
            );
        } else {
            println!("  Orphaned app container stubs, {} removed", removed_count);
            note_activity();
        }
    }
    if failed_count > 0 {
        println!(
            "  Orphaned container stubs: {} could not be removed",
            failed_count
        );
    }

    (removed_kb, removed_count)
}
