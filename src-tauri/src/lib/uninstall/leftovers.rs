//! 新一代残留深度扫描引擎（卸载覆盖面最大化）。
//!
//! `find_app_files` 是"精确候选路径拼接"，覆盖不了四类盲区，本模块按
//! Pearcleaner 的覆盖面方案补齐（安全边界对齐 PureMac）：
//!
//! 1. **UUID 容器解析**：`~/Library/Containers/<UUID>/` 目录名不含 bundle id，
//!    读 `.com.apple.containermanagerd.metadata.plist` 的 `MCMMetadataIdentifier`
//!    精确判定归属（含 app 内嵌 helper/XPC 的容器）。
//! 2. **Group Container 精确解析**：`codesign` 读 entitlements 的
//!    `com.apple.security.application-groups`，直接映射到
//!    `~/Library/Group Containers/<group-id>`，不依赖目录名碰运气。
//! 3. **base bundle id 剥离**：`com.foo.app.helper` → `com.foo.app`，
//!    抓 helper/agent/daemon 派生的 LaunchAgent/Daemon、PrivilegedHelperTools、Preferences。
//! 4. **Library 根 depth-2 厂商目录**：`/Library/Objective-See/LuLu` 这类
//!    厂商根下的产品目录（标准系统子目录一律不进入）。
//!
//! 安全约束：
//! - 不扫 bare `$HOME`（PureMac Locations 原则），只扫 `~/Library` 与 `/Library`。
//! - 系统域（`/Library/...`）命中的路径只进 `review_only`，不进入删除集。
//! - 全部匹配要求精确相等或 bundle id 边界，不做裸 substring。
//! - 外部命令（plutil/codesign）一律带超时，失败静默降级（不影响主流程）。

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use crate::core::base::home_dir;
use crate::core::high_risk_dotpaths::filter_high_risk_paths;
use crate::core::timeout::run_with_timeout_capture;

/// plutil / codesign 单次探测超时（秒）。
const PROBE_TIMEOUT_SEC: f64 = 5.0;

/// 深度扫描结果：可删除集（用户域）与仅审阅集（系统域，前端展示不删）。
#[derive(Debug, Default)]
pub struct DeepLeftovers {
    pub deletable: Vec<String>,
    pub review_only: Vec<String>,
}

impl DeepLeftovers {
    fn is_empty(&self) -> bool {
        self.deletable.is_empty() && self.review_only.is_empty()
    }
}

/// 入口：对单个 app 做深度残留扫描。
///
/// `bundle_id` 为 "unknown"（sibling guard 降级）时，所有 bundle id 派生
/// 匹配自动关闭，只剩名字变体的 depth-2 扫描。
pub fn scan_deep_leftovers(bundle_id: &str, app_name: &str, app_path: &str) -> DeepLeftovers {
    let home = home_dir();
    let bundle_ok = is_valid_bundle_id(bundle_id);
    let mut out = DeepLeftovers::default();

    // 1) UUID 容器（含内嵌 helper / XPC 的容器）
    if bundle_ok {
        let mut identifiers = vec![bundle_id.to_string()];
        identifiers.extend(embedded_bundle_ids(app_path, bundle_id));
        let identifiers: Vec<String> = identifiers
            .into_iter()
            .map(|s| s.to_ascii_lowercase())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        for dir in uuid_container_dirs(&home, &identifiers) {
            out.deletable.push(dir);
        }
    }

    // 2) Group Container（entitlements application-groups 精确映射）
    for gid in app_group_identifiers(app_path) {
        for p in [
            format!("{home}/Library/Group Containers/{gid}"),
            format!("{home}/Library/Application Scripts/{gid}"),
        ] {
            if Path::new(&p).is_dir() {
                out.deletable.push(p);
            }
        }
    }

    // 3) base bundle id 剥离派生路径
    if bundle_ok {
        if let Some(base) = base_bundle_id(bundle_id) {
            merge(&mut out, base_id_paths(&base, &home));
        }
    }

    // 4) Library 根 depth-2 厂商目录
    merge(&mut out, library_root_depth2(app_name, bundle_id, &home));

    // PureMac 安全加固：过滤高风险 dotfile/dotdir（如 ~/.claude、~/.ssh）
    out.deletable = filter_high_risk_paths(out.deletable, &home);
    out.review_only = filter_high_risk_paths(out.review_only, &home);

    out.deletable.sort();
    out.deletable.dedup();
    out.review_only.sort();
    out.review_only.dedup();
    if !out.is_empty() {
        log::info!(
            "[uninstall.deep_leftovers] app={app_name} bundle={bundle_id} deletable={} review_only={}",
            out.deletable.len(),
            out.review_only.len()
        );
    }
    out
}

