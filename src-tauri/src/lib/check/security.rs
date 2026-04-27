//! 与 `lib/check/all.sh` 中 `check_all_security` 对齐:
//! `check_filevault` / `check_firewall` / `check_gatekeeper` / `check_sip`。
//!
//! 同步 export 三个 env(`FILEVAULT_DISABLED` / `FIREWALL_DISABLED` / `GATEKEEPER_DISABLED`),
//! 供 `manage/autofix.rs` 读取用于决策。

use crate::whitelist_optimize;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::process::Command;

const RED: &str = "\x1b[0;31m";
const GREEN: &str = "\x1b[0;32m";
const BLUE: &str = "\x1b[1;34m";
const YELLOW: &str = "\x1b[0;33m";
const GRAY: &str = "\x1b[0;90m";
const NC: &str = "\x1b[0m";
const ICON_WARNING: &str = "◎";
const ICON_ARROW: &str = "➤";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct SecurityReport {
    pub filevault: Option<FilevaultLine>,
    pub firewall: Option<FirewallLine>,
    pub gatekeeper: Option<GatekeeperLine>,
    pub sip: Option<SipLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FilevaultLine {
    Active,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FirewallLine {
    ThirdParty { name: String },
    Builtin,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum GatekeeperLine {
    Active,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SipLine {
    Enabled,
    Disabled,
}

pub fn collect_security_report() -> SecurityReport {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| std::path::PathBuf::from("/"));
        let wl = whitelist_optimize::load_optimize_whitelist_patterns(&home);
        SecurityReport {
            filevault: check_filevault(&wl, &home),
            firewall: check_firewall(&wl, &home),
            gatekeeper: check_gatekeeper(&wl, &home),
            sip: check_sip(&wl, &home),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        SecurityReport {
            filevault: None,
            firewall: None,
            gatekeeper: None,
            sip: None,
        }
    }
}

pub fn generate_security_json_value() -> Value {
    serde_json::to_value(&collect_security_report()).unwrap_or(Value::Null)
}

pub fn check_all_security_ansi() -> String {
    let r = collect_security_report();
    let mut o = String::new();
    o.push_str(&format!(
        "{b}{a}{n} Security Status\n",
        b = BLUE,
        a = ICON_ARROW,
        n = NC
    ));
    if let Some(ref f) = r.filevault {
        o.push_str(&render_filevault(f));
    }
    if let Some(ref fw) = r.firewall {
        o.push_str(&render_firewall(fw));
    }
    if let Some(ref gk) = r.gatekeeper {
        o.push_str(&render_gatekeeper(gk));
    }
    if let Some(ref s) = r.sip {
        o.push_str(&render_sip(s));
    }
    o
}

#[cfg(target_os = "macos")]
fn macos_path_env() -> String {
    let tail = std::env::var("PATH").unwrap_or_default();
    format!("/usr/bin:/bin:/usr/sbin:/sbin:{tail}")
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

/// 对齐 SH 第 103-116 行 `check_filevault`。
#[cfg(target_os = "macos")]
fn check_filevault(wl: &[String], home: &Path) -> Option<FilevaultLine> {
    if whitelist_optimize::is_whitelisted_optimize("check_filevault", wl, home) {
        return None;
    }
    if !has_command("fdesetup") {
        return None;
    }
    let out = Command::new("fdesetup")
        .env("PATH", macos_path_env())
        .arg("status")
        .output()
        .ok();
    let text = out
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    if text.contains("FileVault is On") {
        Some(FilevaultLine::Active)
    } else {
        std::env::set_var("FILEVAULT_DISABLED", "true");
        Some(FilevaultLine::Disabled)
    }
}

/// 对齐 SH 第 118-153 行 `check_firewall`。
#[cfg(target_os = "macos")]
fn check_firewall(wl: &[String], home: &Path) -> Option<FirewallLine> {
    if whitelist_optimize::is_whitelisted_optimize("firewall", wl, home) {
        return None;
    }
    // SH 第 122 行 `unset FIREWALL_DISABLED`,这里同样:重置 env 避免上次残留。
    std::env::remove_var("FIREWALL_DISABLED");

    let third_party = if Path::new("/Applications/Little Snitch.app").is_dir()
        || Path::new("/Library/Little Snitch").is_dir()
    {
        Some("Little Snitch")
    } else if Path::new("/Applications/LuLu.app").is_dir() {
        Some("LuLu")
    } else if Path::new("/Applications/Radio Silence.app").is_dir() {
        Some("Radio Silence")
    } else if Path::new("/Applications/Hands Off!.app").is_dir() {
        Some("Hands Off!")
    } else if Path::new("/Applications/Murus.app").is_dir() {
        Some("Murus")
    } else if Path::new("/Applications/Vallum.app").is_dir() {
        Some("Vallum")
    } else {
        None
    };

    if let Some(name) = third_party {
        return Some(FirewallLine::ThirdParty {
            name: name.to_string(),
        });
    }

    let out = Command::new("/usr/libexec/ApplicationFirewall/socketfilterfw")
        .arg("--getglobalstate")
        .output()
        .ok();
    let text = out
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    if text.contains("State = 1") || text.contains("State = 2") {
        Some(FirewallLine::Builtin)
    } else {
        std::env::set_var("FIREWALL_DISABLED", "true");
        Some(FirewallLine::Disabled)
    }
}

/// 对齐 SH 第 155-169 行 `check_gatekeeper`。
#[cfg(target_os = "macos")]
fn check_gatekeeper(wl: &[String], home: &Path) -> Option<GatekeeperLine> {
    if whitelist_optimize::is_whitelisted_optimize("gatekeeper", wl, home) {
        return None;
    }
    if !has_command("spctl") {
        return None;
    }
    let out = Command::new("spctl")
        .env("PATH", macos_path_env())
        .arg("--status")
        .output()
        .ok();
    let text = out
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    if text.contains("enabled") {
        std::env::remove_var("GATEKEEPER_DISABLED");
        Some(GatekeeperLine::Active)
    } else {
        std::env::set_var("GATEKEEPER_DISABLED", "true");
        Some(GatekeeperLine::Disabled)
    }
}

/// 对齐 SH 第 171-183 行 `check_sip`。
#[cfg(target_os = "macos")]
fn check_sip(wl: &[String], home: &Path) -> Option<SipLine> {
    if whitelist_optimize::is_whitelisted_optimize("check_sip", wl, home) {
        return None;
    }
    if !has_command("csrutil") {
        return None;
    }
    let out = Command::new("csrutil")
        .env("PATH", macos_path_env())
        .arg("status")
        .output()
        .ok();
    let text = out
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    if text.contains("enabled") {
        Some(SipLine::Enabled)
    } else {
        Some(SipLine::Disabled)
    }
}

fn render_filevault(f: &FilevaultLine) -> String {
    match f {
        FilevaultLine::Active => format!(
            "  {g}✓{n} FileVault    Disk encryption active\n",
            g = GREEN,
            n = NC
        ),
        FilevaultLine::Disabled => format!(
            "  {r}✗{n} FileVault    {r2}Disk encryption disabled{n}\n",
            r = RED,
            r2 = RED,
            n = NC
        ),
    }
}

fn render_firewall(fw: &FirewallLine) -> String {
    match fw {
        FirewallLine::ThirdParty { name } => {
            format!("  {g}✓{n} Firewall     {name} active\n", g = GREEN, n = NC)
        }
        FirewallLine::Builtin => format!(
            "  {g}✓{n} Firewall     Network protection enabled\n",
            g = GREEN,
            n = NC
        ),
        FirewallLine::Disabled => format!(
            "  {gray}{icon}{n} Firewall     {y}Network protection disabled{n}\n",
            gray = GRAY,
            icon = ICON_WARNING,
            n = NC,
            y = YELLOW
        ),
    }
}

fn render_gatekeeper(gk: &GatekeeperLine) -> String {
    match gk {
        GatekeeperLine::Active => format!(
            "  {g}✓{n} Gatekeeper   App download protection active\n",
            g = GREEN,
            n = NC
        ),
        GatekeeperLine::Disabled => format!(
            "  {gray}{icon}{n} Gatekeeper   {y}App security disabled{n}\n",
            gray = GRAY,
            icon = ICON_WARNING,
            n = NC,
            y = YELLOW
        ),
    }
}

fn render_sip(s: &SipLine) -> String {
    match s {
        SipLine::Enabled => format!(
            "  {g}✓{n} SIP          System integrity protected\n",
            g = GREEN,
            n = NC
        ),
        SipLine::Disabled => format!(
            "  {gray}{icon}{n} SIP          {y}System protection disabled{n}\n",
            gray = GRAY,
            icon = ICON_WARNING,
            n = NC,
            y = YELLOW
        ),
    }
}
