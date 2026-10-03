//! 更新（Updates）模块探针测试：绕开 Tauri IPC / 前端，直接对 lib/updates 纯函数
//! 做行为验证（语义基线见 controllers/updates.md §6 对齐清单）。
//! 跑法：cargo test --test updates_probe -- --nocapture

use molan_lib::updates::appcast::parse_appcast;
use molan_lib::updates::brew::{brew_progress_phrase, parse_outdated};
use molan_lib::updates::detect::{UpdateSource, detect_update_source, feed_url};
use molan_lib::updates::itunes::parse_itunes_lookup;
use molan_lib::updates::version::{is_version_newer, os_is_installable};

/// 版本比较：去一个前导 v/V、非数字段归 0、缺段补 0、全等 false。
#[test]
fn probe_version_is_newer() {
    let cases: Vec<(&str, &str, bool)> = vec![
        // (remote, local, expected)
        ("1.2", "1.1", true),
        ("1.2", "1.2", false),   // 全等 false
        ("v1.3", "1.2", true),   // 小写 v 前缀
        ("V2.0", "1.9.9", true), // 大写 V 前缀
        ("1.2.3", "1.2", true),  // 缺段补 0
        ("1.2", "1.2.3", false),
        ("1.10", "1.9", true), // 数字段比较
        ("3.1.0", "3.1.0.1", false),
        ("2024b", "2023", false),   // 非数字段归 0：0 < 2023
        ("0", "2024b", false),      // 0 vs 0 → 全等 false
        ("2.0.0", "v2.0.0", false), // 两边都剥 v 后全等
        (" 1.5 ", "1.4", true),     // trim
    ];
    for (remote, local, expected) in cases {
        assert_eq!(
            is_version_newer(remote, local),
            expected,
            "is_version_newer({remote:?}, {local:?}) 应等于 {expected}"
        );
    }
}

/// 系统兼容门：minimum 空 → true；running >= minimum。
#[test]
fn probe_os_is_installable() {
    assert!(os_is_installable(None, "26.5.1"));
    assert!(os_is_installable(Some(""), "26.5.1"));
    assert!(os_is_installable(Some("  "), "26.5.1"));
    assert!(os_is_installable(Some("14"), "26.5.1"));
    assert!(os_is_installable(Some("26.5.0"), "26.5.0")); // 全等满足（atLeast 语义）
    assert!(os_is_installable(Some("26.5"), "26.5.1"));
    assert!(!os_is_installable(Some("26.5.2"), "26.5.1"));
    assert!(!os_is_installable(Some("27"), "26.5.1"));
}

/// appcast 解析：收集所有 enclosure 版本取版本比较最大值；
/// shortVersionString 优先于 version。
#[test]
fn probe_appcast_parse() {
    let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
<channel>
  <title>Test App</title>
  <item>
    <enclosure url="https://example.com/app-2.0.zip" sparkle:version="2.0" length="1" type="application/octet-stream"/>
  </item>
  <item>
    <enclosure url="https://example.com/app-3.1.zip" sparkle:shortVersionString="3.1.0" sparkle:version="3.1.0.1" length="1" type="application/octet-stream"/>
  </item>
  <item>
    <enclosure url="https://example.com/app-2.9.zip" sparkle:shortVersionString="2.9" length="1" type="application/octet-stream"/>
  </item>
</channel>
</rss>"#;
    // 三个版本 {2.0, 3.1.0, 2.9} → 最大 3.1.0（shortVersionString 优先于 version 3.1.0.1）
    assert_eq!(parse_appcast(xml).as_deref(), Some("3.1.0"));

    // 只有 sparkle:version 时回退
    let xml_v = r#"<rss><channel><item><enclosure sparkle:version="1.9" length="1"/></item></channel></rss>"#;
    assert_eq!(parse_appcast(xml_v).as_deref(), Some("1.9"));

    // 解析中途出错但已收集到版本 → 仍返回（宽容语义）
    let xml_broken = r#"<rss><channel><item><enclosure sparkle:version="2.5" length="1"/></item></channel><item><broken"#;
    assert_eq!(parse_appcast(xml_broken).as_deref(), Some("2.5"));

    // 无 enclosure → None
    assert_eq!(parse_appcast("<rss><channel></channel></rss>"), None);
}

