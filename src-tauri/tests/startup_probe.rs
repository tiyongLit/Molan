//! 启动项模块探针测试：plist 解析 / 目录扫描 / BTM 合并 / launchctl 冒烟。

use mole_lib::startup::{
    control,
    inventory::{self, StartupKind, StartupProblem, StartupScope},
    login_items,
};

/// 写一个 plist 文件（XML 格式）。
fn write_plist(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
    let p = dir.join(format!("{name}.plist"));
    std::fs::write(&p, content).unwrap();
    p
}

#[test]
fn probe_item_from_plist_full() {
    let dir = tempfile::tempdir().unwrap();
    let p = write_plist(
        dir.path(),
        "com.example.agent",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>com.example.agent</string>
    <key>Program</key><string>/bin/ls</string>
</dict>
</plist>"#,
    );
    let item = inventory::item_from_plist(&p, StartupKind::LaunchAgent, StartupScope::User);
    assert_eq!(item.label, "com.example.agent");
    assert_eq!(item.executable.as_deref(), Some("/bin/ls"));
    assert_eq!(item.kind, StartupKind::LaunchAgent);
    assert_eq!(item.scope, StartupScope::User);
    assert_eq!(item.problem, None);
    assert!(item.controllable()); // user + agent + 非 bundled + 无 problem
}

#[test]
fn probe_item_from_plist_program_arguments_first() {
    let dir = tempfile::tempdir().unwrap();
    let p = write_plist(
        dir.path(),
        "com.example.args",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>com.example.args</string>
    <key>ProgramArguments</key>
    <array><string>/usr/bin/osascript</string><string>-e</string><string>say hi</string></array>
</dict>
</plist>"#,
    );
    let item = inventory::item_from_plist(&p, StartupKind::LaunchAgent, StartupScope::User);
    // Program 缺失 → ProgramArguments.first
    assert_eq!(item.executable.as_deref(), Some("/usr/bin/osascript"));
    assert_eq!(item.problem, None);
}

#[test]
fn probe_item_from_plist_parse_failed_keeps_row() {
    let dir = tempfile::tempdir().unwrap();
    let p = write_plist(
        dir.path(),
        "com.corrupt.daemon",
        "this is not a plist at all",
    );
    let item = inventory::item_from_plist(&p, StartupKind::LaunchDaemon, StartupScope::System);
    // 失败分类、不消失：fallback label = 文件名
    assert_eq!(item.label, "com.corrupt.daemon");
    assert_eq!(item.problem, Some(StartupProblem::ParseFailed));
    assert_eq!(item.executable, None);
    assert!(!item.controllable());
}

#[test]
fn probe_item_from_plist_label_missing_falls_back_to_filename() {
    let dir = tempfile::tempdir().unwrap();
    let p = write_plist(
        dir.path(),
        "no-label",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict><key>Program</key><string>/bin/true</string></dict>
</plist>"#,
    );
    let item = inventory::item_from_plist(&p, StartupKind::LaunchAgent, StartupScope::User);
    assert_eq!(item.label, "no-label");
}

#[test]
fn probe_item_from_plist_dangling_executable() {
    let dir = tempfile::tempdir().unwrap();
    let p = write_plist(
        dir.path(),
        "com.gone.agent",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>com.gone.agent</string>
    <key>Program</key><string>/definitely/not/here/binary</string>
</dict>
</plist>"#,
    );
    let item = inventory::item_from_plist(&p, StartupKind::LaunchAgent, StartupScope::User);
    assert_eq!(item.problem, Some(StartupProblem::DanglingExecutable));
    assert!(!item.controllable());
}

#[test]
fn probe_scan_sorted_and_plist_only() {
    let dir = tempfile::tempdir().unwrap();
    write_plist(
        dir.path(),
        "zzz.agent",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>Label</key><string>zzz.agent</string></dict></plist>"#,
    );
    write_plist(
        dir.path(),
        "aaa.agent",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>Label</key><string>aaa.agent</string></dict></plist>"#,
    );
    std::fs::write(dir.path().join("notes.txt"), "not a plist").unwrap();
    std::fs::write(dir.path().join("data.plist.bak"), "backup").unwrap();

    let items = inventory::scan(
        dir.path().to_str().unwrap(),
        StartupKind::LaunchAgent,
        StartupScope::User,
    );
    assert_eq!(items.len(), 2); // .bak/.txt 排除
    assert_eq!(items[0].label, "aaa.agent"); // 文件名排序
    assert_eq!(items[1].label, "zzz.agent");
}

#[test]
fn probe_scan_missing_directory_is_empty() {
    let items = inventory::scan(
        "/definitely/not/exists/dir",
        StartupKind::LaunchAgent,
        StartupScope::User,
    );
    assert!(items.is_empty());
}

#[test]
fn probe_merge_end_to_end() {
    // plist 清单 + BTM dump 全链路（对齐 scanLiveIncludingLoginItems 的纯函数部分）
    let plist = vec![inventory::StartupItem {
        label: "com.existing.agent".into(),
        kind: StartupKind::LaunchAgent,
        scope: StartupScope::User,
        plist_path: "/tmp/com.existing.agent.plist".into(),
        executable: Some("/bin/true".into()),
        problem: None,
    }];
    let dump = r#"#42:
        Name: My Login Helper
        Identifier: 42.com.fresh.helper
        Type: developer (1)
        Disposition: [enabled]
"#;
    let merged = inventory::merge(plist, login_items::parse(dump));
    assert_eq!(merged.len(), 2);
    let extra = &merged[1];
    assert_eq!(extra.kind, StartupKind::LoginItem);
    assert_eq!(extra.plist_path, "btm:42.com.fresh.helper");
    assert!(!extra.controllable()); // 登录项 review-only
}

#[test]
fn probe_disabled_labels_smoke() {
    // 真实 launchctl print-disabled gui/$uid：可能为空，但不应 panic
    let labels = control::disabled_labels();
    // 返回类型语义正确即可；内容依赖本机状态，不做具体断言
    let _ = labels.len();
}
