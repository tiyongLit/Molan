//! 临时诊断探针：绕开 Tauri IPC，直接对真实已安装应用跑
//! `leftovers::scan_deep_leftovers`（UUID 容器 / Group Container / base bundle id /
//! Library depth-2），验证深度扫描在真实机器上的命中情况。
//! 跑法：cargo test --test uninstall_deep_probe -- --nocapture

use molestudio_lib::uninstall::leftovers::scan_deep_leftovers;

/// 读 Info.plist 的 CFBundleIdentifier。
fn read_bundle_id(app_path: &str) -> String {
    let plist = format!("{app_path}/Contents/Info.plist");
    std::process::Command::new("plutil")
        .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-", &plist])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// 从 /Applications 挑若干真实应用，跑深度残留扫描并打印命中。
#[test]
fn probe_deep_leftovers_on_real_apps() {
    let mut apps: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/Applications") {
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() && name.ends_with(".app") {
                apps.push(p.to_string_lossy().to_string());
            }
            if apps.len() >= 40 {
                break;
            }
        }
    }
    assert!(!apps.is_empty(), "/Applications 下没有任何 .app，无法验证");

    let mut total_deletable = 0usize;
    let mut total_review = 0usize;
    let mut hit_apps = 0usize;

    for app in &apps {
        let name = std::path::Path::new(app)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .trim_end_matches(".app")
            .to_string();
        let bundle_id = read_bundle_id(app);
        let deep = scan_deep_leftovers(&bundle_id, &name, app);
        if !deep.deletable.is_empty() || !deep.review_only.is_empty() {
            hit_apps += 1;
        }
        total_deletable += deep.deletable.len();
        total_review += deep.review_only.len();
        eprintln!(
            "[probe] {name} bundle={bundle_id} deletable={:?} review_only={:?}",
            deep.deletable, deep.review_only
        );
    }

    eprintln!(
        "[probe] summary apps={} hit_apps={} total_deletable={} total_review={}",
        apps.len(),
        hit_apps,
        total_deletable,
        total_review
    );
    // 只验证「能跑通且不 panic」；命中数量依赖机器环境，不做硬断言。
}
