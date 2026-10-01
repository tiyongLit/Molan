//! 更新来源检测（纯本地文件系统检查，零网络）。
//! 检测优先级：MAS receipt > SUFeedURL > Electron Framework。

use serde::Serialize;

/// 更新机制来源。homebrew 单独走 brew 行（`brew outdated`），不在 app 来源检测内。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateSource {
    Sparkle,
    AppStore,
    Electron,
}

impl UpdateSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            UpdateSource::Sparkle => "sparkle",
            UpdateSource::AppStore => "app_store",
            UpdateSource::Electron => "electron",
        }
    }
}

/// 检测 app 的更新机制。优先级（Receipt beats feed）：
/// 1. `Contents/_MASReceipt/receipt` 存在 → AppStore（MAS 拷贝走商店更新，即使内嵌 Sparkle）
/// 2. Info.plist 含 `SUFeedURL` key → Sparkle
/// 3. `Contents/Frameworks/Electron Framework.framework` 存在 → Electron
/// 4. 否则 None（无已知自更新机制，进 uncheckable 分区）
pub fn detect_update_source(app_path: &str) -> Option<UpdateSource> {
    let contents = format!("{}/Contents", app_path);
    if std::path::Path::new(&format!("{contents}/_MASReceipt/receipt")).exists() {
        return Some(UpdateSource::AppStore);
    }
    if info_has_sufeed_url(app_path) {
        return Some(UpdateSource::Sparkle);
    }
    if std::path::Path::new(&format!(
        "{contents}/Frameworks/Electron Framework.framework"
    ))
    .exists()
    {
        return Some(UpdateSource::Electron);
    }
    None
}

/// Info.plist 是否含 `SUFeedURL` key（只查 key 存在，不校验值）。
fn info_has_sufeed_url(app_path: &str) -> bool {
    let plist_path = format!("{app_path}/Contents/Info.plist");
    if !std::path::Path::new(&plist_path).is_file() {
        return false;
    }
    plist::Value::from_file(&plist_path)
        .ok()
        .and_then(|v| v.into_dictionary())
        .map(|d| d.contains_key("SUFeedURL"))
        .unwrap_or(false)
}

/// app 的 Sparkle feed URL（要求值是 String）。
/// 非空返回；用于 `mole_updates_check` 拉 appcast。
pub fn feed_url(app_path: &str) -> String {
    let plist_path = format!("{app_path}/Contents/Info.plist");
    if !std::path::Path::new(&plist_path).is_file() {
        return String::new();
    }
    crate::core::bundle_id_anchor::plist_string_key(std::path::Path::new(&plist_path), "SUFeedURL")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_default()
}
