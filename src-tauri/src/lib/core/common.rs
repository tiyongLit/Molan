use std::path::Path;
use std::process::Command;

use super::base::{home_dir, mktemp_file};
use super::file_ops::safe_remove;
use super::log::log_error;

/// 移除尾部斜杠但保留 root("/" 不会变成空)。对齐 base.sh:mole_normalize_path()
pub fn mole_normalize_path(path: &str) -> String {
    if path == "/" {
        return path.to_string();
    }
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        path.to_string()
    } else {
        trimmed.to_string()
    }
}

/// 路径稳定 ID:对存在的路径用 dev:inode,否则退回字面量。
/// 这样大小写不敏感的卷或软链接会塌缩到同一标识。对齐 base.sh:mole_path_identity()
pub fn mole_path_identity(path: &str) -> String {
    let normalized = mole_normalize_path(path);
    let p = Path::new(&normalized);
    let exists_or_symlink = p.exists() || p.is_symlink();

    if exists_or_symlink {
        // 优先 stat -L(follow);失败再 stat(对齐 SH 第 50 行)
        let try_stat = |args: &[&str]| -> Option<String> {
            Command::new("stat").args(args).output().ok().and_then(|o| {
                if o.status.success() {
                    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                } else {
                    None
                }
            })
        };

        let id_candidates = [
            try_stat(&["-L", "-f", "%d:%i", &normalized]),
            try_stat(&["-f", "%d:%i", &normalized]),
        ];
        for opt in id_candidates.iter().flatten() {
            // 严格校验:^[0-9]+:[0-9]+$
            let bytes = opt.as_bytes();
            let mut seen_colon = false;
            let mut left_digits = 0usize;
            let mut right_digits = 0usize;
            let mut valid = true;
            for &b in bytes {
                if b == b':' {
                    if seen_colon {
                        valid = false;
                        break;
                    }
                    seen_colon = true;
                } else if b.is_ascii_digit() {
                    if seen_colon {
                        right_digits += 1;
                    } else {
                        left_digits += 1;
                    }
                } else {
                    valid = false;
                    break;
                }
            }
            if valid && seen_colon && left_digits > 0 && right_digits > 0 {
                return format!("inode:{opt}");
            }
        }
    }

    format!("path:{normalized}")
}

pub fn mole_identity_in_list(needle: &str, list: &[String]) -> bool {
    list.iter().any(|x| x == needle)
}

/// 描述 update_via_homebrew 的执行结果
pub enum HomebrewUpdateOutcome {
    /// 已经是最新版本(brew 输出含 "already installed"),携带探测出的版本号
    AlreadyLatest { version: String },
    /// 升级成功,携带新版本号
    Updated { version: String },
    /// brew 报错
    Error(String),
}

