//! 孤儿残留扫描与安全策略（对齐 PureMac `OrphanSafetyPolicy.swift` + `AppState.findOrphans()`）。
//!
//! "孤儿"指已卸载 app 留下的残留文件——用户在用 Molan 之前通过拖到废纸篓等
//! 方式卸载的 app，其残留数据靠正向扫描（find_app_files）抓不到，因为根本没有
//! 目标 app 可作为输入。本模块实现反向扫描：遍历一组固定路径，过滤掉属于
//! 已安装 app 的条目，剩下的就是孤儿候选。
//!
//! 安全约束（对齐 PureMac OrphanSafetyPolicy 双层机制）：
//! - **白名单 root**：孤儿删除只能落在 Caches/Logs/HTTPStorages/WebKit/CrashReporter
//!   等"易失数据目录"——Preferences、Containers 等"持久状态目录"不在白名单，
//!   孤儿扫描发现了也只展示不删（`deletable = false`）。
//! - **黑名单 fragment**：即使命中白名单 root，路径中包含敏感 fragment 仍然拒绝。
//! - **高风险 dotpath**：调用 `high_risk_dotpaths::is_high_risk_dotpath` 兜底。
//! - **Apple 系统前缀**：文件名以 `com.apple.` 开头的一律跳过。

use std::path::Path;

use crate::core::high_risk_dotpaths;

// ---- 编译期常量表 ----

/// 孤儿扫描白名单根目录：只有这些"易失数据"目录下的文件才允许被删除。
/// 对齐 PureMac `OrphanSafetyPolicy.allowedRoots`。
const ORPHAN_ALLOWED_ROOTS: &[&str] = &[
    "~/Library/Caches",
    "~/Library/Logs",
    "~/Library/Saved Application State",
    "~/Library/HTTPStorages",
    "~/Library/WebKit",
    "~/Library/Application Support/CrashReporter",
    "/Library/Caches",
    "/Library/Logs",
];

/// 黑名单片段：即使命中白名单根，路径中包含这些片段仍然拒绝删除。
/// 对齐 PureMac `OrphanSafetyPolicy.blockedFragments`。
const ORPHAN_BLOCKED_FRAGMENTS: &[&str] = &[
    "/Library/Preferences",
    "/Library/PreferencePanes",
    "/Library/Containers",
    "/Library/Group Containers",
    "/Library/Application Scripts",
    "/Library/LaunchAgents",
    "/Library/LaunchDaemons",
    "/Library/PrivilegedHelperTools",
    "/Library/Keychains",
    "/Library/Mail",
    "/Library/Safari",
    "/Library/Messages",
    "/Library/Calendars",
    "/Library/Accounts",
    "/Library/Mobile Documents",
    "/Library/CloudStorage",
];

/// 孤儿反向扫描路径集（对齐 PureMac `Locations.reverseSearch.paths`）。
///
/// 注意：Preferences/Containers 等持久状态目录虽在扫描集中（用于展示），
/// 但 `is_safe_orphan_candidate` 会阻止其被标记为 `deletable`。
const ORPHAN_SCAN_PATHS: &[&str] = &[
    "~/Library/Application Scripts",
    "~/Library/Application Support",
    "~/Library/Caches",
    "~/Library/Containers",
    "~/Library/HTTPStorages",
    "~/Library/Internet Plug-Ins",
    "~/Library/LaunchAgents",
    "~/Library/Logs",
    "~/Library/Preferences",
    "~/Library/PreferencePanes",
    "~/Library/Preferences/ByHost",
    "~/Library/Saved Application State",
    "~/Library/WebKit",
    "/Users/Shared/Library/Application Support",
    "/Library/Application Support",
    "/Library/Internet Plug-Ins",
    "/Library/LaunchAgents",
    "/Library/LaunchDaemons",
    "/Library/PrivilegedHelperTools",
];

