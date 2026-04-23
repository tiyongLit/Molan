//! Lemon 式「分类 → 子项 → 路径」：**编译期内嵌**规则 + **按本机已装应用动态生成**应用类条目。
//!
//! - 系统类：固定路径（含 Lemon 对齐的 `/Library/Caches`、系统日志等），仅当展开路径在磁盘上存在时才加入。
//! - 应用 / 上网类：规则绑定 **CFBundleIdentifier**（精确或前缀），从 `/Applications` 与 `~/Applications`
//!   扫描 `.app` 读 `Contents/Info.plist` 得到本机 Bundle ID 列表后做交集；**未安装则不出现该项**。
//! - Chrome 等路径与 Bundle ID 目录名不一致时，使用规则内显式 `rel_paths`，不依赖 `{bundle_id}` 猜路径。

use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleItemBlueprint {
    pub id: String,
    pub title: String,
    pub tips: String,
    pub recommend: bool,
    pub cautious: bool,
    pub relative_paths: Vec<String>,
    /// 命中的已安装应用 Bundle ID（仅动态规则项有值）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_bundle_id: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleCategoryBlueprint {
    pub id: String,
    pub title: String,
    pub tips: String,
    pub items: Vec<RuleItemBlueprint>,
}

// --- 系统垃圾：固定路径 -----------------------------------------------------

struct StaticPathItem {
    id: &'static str,
    title: &'static str,
    tips: &'static str,
    recommend: bool,
    cautious: bool,
    paths: &'static [&'static str],
}

const STATIC_SYSTEM_ITEMS: &[StaticPathItem] = &[
    StaticPathItem {
        id: "system_library_caches",
        title: "系统缓存",
        tips: "系统级 /Library/Caches（与 Lemon garbage1_zh 系统垃圾项 1001 对齐；实际删除常需管理员权限）。",
        recommend: true,
        cautious: false,
        paths: &["/Library/Caches"],
    },
    StaticPathItem {
        id: "system_logs",
        title: "系统日志",
        tips: "系统运行时日志与诊断目录（与 Lemon 项 1002 中主要路径对齐）。",
        recommend: true,
        cautious: false,
        paths: &[
            "/Library/Logs",
            "/Library/Logs/DiagnosticReports",
            "/private/var/log/asl",
            "/private/var/log/DiagnosticMessages",
            "/private/var/log/cups",
            "/private/var/log",
            "/private/var/db/diagnostics",
        ],
    },
    StaticPathItem {
        id: "user_library_caches",
        title: "用户缓存目录",
        tips: "各应用写入 ~/Library/Caches 的缓存数据。",
        recommend: true,
        cautious: false,
        paths: &["~/Library/Caches"],
    },
    StaticPathItem {
        id: "user_library_logs",
        title: "用户日志目录",
        tips: "应用日志多位于 ~/Library/Logs。",
        recommend: true,
        cautious: false,
        paths: &["~/Library/Logs"],
    },
    StaticPathItem {
        id: "downloads",
        title: "下载",
        tips: "清理前请确认下载文件夹中的文件均可删除。",
        recommend: false,
        cautious: true,
        paths: &["~/Downloads"],
    },
    StaticPathItem {
        id: "trash",
        title: "废纸篓",
        tips: "清理前请确认废纸篓中的文件均可删除。",
        recommend: false,
        cautious: true,
        paths: &["~/.Trash"],
    },
];

// --- 动态：Bundle 匹配 + 显式清理路径 ----------------------------------------

