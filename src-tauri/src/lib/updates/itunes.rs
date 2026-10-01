//! Mac App Store（iTunes lookup）查询与解析。
//! 解析 `results` 数组第一项的 version / trackViewUrl / minimumOsVersion。

use serde::{Deserialize, Serialize};

/// MAS lookup 结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasLookup {
    pub version: String,
    pub page_url: Option<String>,
    pub minimum_os_version: Option<String>,
}

/// iTunes lookup URL（bundleID 原样拼接，不做 URL 编码）。
pub fn itunes_lookup_url(bundle_id: &str) -> String {
    format!("https://itunes.apple.com/lookup?bundleId={bundle_id}")
}

/// 解析 lookup 返回：`results` 非空取第一项；`version` 缺失（如 resultCount=0）→ None。
pub fn parse_itunes_lookup(json: &str) -> Option<MasLookup> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let first = v.get("results")?.as_array()?.first()?;
    let version = first.get("version")?.as_str()?.to_string();
    Some(MasLookup {
        version,
        page_url: first
            .get("trackViewUrl")
            .and_then(|x| x.as_str())
            .map(String::from),
        minimum_os_version: first
            .get("minimumOsVersion")
            .and_then(|x| x.as_str())
            .map(String::from),
    })
}
