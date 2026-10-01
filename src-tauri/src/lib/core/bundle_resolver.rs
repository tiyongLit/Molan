use std::path::Path;

use super::base::home_dir;
use super::timeout::run_with_timeout_capture_rc;

/// 对齐 SH `bundle_has_installed_app`(lib/core/bundle_resolver.sh):
/// 返回 true=已安装 / false=未安装。mdfind 探测失败、超时或信号中断时
/// 降级为文件系统扫描兜底(clean 模块的提示/清理路径继续使用此语义)。
pub fn bundle_has_installed_app(bundle_id: &str) -> bool {
    bundle_has_installed_app_checked(bundle_id)
        .unwrap_or_else(|| filesystem_scan_finds_app(bundle_id))
}

/// 三态版(对齐 SH 26f4d47a 加固的 rc 传播语义):
/// - `Some(true)`:确认已安装
/// - `Some(false)`:mdfind 正常返回未命中 + 文件系统扫描也未找到,确认未安装
/// - `None`:mdfind 被信号中断(rc>=128),fail-closed——调用方不得当作"未安装"删除
pub fn bundle_has_installed_app_checked(bundle_id: &str) -> Option<bool> {
    if bundle_id.is_empty() {
        return Some(false);
    }

    if !bundle_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Some(false);
    }

    // mdfind 在 Spotlight 索引未就绪时会卡很久,GUI 后端必须套 2s 超时(对齐 SH 第 56 行)。
    // 超时(124)/普通失败继续走文件系统扫描兜底;仅信号中断(>=128)才 fail-closed。
    let mdfind_query = format!("kMDItemCFBundleIdentifier == '{bundle_id}'");
    let (rc, hit) = run_with_timeout_capture_rc(2.0, "mdfind", &[&mdfind_query]);
    if rc >= 128 {
        return None;
    }
    if hit
        .as_deref()
        .map(|s| s.lines().any(|l| !l.is_empty()))
        .unwrap_or(false)
    {
        return Some(true);
    }

    Some(filesystem_scan_finds_app(bundle_id))
}

/// 慢路径:遍历已知 app 根目录,读 Info.plist 的 CFBundleIdentifier,
/// 并检查 SMJobBless helper(Contents/Library/LaunchServices/<id>)。
/// 覆盖 Spotlight 索引漏掉的应用(issue #732)与内嵌特权 helper(issue #733)。
fn filesystem_scan_finds_app(bundle_id: &str) -> bool {
    let parent_id = [".helper", ".daemon", ".agent", ".xpc"]
        .iter()
        .find_map(|suffix| bundle_id.strip_suffix(suffix).map(|s| s.to_string()));

    let mapped_app_bundles: Vec<&str> = match bundle_id {
        "com.microsoft.autoupdate.helper" | "com.microsoft.office.licensingV2.helper" => {
            vec![
                "com.microsoft.Word",
                "com.microsoft.Excel",
                "com.microsoft.Powerpoint",
                "com.microsoft.Outlook",
                "com.microsoft.OneNote",
            ]
        }
        _ => Vec::new(),
    };

    let app_roots = [
        "/Applications",
        "/Applications/Setapp",
        "/Applications/Utilities",
    ];
    let home_apps = format!("{}/Applications", home_dir());

    let all_roots: Vec<&str> = app_roots
        .iter()
        .chain(std::iter::once(&home_apps.as_str()))
        .copied()
        .collect();

    for app_root in &all_roots {
        if !Path::new(app_root).is_dir() {
            continue;
        }
        let entries = match std::fs::read_dir(app_root) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let app = entry.path();
            let app_str = app.to_string_lossy();
            if !app_str.ends_with(".app") {
                continue;
            }

            let launch_services = app.join("Contents/Library/LaunchServices").join(bundle_id);
            if launch_services.exists() {
                return true;
            }

            let info_plist = app.join("Contents/Info.plist");
            if !info_plist.is_file() {
                continue;
            }

            // 纯 Rust 解析（原 `plutil -extract ... raw` 子进程的原生替代）。
            let app_bundle = super::bundle_id_anchor::read_bundle_id_from_plist(&info_plist)
                .map(|s| s.trim().to_string())
                .unwrap_or_default();

            if app_bundle == bundle_id {
                return true;
            }
            if let Some(ref pid) = parent_id {
                if app_bundle == *pid {
                    return true;
                }
            }
            if mapped_app_bundles.contains(&app_bundle.as_str()) {
                return true;
            }
        }
    }

    false
}
