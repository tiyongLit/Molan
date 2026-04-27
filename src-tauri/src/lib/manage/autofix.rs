//! 对齐 `lib/manage/autofix.sh`。
//!
//! GUI 端不需要 `read_key` 这种 TTY 交互,因此 `ask_for_auto_fix` 不会真正读键,
//! 仅基于 `HAS_AUTO_FIX_SUGGESTIONS` 环境变量返回 false(由前端弹窗代替)。
//! 其它逻辑(建议生成、自动修复执行、AUTO_FIX_SUMMARY 导出)按 SH 严格翻译。

use crate::core::sudo::{self, sudo_output};

pub struct AutoFixResult {
    pub applied: usize,
    pub items: Vec<String>,
}

/// 对齐 `autofix.sh:7-92` 中的 `show_suggestions`。
///
/// 返回 `(can_auto_fix, auto_fix_items, manual_items)`。
/// 同时按 SH 第 91 行 `export HAS_AUTO_FIX_SUGGESTIONS` 设置环境变量,以便其它子进程读取。
pub fn show_suggestions() -> (bool, Vec<String>, Vec<(String, String)>) {
    let mut has_suggestions = false;
    let mut can_auto_fix = false;
    let mut auto_fix_items: Vec<String> = Vec::new();
    let mut manual_items: Vec<(String, String)> = Vec::new();

    // 对齐 SH 第 13-16 行
    let skip_security_autofix =
        std::env::var("MOLE_SECURITY_FIXES_SHOWN").unwrap_or_default() == "true";

    // 对齐 SH 第 19-23 行
    if !skip_security_autofix && std::env::var("FIREWALL_DISABLED").unwrap_or_default() == "true" {
        auto_fix_items.push("Enable Firewall for better security".to_string());
        has_suggestions = true;
        can_auto_fix = true;
    }

    // 对齐 SH 第 25-28 行
    if std::env::var("FILEVAULT_DISABLED").unwrap_or_default() == "true" {
        manual_items.push((
            "Enable FileVault".to_string(),
            "System Settings → Privacy & Security → FileVault".to_string(),
        ));
        has_suggestions = true;
    }

    // 对齐 SH 第 31-35 行
    if !skip_security_autofix
        && std::env::var("TOUCHID_NOT_CONFIGURED").unwrap_or_default() == "true"
    {
        auto_fix_items.push("Enable Touch ID for sudo".to_string());
        has_suggestions = true;
        can_auto_fix = true;
    }

    // 对齐 SH 第 37-41 行
    if std::env::var("ROSETTA_NOT_INSTALLED").unwrap_or_default() == "true" {
        auto_fix_items.push("Install Rosetta 2 for Intel app support".to_string());
        has_suggestions = true;
        can_auto_fix = true;
    }

    // 对齐 SH 第 44-50 行:CACHE_SIZE_GB > 5 时建议清理
    let cache_gb_str = std::env::var("CACHE_SIZE_GB").unwrap_or_default();
    if !cache_gb_str.is_empty() {
        if let Ok(cache_gb) = cache_gb_str.parse::<f64>() {
            if cache_gb > 5.0 {
                manual_items.push((
                    format!("Free up {cache_gb_str}GB by cleaning caches"),
                    "Run: mo clean".to_string(),
                ));
                has_suggestions = true;
            }
        }
    }

    // 对齐 SH 第 52-55 行
    if std::env::var("BREW_HAS_WARNINGS").unwrap_or_default() == "true" {
        manual_items.push((
            "Fix Homebrew warnings".to_string(),
            "Run: brew doctor to see details".to_string(),
        ));
        has_suggestions = true;
    }

    // 对齐 SH 第 57-62 行:DISK_FREE_GB < 50 且 CACHE_SIZE_GB <= 5 时提醒
    let disk_free_str = std::env::var("DISK_FREE_GB").unwrap_or_default();
    if !disk_free_str.is_empty() {
        let disk_free: i64 = disk_free_str.parse().unwrap_or(0);
        if disk_free < 50 {
            let cache_small = cache_gb_str.is_empty()
                || cache_gb_str
                    .parse::<f64>()
                    .map(|v| v <= 5.0)
                    .unwrap_or(true);
            if cache_small {
                manual_items.push((
                    format!("Low disk space, {disk_free_str}GB free"),
                    "Run: mo analyze to find large files".to_string(),
                ));
                has_suggestions = true;
            }
        }
    }

    // 对齐 SH 第 67-71、91 行的 export
    let flag = if has_suggestions && can_auto_fix {
        "true"
    } else {
        "false"
    };
    unsafe {
        std::env::set_var("HAS_AUTO_FIX_SUGGESTIONS", flag);
    }

    (can_auto_fix, auto_fix_items, manual_items)
}