/// 孤儿扫描跳过前缀（对齐 PureMac `Conditions.skipReverse`）。
/// 已知系统/框架/通用组件前缀，不参与孤儿判定。
/// 匹配方式：`normalized_filename.starts_with(prefix)`，normalized = 小写 + 去除空格/-/_/.
const ORPHAN_SKIP_PREFIXES: &[&str] = &[
    // Apple & System
    "apple",
    "temporary",
    "btserver",
    "proapps",
    "scripteditor",
    "ilife",
    "livefsd",
    "siritoday",
    "addressbook",
    "animoji",
    "appstore",
    "askpermission",
    "callhistory",
    "clouddocs",
    "diskimages",
    "dock",
    "facetime",
    "fileprovider",
    "instruments",
    "knowledge",
    "mobilesync",
    "syncservices",
    "homeenergyd",
    "icloud",
    "icdd",
    "networkserviceproxy",
    "familycircle",
    "geoservices",
    "installation",
    "passkit",
    "sharedimagecache",
    "desktop",
    "mbuseragent",
    "swiftpm",
    "baseband",
    "coresimulator",
    "photoslegacyupgrade",
    "photosupgrade",
    "siritts",
    "ipod",
    "globalpreferences",
    // Analytics & Telemetry
    "apmanalytics",
    "apmexperiment",
    "avatarcache",
    "byhost",
    "contextstoreagent",
    "mobilemeaccounts",
    "mobiledocuments",
    "mobile",
    "intentbuilderc",
    "loginwindow",
    "momc",
    "replayd",
    "sharedfilelistd",
    // Build Tools & Compilers
    "clang",
    "audiocomponent",
    "csexattrcryptoservice",
    "livetranscriptionagent",
    "sandboxhelper",
    "statuskitagent",
    // System Daemons
    "betaenrollmentd",
    "contentlinkingd",
    "diagnosticextensionsd",
    "gamed",
    "heard",
    "homed",
    "itunescloudd",
    "lldb",
    "mds",
    "mediaanalysisd",
    "metrickitd",
    "mobiletimerd",
    "proactived",
    "ptpcamerad",
    "studentd",
    "talagent",
    "watchlistd",
    "apptranslocation",
    "xcrun",
    // Generic Infrastructure
    "ds_store",
    "caches",
    "crashreporter",
    "trash",
    // Molan / MoleStudio 历史口径自身（含 CLI 遗留），一律绝不清理
    "molan",
    "molestudio",
    "mole",
    // Common SDKs and Shared Components
    "amsdatamigratortool",
    "arfilecache",
    "assistant",
    "chromium",
    "cloudkit",
    "webkit",
    "databases",
    "diagnostic",
    "cache",
    "gamekit",
    "homebrew",
    "logi",
    "microsoft",
    "mozilla",
    "sync",
    "google",
    "sentinel",
    "hexnode",
    "sentry",
    "tvappservices",
    "reminders",
    "pbs",
    "notarytool",
    "differentialprivacy",
    "storeassetd",
    "webpush",
    "storedownloadd",
    "fsck",
    "crash",
    "python",
    "discrecording",
    "photossearch",
    "pylint",
    "jamf",
    "scopedbookmarkagent",
    "anonymous",
    "identifier",
    "isolated",
    "nobackup",
    "privacypreservingmeasurement",
    "symbols",
    "stickersd",
    "privatecloudcomputed",
    "tipsd",
    "controlcenter",
    "contactsd",
    "staticcheck",
    "index",
    "segment",
    "sparkle",
    "summaryevents",
    "launchdarkly",
    "identityservicesd",
    "embeddedbinaryvalidationutility",
    "aaprofilepicture",
    "minilauncher",
    "jna",
    "automator",
    "locationaccessstored",
    "spotlight",
    "cef",
];

// ---- 数据结构 ----

/// 孤儿条目分类。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrphanCategory {
    Cache,
    Log,
    SavedState,
    HttpStorage,
    WebKit,
    CrashReporter,
    Preference,
    Container,
    LaunchAgent,
    ApplicationSupport,
    Other,
}

/// 单个孤儿残留条目。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OrphanEntry {
    /// 完整路径
    pub path: String,
    /// 文件名（最后一段）
    pub file_name: String,
    /// 大小（字节），目录为递归统计
    pub size_bytes: u64,
    /// 人类可读大小
    pub size_human: String,
    /// 分类
    pub category: OrphanCategory,
    /// 是否可删除（白名单内 = true，仅展示 = false）
    pub deletable: bool,
}

// ---- 公开函数 ----

