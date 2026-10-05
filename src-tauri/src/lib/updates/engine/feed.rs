//! 引擎侧 feed 逻辑：从目标 App 读取 Sparkle 元数据、从 appcast 条目中选择目标更新包。

use crate::updates::appcast::AppcastItem;
use crate::updates::version::is_version_newer;

/// 目标 App 的 Sparkle 元数据（均来自 `Contents/Info.plist`）。
#[derive(Debug, Clone, Default)]
pub struct SparkleMeta {
    /// `SUFeedURL`（空 = 不是 Sparkle 源或缺少配置）
    pub feed_url: String,
    /// `SUPublicEDKey`（base64 的 32 字节 Ed25519 公钥；
    /// 空 = Sparkle 1.x 或无 EdDSA 配置 → P0 不支持，回退交接）
    pub public_key_b64: String,
}

/// 读取目标 App 的 Sparkle 元数据（纯 plist 读取，无网络）。
pub fn read_sparkle_meta(app_path: &str) -> SparkleMeta {
    let plist_path = format!("{app_path}/Contents/Info.plist");
    let read_key = |key: &str| {
        crate::core::bundle_id_anchor::plist_string_key(std::path::Path::new(&plist_path), key)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_default()
    };
    SparkleMeta {
        feed_url: read_key("SUFeedURL"),
        public_key_b64: read_key("SUPublicEDKey"),
    }
}

/// 解析 SUFeedURL 上订阅的 channel（`?channel=beta` / `&channel=beta`）。
/// 无该参数 = 默认 stable 订阅。
pub fn subscribed_channel(feed_url: &str) -> Option<String> {
    let query = feed_url.split_once('?')?.1;
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            if key == "channel" && !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// 从 appcast 条目中选择要安装的目标条目（P0 口径）：
/// - 仅考虑**持有 `sparkle:edSignature` 且带下载 URL**的条目——没有签名就无法
///   通过门 1，绝不作为候选（宁可回退交接，也不降级安装）；
/// - 跳过 delta 条目（`sparkle:deltaFrom` 存在 = 增量包，P0 只用全量包）；
/// - **channel 过滤（对齐 Sparkle 语义）**：无 channel 标记的条目总是候选；
///   带 channel 标记的条目仅当与 `subscribed_channel` 一致时才是候选——
///   stable 订阅者不会收到 beta 条目（即便其版本更高）；
/// - 在剩余条目中选版本（`comparable_version`，与检查层同口径）比
///   `current_version` 新的**最高者**；同版本多条目时取靠后者（内容应一致）。
///
/// `minimumSystemVersion` 门不在此判断（检查层已有系统兼容门，由上层编排处理）。
pub fn select_update<'a>(
    items: &'a [AppcastItem],
    current_version: &str,
    subscribed_channel: Option<&str>,
) -> Option<&'a AppcastItem> {
    items
        .iter()
        .filter(|it| it.delta_from.is_none())
        .filter(|it| match it.channel.as_deref() {
            None => true,
            Some(c) => subscribed_channel == Some(c),
        })
        .filter(|it| {
            it.ed_signature
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
        })
        .filter(|it| {
            it.download_url
                .as_deref()
                .is_some_and(|u| !u.trim().is_empty())
        })
        .filter(|it| is_version_newer(it.comparable_version(), current_version))
        .max_by(|a, b| {
            let (av, bv) = (a.comparable_version(), b.comparable_version());
            if is_version_newer(av, bv) {
                std::cmp::Ordering::Greater
            } else if is_version_newer(bv, av) {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(version: &str, sig: Option<&str>, delta: Option<&str>) -> AppcastItem {
        AppcastItem {
            version: version.to_string(),
            short_version: Some(version.to_string()),
            download_url: Some(format!("https://example.com/{version}.zip")),
            ed_signature: sig.map(str::to_string),
            delta_from: delta.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn selects_highest_newer_signed_full_package() {
        let items = vec![
            item("3.5.13", Some("s"), None),
            item("3.5.14", Some("s"), None),
            item("3.5.15", Some("s"), Some("3.5.14")), // delta：跳过
            item("3.5.16", None, None),                // 无签名：跳过
            item("3.5.12", Some("s"), None),
        ];
        let picked = select_update(&items, "3.5.13", None).expect("应选中 3.5.14");
        assert_eq!(picked.version, "3.5.14");
    }

    #[test]
    fn no_newer_selects_none() {
        let items = vec![item("3.5.13", Some("s"), None)];
        assert!(select_update(&items, "3.5.14", None).is_none());
        assert!(select_update(&items, "3.5.13", None).is_none());
    }

    #[test]
    fn unsigned_only_selects_none() {
        let items = vec![item("9.9.9", None, None)];
        assert!(select_update(&items, "1.0", None).is_none());
    }

    #[test]
    fn missing_download_url_skipped() {
        let mut no_url = item("9.9.9", Some("s"), None);
        no_url.download_url = None;
        let items = vec![no_url];
        assert!(select_update(&items, "1.0", None).is_none());
    }

    fn with_channel(mut it: AppcastItem, channel: &str) -> AppcastItem {
        it.channel = Some(channel.to_string());
        it
    }

    #[test]
    fn stable_subscriber_skips_beta_even_if_newer() {
        let items = vec![
            item("3.5.14", Some("s"), None),
            with_channel(item("3.6.0", Some("s"), None), "beta"), // 更高但是 beta
        ];
        let picked = select_update(&items, "3.5.13", None).expect("stable 应选 3.5.14");
        assert_eq!(picked.version, "3.5.14");
    }

    #[test]
    fn beta_subscriber_receives_beta_and_stable() {
        let items = vec![
            item("3.5.14", Some("s"), None),
            with_channel(item("3.6.0", Some("s"), None), "beta"),
        ];
        let picked =
            select_update(&items, "3.5.13", Some("beta")).expect("beta 订阅应选中 3.6.0");
        assert_eq!(picked.version, "3.6.0");
    }

    #[test]
    fn beta_only_feed_gives_nothing_to_stable() {
        let items = vec![with_channel(item("3.6.0", Some("s"), None), "beta")];
        assert!(select_update(&items, "3.5.13", None).is_none());
    }

    #[test]
    fn parses_subscribed_channel_from_feed_url() {
        assert_eq!(
            subscribed_channel("https://iterm2.com/appcasts/final.xml?channel=beta"),
            Some("beta".to_string())
        );
        assert_eq!(
            subscribed_channel("https://iterm2.com/appcasts/final.xml?os=11&channel=beta"),
            Some("beta".to_string())
        );
        assert_eq!(
            subscribed_channel("https://iterm2.com/appcasts/final.xml"),
            None
        );
        assert_eq!(subscribed_channel("https://x/f.xml?channel="), None);
    }
}