/// 路径父子去重（对齐 Lemon `filepathExistsArray` 倒序算法的正向等价）：
/// 任一路径的真祖先在集合中时，丢弃子路径。用于合并后的最终清单，
/// 避免 `Support/Chrome` 与 `Support/Chrome/Default` 同时出现造成重复统计。
pub fn dedupe_parents_child(paths: Vec<String>) -> Vec<String> {
    let mut set: HashSet<String> = HashSet::new();
    for p in paths {
        let p = p.trim_end_matches('/').to_string();
        if !p.is_empty() {
            set.insert(p);
        }
    }
    let mut sorted: Vec<String> = set.iter().cloned().collect();
    sorted.sort();
    sorted.retain(|p| {
        // 逐级检查真祖先是否在集合里（以 set 为全集，避免 retain 自引用）
        let mut prefix = p.clone();
        while let Some(i) = prefix.rfind('/') {
            if i == 0 {
                break;
            }
            prefix.truncate(i);
            if set.contains(&prefix) {
                return false;
            }
        }
        true
    });
    sorted
}

fn merge(out: &mut DeepLeftovers, other: DeepLeftovers) {
    out.deletable.extend(other.deletable);
    out.review_only.extend(other.review_only);
}

// ============================================================================
// 1) UUID 容器解析
// ============================================================================

