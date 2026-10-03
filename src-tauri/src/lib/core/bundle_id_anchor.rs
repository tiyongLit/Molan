//! Bundle ID 读取与锚定匹配（锚定匹配对齐 PureMac `AppPathFinder.swift#bundleIDMatchesCondition`）。
//!
//! 防止恶意 app 通过 substring 匹配劫持规则：
//! - `com.evil.jetbrainsapp` 不能命中 `jetbrains` 规则
//! - `com.todesktopX.malware` 不能命中 `com.todesktop` 的 Prefix 规则
//!
//! 三种合法匹配方式：
//!   1. 精确相等：`app == condition`
//!   2. 子级扩展：`app.starts_with(condition + ".")`
//!   3. 父级后缀：`app.ends_with("." + condition)`
//!
//! 拒绝裸 substring 匹配（`app.contains(condition)` 不合法）。

use std::path::Path;

// ---- Info.plist 读取（bundle id 与同族字符串键的唯一实现） ----

/// 读取 plist 顶层字符串键（纯 Rust，原 `plutil -extract <key> raw` / `defaults read` 的原生替代）。
///
/// 文件缺失、解析失败、键缺失或值非字符串时返回 None；值原样返回（不 trim），
/// 由调用方按自己的失败语义归一（None / 空串 / "unknown" 等）。
pub fn plist_string_key(plist_path: &Path, key: &str) -> Option<String> {
    match plist::Value::from_file(plist_path)
        .ok()
        .and_then(|v| v.into_dictionary())
        .and_then(|mut d| d.remove(key))
    {
        Some(plist::Value::String(s)) => Some(s),
        _ => None,
    }
}

/// 读取指定 `Info.plist`（完整路径）的 `CFBundleIdentifier`。
pub fn read_bundle_id_from_plist(plist_path: &Path) -> Option<String> {
    plist_string_key(plist_path, "CFBundleIdentifier")
}

/// 读取 `.app` 包（目录路径）的 `CFBundleIdentifier`。
pub fn read_bundle_id_of_app(app_path: &Path) -> Option<String> {
    read_bundle_id_from_plist(&app_path.join("Contents").join("Info.plist"))
}

/// 锚定式 bundle ID 匹配（对齐 PureMac `bundleIDMatchesCondition`）。
///
/// 两个参数均会被 normalize（小写 + trim）后比较。
/// 空 condition 始终返回 false（防止空字符串匹配一切）。
///
/// # Examples
/// ```
/// use molan_lib::core::bundle_id_anchor::bundle_id_matches_anchor;
/// assert!(bundle_id_matches_anchor("com.jetbrains.intellij", "com.jetbrains"));
/// assert!(bundle_id_matches_anchor("com.apple.dt.Xcode", "com.apple.dt.Xcode"));
/// assert!(!bundle_id_matches_anchor("com.evil.jetbrainsapp", "jetbrains"));
/// assert!(!bundle_id_matches_anchor("com.todesktopX.malware", "com.todesktop"));
/// ```
pub fn bundle_id_matches_anchor(app_bundle_id: &str, condition_bundle_id: &str) -> bool {
    let app = normalize(app_bundle_id);
    let cond = normalize(condition_bundle_id);
    if cond.is_empty() {
        return false;
    }
    // 1. 精确相等
    if app == cond {
        return true;
    }
    // 2. 子级扩展：app 以 "condition." 开头
    if app.starts_with(&format!("{cond}.")) {
        return true;
    }
    // 3. 父级后缀：app 以 ".condition" 结尾
    if app.ends_with(&format!(".{cond}")) {
        return true;
    }
    false
}

