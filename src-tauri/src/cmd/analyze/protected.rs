//! 磁盘分析模块中的受保护目录 —— 不可勾选、不可删除。
//!
//! 参考 CleanMyMac X 的做法：macOS 系统级标准目录
//! 用户应点进去清理具体文件，而不应将目录本身勾选移除。
//!
//! 核心判断函数 `is_protected_entry_path(path)` 基于完整路径匹配：
//!   - 扫描时 Rust 端直接写入 entry.protected，前端零计算
//!   - ~/work/mail 不会被误锁（它不在系统 Library 下）
//!   - 将来后端删除校验也复用此函数

use std::path::Path;

/// 系统级别受保护的前缀 —— 任何在此前缀下的目录都不可删除。
/// 对齐 Go `isCriticalAnalyzeDeletePath` 的 protectedTrees：
/// /private 与 /opt 本身按精确路径保护（见 is_system_root_path），
/// 仅其系统子树受前缀保护；用户区（/private/var/folders、/private/var/log、/opt/local 等）
/// 不整树拦截，其中 EDR 缓存由 delete.rs 的 is_endpoint_security_cache_path 单独保护。
static SYSTEM_PREFIXES: &[&str] = &[
    "/System/",
    "/bin/",
    "/sbin/",
    "/usr/",
    "/dev/",
    "/private/etc/",
    "/private/var/audit/",
    "/private/var/db/",
    "/private/var/root/",
];

/// ~/Library/ 和 /Library 下受保护的子目录名（小写）。
/// 仅当 parent 是系统 Library（/Library 或 /Users/*/Library）时才生效。
static PROTECTED_LIBRARY_SUBDIRS: &[&str] = &[
    "application support",
    "caches",
    "preferences",
    "frameworks",
    "containers",
    "group containers",
    "launchagents",
    "launchdaemons",
    "startupitems",
    "saved application state",
    "receipts",
    "mail",
    "messages",
    "safari",
    "keychains",
    "accounts",
    "calendars",
    "cookies",
    "dictionaries",
    "fontcollections",
    "fonts",
    "passes",
    "callservices",
    "personas",
    "gamekit",
    "preferencepanes",
    "assistants",
    "keyboard layouts",
    "keyboardservices",
    "colorsync",
    "compositions",
    "identityservices",
    "internet plug-ins",
    "quicklook",
    "screen savers",
    "services",
    "sounds",
    "speech",
    "spelling",
    "languagemodeling",
    "metadata",
    "printers",
    "extensions",
    "syncedpreferences",
    "syndication",
    "fileprovider",
    "sharing",
    "spotlight",
    "tcc",
    "logs",
    "textinput",
    "addressbook",
    "callhistory",
    "mobile documents",
    "cloudservices",
    "siri",
    "homekit",
    "widgets",
    "wifi",
    "bluetooth",
    "priviledgedhelpertools",
    "input methods",
    "configuration profiles",
    "desktop pictures",
    "notifications",
    "notificationcenter",
    "coreservices",
    "managed preferences",
    "autosave information",
    "filesystems",
    "image capture",
    "audio",
    "pdf services",
    "sandbox",
    "locations",
    "screen time",
    "safari safe browsing",
    "crashreporter",
    "reminders",
    "shortcuts",
    "news",
    "podcasts",
    "stocks",
    "weather",
    "ubiquity",
    "application scripts",
    "personalizationdata",
    "updates",
];

/// 基于完整路径判断目录是否受保护（路径匹配，非唯名匹配）。
///
/// # 规则
///
/// 1. `/` 根目录
/// 2. 精确匹配根级系统路径：/System, /Library, /Applications, /usr, /opt …
/// 3. 精确匹配用户 Home 标准目录：/Users/*/Documents, /Users/*/Library …
/// 4. 系统前缀下任意子目录：/System/Library/Extensions …
/// 5. 系统 Library 下的标准子目录（parent 必须是 /Library 或 /Users/*/Library）
///
/// # 示例
///
/// ```ignore
/// assert!(!is_protected_entry_path("/Users/foo/work/mail"));
/// assert!(is_protected_entry_path("/Users/foo/Library/Mail"));
/// assert!(is_protected_entry_path("/System/Library/CoreServices"));
/// ```
pub fn is_protected_entry_path(raw: &str) -> bool {
    let path = raw.trim_end_matches('/');
    if path.is_empty() || path == "/" {
        return true;
    }

    // 规则 1+2: 根级系统路径
    if is_system_root_path(path) {
        return true;
    }

    // 规则 3: 用户 Home 标准目录
    if is_home_standard_path(path) {
        return true;
    }

    // 规则 4: 系统前缀下
    if SYSTEM_PREFIXES.iter().any(|p| path.starts_with(p)) {
        return true;
    }

    // 规则 5: 系统 Library 下的标准子目录
    is_protected_library_subdir(path)
}