/// 目录名是否是 UUID 形态（8-4-4-4-12 十六进制）。
fn is_uuid_name(name: &str) -> bool {
    if name.len() != 36 {
        return false;
    }
    for (i, c) in name.bytes().enumerate() {
        let is_sep = matches!(i, 8 | 13 | 18 | 23);
        if is_sep {
            if c != b'-' {
                return false;
            }
        } else if !c.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// 读容器 metadata plist 的 MCMMetadataIdentifier（plutil + 超时，失败返回 None）。
fn container_metadata_identifier(dir: &str) -> Option<String> {
    let plist = format!("{dir}/.com.apple.containermanagerd.metadata.plist");
    if !Path::new(&plist).is_file() {
        return None;
    }
    run_with_timeout_capture(
        PROBE_TIMEOUT_SEC,
        "plutil",
        &[
            "-extract",
            "MCMMetadataIdentifier",
            "raw",
            "-o",
            "-",
            &plist,
        ],
    )
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

/// 遍历 `~/Library/Containers`，把归属本 app（含内嵌 bundle id）的 UUID 容器挑出来。
fn uuid_container_dirs(home: &str, identifiers: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let root = format!("{home}/Library/Containers");
    let Ok(rd) = std::fs::read_dir(&root) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !is_uuid_name(name) {
            continue; // 非 UUID 目录走 find_app_files 的边界匹配
        }
        let path_str = p.to_string_lossy().to_string();
        let Some(owner) = container_metadata_identifier(&path_str) else {
            continue;
        };
        if identifiers.contains(&owner.to_ascii_lowercase()) {
            out.push(path_str);
        }
    }
    out
}

/// app 内嵌的次级 bundle id（LoginItems/*.app、XPCServices/*.xpc、PlugIns/*.appex）。
/// 对齐 app_protection::embedded_bundle_ids 的口径，独立实现避免跨模块私有依赖。
fn embedded_bundle_ids(app_path: &str, primary: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let roots: [(String, &str); 3] = [
        (format!("{app_path}/Contents/Library/LoginItems"), "app"),
        (format!("{app_path}/Contents/XPCServices"), "xpc"),
        (format!("{app_path}/Contents/PlugIns"), "appex"),
    ];
    for (root, ext) in roots {
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !name.ends_with(&format!(".{ext}")) {
                continue;
            }
            let info = format!("{}/Contents/Info.plist", p.display());
            if !Path::new(&info).is_file() {
                continue;
            }
            let Some(id) = run_with_timeout_capture(
                PROBE_TIMEOUT_SEC,
                "plutil",
                &["-extract", "CFBundleIdentifier", "raw", "-o", "-", &info],
            )
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "(null)") else {
                continue;
            };
            if !is_valid_bundle_id(&id) || id == primary {
                continue;
            }
            // 共享框架服务不归每个 app 所有。
            if id.starts_with("org.sparkle-project.") {
                continue;
            }
            ids.push(id);
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

// ============================================================================
// 2) Group Container（entitlements application-groups）
// ============================================================================

/// codesign 读 app entitlements 的 application-groups 数组。
/// 流程：`codesign -d --entitlements :-` → 剥离 blob 头 → 临时文件 →
/// `plutil -extract ... json`。任一步失败返回空（降级为 find_app_files 的目录名边界匹配）。
fn app_group_identifiers(app_path: &str) -> Vec<String> {
    if !Path::new(app_path).is_dir() {
        return Vec::new();
    }
    let Ok(out) = Command::new("codesign")
        .args(["-d", "--entitlements", ":-", app_path])
        .output()
    else {
        return Vec::new();
    };
    // codesign 的 `:-` 输出带二进制 blob 头（`??<?xml` / bplist 前缀字节），
    // plutil -extract 对脏字节报错，必须先定位到真正的 plist 起点。
    let plist_bytes = strip_entitlement_blob_header(&out.stdout);
    if plist_bytes.is_empty() {
        return Vec::new();
    }
    let tmp = std::env::temp_dir().join(format!(
        "molestudio_entitlements_{}_{}.plist",
        std::process::id(),
        app_path.len()
    ));
    if std::fs::write(&tmp, plist_bytes).is_err() {
        return Vec::new();
    }
    let tmp_str = tmp.to_string_lossy().to_string();
    // 注意：不能用 `plutil -extract com.apple.security.application-groups`——
    // -extract 对带点的 key 路径会误解析成嵌套路径而报 No value at that key path。
    // 转 JSON 后自行取键。
    let json = run_with_timeout_capture(
        PROBE_TIMEOUT_SEC,
        "plutil",
        &["-convert", "json", "-o", "-", &tmp_str],
    );
    let _ = std::fs::remove_file(&tmp);
    let Some(json) = json else {
        return Vec::new();
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&json) else {
        return Vec::new();
    };
    // DER entitlements 可能把键包在一层 dict 里，递归找 application-groups。
    let Some(groups_value) = find_key_recursive(&root, "com.apple.security.application-groups")
    else {
        return Vec::new();
    };
    let Some(arr) = groups_value.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|g| {
            !g.is_empty()
                && g.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        })
        .collect()
}

/// 在 JSON 树里递归找指定键（深度优先，命中即返）。
fn find_key_recursive<'a>(
    value: &'a serde_json::Value,
    key: &str,
) -> Option<&'a serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(v) = map.get(key) {
                return Some(v);
            }
            for v in map.values() {
                if let Some(found) = find_key_recursive(v, key) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => arr.iter().find_map(|v| find_key_recursive(v, key)),
        _ => None,
    }
}

/// 剥离 entitlements 输出的 blob 头，返回合法 plist 起点之后的字节。
/// 支持 XML（`<?xml`）与二进制（`bplist`）两种格式；找不到返回空。
fn strip_entitlement_blob_header(bytes: &[u8]) -> &[u8] {
    if bytes.is_empty() {
        return &[];
    }
    if let Some(i) = bytes.windows(5).position(|w| w == b"<?xml") {
        return &bytes[i..];
    }
    if let Some(i) = bytes.windows(6).position(|w| w == b"bplist") {
        return &bytes[i..];
    }
    &[]
}