/// 对齐 `autofix.sh:96-119` 中的 `ask_for_auto_fix`。
///
/// SH 端通过 `read_key` 读取回车 / 空格;GUI 端无 TTY,这里仅在
/// `HAS_AUTO_FIX_SUGGESTIONS=true` 时返回 false,实际确认由前端对话框完成。
pub fn ask_for_auto_fix() -> bool {
    if std::env::var("HAS_AUTO_FIX_SUGGESTIONS").unwrap_or_default() != "true" {
        return false;
    }
    false
}

/// 对齐 `autofix.sh:123-193` 中的 `perform_auto_fix`。
///
/// 副作用:与 SH 一致,导出 `AUTO_FIX_SUMMARY` 与 `AUTO_FIX_DETAILS` 环境变量。
pub fn perform_auto_fix() -> AutoFixResult {
    let mut fixed_count = 0usize;
    let mut fixed_items: Vec<String> = Vec::new();

    // 对齐 SH 第 128-134 行
    if !sudo::ensure_admin_session() {
        eprintln!("Skipping auto fixes, admin authentication required");
        let summary = "Auto fixes skipped: No changes were required".to_string();
        unsafe {
            std::env::set_var("AUTO_FIX_SUMMARY", &summary);
            std::env::set_var("AUTO_FIX_DETAILS", "");
        }
        return AutoFixResult {
            applied: 0,
            items: vec![],
        };
    }

    // 对齐 SH 第 137-147 行:启用防火墙
    if std::env::var("FIREWALL_DISABLED").unwrap_or_default() == "true" {
        let ok = sudo_output(&[
            "/usr/libexec/ApplicationFirewall/socketfilterfw",
            "--setglobalstate",
            "on",
        ])
        .status
        .success();
        if ok {
            fixed_count += 1;
            fixed_items.push("Firewall enabled".to_string());
        }
    }

    // 对齐 SH 第 150-165 行:Touch ID for sudo。
    // 关键:必须使用 `/usr/bin/sed` 绝对路径,否则 Homebrew gnu-sed 会破坏 `-i ''` 语法。
    if std::env::var("TOUCHID_NOT_CONFIGURED").unwrap_or_default() == "true" {
        let pam_file = "/etc/pam.d/sudo";
        let already_has = std::fs::read_to_string(pam_file)
            .map(|c| c.contains("pam_tid.so"))
            .unwrap_or(false);
        let ok = if already_has {
            true
        } else {
            // 对齐 SH `sudo bash -c "grep -q ... || /usr/bin/sed -i '' '2i\\\nauth ...' '$pam_file'"`
            let script = format!(
                "grep -q 'pam_tid.so' '{pam_file}' 2>/dev/null || /usr/bin/sed -i '' '2i\\\nauth       sufficient     pam_tid.so\n' '{pam_file}'"
            );
            sudo_output(&["/bin/bash", "-c", &script]).status.success()
        };
        if ok {
            fixed_count += 1;
            fixed_items.push("Touch ID configured for sudo".to_string());
        }
    }

    // 对齐 SH 第 168-178 行:安装 Rosetta 2
    if std::env::var("ROSETTA_NOT_INSTALLED").unwrap_or_default() == "true" {
        let out = sudo_output(&[
            "/usr/sbin/softwareupdate",
            "--install-rosetta",
            "--agree-to-license",
        ]);
        let s = String::from_utf8_lossy(&out.stdout);
        let ok =
            s.contains("Installing") || s.contains("Installed") || s.contains("already installed");
        if ok {
            fixed_count += 1;
            fixed_items.push("Rosetta 2 installed".to_string());
        }
    }

    // 对齐 SH 第 180-191 行:导出 summary
    let (summary, details) = if fixed_count > 0 {
        let summary = format!("Auto fixes applied: {fixed_count} issues");
        let details = fixed_items.join("\n");
        (summary, details)
    } else {
        (
            "Auto fixes skipped: No changes were required".to_string(),
            String::new(),
        )
    };
    unsafe {
        std::env::set_var("AUTO_FIX_SUMMARY", &summary);
        std::env::set_var("AUTO_FIX_DETAILS", &details);
    }

    AutoFixResult {
        applied: fixed_count,
        items: fixed_items,
    }
}
