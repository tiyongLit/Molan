//! Sparkle appcast 解析。
//! 对齐 Burrow `UpdateSources.parseAppcast(_:)`：收集所有 `<enclosure>` 的
//! `sparkle:shortVersionString`（优先，人类可读版本，与 CFBundleShortVersionString 可比）
//! / `sparkle:version`（回退），返回 `is_version_newer` 意义下的最大版本。

/// 解析 appcast XML，返回其中声明的最高版本。
/// 解析失败但已收集到版本时仍返回已有最大值（对齐 Burrow
/// `parser.parse() || !delegate.versions.isEmpty` 的宽容语义）。
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

/// isNewer 意义下的最大版本（相等不偏置，取先出现的）。
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