enum BundleMatch {
    /// CFBundleIdentifier 全等
    Exact(&'static str),
    /// `starts_with`，用于 Cursor 等 `com.todesktop.*`
    Prefix(&'static str),
}

struct BundleCleanRule {
    item_id: &'static str,
    bundle: BundleMatch,
    title: &'static str,
    tips: &'static str,
    recommend: bool,
    cautious: bool,
    /// 展示用路径（含 `~`）；已与各应用真实目录对齐，不猜 Reverse-DNS 目录名
    rel_paths: &'static [&'static str],
}

/// 应用垃圾：命中已安装 Bundle 才出现
const APP_JUNK_RULES: &[BundleCleanRule] = &[
    BundleCleanRule {
        item_id: "xcode_junk",
        bundle: BundleMatch::Exact("com.apple.dt.Xcode"),
        title: "Xcode 开发垃圾",
        tips: "DerivedData 等构建缓存；删除后下次编译会重新生成。",
        recommend: true,
        cautious: false,
        rel_paths: &["~/Library/Developer/Xcode/DerivedData"],
    },
    BundleCleanRule {
        item_id: "sketch_junk",
        bundle: BundleMatch::Exact("com.bohemiancoding.sketch3"),
        title: "Sketch 开发垃圾",
        tips: "Sketch 缓存与日志（与 Lemon garbage1_zh 应用垃圾项 2102 对齐；不含 DocumentRevisions）。",
        recommend: true,
        cautious: false,
        rel_paths: &[
            "~/Library/Caches/com.bohemiancoding.sketch3",
            "~/Library/Application Support/com.bohemiancoding.sketch3/crash.log",
            "~/Library/Logs/com.bohemiancoding.sketch3",
        ],
    },
    // [Mole Extension] 非 Lemon garbage*.xml 内建项，面向开发者常用编辑器缓存。
    BundleCleanRule {
        item_id: "vscode_cache",
        bundle: BundleMatch::Exact("com.microsoft.VSCode"),
        title: "Visual Studio Code",
        tips: "VS Code 缓存目录（若存在）。",
        recommend: true,
        cautious: false,
        rel_paths: &["~/Library/Application Support/Code/CachedData"],
    },
    // [Mole Extension]
    BundleCleanRule {
        item_id: "cursor_cache",
        bundle: BundleMatch::Prefix("com.todesktop."),
        title: "Cursor",
        tips: "基于 Todesktop 的编辑器（如 Cursor）缓存；前缀匹配 `com.todesktop.*`。",
        recommend: true,
        cautious: false,
        rel_paths: &["~/Library/Application Support/Cursor/CachedData"],
    },
];

/// 上网垃圾：浏览器 / 邮件等
const INTERNET_RULES: &[BundleCleanRule] = &[
    BundleCleanRule {
        item_id: "safari_cache",
        bundle: BundleMatch::Exact("com.apple.Safari"),
        title: "Safari",
        tips: "Safari 页面与资源缓存（与 Lemon garbage1_zh 上网垃圾项 306 路径对齐）。",
        recommend: true,
        cautious: false,
        rel_paths: &[
            "~/Library/Caches/com.apple.Safari",
            "~/Library/Caches/Metadata/Safari",
            "~/Library/Containers/com.apple.Safari.CacheDeleteExtension",
            "~/Library/Caches/com.apple.Safari.SafeBrowsing",
            "~/Library/Caches/com.apple.safaridavclient",
        ],
    },
    BundleCleanRule {
        item_id: "chrome_cache",
        bundle: BundleMatch::Exact("com.google.Chrome"),
        title: "Chrome",
        tips: "Chrome 默认 Profile 下磁盘缓存。",
        recommend: true,
        cautious: false,
        rel_paths: &["~/Library/Caches/Google/Chrome/Default/Cache"],
    },
    BundleCleanRule {
        item_id: "firefox_cache",
        bundle: BundleMatch::Exact("org.mozilla.firefox"),
        title: "Firefox",
        tips: "Firefox 磁盘缓存目录。",
        recommend: true,
        cautious: false,
        rel_paths: &["~/Library/Caches/Firefox"],
    },
    BundleCleanRule {
        item_id: "edge_cache",
        bundle: BundleMatch::Exact("com.microsoft.edgemac"),
        title: "Microsoft Edge",
        tips: "Edge 默认配置缓存。",
        recommend: true,
        cautious: false,
        rel_paths: &["~/Library/Caches/Microsoft Edge/Default/Cache"],
    },
    BundleCleanRule {
        item_id: "mail_attachments",
        bundle: BundleMatch::Exact("com.apple.mail"),
        title: "Mail 缓存",
        tips: "邮件附件下载缓存（容器路径）。",
        recommend: false,
        cautious: false,
        rel_paths: &["~/Library/Containers/com.apple.mail/Data/Library/Mail Downloads"],
    },
];

fn expand_home_path(home: &Path, rel: &str) -> PathBuf {
    if let Some(rest) = rel.strip_prefix("~/") {
        home.join(rest)
    } else if let Some(rest) = rel.strip_prefix('~') {
        home.join(rest.trim_start_matches('/'))
    } else {
        PathBuf::from(rel)
    }
}

fn path_exists_under_home(home: &Path, display_path: &str) -> bool {
    expand_home_path(home, display_path).exists()
}

fn build_static_system_items(home: &Path) -> Vec<RuleItemBlueprint> {
    let mut out = Vec::new();
    for it in STATIC_SYSTEM_ITEMS {
        let paths: Vec<String> = it
            .paths
            .iter()
            .filter(|p| path_exists_under_home(home, p))
            .map(|s| (*s).to_string())
            .collect();
        if paths.is_empty() {
            continue;
        }
        out.push(RuleItemBlueprint {
            id: it.id.to_string(),
            title: it.title.to_string(),
            tips: it.tips.to_string(),
            recommend: it.recommend,
            cautious: it.cautious,
            relative_paths: paths,
            matched_bundle_id: None,
        });
    }
    out
}

fn bundle_rule_matches(installed: &str, rule: &BundleCleanRule) -> bool {
    match &rule.bundle {
        BundleMatch::Exact(s) => installed == *s,
        BundleMatch::Prefix(p) => installed.starts_with(p),
    }
}

fn first_matching_bundle(installed: &[String], rule: &BundleCleanRule) -> Option<String> {
    installed
        .iter()
        .find(|bid| bundle_rule_matches(bid, rule))
        .cloned()
}

fn build_bundle_category_items(home: &Path, rules: &[BundleCleanRule], installed: &[String]) -> Vec<RuleItemBlueprint> {
    let mut out = Vec::new();
    for rule in rules {
        let Some(bid) = first_matching_bundle(installed, rule) else {
            continue;
        };
        let paths: Vec<String> = rule
            .rel_paths
            .iter()
            .filter(|p| path_exists_under_home(home, p))
            .map(|s| (*s).to_string())
            .collect();
        // 已安装但路径尚不存在（未产生缓存）：仍展示一项，路径列表可为空，前端可显示「很干净」类提示
        out.push(RuleItemBlueprint {
            id: rule.item_id.to_string(),
            title: rule.title.to_string(),
            tips: rule.tips.to_string(),
            recommend: rule.recommend,
            cautious: rule.cautious,
            relative_paths: paths,
            matched_bundle_id: Some(bid),
        });
    }
    out
}

#[cfg(target_os = "macos")]
fn read_bundle_id(plist_path: &Path) -> Option<String> {
    let v = plist::Value::from_file(plist_path).ok()?;
    v.as_dictionary()?
        .get("CFBundleIdentifier")?
        .as_string()
        .map(|s| s.to_string())
}

#[cfg(target_os = "macos")]
fn collect_bundle_ids_in_dir(applications_dir: &Path, out: &mut HashSet<String>) {
    let Ok(rd) = std::fs::read_dir(applications_dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        let is_app_bundle = p.is_dir()
            && p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".app"));
        if is_app_bundle {
            let plist_path = p.join("Contents/Info.plist");
            if let Some(bid) = read_bundle_id(&plist_path) {
                out.insert(bid);
            }
        }
    }
}