// ── 内部辅助函数 ──

fn is_system_root_path(path: &str) -> bool {
    // 精确根对齐 Go criticalRoots：/private、/opt 等只保护本身，不保护整棵子树。
    let roots = [
        "/System",
        "/Library",
        "/Applications",
        "/Users",
        "/opt",
        "/opt/homebrew",
        "/usr",
        "/private",
        "/private/etc",
        "/private/tmp",
        "/private/var",
        "/private/var/audit",
        "/private/var/db",
        "/private/var/root",
        "/private/var/tmp",
        "/private/var/folders",
        "/bin",
        "/sbin",
        "/cores",
        "/Network",
        "/Volumes",
        "/etc",
        "/tmp",
        "/var",
        "/dev",
        "/home",
    ];
    roots.contains(&path)
}

fn is_home_standard_path(path: &str) -> bool {
    let home_dirs = [
        "Desktop",
        "Documents",
        "Downloads",
        "Movies",
        "Music",
        "Pictures",
        "Public",
        "Library",
    ];
    let p = Path::new(path);
    let comps: Vec<&str> = p.iter().filter_map(|c| c.to_str()).collect();
    // /Users/<name>/<dir> → comps: ["/", "Users", "<name>", "<dir>"]
    comps.len() == 4 && comps[1] == "Users" && home_dirs.contains(&comps[3])
}

fn is_protected_library_subdir(path: &str) -> bool {
    let p = Path::new(path);
    let parent = match p.parent() {
        Some(par) => par.to_string_lossy(),
        None => return false,
    };
    // parent 必须是系统 Library
    if !is_system_library_path(&parent) {
        return false;
    }
    let name = match p.file_name().and_then(|n| n.to_str()) {
        Some(n) => n.to_ascii_lowercase(),
        None => return false,
    };
    PROTECTED_LIBRARY_SUBDIRS.contains(&name.as_str())
}

fn is_system_library_path(path: &str) -> bool {
    if path == "/Library" {
        return true;
    }
    let p = Path::new(path);
    let parts: Vec<&str> = p.iter().filter_map(|c| c.to_str()).collect();
    // /Users/<name>/Library → comps: ["/", "Users", "<name>", "Library"]
    parts.len() == 4 && parts[1] == "Users" && parts[3] == "Library"
}

// ── 兼容旧接口：供 Tauri 命令 mole_get_protected_analyze_paths 使用 ──