/// 判断一个路径是否是安全的孤儿删除候选。
///
/// 四步管线（对齐 PureMac `OrphanSafetyPolicy.isSafeCandidate`）：
///   1. 高风险 dotpath 检查
///   2. 白名单根目录检查（必须在 `ORPHAN_ALLOWED_ROOTS` 之下）
///   3. 黑名单片段检查（不能包含 `ORPHAN_BLOCKED_FRAGMENTS`）
///   4. Apple 系统前缀检查（文件名不以 `com.apple.` 开头）
pub fn is_safe_orphan_candidate(path: &str, home: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    // 1. 高风险 dotpath
    if high_risk_dotpaths::is_high_risk_dotpath(path, home) {
        return false;
    }
    let lower = path.to_lowercase();
    // 2. 白名单根目录
    let in_allowed_root = ORPHAN_ALLOWED_ROOTS.iter().any(|r| {
        let root = expand_tilde(r, home).to_lowercase();
        lower.starts_with(&format!("{root}/"))
    });
    if !in_allowed_root {
        return false;
    }
    // 3. 黑名单片段
    if ORPHAN_BLOCKED_FRAGMENTS
        .iter()
        .any(|f| lower.contains(&f.to_lowercase()))
    {
        return false;
    }
    // 4. Apple 系统前缀
    let file_name = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if file_name.starts_with("com.apple.") || file_name == ".globalpreferences.plist" {
        return false;
    }
    true
}

/// 孤儿扫描主入口：反向扫描 `ORPHAN_SCAN_PATHS`，
/// 过滤掉属于已安装 app 的条目，返回孤儿列表。
///
/// `installed_apps` 为 `(bundle_id, app_name)` 对列表，
/// 来自 `mole_list_apps` 的结果。
pub fn scan_orphans(installed_apps: &[(String, String)], home: &str) -> Vec<OrphanEntry> {
    // 预计算已安装 app 的 normalized 标识集
    let known_ids: Vec<String> = installed_apps
        .iter()
        .map(|(bid, _)| normalize_for_matching(bid))
        .filter(|s| !s.is_empty() && s.len() >= 3)
        .collect();
    let known_names: Vec<String> = installed_apps
        .iter()
        .map(|(_, name)| normalize_for_matching(name))
        .filter(|s| !s.is_empty() && s.len() >= 3)
        .collect();

    let orphans = scan_candidates(home, |normalized| {
        // 排除属于已安装 app 的条目
        !(known_ids.iter().any(|id| normalized.contains(id.as_str()))
            || known_names
                .iter()
                .any(|name| normalized.contains(name.as_str())))
    });

    log::info!(
        "[orphan_scan] found {} orphan(s), {} deletable",
        orphans.len(),
        orphans.iter().filter(|o| o.deletable).count()
    );
    orphans
}

/// 定向孤儿扫描：只返回与指定 app 相关的残留（通知点击 → 卸载页定向清理链路）。
///
/// 与 `scan_orphans` 共用候选收集与安全策略管线（跳前缀 / 白名单 / 黑名单 /
/// `deletable` 规则），差异仅在**命中判定**——安全口径不因定向放宽。
///
/// 命中规则：
/// - `bundle_id` 非空 → 归一化后 `file_name.contains(bundle_id)`；
/// - `bundle_id` 为空 → 归一化后 `file_name.contains(app_name)`，且 app_name
///   归一化长度 ≥ 3（短名误报面过大，仅允许走 bundleId）。
///
/// 不套用"已安装 app 排除"逻辑：目标即已卸载 app，该函数仅面向目标匹配。
pub fn scan_orphans_for(bundle_id: Option<&str>, app_name: &str, home: &str) -> Vec<OrphanEntry> {
    let target_id = bundle_id
        .map(normalize_for_matching)
        .filter(|s| !s.is_empty());
    let target_name = {
        let normalized = normalize_for_matching(app_name);
        (normalized.len() >= 3).then_some(normalized)
    };

    let orphans = scan_candidates(home, |normalized| match &target_id {
        Some(id) => normalized.contains(id.as_str()),
        None => target_name
            .as_deref()
            .is_some_and(|name| normalized.contains(name)),
    });

    log::info!(
        "[orphan_scan] targeted scan (bundleId={:?}, name={:?}) hit {} entr(ies)",
        bundle_id,
        app_name,
        orphans.len()
    );
    orphans
}

// ---- 内部工具 ----