// ============================================================================
// 3) base bundle id 剥离
// ============================================================================

/// 需要剥离的次级服务后缀（对齐 Pearcleaner AppPathsFetch 的 base bundle id 逻辑）。
const BUNDLE_SUFFIX_STRIPS: &[&str] = &[
    ".helper",
    ".agent",
    ".daemon",
    ".service",
    ".xpc",
    ".launcher",
    ".updater",
    ".cli",
    ".menu",
    ".widget",
    ".extension",
    ".sysexthelper",
    ".fileprovider",
    ".browserextension",
    ".shareextension",
];

/// 反复剥离次级服务后缀，得到 base bundle id；无法剥离或剥完不合法返回 None。
fn base_bundle_id(bundle_id: &str) -> Option<String> {
    let mut cur = bundle_id.to_string();
    let mut stripped = false;
    loop {
        let lower = cur.to_ascii_lowercase();
        let hit = BUNDLE_SUFFIX_STRIPS
            .iter()
            .find(|s| lower.ends_with(**s))
            .copied();
        match hit {
            Some(s) => {
                cur.truncate(cur.len() - s.len());
                stripped = true;
            }
            None => break,
        }
    }
    if stripped && cur.split('.').count() >= 3 && is_valid_bundle_id(&cur) {
        Some(cur)
    } else {
        None
    }
}