/// 通过 Homebrew 更新 mole;对齐 common.sh:update_via_homebrew()
/// 与 SH 不同:GUI 后端不打印 spinner,而是把结构化结果返回给前端,由前端决定显示方式。
pub fn update_via_homebrew(current_version: &str) -> HomebrewUpdateOutcome {
    let temp_update = mktemp_file("brew_update");
    let temp_upgrade = mktemp_file("brew_upgrade");

    // brew update;失败也继续(对齐 SH `wait || true`)
    let _ = Command::new("brew").arg("update").output();

    // brew upgrade mole
    let upgrade_out = Command::new("brew").args(["upgrade", "mole"]).output();
    let combined = match &upgrade_out {
        Ok(o) => format!(
            "{}\n{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(e) => format!("brew upgrade error: {e}"),
    };

    if let Some(p) = &temp_update {
        let _ = safe_remove(p.to_string_lossy().as_ref(), true);
    }
    if let Some(p) = &temp_upgrade {
        let _ = safe_remove(p.to_string_lossy().as_ref(), true);
    }

    let probe_version = || -> String {
        if let Some(out) = Command::new("brew")
            .args(["list", "--versions", "mole"])
            .output()
            .ok()
        {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = s.lines().next() {
                if let Some(v) = line.split_whitespace().nth(1) {
                    return v.to_string();
                }
            }
        }
        if let Some(out) = Command::new("mo").arg("--version").output().ok() {
            let s = String::from_utf8_lossy(&out.stdout);
            for line in s.lines() {
                if line.contains("Mole version") {
                    if let Some(v) = line.split_whitespace().nth(2) {
                        return v.to_string();
                    }
                }
            }
        }
        current_version.to_string()
    };

    let outcome = if combined.contains("already installed") {
        HomebrewUpdateOutcome::AlreadyLatest {
            version: probe_version(),
        }
    } else if combined.contains("Error:") {
        log_error("Homebrew upgrade failed");
        let err_lines: String = combined
            .lines()
            .filter(|l| l.contains("Error:"))
            .collect::<Vec<_>>()
            .join("\n");
        HomebrewUpdateOutcome::Error(err_lines)
    } else if let Ok(o) = &upgrade_out {
        if o.status.success() {
            HomebrewUpdateOutcome::Updated {
                version: probe_version(),
            }
        } else {
            HomebrewUpdateOutcome::Error(String::from_utf8_lossy(&o.stderr).trim().to_string())
        }
    } else {
        HomebrewUpdateOutcome::Error("brew upgrade did not run".to_string())
    };

    // 清理 mole 自身的版本检查缓存,避免后台 update message 仍显示旧版
    let home = home_dir();
    if !home.is_empty() {
        let _ = std::fs::remove_file(format!("{home}/.cache/mole/version_check"));
        let _ = std::fs::remove_file(format!("{home}/.cache/mole/update_message"));
    }

    outcome
}

/// 把目标路径规范化成 Dock plist 里能匹配的绝对路径,对齐 common.sh:remove_apps_from_dock() 第 172-194 行
fn normalize_dock_target(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.chars().any(|c| c.is_control()) {
        return None;
    }
    let p = Path::new(raw);
    // 1. 路径存在 → 用 canonicalize 取绝对真实路径
    if p.exists() {
        if let Ok(canon) = p.canonicalize() {
            return Some(canon.to_string_lossy().to_string());
        }
        // canonicalize 失败也尝试 dirname-pwd-basename 这套(对齐 SH)
        if let Some(parent) = p.parent() {
            if let Ok(parent_abs) = parent.canonicalize() {
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    return Some(format!("{}/{}", parent_abs.to_string_lossy(), name));
                }
            }
        }
        return None;
    }
    // 2. 不存在但格式像绝对路径 → 直接用
    if raw.starts_with('/') {
        return Some(raw.to_string());
    }
    // 3. ~/foo → 展开 HOME
    if let Some(stripped) = raw.strip_prefix("~/") {
        let home = home_dir();
        if !home.is_empty() {
            return Some(format!("{home}/{stripped}"));
        }
    }
    if raw == "~" {
        let home = home_dir();
        if !home.is_empty() {
            return Some(home);
        }
    }
    // 4. 其他情况(纯文件名等) → 跳过,与 SH 保持一致
    None
}

pub fn remove_apps_from_dock(targets: &[String]) -> bool {
    if targets.is_empty() {
        return true;
    }
    let plist = format!("{}/Library/Preferences/com.apple.dock.plist", home_dir());
    if !Path::new(&plist).exists() {
        return true;
    }
    if !Path::new("/usr/libexec/PlistBuddy").exists() {
        return true;
    }
    let mut changed = false;
    for target in targets {
        let full_path = match normalize_dock_target(target) {
            Some(p) => p,
            None => continue,
        };

        // SH `${full_path// /%20}` 仅替换空格,不做完整 URL encode
        let encoded = full_path.replace(' ', "%20");
        if encoded.is_empty() {
            continue;
        }

        let mut i = 0usize;
        loop {
            let label = Command::new("/usr/libexec/PlistBuddy")
                .args([
                    "-c",
                    &format!("Print :persistent-apps:{i}:tile-data:file-label"),
                    &plist,
                ])
                .output();
            let Ok(label) = label else { break };
            if !label.status.success() {
                break;
            }
            let label_str = String::from_utf8_lossy(&label.stdout).trim().to_string();
            if label_str.is_empty() {
                break;
            }

            let url = Command::new("/usr/libexec/PlistBuddy")
                .args([
                    "-c",
                    &format!("Print :persistent-apps:{i}:tile-data:file-data:_CFURLString"),
                    &plist,
                ])
                .output();
            let url_str = match url {
                Ok(o) if o.status.success() => {
                    String::from_utf8_lossy(&o.stdout).trim().to_string()
                }
                _ => {
                    i += 1;
                    continue;
                }
            };

            if url_str.contains(&encoded) {
                if Command::new("/usr/libexec/PlistBuddy")
                    .args(["-c", &format!("Delete :persistent-apps:{i}"), &plist])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                {
                    changed = true;
                    // 删除后当前 i 指向下一项,不递增
                    continue;
                }
            }
            i += 1;
        }
    }

    if changed {
        let _ = Command::new("killall").arg("Dock").output();
    }
    true
}