/// 扫描 `/Applications` 与 `~/Applications` 下一层 `.app`，读取 Bundle ID。
#[cfg(target_os = "macos")]
pub fn discover_installed_bundle_ids(home: &Path) -> Vec<String> {
    let mut set = HashSet::new();
    collect_bundle_ids_in_dir(Path::new("/Applications"), &mut set);
    collect_bundle_ids_in_dir(&home.join("Applications"), &mut set);
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort();
    v
}

#[cfg(not(target_os = "macos"))]
pub fn discover_installed_bundle_ids(_home: &Path) -> Vec<String> {
    Vec::new()
}

/// 供 `ScanResult.rule_categories`：系统项按存在性过滤；应用 / 上网项仅在本机已安装对应 App 时出现。
pub fn build_rule_categories(home: &Path) -> Vec<RuleCategoryBlueprint> {
    let installed = discover_installed_bundle_ids(home);

    let system_items = build_static_system_items(home);
    let app_items = build_bundle_category_items(home, APP_JUNK_RULES, &installed);
    let net_items = build_bundle_category_items(home, INTERNET_RULES, &installed);

    vec![
        RuleCategoryBlueprint {
            id: "system_junk".to_string(),
            title: "系统垃圾".to_string(),
            tips: "系统与用户目录下常见缓存、日志等（路径存在才列出子项）。".to_string(),
            items: system_items,
        },
        RuleCategoryBlueprint {
            id: "app_junk".to_string(),
            title: "应用垃圾".to_string(),
            tips: "根据本机已安装应用（/Applications、~/Applications 的 Bundle ID）动态生成。"
                .to_string(),
            items: app_items,
        },
        RuleCategoryBlueprint {
            id: "internet_junk".to_string(),
            title: "上网垃圾".to_string(),
            tips: "浏览器与邮件等；仅当检测到对应应用已安装时出现条目。".to_string(),
            items: net_items,
        },
    ]
}
