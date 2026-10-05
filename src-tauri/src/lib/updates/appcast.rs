//! Sparkle appcast 解析。
//! 收集所有 `<enclosure>` 的
//! `sparkle:shortVersionString`（优先，人类可读版本，与 CFBundleShortVersionString 可比）
//! / `sparkle:version`（回退），返回 `is_version_newer` 意义下的最大版本。
//!
//! 更新执行引擎（`engine/`）另用 [`parse_appcast_items`] 做完整条目解析
//! （下载 URL / edSignature / 系统门槛 / delta 标记），供原地更新选择目标包。

/// 解析 appcast XML，返回其中声明的最高版本。
/// 解析失败但已收集到版本时仍返回已有最大值（宽容语义）。
pub fn parse_appcast(xml: &str) -> Option<String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut versions: Vec<String> = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) | Ok(quick_xml::events::Event::Empty(e)) => {
                if e.name().as_ref() == b"enclosure" {
                    if let Some(v) = enclosure_version(&e) {
                        versions.push(v);
                    }
                }
            }
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    max_version(versions)
}

/// 单个 `<enclosure>` 的版本：优先 `sparkle:shortVersionString`，无则 `sparkle:version`。
fn enclosure_version(e: &quick_xml::events::BytesStart) -> Option<String> {
    let mut short: Option<String> = None;
    let mut raw: Option<String> = None;
    for attr in e.attributes().flatten() {
        match attr.key.as_ref() {
            b"sparkle:shortVersionString" => {
                short = attr.unescape_value().ok().map(|c| c.into_owned())
            }
            b"sparkle:version" => raw = attr.unescape_value().ok().map(|c| c.into_owned()),
            _ => {}
        }
    }
    short.or(raw)
}

/// 版本比较意义下的最大版本（相等不偏置，取先出现的）。
fn max_version(versions: Vec<String>) -> Option<String> {
    versions.into_iter().max_by(|a, b| {
        if super::version::is_version_newer(a, b) {
            std::cmp::Ordering::Greater
        } else if super::version::is_version_newer(b, a) {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    })
}

// ── 完整条目解析（更新执行引擎用）──

/// 单个 appcast `<item>` 的完整关键字段（口径对齐 Sparkle 2）：
/// - `version`：`sparkle:version`（enclosure 属性优先，item 子元素回退）；
/// - `short_version`：`sparkle:shortVersionString`（同优先级规则）；
/// - `download_url` / `length` / `content_type` / `ed_signature` / `delta_from`
///   取自 `<enclosure>` 属性；
/// - `minimum_system_version` / `channel` 取自 item 子元素。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppcastItem {
    pub version: String,
    pub short_version: Option<String>,
    pub download_url: Option<String>,
    pub length: Option<u64>,
    pub content_type: Option<String>,
    pub ed_signature: Option<String>,
    /// 有值 = 增量包（P0 只用全量包，引擎选择时跳过）
    pub delta_from: Option<String>,
    pub minimum_system_version: Option<String>,
    pub channel: Option<String>,
}

impl AppcastItem {
    /// 版本比较口径（与检查层 `parse_appcast` 一致）：shortVersionString 优先，回退 version。
    pub fn comparable_version(&self) -> &str {
        self.short_version
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.version)
    }
}