/// base bundle id 派生路径。用户域进删除集，系统域（/Library）进审阅集。
fn base_id_paths(base: &str, home: &str) -> DeepLeftovers {
    let mut out = DeepLeftovers::default();

    // 用户域 LaunchAgents（可删）：名字 == base.plist 或 base.* 边界。
    let user_la = format!("{home}/Library/LaunchAgents");
    if let Ok(rd) = std::fs::read_dir(&user_la) {
        for e in rd.flatten() {
            let p = e.path();
            let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.ends_with(".plist") && name_boundary_matches(name, base) {
                out.deletable.push(p.to_string_lossy().to_string());
            }
        }
    }

    // 用户域 Preferences（可删）。
    let pref = format!("{home}/Library/Preferences/{base}.plist");
    if Path::new(&pref).is_file() {
        out.deletable.push(pref);
    }

    // 系统域：只审阅，不删除（执行期由 sudo 流程与用户确认另行处理）。
    for root in ["/Library/LaunchAgents", "/Library/LaunchDaemons"] {
        if let Ok(rd) = std::fs::read_dir(root) {
            for e in rd.flatten() {
                let p = e.path();
                let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if name.ends_with(".plist") && name_boundary_matches(name, base) {
                    out.review_only.push(p.to_string_lossy().to_string());
                }
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir("/Library/PrivilegedHelperTools") {
        for e in rd.flatten() {
            let p = e.path();
            let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name_boundary_matches(name, base) {
                out.review_only.push(p.to_string_lossy().to_string());
            }
        }
    }
    out
}

/// 名字 == base / base.* / *.base / *.base.*（bundle id 边界，防 com.foo 撞 com.foobar）。
fn name_boundary_matches(name: &str, base: &str) -> bool {
    if name == base {
        return true;
    }
    if let Some(rest) = name.strip_prefix(base) {
        if rest.starts_with('.') {
            return true;
        }
    }
    let dotted = format!(".{base}");
    name.ends_with(&dotted) || name.contains(&format!("{dotted}."))
}

// ============================================================================
// 4) Library 根 depth-2 厂商目录
// ============================================================================

/// depth-2 扫描时，root 下不进入第二层的标准系统子目录。
/// 对齐 Pearcleaner skipDeepSearch 思路：只允许厂商自建目录被下钻。
const LIBRARY_STANDARD_DIRS: &[&str] = &[
    "Application Support",
    "Application Scripts",
    "Accounts",
    "Assistant",
    "Audio",
    "Autosave Information",
    "Caches",
    "Calendars",
    "CloudKit",
    "CloudStorage",
    "Colors",
    "Compositions",
    "Containers",
    "Contextual Menu Items",
    "Cookies",
    "Desktop Pictures",
    "Developer",
    "Dictionaries",
    "Documentation",
    "Extensions",
    "Fonts",
    "Frameworks",
    "Games",
    "Group Containers",
    "HTTPStorages",
    "IdentityServices",
    "Input Methods",
    "Internet Plug-Ins",
    "Keychains",
    "Keyboard Layouts",
    "LanguageModeling",
    "LaunchAgents",
    "LaunchDaemons",
    "LinguisticData",
    "Logs",
    "Mail",
    "Maps",
    "Messages",
    "Metadata",
    "MigrationWizard",
    "Mobile Documents",
    "News",
    "OpenDirectory",
    "PDF Services",
    "PairedDevices",
    "Passes",
    "PersonalizationPortrait",
    "PreferencePanes",
    "Preferences",
    "Printing",
    "PrivilegedHelperTools",
    "ProtocolBuffer",
    "QuickLook",
    "Receipts",
    "Reminders",
    "Safari",
    "Saved Application State",
    "Screen Savers",
    "ScriptingAdditions",
    "Scripts",
    "Security",
    "Services",
    "Sounds",
    "Speech",
    "SpellCheckers",
    "Spotlight",
    "StartupItems",
    "Suggestions",
    "SyncServices",
    "SyncedPreferences",
    "SystemExtensions",
    "TrustedPeers",
    "Updates",
    "User Pictures",
    "VoiceServices",
    "WebKit",
    "WebServer",
    "WidgetKit",
    "Widgets",
    "Workflows",
];

/// 高频通用词：名字命中时禁止参与 depth-2 名字匹配（防误删）。
const COMMON_APP_WORDS: &[&str] = &[
    "Music",
    "Notes",
    "Photos",
    "Finder",
    "Safari",
    "Preview",
    "Calendar",
    "Contacts",
    "Messages",
    "Reminders",
    "Clock",
    "Weather",
    "Stocks",
    "Books",
    "News",
    "Podcasts",
    "Voice",
    "Files",
    "Store",
    "System",
    "Helper",
    "Agent",
    "Daemon",
    "Service",
    "Update",
    "Sync",
    "Backup",
    "Cloud",
    "Manager",
    "Monitor",
    "Server",
    "Client",
    "Worker",
    "Runner",
    "Launcher",
    "Driver",
    "Plugin",
    "Extension",
    "Widget",
    "Utility",
    "Mail",
    "Maps",
    "Terminal",
    "FaceTime",
    "Utilities",
];

/// app 名字的小写变体集（原样 / 无空格 / 连字符 / 下划线），短名与通用词拒绝。
fn app_name_variants(app_name: &str) -> Vec<String> {
    if app_name.len() < 4 || COMMON_APP_WORDS.iter().any(|w| *w == app_name) {
        return Vec::new();
    }
    let mut v = vec![
        app_name.to_ascii_lowercase(),
        app_name.replace(' ', "").to_ascii_lowercase(),
        app_name.replace(' ', "-").to_ascii_lowercase(),
        app_name.replace(' ', "_").to_ascii_lowercase(),
    ];
    v.sort();
    v.dedup();
    v.retain(|s| s.len() >= 4);
    v
}

/// 叶子名是否命中：名字变体精确相等，或 bundle id 边界。
fn leaf_matches(name: &str, variants: &[String], bundle_id: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if variants.iter().any(|v| *v == lower) {
        return true;
    }
    if is_valid_bundle_id(bundle_id) && name_boundary_matches(name, bundle_id) {
        return true;
    }
    false
}

/// Library 根 depth-2 扫描：
/// - depth-1 名字变体精确命中 → 收录（如 `/Library/LuLu`）；
/// - depth-1 是厂商自建目录（不在标准集、非 com.apple.*）→ 下钻一层，
///   叶子命中名字变体 / bundle id 边界 → 收录（如 `/Library/Objective-See/LuLu`）。
/// `/Library` 根下的命中进审阅集（系统域），`~/Library` 进删除集。
fn library_root_depth2(app_name: &str, bundle_id: &str, home: &str) -> DeepLeftovers {
    let mut out = DeepLeftovers::default();
    let variants = app_name_variants(app_name);
    if variants.is_empty() && !is_valid_bundle_id(bundle_id) {
        return out;
    }
    for (root, system_domain) in [
        ("/Library".to_string(), true),
        (format!("{home}/Library"), false),
    ] {
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            // depth-1 精确命中（如 /Library/LuLu）
            if leaf_matches(name, &variants, bundle_id) {
                push_domain(&mut out, p.to_string_lossy().to_string(), system_domain);
                continue;
            }
            // 标准系统目录不下钻
            if LIBRARY_STANDARD_DIRS.contains(&name) || name.starts_with("com.apple.") {
                continue;
            }
            // depth-2：厂商目录下的产品目录
            let Ok(rd2) = std::fs::read_dir(&p) else {
                continue;
            };
            for e2 in rd2.flatten() {
                let p2 = e2.path();
                let Some(n2) = p2.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if leaf_matches(n2, &variants, bundle_id) {
                    push_domain(&mut out, p2.to_string_lossy().to_string(), system_domain);
                }
            }
        }
    }
    out
}

fn push_domain(out: &mut DeepLeftovers, path: String, system_domain: bool) {
    if system_domain {
        out.review_only.push(path);
    } else {
        out.deletable.push(path);
    }
}

// ============================================================================
// 公共校验
// ============================================================================

/// reverse-DNS bundle id 校验（轻量版）：至少一个点、仅字母数字 . - _。
fn is_valid_bundle_id(s: &str) -> bool {
    if s.is_empty() || s == "unknown" || !s.contains('.') {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_name_shape() {
        assert!(is_uuid_name("F5B2C1D4-3E6F-4A7B-8C9D-0E1F2A3B4C5D"));
        assert!(is_uuid_name("f5b2c1d4-3e6f-4a7b-8c9d-0e1f2a3b4c5d"));
        assert!(!is_uuid_name("com.apple.Safari"));
        assert!(!is_uuid_name("F5B2C1D4-3E6F-4A7B-8C9D-0E1F2A3B4C5")); // 35 字符
        assert!(!is_uuid_name("F5B2C1D4-3E6F-4A7B-8C9D-0E1F2A3B4C5DE")); // 37 字符
        assert!(!is_uuid_name("F5B2C1D4X3E6F-4A7B-8C9D-0E1F2A3B4C5D")); // 非法字符
    }

    #[test]
    fn base_bundle_id_strips_helper_chain() {
        assert_eq!(
            base_bundle_id("com.objective-see.blockblock.helper"),
            Some("com.objective-see.blockblock".into())
        );
        assert_eq!(
            base_bundle_id("com.foo.app.updater.xpc"),
            Some("com.foo.app".into())
        );
        // 无后缀可剥
        assert_eq!(base_bundle_id("com.apple.Safari"), None);
        // 剥完不足三段
        assert_eq!(base_bundle_id("com.foo.helper"), None);
    }

    #[test]
    fn name_boundary_no_substring_hijack() {
        assert!(name_boundary_matches("com.foo.app.plist", "com.foo.app"));
        assert!(name_boundary_matches("com.foo.app", "com.foo.app"));
        assert!(name_boundary_matches(
            "com.foo.app.helper.plist",
            "com.foo.app"
        ));
        // com.evil.jetbrainsapp 不能劫持 jetbrains
        assert!(!name_boundary_matches(
            "com.evil.jetbrainsapp.plist",
            "jetbrains"
        ));
        assert!(!name_boundary_matches("com.foobar.plist", "com.foo"));
    }

    #[test]
    fn dedupe_parent_child_drops_children() {
        let input = vec![
            "~/Library/Application Support/Chrome/Default".to_string(),
            "~/Library/Application Support/Chrome".to_string(),
            "~/Library/Caches/com.foo.app".to_string(),
        ];
        let out = dedupe_parents_child(input);
        assert_eq!(out.len(), 2);
        assert!(out.contains(&"~/Library/Application Support/Chrome".to_string()));
        assert!(out.contains(&"~/Library/Caches/com.foo.app".to_string()));
    }

    #[test]
    fn dedupe_no_false_prefix() {
        // Chrome 与 ChromeBeta 不是父子关系
        let input = vec!["/A/Chrome".to_string(), "/A/ChromeBeta".to_string()];
        let out = dedupe_parents_child(input);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn name_variants_guard_short_and_common() {
        assert!(app_name_variants("Tim").is_empty()); // 短名
        assert!(app_name_variants("Music").is_empty()); // 通用词
        let v = app_name_variants("LuLu");
        assert!(v.contains(&"lulu".to_string()));
        let v2 = app_name_variants("Visual Studio Code");
        assert!(v2.contains(&"visualstudiocode".to_string()));
        assert!(v2.contains(&"visual-studio-code".to_string()));
    }

    #[test]
    fn leaf_matches_variants_and_bundle() {
        let variants = app_name_variants("LuLu");
        assert!(leaf_matches("LuLu", &variants, ""));
        assert!(leaf_matches("lulu", &variants, ""));
        assert!(!leaf_matches("LuLuHelper", &variants, ""));
        assert!(leaf_matches(
            "com.objective-see.lulu.plist",
            &[],
            "com.objective-see.lulu"
        ));
    }

    #[test]
    fn application_groups_from_json_variants() {
        // 平铺形态（XML entitlements）
        let flat = r#"{"com.apple.security.app-sandbox":true,"com.apple.security.application-groups":["TEAMID.com.foo.app","bad/group"]}"#;
        let root: serde_json::Value = serde_json::from_str(flat).unwrap();
        let groups = find_key_recursive(&root, "com.apple.security.application-groups").unwrap();
        let arr: Vec<String> = groups
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|g| {
                !g.is_empty()
                    && g.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
            })
            .collect();
        assert_eq!(arr, vec!["TEAMID.com.foo.app".to_string()]); // bad/group 被过滤
        // DER 包裹形态（嵌套 dict）
        let nested = r#"{"com.apple.security.app-sandbox":true,"der":{"com.apple.security.application-groups":["group.com.bar"]}}"#;
        let root2: serde_json::Value = serde_json::from_str(nested).unwrap();
        let found = find_key_recursive(&root2, "com.apple.security.application-groups").unwrap();
        assert_eq!(found[0].as_str(), Some("group.com.bar"));
    }

    #[test]
    fn entitlement_blob_header_stripping() {
        // XML 带脏字节前缀（codesign `:-` 的真实输出形态）
        let dirty = b"\xfa\xf7\x00\x00<?xml version=\"1.0\"?><plist></plist>";
        assert_eq!(
            strip_entitlement_blob_header(dirty),
            b"<?xml version=\"1.0\"?><plist></plist>"
        );
        // 二进制 plist
        let bpl = b"\x00\x00\x00bplist00\xd1";
        assert_eq!(strip_entitlement_blob_header(bpl), b"bplist00\xd1");
        // 无合法起点
        assert!(strip_entitlement_blob_header(b"\x01\x02\x03").is_empty());
        assert!(strip_entitlement_blob_header(b"").is_empty());
    }

    #[test]
    fn bundle_id_validation() {
        assert!(is_valid_bundle_id("com.apple.Safari"));
        assert!(!is_valid_bundle_id("unknown"));
        assert!(!is_valid_bundle_id(""));
        assert!(!is_valid_bundle_id("nodots"));
        assert!(!is_valid_bundle_id("com.foo/bar"));
    }
}
