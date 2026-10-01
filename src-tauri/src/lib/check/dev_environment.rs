//! 与 `src/lib/check/dev_environment.sh` 对齐（JSON 事实层 + ANSI 文本层）。

use crate::whitelist_optimize;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const GREEN: &str = "\x1b[0;32m";
const BLUE: &str = "\x1b[1;34m";
const YELLOW: &str = "\x1b[0;33m";
const GRAY: &str = "\x1b[0;90m";
const NC: &str = "\x1b[0m";
const ICON_WARNING: &str = "◎";
const ICON_ARROW: &str = "➤";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DevEnvironmentOptions {
    pub apply_optimize_whitelist: bool,
}

impl Default for DevEnvironmentOptions {
    fn default() -> Self {
        Self {
            apply_optimize_whitelist: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct DevEnvironmentReport {
    pub launch_agents: Option<LaunchAgentsSection>,
    pub dev_tools: Option<DevToolsSection>,
    pub versions: Option<VersionsSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchAgentsSection {
    AllHealthy,
    Broken { broken_count: u32, detail: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DevToolsSection {
    NoneDetected,
    Found { count: usize, tools: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VersionsSection {
    NoConflicts,
    Conflicts { messages: Vec<String> },
}

pub fn generate_dev_environment_json_value() -> Result<Value, String> {
    let r = collect_dev_environment_report()?;
    serde_json::to_value(&r).map_err(|e| e.to_string())
}

pub fn collect_dev_environment_report() -> Result<DevEnvironmentReport, String> {
    collect_dev_environment_report_with_options(DevEnvironmentOptions::default())
}

pub fn collect_dev_environment_report_with_options(
    opt: DevEnvironmentOptions,
) -> Result<DevEnvironmentReport, String> {
    #[cfg(target_os = "macos")]
    {
        let home = crate::core::base::home_dir_opt().ok_or_else(|| "无法解析主目录".to_string())?;
        let whitelist = if opt.apply_optimize_whitelist {
            whitelist_optimize::load_optimize_whitelist_patterns(&home)
        } else {
            Vec::new()
        };
        let launch_agents = if whitelist_optimize::is_whitelisted_optimize(
            "check_launch_agents",
            &whitelist,
            &home,
        ) {
            None
        } else {
            Some(check_launch_agents())
        };
        let dev_tools =
            if whitelist_optimize::is_whitelisted_optimize("check_dev_tools", &whitelist, &home) {
                None
            } else {
                Some(check_dev_tools())
            };
        let versions = if whitelist_optimize::is_whitelisted_optimize(
            "check_version_mismatches",
            &whitelist,
            &home,
        ) {
            None
        } else {
            Some(check_version_mismatches())
        };
        Ok(DevEnvironmentReport {
            launch_agents,
            dev_tools,
            versions,
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("dev_environment 仅在 macOS 上可用（与 dev_environment.sh 一致）".to_string())
    }
}

pub fn check_all_dev_environment_ansi() -> Result<String, String> {
    check_all_dev_environment_ansi_with_options(DevEnvironmentOptions::default())
}

pub fn check_all_dev_environment_ansi_with_options(
    opt: DevEnvironmentOptions,
) -> Result<String, String> {
    let r = collect_dev_environment_report_with_options(opt)?;
    let mut out = String::new();
    out.push_str(&format!("{BLUE}{ICON_ARROW}{NC} Dev Environment\n"));
    if let Some(s) = r.launch_agents.as_ref() {
        out.push_str(&render_launch_agents_ansi(s));
    }
    if let Some(s) = r.dev_tools.as_ref() {
        out.push_str(&render_dev_tools_ansi(s));
    }
    if let Some(s) = r.versions.as_ref() {
        out.push_str(&render_versions_ansi(s));
    }
    Ok(out)
}

fn render_launch_agents_ansi(s: &LaunchAgentsSection) -> String {
    match s {
        LaunchAgentsSection::AllHealthy => format!("  {GREEN}✓{NC} Launch Agents All healthy\n"),
        LaunchAgentsSection::Broken {
            broken_count,
            detail,
        } => {
            format!(
                "  {GRAY}{ICON_WARNING}{NC} {:<14} {YELLOW}{} broken{NC}\n    {GRAY}{}{NC}\n",
                "Launch Agents", broken_count, detail
            )
        }
    }
}

fn render_dev_tools_ansi(s: &DevToolsSection) -> String {
    match s {
        DevToolsSection::NoneDetected => format!("  {GREEN}✓{NC} Dev Tools      None detected\n"),
        DevToolsSection::Found { count, tools } => {
            format!(
                "  {GREEN}✓{NC} Dev Tools      {} found ({})\n",
                count,
                tools.join(", ")
            )
        }
    }
}

fn render_versions_ansi(s: &VersionsSection) -> String {
    match s {
        VersionsSection::NoConflicts => format!("  {GREEN}✓{NC} Versions       No conflicts\n"),
        VersionsSection::Conflicts { messages } => format!(
            "  {GRAY}{ICON_WARNING}{NC} {:<14} {YELLOW}{}{NC}\n",
            "Versions",
            messages.join("; ")
        ),
    }
}

fn extract_major_minor(text: &str) -> Option<String> {
    let s: String = text.chars().skip_while(|c| !c.is_ascii_digit()).collect();
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i >= bytes.len() || bytes[i] != b'.' {
        return None;
    }
    let mut k = i + 1;
    while k < bytes.len() && bytes[k].is_ascii_digit() {
        k += 1;
    }
    if k == i + 1 {
        return None;
    }
    Some(s[..k].to_string())
}

fn command_v(bin: &str) -> bool {
    Command::new("/bin/sh")
        .args(["-c", &format!("command -v {bin} >/dev/null 2>&1")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn cmd_version_merged(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd)
        .args(args)
        .output()
        .map(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            let err = String::from_utf8_lossy(&o.stderr);
            if !out.trim().is_empty() {
                out.into_owned()
            } else {
                err.into_owned()
            }
        })
        .unwrap_or_default()
}

fn plist_buddy_print(plist: &Path, query: &str) -> Option<String> {
    let out = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", query, &plist.to_string_lossy()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if line.is_empty() { None } else { Some(line) }
}

fn check_launch_agents() -> LaunchAgentsSection {
    let agents_dir = crate::core::base::home_dir_opt()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join("Library/LaunchAgents");
    if !agents_dir.is_dir() {
        return LaunchAgentsSection::AllHealthy;
    }
    let mut broken_labels: Vec<String> = Vec::new();
    let entries = match fs::read_dir(&agents_dir) {
        Ok(e) => e,
        Err(_) => return LaunchAgentsSection::AllHealthy,
    };
    for plist in entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("plist"))
    {
        let label = plist
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let mut binary = plist_buddy_print(&plist, "Print :ProgramArguments:0");
        if binary.as_deref().unwrap_or("").is_empty() {
            binary = plist_buddy_print(&plist, "Print :Program");
        }
        if let Some(bin) = binary {
            if !bin.is_empty() && !Path::new(&bin).exists() {
                broken_labels.push(label);
            }
        }
    }
    if broken_labels.is_empty() {
        LaunchAgentsSection::AllHealthy
    } else {
        let preview = 3usize.min(broken_labels.len());
        let mut detail = broken_labels[..preview].join(", ");
        if broken_labels.len() > preview {
            detail.push_str(&format!(" +{}", broken_labels.len() - preview));
        }
        LaunchAgentsSection::Broken {
            broken_count: broken_labels.len() as u32,
            detail,
        }
    }
}

fn check_dev_tools() -> DevToolsSection {
    let tools = ["git", "node", "python3", "brew", "go", "xcode-select"];
    let found: Vec<String> = tools
        .iter()
        .filter(|t| command_v(t))
        .map(|t| (*t).to_string())
        .collect();
    if found.is_empty() {
        DevToolsSection::NoneDetected
    } else {
        DevToolsSection::Found {
            count: found.len(),
            tools: found,
        }
    }
}

fn check_version_mismatches() -> VersionsSection {
    let mut conflicts = Vec::new();
    if command_v("psql") && command_v("postgres") {
        let psql_ver = extract_major_minor(&cmd_version_merged("psql", &["--version"]));
        let postgres_ver = extract_major_minor(&cmd_version_merged("postgres", &["--version"]));
        if let (Some(a), Some(b)) = (psql_ver, postgres_ver) {
            if a != b {
                conflicts.push(format!("psql {a} vs server {b}"));
            }
        }
    }
    if command_v("python3") && command_v("pyenv") {
        let python_ver = extract_major_minor(&cmd_version_merged("python3", &["--version"]));
        let pyenv_ver_raw = cmd_version_merged("pyenv", &["version"])
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().next())
            .unwrap_or("")
            .to_string();
        if !pyenv_ver_raw.is_empty() && pyenv_ver_raw != "system" {
            if let (Some(a), Some(b)) = (python_ver, extract_major_minor(&pyenv_ver_raw)) {
                if a != b {
                    conflicts.push(format!("python3 {a} vs pyenv {b}"));
                }
            }
        }
    }
    if conflicts.is_empty() {
        VersionsSection::NoConflicts
    } else {
        VersionsSection::Conflicts {
            messages: conflicts,
        }
    }
}