/// 候选收集公共管线：遍历 `ORPHAN_SCAN_PATHS` → 跳过系统项 → 目标判定 →
/// 安全策略判定 + 分类 + 大小统计 → 按文件名排序。
///
/// `include` 由调用方提供命中口径（全量扫描 = 排除已安装 app；定向扫描 =
/// 命中目标 app），安全部分两条链路完全一致。
fn scan_candidates(home: &str, include: impl Fn(&str) -> bool) -> Vec<OrphanEntry> {
    let mut orphans: Vec<OrphanEntry> = Vec::new();

    for scan_path in ORPHAN_SCAN_PATHS {
        let expanded = expand_tilde(scan_path, home);
        let dir = Path::new(&expanded);
        if !dir.is_dir() {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let item_path = entry.path();
            let Some(file_name) = item_path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            let full_path = item_path.to_string_lossy().to_string();
            let normalized = normalize_for_matching(file_name);

            // 1. 跳过已知系统项
            if ORPHAN_SKIP_PREFIXES
                .iter()
                .any(|prefix| normalized.starts_with(prefix))
            {
                continue;
            }

            // 2. 目标判定（口径由调用方决定）
            if !include(&normalized) {
                continue;
            }

            // 3. 安全策略判定
            let deletable = is_safe_orphan_candidate(&full_path, home);
            let category = classify_orphan_path(&full_path);
            let size_bytes = compute_path_size(&item_path);

            orphans.push(OrphanEntry {
                path: full_path,
                file_name: file_name.to_string(),
                size_bytes,
                size_human: format_size(size_bytes),
                category,
                deletable,
            });
        }
    }

    // 按文件名排序
    orphans.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    orphans
}

/// 展开 `~` 为用户 home 目录。
fn expand_tilde(path: &str, home: &str) -> String {
    if path.starts_with("~/") {
        format!("{}{}", home, &path[1..])
    } else if path == "~" {
        home.to_string()
    } else {
        path.to_string()
    }
}

/// 归一化文件名用于匹配：小写 + 去除空格/-/_/.
/// 对齐 PureMac `String.normalizedForMatching()`。
fn normalize_for_matching(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_' | '.'))
        .collect()
}

/// 根据路径推断孤儿分类。
fn classify_orphan_path(path: &str) -> OrphanCategory {
    let lower = path.to_lowercase();
    if lower.contains("/library/caches") {
        OrphanCategory::Cache
    } else if lower.contains("/library/logs") {
        OrphanCategory::Log
    } else if lower.contains("/saved application state") {
        OrphanCategory::SavedState
    } else if lower.contains("/httpstorages") {
        OrphanCategory::HttpStorage
    } else if lower.contains("/webkit") {
        OrphanCategory::WebKit
    } else if lower.contains("/crashreporter") {
        OrphanCategory::CrashReporter
    } else if lower.contains("/library/preferences") {
        OrphanCategory::Preference
    } else if lower.contains("/library/containers") {
        OrphanCategory::Container
    } else if lower.contains("/launchagents") || lower.contains("/launchdaemons") {
        OrphanCategory::LaunchAgent
    } else if lower.contains("/application support") {
        OrphanCategory::ApplicationSupport
    } else {
        OrphanCategory::Other
    }
}

/// 计算路径大小（目录递归，文件直接取 metadata）。
fn compute_path_size(path: &Path) -> u64 {
    if path.is_file() || path.is_symlink() {
        return path.metadata().map(|m| m.len()).unwrap_or(0);
    }
    if path.is_dir() {
        return dir_size_recursive(path, 3); // 最多递归 3 层，避免过慢
    }
    0
}

/// 递归目录大小统计（限制深度）。
fn dir_size_recursive(dir: &Path, max_depth: u32) -> u64 {
    if max_depth == 0 {
        return 0;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_file() || p.is_symlink() {
            total += p.metadata().map(|m| m.len()).unwrap_or(0);
        } else if p.is_dir() {
            total += dir_size_recursive(&p, max_depth - 1);
        }
    }
    total
}