/// iTunes lookup 解析：results 第一项；resultCount=0 → None。
#[test]
fn probe_itunes_parse() {
    let json = r#"{
      "resultCount": 2,
      "results": [
        {
          "version": "3.14.1",
          "trackViewUrl": "https://apps.apple.com/us/app/xxx/id123",
          "minimumOsVersion": "15.0"
        },
        {"version": "9.9"}
      ]
    }"#;
    let m = parse_itunes_lookup(json).expect("应解析出第一项");
    assert_eq!(m.version, "3.14.1");
    assert_eq!(
        m.page_url.as_deref(),
        Some("https://apps.apple.com/us/app/xxx/id123")
    );
    assert_eq!(m.minimum_os_version.as_deref(), Some("15.0"));

    assert!(parse_itunes_lookup(r#"{"resultCount":0,"results":[]}"#).is_none());
    assert!(parse_itunes_lookup("not json").is_none());
}

/// 更新源检测：MAS receipt > SUFeedURL > Electron Framework。
#[test]
fn probe_detect_sources() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    let mas_app = root.join("MASApp.app");
    std::fs::create_dir_all(mas_app.join("Contents/_MASReceipt")).unwrap();
    std::fs::write(mas_app.join("Contents/_MASReceipt/receipt"), b"x").unwrap();
    // 同时埋 SUFeedURL：receipt 优先（Receipt beats feed）
    write_plist_sufeed(&mas_app, "https://example.com/appcast.xml");
    assert_eq!(
        detect_update_source(mas_app.to_str().unwrap()),
        Some(UpdateSource::AppStore)
    );

    let sparkle_app = root.join("SparkleApp.app");
    std::fs::create_dir_all(sparkle_app.join("Contents")).unwrap();
    write_plist_sufeed(&sparkle_app, "https://example.com/appcast.xml");
    assert_eq!(
        detect_update_source(sparkle_app.to_str().unwrap()),
        Some(UpdateSource::Sparkle)
    );
    assert_eq!(
        feed_url(sparkle_app.to_str().unwrap()),
        "https://example.com/appcast.xml"
    );

    let electron_app = root.join("ElectronApp.app");
    std::fs::create_dir_all(electron_app.join("Contents/Frameworks/Electron Framework.framework"))
        .unwrap();
    assert_eq!(
        detect_update_source(electron_app.to_str().unwrap()),
        Some(UpdateSource::Electron)
    );

    let plain_app = root.join("PlainApp.app");
    std::fs::create_dir_all(plain_app.join("Contents")).unwrap();
    assert_eq!(detect_update_source(plain_app.to_str().unwrap()), None);
    assert_eq!(feed_url(plain_app.to_str().unwrap()), "");
}

fn write_plist_sufeed(app_dir: &std::path::Path, url: &str) {
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.example.test</string>
    <key>SUFeedURL</key>
    <string>{url}</string>
</dict>
</plist>"#
    );
    std::fs::write(app_dir.join("Contents/Info.plist"), plist).unwrap();
}

/// brew outdated 解析：installed 取 installed_versions **第一项**，缺省 "?"。
#[test]
fn probe_parse_outdated() {
    let json = r#"{
      "formulae": [
        {"name": "go", "installed_versions": ["1.22.5", "1.23.1"], "current_version": "1.24.0"}
      ],
      "casks": [
        {"name": "dbeaver-community", "installed_versions": ["25.2.0"], "current_version": "25.3.1"}
      ]
    }"#;
    let items = parse_outdated(json);
    assert_eq!(items.len(), 2);
    let go = &items[0];
    assert_eq!(go.name, "go");
    assert_eq!(go.kind, "formula");
    assert_eq!(go.installed, "1.22.5"); // first，不是 last
    assert_eq!(go.latest, "1.24.0");
    let dbeaver = &items[1];
    assert_eq!(dbeaver.name, "dbeaver-community");
    assert_eq!(dbeaver.kind, "cask");
    assert_eq!(dbeaver.installed, "25.2.0");
    assert_eq!(dbeaver.latest, "25.3.1");

    assert!(parse_outdated("not json").is_empty());
    assert!(parse_outdated("{}").is_empty());
}

/// 进度短语提取：`==> ` 前缀行取短语，其余噪声。
#[test]
fn probe_brew_progress_phrase() {
    assert_eq!(
        brew_progress_phrase("==> Pouring go--1.24.0.arm64_sonoma.bottle.tar.gz").as_deref(),
        Some("Pouring go--1.24.0.arm64_sonoma.bottle.tar.gz")
    );
    assert_eq!(
        brew_progress_phrase("  ==>   Cleaning up  ").as_deref(),
        Some("Cleaning up")
    );
    assert_eq!(brew_progress_phrase("==> "), None);
    assert_eq!(brew_progress_phrase("==>"), None);
    assert_eq!(brew_progress_phrase("Downloading..."), None);
    assert_eq!(brew_progress_phrase(""), None);
}