/// 判断 app_bundle_id 是否属于 condition_bundle_id 的"家族"。
///
/// 比 [`bundle_id_matches_anchor`] 更宽松：额外接受 base_bundle_id 剥离后的匹配。
/// 用于 `leftovers.rs` 的深度扫描场景，抓 helper/agent/daemon 派生路径。
///
/// 例：`com.foo.app.helper` 属于 `com.foo.app` 家族（剥离 `.helper` 后精确匹配）。
pub fn bundle_id_in_family(app_bundle_id: &str, condition_bundle_id: &str) -> bool {
    // 先走标准锚定匹配
    if bundle_id_matches_anchor(app_bundle_id, condition_bundle_id) {
        return true;
    }
    // 尝试剥离 app 的次级服务后缀后再匹配
    let app = normalize(app_bundle_id);
    let cond = normalize(condition_bundle_id);
    if let Some(base) = strip_service_suffix(&app) {
        if base == cond
            || base.starts_with(&format!("{cond}."))
            || base.ends_with(&format!(".{cond}"))
        {
            return true;
        }
    }
    // 尝试剥离 condition 的次级服务后缀后再匹配
    if let Some(base_cond) = strip_service_suffix(&cond) {
        if app == base_cond
            || app.starts_with(&format!("{base_cond}."))
            || app.ends_with(&format!(".{base_cond}"))
        {
            return true;
        }
    }
    false
}

/// 从 bundle_id 中提取公司名（第二段），用于 depth-2 厂商目录匹配。
///
/// 要求至少 3 段且第二段长度 >= 3（防短名碰撞如 "a.b" 中的 "b"）。
///
/// # Examples
/// ```
/// use molan_lib::core::bundle_id_anchor::bundle_company_name;
/// assert_eq!(bundle_company_name("com.jetbrains.intellij"), Some("jetbrains".to_string()));
/// assert_eq!(bundle_company_name("com.apple.Safari"), Some("apple".to_string()));
/// assert_eq!(bundle_company_name("com.foo"), None); // 只有 2 段
/// assert_eq!(bundle_company_name("a.b.c"), None);    // 第二段 "b" 长度 < 3
/// ```
pub fn bundle_company_name(bundle_id: &str) -> Option<String> {
    let normalized = normalize(bundle_id);
    let parts: Vec<&str> = normalized.split('.').collect();
    if parts.len() < 3 {
        return None;
    }
    let company = parts[1];
    if company.len() < 3 {
        return None;
    }
    Some(company.to_string())
}

// ---- 内部工具 ----