/// 格式化大小为人类可读字符串。
fn format_size(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_root_caches_is_deletable() {
        let home = "/Users/testuser";
        assert!(is_safe_orphan_candidate(
            "/Users/testuser/Library/Caches/com.oldapp.cache",
            home
        ));
    }

    #[test]
    fn allowed_root_logs_is_deletable() {
        let home = "/Users/testuser";
        assert!(is_safe_orphan_candidate(
            "/Users/testuser/Library/Logs/OldApp",
            home
        ));
    }

    #[test]
    fn preferences_not_deletable() {
        let home = "/Users/testuser";
        // Preferences 不在白名单 root 中
        assert!(!is_safe_orphan_candidate(
            "/Users/testuser/Library/Preferences/com.oldapp.plist",
            home
        ));
    }

    #[test]
    fn containers_not_deletable() {
        let home = "/Users/testuser";
        assert!(!is_safe_orphan_candidate(
            "/Users/testuser/Library/Containers/com.oldapp",
            home
        ));
    }

    #[test]
    fn blocked_fragment_overrides_allowed_root() {
        let home = "/Users/testuser";
        // 即使命中 Caches 白名单，包含 /Library/Preferences 片段仍拒绝
        assert!(!is_safe_orphan_candidate(
            "/Users/testuser/Library/Caches/Library/Preferences/something",
            home
        ));
    }

    #[test]
    fn apple_prefix_rejected() {
        let home = "/Users/testuser";
        assert!(!is_safe_orphan_candidate(
            "/Users/testuser/Library/Caches/com.apple.Safari",
            home
        ));
    }

    #[test]
    fn high_risk_dotpath_rejected() {
        let home = "/Users/testuser";
        assert!(!is_safe_orphan_candidate("/Users/testuser/.ssh", home));
        assert!(!is_safe_orphan_candidate("/Users/testuser/.claude", home));
    }

    #[test]
    fn normalize_removes_separators() {
        assert_eq!(normalize_for_matching("My-App_Name"), "myappname");
        assert_eq!(normalize_for_matching("com.foo.bar"), "comfoobar");
    }

    #[test]
    fn classify_paths() {
        assert_eq!(
            classify_orphan_path("/Users/x/Library/Caches/com.foo"),
            OrphanCategory::Cache
        );
        assert_eq!(
            classify_orphan_path("/Users/x/Library/Logs/Foo"),
            OrphanCategory::Log
        );
        assert_eq!(
            classify_orphan_path("/Users/x/Library/Preferences/com.foo.plist"),
            OrphanCategory::Preference
        );
        assert_eq!(
            classify_orphan_path("/Users/x/Library/Containers/com.foo"),
            OrphanCategory::Container
        );
    }

    #[test]
    fn expand_tilde_works() {
        assert_eq!(
            expand_tilde("~/Library/Caches", "/Users/test"),
            "/Users/test/Library/Caches"
        );
        assert_eq!(
            expand_tilde("/Library/Caches", "/Users/test"),
            "/Library/Caches"
        );
        assert_eq!(expand_tilde("~", "/Users/test"), "/Users/test");
    }

    #[test]
    fn format_size_works() {
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(2048), "2 KB");
        assert_eq!(format_size(5_242_880), "5.0 MB");
        assert_eq!(format_size(2_147_483_648), "2.0 GB");
    }

    /// 造一个临时 home，在 `Library/Caches` 下预置残留候选。
    fn make_fake_home(files: &[&str]) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let caches = dir.path().join("Library/Caches");
        std::fs::create_dir_all(&caches).unwrap();
        for f in files {
            std::fs::write(caches.join(f), b"x").unwrap();
        }
        let home = dir.path().to_string_lossy().to_string();
        (dir, home)
    }

    #[test]
    fn targeted_scan_matches_bundle_id_only() {
        let (_dir, home) = make_fake_home(&[
            "com.testtarget.cachedir",
            "com.testtarget.helper",
            "com.otherbigapp.cachedir",
        ]);
        let hits = scan_orphans_for(Some("com.testtarget"), "NoSuchName", &home);
        assert!(hits.iter().any(|o| o.file_name == "com.testtarget.cachedir"));
        assert!(hits.iter().any(|o| o.file_name == "com.testtarget.helper"));
        // 非目标 app 的残留不得混入
        assert!(!hits.iter().any(|o| o.file_name == "com.otherbigapp.cachedir"));
    }

    #[test]
    fn targeted_scan_short_app_name_gated() {
        let (_dir, home) = make_fake_home(&["ab-thing", "com.testtarget.cachedir"]);
        // 短名（归一化后 < 3 字符）且无 bundleId → 一律不命中（防误报）
        assert!(scan_orphans_for(None, "Ab", &home).is_empty());
        // ≥3 字符时可走 appName 匹配（bundleId 缺失的兜底路径）
        let hits = scan_orphans_for(None, "TestTarget", &home);
        assert!(hits.iter().any(|o| o.file_name == "com.testtarget.cachedir"));
    }

    #[test]
    fn targeted_scan_preserves_safety_policy() {
        let (dir, home) = make_fake_home(&["com.testtarget.cachedir"]);
        let prefs = dir.path().join("Library/Preferences");
        std::fs::create_dir_all(&prefs).unwrap();
        std::fs::write(prefs.join("com.testtarget.plist"), b"x").unwrap();

        let hits = scan_orphans_for(Some("com.testtarget"), "TestTarget", &home);
        let cache_hit = hits
            .iter()
            .find(|o| o.file_name == "com.testtarget.cachedir")
            .expect("cache hit should be included");
        let pref_hit = hits
            .iter()
            .find(|o| o.file_name == "com.testtarget.plist")
            .expect("preference hit should be shown");
        // 安全口径不因定向放宽：Caches 可删，Preferences 仅展示
        assert!(cache_hit.deletable);
        assert!(!pref_hit.deletable);
    }
}