/// 完整解析 appcast，返回所有有效 `<item>`（`version` 为空或缺失的条目跳过）。
/// 宽容语义：XML 中途损坏时返回已收集到的条目（与 `parse_appcast` 一致）。
pub fn parse_appcast_items(xml: &str) -> Vec<AppcastItem> {
    use quick_xml::events::Event;

    let mut reader = quick_xml::Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut items: Vec<AppcastItem> = Vec::new();
    let mut cur: Option<AppcastItem> = None;
    // 当前正在收集文本的 item 级元素名（`sparkle:*` 叶子元素）
    let mut text_key: Option<Vec<u8>> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"item" => cur = Some(AppcastItem::default()),
                b"enclosure" => {
                    if let Some(it) = cur.as_mut() {
                        apply_enclosure(it, &e);
                    }
                }
                name @ (b"sparkle:version"
                | b"sparkle:shortVersionString"
                | b"sparkle:minimumSystemVersion"
                | b"sparkle:channel") => text_key = Some(name.to_vec()),
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if e.name().as_ref() == b"enclosure" {
                    if let Some(it) = cur.as_mut() {
                        apply_enclosure(it, &e);
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if let (Some(key), Some(it)) = (text_key.as_deref(), cur.as_mut()) {
                    // quick-xml 0.41：先按 XML 1.0 解码内容，再解析实体引用。
                    if let Ok(txt) = t.xml10_content() {
                        if let Ok(s) = quick_xml::escape::unescape(&txt) {
                            let s = s.trim();
                            if !s.is_empty() {
                                apply_item_text(it, key, s);
                            }
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                if e.name().as_ref() == b"item" {
                    if let Some(it) = cur.take() {
                        if !it.version.is_empty() {
                            items.push(it);
                        }
                    }
                }
                if text_key.as_deref() == Some(e.name().as_ref()) {
                    text_key = None;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    items
}

/// enclosure 属性写入（属性优先于 item 子元素：无条件覆盖，text 只填空）。
fn apply_enclosure(it: &mut AppcastItem, e: &quick_xml::events::BytesStart) {
    for attr in e.attributes().flatten() {
        let key = attr.key.as_ref();
        let Ok(value) = attr.normalized_value(quick_xml::XmlVersion::Implicit1_0) else {
            continue;
        };
        let value = value.into_owned();
        match key {
            b"url" => it.download_url = Some(value),
            b"length" => it.length = value.parse().ok(),
            b"type" => it.content_type = Some(value),
            b"sparkle:edSignature" => it.ed_signature = Some(value),
            b"sparkle:deltaFrom" => it.delta_from = Some(value),
            b"sparkle:version" => {
                if !value.is_empty() {
                    it.version = value;
                }
            }
            b"sparkle:shortVersionString" => {
                if !value.is_empty() {
                    it.short_version = Some(value);
                }
            }
            _ => {}
        }
    }
}

/// item 子元素文本写入（只在字段为空时填，保证 enclosure 属性优先）。
fn apply_item_text(it: &mut AppcastItem, key: &[u8], value: &str) {
    match key {
        b"sparkle:version" => {
            if it.version.is_empty() {
                it.version = value.to_string();
            }
        }
        b"sparkle:shortVersionString" => {
            if it.short_version.is_none() {
                it.short_version = Some(value.to_string());
            }
        }
        b"sparkle:minimumSystemVersion" => {
            if it.minimum_system_version.is_none() {
                it.minimum_system_version = Some(value.to_string());
            }
        }
        b"sparkle:channel" => {
            if it.channel.is_none() {
                it.channel = Some(value.to_string());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>iTerm2</title>
    <item>
      <title>3.5.13</title>
      <sparkle:version>3.5.13</sparkle:version>
      <sparkle:shortVersionString>3.5.13</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>10.15</sparkle:minimumSystemVersion>
      <enclosure url="https://iterm2.com/downloads/stable/iTerm2-3_5_13.zip"
        sparkle:version="3.5.13" sparkle:shortVersionString="3.5.13"
        length="29310551" type="application/octet-stream"
        sparkle:edSignature="AAAA"/>
    </item>
    <item>
      <title>3.5.14</title>
      <sparkle:version>3.5.14</sparkle:version>
      <sparkle:shortVersionString>3.5.14</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>10.15</sparkle:minimumSystemVersion>
      <sparkle:channel>stable</sparkle:channel>
      <enclosure url="https://iterm2.com/downloads/stable/iTerm2-3_5_14.zip"
        sparkle:version="3.5.14" sparkle:shortVersionString="3.5.14"
        length="29400000" type="application/octet-stream"
        sparkle:edSignature="BBBB"/>
    </item>
  </channel>
</rss>"#;

    #[test]
    fn parses_full_items() {
        let items = parse_appcast_items(SAMPLE);
        assert_eq!(items.len(), 2);
        let it = &items[1];
        assert_eq!(it.version, "3.5.14");
        assert_eq!(it.short_version.as_deref(), Some("3.5.14"));
        assert_eq!(
            it.download_url.as_deref(),
            Some("https://iterm2.com/downloads/stable/iTerm2-3_5_14.zip")
        );
        assert_eq!(it.length, Some(29_400_000));
        assert_eq!(it.ed_signature.as_deref(), Some("BBBB"));
        assert_eq!(it.minimum_system_version.as_deref(), Some("10.15"));
        assert_eq!(it.channel.as_deref(), Some("stable"));
        assert!(it.delta_from.is_none());
        assert_eq!(it.comparable_version(), "3.5.14");
    }

    #[test]
    fn enclosure_attributes_take_priority_over_item_elements() {
        // text 先、enclosure 后：属性覆盖
        let xml = r#"<rss><channel><item>
          <sparkle:version>100</sparkle:version>
          <sparkle:shortVersionString>1.0</sparkle:shortVersionString>
          <enclosure url="https://x/y.zip" sparkle:version="101" sparkle:shortVersionString="1.1" sparkle:edSignature="s"/>
        </item></channel></rss>"#;
        let items = parse_appcast_items(xml);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].version, "101");
        assert_eq!(items[0].short_version.as_deref(), Some("1.1"));

        // enclosure 先、text 后：text 不覆盖已写入的字段
        let xml2 = r#"<rss><channel><item>
          <enclosure url="https://x/y.zip" sparkle:version="201" sparkle:shortVersionString="2.1" sparkle:edSignature="s"/>
          <sparkle:version>200</sparkle:version>
          <sparkle:shortVersionString>2.0</sparkle:shortVersionString>
        </item></channel></rss>"#;
        let items2 = parse_appcast_items(xml2);
        assert_eq!(items2[0].version, "201");
        assert_eq!(items2[0].short_version.as_deref(), Some("2.1"));
    }

    #[test]
    fn delta_item_keeps_delta_marker() {
        let xml = r#"<rss><channel><item>
          <sparkle:version>3.5.14</sparkle:version>
          <enclosure url="https://x/delta.zip" sparkle:version="3.5.14" sparkle:deltaFrom="3.5.13" sparkle:edSignature="s"/>
        </item></channel></rss>"#;
        let items = parse_appcast_items(xml);
        assert_eq!(items[0].delta_from.as_deref(), Some("3.5.13"));
    }

    #[test]
    fn versionless_item_skipped_and_broken_xml_keeps_collected() {
        let xml = r#"<rss><channel>
          <item><title>x</title></item>
          <item><sparkle:version>2.0</sparkle:version><enclosure url="https://x/y.zip"/></item>
        </channel><broken"#;
        let items = parse_appcast_items(xml);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].version, "2.0");
    }
}