/// 归一化：小写 + trim 首尾空白。
fn normalize(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

/// 需要剥离的次级服务后缀（对齐 PureMac / Pearcleaner base bundle id 逻辑）。
const SERVICE_SUFFIXES: &[&str] = &[
    ".helper",
    ".agent",
    ".daemon",
    ".service",
    ".xpc",
    ".launcher",
    ".updater",
    ".installer",
    ".uninstaller",
    ".watcher",
    ".monitor",
    ".bridge",
    ".proxy",
    ".cli",
    ".tool",
];

/// 反复剥离次级服务后缀，得到 base bundle id。
/// 无法剥离或剥完不合法（少于 2 段）时返回 None。
fn strip_service_suffix(bundle_id: &str) -> Option<String> {
    let mut current = bundle_id.to_string();
    let mut stripped = false;
    loop {
        let mut found = false;
        for suffix in SERVICE_SUFFIXES {
            if current.ends_with(suffix) {
                current = current[..current.len() - suffix.len()].to_string();
                found = true;
                stripped = true;
                break;
            }
        }
        if !found {
            break;
        }
    }
    if !stripped {
        return None;
    }
    // 剥完至少要有 2 段（如 com.foo）
    if current.split('.').count() < 2 || current.is_empty() {
        return None;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- bundle_id_matches_anchor ----

    #[test]
    fn exact_match() {
        assert!(bundle_id_matches_anchor(
            "com.apple.dt.Xcode",
            "com.apple.dt.Xcode"
        ));
    }

    #[test]
    fn exact_match_case_insensitive() {
        assert!(bundle_id_matches_anchor(
            "com.apple.dt.xcode",
            "com.apple.dt.Xcode"
        ));
    }

    #[test]
    fn child_extension_match() {
        assert!(bundle_id_matches_anchor(
            "com.jetbrains.intellij",
            "com.jetbrains"
        ));
        assert!(bundle_id_matches_anchor(
            "com.jetbrains.intellij.ce",
            "com.jetbrains.intellij"
        ));
    }

    #[test]
    fn parent_suffix_match() {
        assert!(bundle_id_matches_anchor(
            "com.jetbrains.intellij",
            "intellij"
        ));
    }

    #[test]
    fn rejects_substring_hijack() {
        // 恶意 app 不能通过 substring 命中规则
        assert!(!bundle_id_matches_anchor(
            "com.evil.jetbrainsapp",
            "jetbrains"
        ));
        assert!(!bundle_id_matches_anchor(
            "com.todesktopX.malware",
            "com.todesktop"
        ));
    }

    #[test]
    fn rejects_empty_condition() {
        assert!(!bundle_id_matches_anchor("com.foo.bar", ""));
        assert!(!bundle_id_matches_anchor("com.foo.bar", "  "));
    }

    #[test]
    fn prefix_with_dot_boundary() {
        assert!(bundle_id_matches_anchor(
            "com.todesktop.abc123",
            "com.todesktop"
        ));
        // 无 . 边界 → 拒绝
        assert!(!bundle_id_matches_anchor(
            "com.todesktopX.abc123",
            "com.todesktop"
        ));
    }

    // ---- bundle_id_in_family ----

    #[test]
    fn family_includes_helper_stripping() {
        // com.foo.app.helper 属于 com.foo.app 家族
        assert!(bundle_id_in_family("com.foo.app.helper", "com.foo.app"));
        // com.foo.app.updater.xpc 属于 com.foo.app 家族
        assert!(bundle_id_in_family(
            "com.foo.app.updater.xpc",
            "com.foo.app"
        ));
    }

    #[test]
    fn family_still_does_exact_match() {
        assert!(bundle_id_in_family("com.foo.app", "com.foo.app"));
        assert!(bundle_id_in_family("com.foo.app.child", "com.foo.app"));
    }

    #[test]
    fn family_rejects_unrelated() {
        assert!(!bundle_id_in_family("com.evil.app", "com.foo.app"));
    }

    // ---- bundle_company_name ----

    #[test]
    fn company_name_extraction() {
        assert_eq!(
            bundle_company_name("com.jetbrains.intellij"),
            Some("jetbrains".to_string())
        );
        assert_eq!(
            bundle_company_name("com.apple.Safari"),
            Some("apple".to_string())
        );
    }

    #[test]
    fn company_name_rejects_short() {
        // 只有 2 段
        assert_eq!(bundle_company_name("com.foo"), None);
        // 第二段长度 < 3
        assert_eq!(bundle_company_name("a.b.c"), None);
    }

    #[test]
    fn company_name_rejects_empty() {
        assert_eq!(bundle_company_name(""), None);
    }

    // ---- strip_service_suffix ----

    #[test]
    fn strips_helper_chain() {
        assert_eq!(
            strip_service_suffix("com.objective-see.blockblock.helper"),
            Some("com.objective-see.blockblock".to_string())
        );
        assert_eq!(
            strip_service_suffix("com.foo.app.updater.xpc"),
            Some("com.foo.app".to_string())
        );
    }

    #[test]
    fn no_strip_returns_none() {
        assert_eq!(strip_service_suffix("com.apple.Safari"), None);
    }

    // ---- Info.plist 读取 ----

    #[test]
    fn read_bundle_id_of_app_reads_info_plist() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Foo.app");
        let contents = app.join("Contents");
        std::fs::create_dir_all(&contents).unwrap();
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.example.foo</string>
</dict></plist>"#;
        std::fs::write(contents.join("Info.plist"), plist).unwrap();
        assert_eq!(
            read_bundle_id_of_app(&app).as_deref(),
            Some("com.example.foo")
        );
    }

    #[test]
    fn read_bundle_id_of_app_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_bundle_id_of_app(&dir.path().join("Missing.app")), None);
    }

    #[test]
    fn plist_string_key_non_string_value_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let plist_path = dir.path().join("Sample.plist");
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><integer>42</integer>
</dict></plist>"#;
        std::fs::write(&plist_path, plist).unwrap();
        assert_eq!(plist_string_key(&plist_path, "CFBundleIdentifier"), None);
    }

    #[test]
    fn plist_string_key_returns_raw_value() {
        // 值原样返回（不 trim），trim 由各调用方按原语义处理。
        let dir = tempfile::tempdir().unwrap();
        let plist_path = dir.path().join("Sample.plist");
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>Key</key><string>  spaced  </string>
</dict></plist>"#;
        std::fs::write(&plist_path, plist).unwrap();
        assert_eq!(
            plist_string_key(&plist_path, "Key"),
            Some("  spaced  ".to_string())
        );
    }
}
