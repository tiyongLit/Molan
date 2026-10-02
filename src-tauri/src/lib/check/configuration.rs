//! 与 `lib/check/all.sh` 中 `check_all_config` 对齐:
//! `check_touchid_sudo` / `check_rosetta` / `check_git_config`。
//!
//! GUI 不渲染 ANSI,但保留 `render_configuration_ansi` 以便 `mo check` 风格的纯文本输出。

use crate::manage::whitelist_optimize;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::process::Command;

#[cfg(target_os = "macos")]
use super::macos_path_env;

const GREEN: &str = "\x1b[0;32m";
const BLUE: &str = "\x1b[1;34m";
const YELLOW: &str = "\x1b[0;33m";
const GRAY: &str = "\x1b[0;90m";
const NC: &str = "\x1b[0m";
const ICON_WARNING: &str = "◎";
const ICON_ARROW: &str = "➤";
const ICON_EMPTY: &str = "○";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ConfigurationReport {
    pub touch_id: Option<TouchIdLine>,
    pub rosetta: Option<RosettaLine>,
    pub git: Option<GitConfigLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TouchIdLine {
    Configured,
    NotConfigured,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RosettaLine {
    Ready,
    NotInstalled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum GitConfigLine {
    Configured,
    Missing,
}

pub fn collect_configuration_report() -> ConfigurationReport {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(crate::core::base::home_dir_opt)
            .unwrap_or_else(|| std::path::PathBuf::from("/"));
        let wl = whitelist_optimize::load_optimize_whitelist_patterns(&home);
        ConfigurationReport {
            touch_id: check_touchid_sudo(&wl, &home),
            rosetta: check_rosetta(&wl, &home),
            git: check_git_config(&wl, &home),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        ConfigurationReport {
            touch_id: None,
            rosetta: None,
            git: None,
        }
    }
}

pub fn generate_configuration_json_value() -> Value {
    serde_json::to_value(&collect_configuration_report()).unwrap_or(Value::Null)
}

pub fn check_all_configuration_ansi() -> String {
    let r = collect_configuration_report();
    let mut o = String::new();
    o.push_str(&format!(
        "{b}{a}{n} System Configuration\n",
        b = BLUE,
        a = ICON_ARROW,
        n = NC
    ));
    if let Some(ref t) = r.touch_id {
        o.push_str(&render_touch_id(t));
    }
    if let Some(ref ro) = r.rosetta {
        o.push_str(&render_rosetta(ro));
    }
    if let Some(ref g) = r.git {
        o.push_str(&render_git(g));
    }
    o
}

#[cfg(target_os = "macos")]
fn has_command(name: &str) -> bool {
    Command::new("/bin/sh")
        .env("PATH", macos_path_env())
        .args(["-c", &format!("command -v {name} >/dev/null 2>&1")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 对齐 SH 第 37-61 行 `check_touchid_sudo`。
/// 与 SH 一致:仅在「未配置且 Touch ID 受支持」时 export `TOUCHID_NOT_CONFIGURED=true`;
/// 已配置或不支持时不 export(即不会出现在输出枚举中,枚举只覆盖被 SH 真正打印的两种情况)。
#[cfg(target_os = "macos")]
fn check_touchid_sudo(wl: &[String], home: &Path) -> Option<TouchIdLine> {
    if whitelist_optimize::is_whitelisted_optimize("check_touchid", wl, home) {
        return None;
    }
    let pam_files = ["/etc/pam.d/sudo", "/etc/pam.d/sudo_local"];
    let mut configured = false;
    for f in &pam_files {
        if let Ok(content) = std::fs::read_to_string(f) {
            if content.contains("pam_tid.so") {
                configured = true;
                break;
            }
        }
    }
    if configured {
        // SH 不显式 unset,但已配置时不会再 export。GUI 进程内 env 不会从前一次 check 残留(每次都会重写)。
        return Some(TouchIdLine::Configured);
    }

    // 检查是否支持 Touch ID
    let mut is_supported = false;
    if has_command("bioutil") {
        if let Ok(out) = Command::new("bioutil")
            .env("PATH", macos_path_env())
            .args(["-r"])
            .output()
        {
            if String::from_utf8_lossy(&out.stdout).contains("Touch ID") {
                is_supported = true;
            }
        }
    } else {
        let arch = Command::new("/usr/bin/uname")
            .arg("-m")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        if arch == "arm64" {
            is_supported = true;
        }
    }

    if is_supported {
        std::env::set_var("TOUCHID_NOT_CONFIGURED", "true");
        Some(TouchIdLine::NotConfigured)
    } else {
        // 不支持 Touch ID:与 SH 一致——什么都不打印,也不导出 env。
        None
    }
}

/// 对齐 SH 第 63-74 行 `check_rosetta`。仅在 arm64 上输出。
#[cfg(target_os = "macos")]
fn check_rosetta(wl: &[String], home: &Path) -> Option<RosettaLine> {
    if whitelist_optimize::is_whitelisted_optimize("check_rosetta", wl, home) {
        return None;
    }
    let arch = Command::new("/usr/bin/uname")
        .arg("-m")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if arch != "arm64" {
        return None;
    }
    if Path::new("/Library/Apple/usr/share/rosetta/rosetta").is_file() {
        Some(RosettaLine::Ready)
    } else {
        Some(RosettaLine::NotInstalled)
    }
}

/// 对齐 SH 第 76-90 行 `check_git_config`。
#[cfg(target_os = "macos")]
fn check_git_config(wl: &[String], home: &Path) -> Option<GitConfigLine> {
    if whitelist_optimize::is_whitelisted_optimize("check_git_config", wl, home) {
        return None;
    }
    if !has_command("git") {
        return None;
    }
    let read = |key: &str| -> String {
        Command::new("git")
            .env("PATH", macos_path_env())
            .args(["config", "--global", key])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    let name = read("user.name");
    let email = read("user.email");
    if !name.is_empty() && !email.is_empty() {
        Some(GitConfigLine::Configured)
    } else {
        Some(GitConfigLine::Missing)
    }
}

fn render_touch_id(t: &TouchIdLine) -> String {
    match t {
        TouchIdLine::Configured => format!(
            "  {g}✓{n} Touch ID     Biometric authentication enabled\n",
            g = GREEN,
            n = NC
        ),
        TouchIdLine::NotConfigured => format!(
            "  {gray}{icon}{n} Touch ID     {y}Not configured for sudo{n}\n",
            gray = GRAY,
            icon = ICON_WARNING,
            n = NC,
            y = YELLOW
        ),
    }
}

fn render_rosetta(r: &RosettaLine) -> String {
    match r {
        RosettaLine::Ready => format!(
            "  {g}✓{n} Rosetta 2    Intel app translation ready\n",
            g = GREEN,
            n = NC
        ),
        RosettaLine::NotInstalled => format!(
            "  {gray}{icon}{n} Rosetta 2    {gray2}Not installed{n}\n",
            gray = GRAY,
            icon = ICON_EMPTY,
            gray2 = GRAY,
            n = NC
        ),
    }
}

fn render_git(g: &GitConfigLine) -> String {
    match g {
        GitConfigLine::Configured => format!(
            "  {g}✓{n} Git          Global identity configured\n",
            g = GREEN,
            n = NC
        ),
        GitConfigLine::Missing => format!(
            "  {gray}{icon}{n} Git          {y}User identity not set{n}\n",
            gray = GRAY,
            icon = ICON_WARNING,
            n = NC,
            y = YELLOW
        ),
    }
}