pub fn get_protected_dir_names() -> Vec<String> {
    vec![
        "System",
        "Library",
        "Applications",
        "Users",
        "opt",
        "usr",
        "private",
        "bin",
        "sbin",
        "cores",
        "Network",
        "Volumes",
        "Desktop",
        "Documents",
        "Downloads",
        "Movies",
        "Music",
        "Pictures",
        "Public",
        "Application Support",
        "Caches",
        "Preferences",
        "Frameworks",
        "Containers",
        "Group Containers",
        "LaunchAgents",
        "LaunchDaemons",
        "StartupItems",
        "Saved Application State",
        "Receipts",
        "Mail",
        "Messages",
        "Safari",
        "Keychains",
        "Accounts",
        "Calendars",
        "Cookies",
        "Dictionaries",
        "FontCollections",
        "Fonts",
        "Passes",
        "CallServices",
        "Personas",
        "GameKit",
        "PreferencePanes",
        "Assistants",
        "Keyboard Layouts",
        "KeyboardServices",
        "ColorSync",
        "Compositions",
        "IdentityServices",
        "Internet Plug-Ins",
        "QuickLook",
        "Screen Savers",
        "Services",
        "Sounds",
        "Speech",
        "Spelling",
        "LanguageModeling",
        "Metadata",
        "Printers",
        "Extensions",
        "SyncedPreferences",
        "Syndication",
        "FileProvider",
        "Sharing",
        "Spotlight",
        "TCC",
        "Logs",
        "TextInput",
        "AddressBook",
        "CallHistory",
        "Mobile Documents",
        "CloudServices",
        "Siri",
        "HomeKit",
        "Widgets",
        "WiFi",
        "Bluetooth",
        "PrivilegedHelperTools",
        "Input Methods",
        "Configuration Profiles",
        "Desktop Pictures",
        "Notifications",
        "NotificationCenter",
        "CoreServices",
        "Managed Preferences",
        "Autosave Information",
        "Filesystems",
        "Image Capture",
        "Audio",
        "PDF Services",
        "Sandbox",
        "Locations",
        "Screen Time",
        "Safari Safe Browsing",
        "CrashReporter",
        "Reminders",
        "Shortcuts",
        "News",
        "Podcasts",
        "Stocks",
        "Weather",
        "Ubiquity",
        "Application Scripts",
        "PersonalizationData",
        "Updates",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_protected_entry_path ──

    #[test]
    fn test_root_path() {
        assert!(is_protected_entry_path("/"));
        assert!(is_protected_entry_path(""));
    }

    #[test]
    fn test_system_roots() {
        for d in &[
            "/System",
            "/Library",
            "/Applications",
            "/Users",
            "/opt",
            "/usr",
            "/private",
            "/bin",
            "/sbin",
            "/cores",
            "/Network",
            "/Volumes",
            "/etc",
            "/tmp",
            "/var",
        ] {
            assert!(is_protected_entry_path(d), "expected protected: {d}");
        }
    }

    #[test]
    fn test_home_standard() {
        for d in &[
            "Documents",
            "Downloads",
            "Desktop",
            "Library",
            "Movies",
            "Music",
            "Pictures",
            "Public",
        ] {
            assert!(is_protected_entry_path(&format!("/Users/foo/{d}")));
        }
    }

    #[test]
    fn test_library_subdirs() {
        for d in &[
            "Caches",
            "Application Support",
            "Preferences",
            "LaunchDaemons",
            "Fonts",
        ] {
            assert!(is_protected_entry_path(&format!("/Library/{d}")));
        }
        for d in &[
            "Mail",
            "Cookies",
            "Keychains",
            "Messages",
            "Dictionaries",
            "FontCollections",
            "PreferencePanes",
            "Assistants",
            "LaunchAgents",
            "Safari",
            "Containers",
            "TCC",
            "QuickLook",
            "Spotlight",
            "Siri",
        ] {
            assert!(is_protected_entry_path(&format!("/Users/foo/Library/{d}")));
        }
    }

    #[test]
    fn test_system_prefix_subdirs() {
        assert!(is_protected_entry_path("/System/Library/CoreServices"));
        assert!(is_protected_entry_path("/System/Library/Extensions"));
        assert!(is_protected_entry_path("/usr/libexec"));
        assert!(is_protected_entry_path("/opt/homebrew"));
        // 对齐 Go：/private 仅保护系统子树
        assert!(is_protected_entry_path("/private/etc/ssh"));
        assert!(is_protected_entry_path("/private/var/db"));
        assert!(is_protected_entry_path("/private/var/audit/current"));
        // 用户区不受整树拦截：/private/var/folders 用户 temp、/private/var/log 日志、
        // /opt/local 包管理器目录都是合法清理目标（EDR 缓存由删除链单独拦截）
        assert!(!is_protected_entry_path("/private/var/log"));
        assert!(!is_protected_entry_path(
            "/private/var/folders/zz/aa/T/.tmpXYZ/file"
        ));
        assert!(!is_protected_entry_path("/opt/local/bin"));
    }

    #[test]
    fn test_user_work_not_protected() {
        assert!(!is_protected_entry_path("/Users/foo/work/mail"));
        assert!(!is_protected_entry_path("/Users/foo/projects/cookies"));
        assert!(!is_protected_entry_path("/Users/foo/work/fonts"));
        assert!(!is_protected_entry_path("/Volumes/External/spotlight"));
        assert!(!is_protected_entry_path("/Users/foo/Downloads_Bak"));
    }

    #[test]
    fn test_nested_library_not_protected() {
        assert!(!is_protected_entry_path("/Users/foo/work/Library/Mail"));
        assert!(!is_protected_entry_path("/foo/Library/Caches"));
    }

    #[test]
    fn test_user_dirs_not_protected() {
        assert!(!is_protected_entry_path("/Users/foo/work"));
        assert!(!is_protected_entry_path("/Users/foo/projects"));
        assert!(!is_protected_entry_path("/Users/foo/node_modules"));
    }

    #[test]
    fn test_docs_subdir_not_auto_protected() {
        assert!(!is_protected_entry_path("/Users/foo/Documents/reports"));
        assert!(!is_protected_entry_path("/Users/foo/Desktop/screenshots"));
    }

    #[test]
    fn test_trailing_slash() {
        assert!(is_protected_entry_path("/Library/"));
        assert!(is_protected_entry_path("/Users/foo/Library/Mail/"));
    }

    #[test]
    fn test_get_names() {
        let names = get_protected_dir_names();
        assert!(names.contains(&"Library".to_string()));
        assert!(names.contains(&"Downloads".to_string()));
        assert!(names.contains(&"Application Support".to_string()));
        assert!(names.contains(&"PreferencePanes".to_string()));
        assert!(names.contains(&"Cookies".to_string()));
        assert!(names.contains(&"LaunchDaemons".to_string()));
        assert!(names.contains(&"Updates".to_string()));
    }
}
