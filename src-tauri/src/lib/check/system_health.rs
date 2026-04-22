//! 与 `src/lib/check/all.sh` 中 `check_system_health` 及子检查对齐。

use crate::whitelist_optimize;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use wait_timeout::ChildExt;

const RED: &str = "\x1b[0;31m";
const GREEN: &str = "\x1b[0;32m";
const BLUE: &str = "\x1b[1;34m";
const YELLOW: &str = "\x1b[0;33m";
const GRAY: &str = "\x1b[0;90m";
const NC: &str = "\x1b[0m";
const ICON_WARNING: &str = "◎";
const ICON_ARROW: &str = "➤";
const ICON_INFO: &str = "ℹ";
const ICON_EMPTY: &str = "○";

#[derive(Debug, Clone, Copy)]
pub struct SystemHealthOptions {
    pub apply_optimize_whitelist: bool,
}

impl Default for SystemHealthOptions {
    fn default() -> Self {
        Self {
            apply_optimize_whitelist: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct SystemHealthReport {
    pub disk_space: DiskSpaceLine,
    pub memory_usage: MemoryUsageLine,
    pub swap_usage: Option<SwapUsageLine>,
    pub login_items: Option<LoginItemsLine>,
    pub disk_smart: Option<DiskSmartLine>,
    pub orphan_launch_agents: Option<OrphanLaunchAgentsLine>,
    pub brew_health: Option<BrewHealthLine>,
    pub brew_outdated: Option<BrewOutdatedLine>,
    pub macos_update: Option<MacOSUpdateLine>,
    pub nonstandard_apps: Option<NonstandardAppsLine>,
    pub cache_size: CacheSizeLine,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DiskSpaceLine {
    Ok { free_gb: String },
    Low { free_gb: String },
    Critical { free_gb: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MemoryUsageLine {
    Ok { used_percent: u32 },
    High { used_percent: u32 },
    Critical { used_percent: u32 },
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SwapUsageLine {
    Ok { display: String },
    High { display: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LoginItemsLine {
    None,
    Ok { count: u32, preview: String },
    Many { count: u32, preview: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DiskSmartLine {
    Verified,
    Failing,
    Other { status: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum OrphanLaunchAgentsLine {
    None,
    Some { count: u32, preview: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum BrewHealthLine {
    AllTapsInUse,
    UnusedTaps { count: u32, preview: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum NonstandardAppsLine {
    None,
    Some { count: u32, preview: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CacheSizeLine {
    Ok { size_gb: String },
    Warning { size_gb: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum BrewOutdatedLine {
    NotInstalled,
    UpToDate,
    Outdated {
        formula_count: u32,
        cask_count: u32,
        detail: String,
    },
    TimedOut,
    CheckFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MacOSUpdateLine {
    UpToDate,
    UpdateAvailable { summary: String },
}

pub fn generate_system_health_json_value() -> Result<Value, String> {
    let r = collect_system_health_report()?;
    serde_json::to_value(&r).map_err(|e| e.to_string())
}

pub fn collect_system_health_report() -> Result<SystemHealthReport, String> {
    collect_system_health_report_with_options(SystemHealthOptions::default())
}

pub fn collect_system_health_report_with_options(
    opt: SystemHealthOptions,
) -> Result<SystemHealthReport, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(collect_system_health_macos(opt))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("system_health 仅在 macOS 上可用（与 all.sh 一致）".to_string())
    }
}

pub fn check_system_health_ansi() -> Result<String, String> {
    check_system_health_ansi_with_options(SystemHealthOptions::default())
}

pub fn check_system_health_ansi_with_options(opt: SystemHealthOptions) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let r = collect_system_health_report_with_options(opt)?;
        Ok(render_system_health_ansi(&r))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("system_health 仅在 macOS 上可用（与 all.sh 一致）".to_string())
    }
}

/// 与 bash 侧 `~` / `$HOME` 对齐：空 `HOME` 时 bash 不会回退到 passwd，但本处与 `dirs` 一致
/// 供缓存与 LaunchAgent 等路径使用；避免「Rust 用 passwd、fixture 传空 HOME」导致对拍偏差。
#[cfg(target_os = "macos")]
fn resolve_user_home() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(target_os = "macos")]
fn collect_system_health_macos(opt: SystemHealthOptions) -> SystemHealthReport {
    let home = resolve_user_home();
    let wl = if opt.apply_optimize_whitelist {
        whitelist_optimize::load_optimize_whitelist_patterns(&home)
    } else {
        Vec::new()
    };

    SystemHealthReport {
        disk_space: check_disk_space_line(),
        memory_usage: check_memory_line(),
        swap_usage: check_swap_line(),
        login_items: check_login_items_line(&wl, &home),
        disk_smart: check_disk_smart_line(&wl, &home),
        orphan_launch_agents: check_orphan_agents_line(&wl, &home),
        brew_health: check_brew_health_line(&wl, &home),
        brew_outdated: check_brew_outdated_line(&wl, &home),
        macos_update: check_macos_update_line(&wl, &home),
        nonstandard_apps: check_nonstandard_apps_line(&wl, &home),
        cache_size: check_cache_size_line(&home),
    }
}

#[cfg(target_os = "macos")]
fn render_system_health_ansi(r: &SystemHealthReport) -> String {
    let mut o = String::new();
    o.push_str(&format!(
        "{b}{a}{n} System Health\n",
        b = BLUE,
        a = ICON_ARROW,
        n = NC
    ));
    o.push_str(&render_disk_space(&r.disk_space));
    o.push_str(&render_memory(&r.memory_usage));
    if let Some(ref s) = r.swap_usage {
        o.push_str(&render_swap(s));
    }
    if let Some(ref l) = r.login_items {
        o.push_str(&render_login_items(l));
    }
    if let Some(ref d) = r.disk_smart {
        o.push_str(&render_disk_smart(d));
    }
    if let Some(ref x) = r.orphan_launch_agents {
        o.push_str(&render_orphan(x));
    }
    if let Some(ref b) = r.brew_health {
        o.push_str(&render_brew(b));
    }
    if let Some(ref bo) = r.brew_outdated {
        o.push_str(&render_brew_outdated(bo));
    }
    if let Some(ref mu) = r.macos_update {
        o.push_str(&render_macos_update(mu));
    }
    if let Some(ref p) = r.nonstandard_apps {
        o.push_str(&render_nonstandard(p));
    }
    o.push_str(&render_cache(&r.cache_size));
    o
}

#[cfg(target_os = "macos")]
fn render_disk_space(d: &DiskSpaceLine) -> String {
    match d {
        DiskSpaceLine::Ok { free_gb } => format!(
            "  {g}✓{n} Disk Space   {fg}GB free\n",
            g = GREEN,
            n = NC,
            fg = free_gb
        ),
        DiskSpaceLine::Low { free_gb } => format!(
            "  {gray}{icon}{n} Disk Space   {y}{fg}GB free{n}, Low\n",
            gray = GRAY,
            icon = ICON_WARNING,
            n = NC,
            y = YELLOW,
            fg = free_gb
        ),
        DiskSpaceLine::Critical { free_gb } => format!(
            "  {r}✗{n} Disk Space   {r2}{fg}GB free{n2}, Critical\n",
            r = RED,
            r2 = RED,
            n = NC,
            fg = free_gb,
            n2 = NC
        ),
    }
}

#[cfg(target_os = "macos")]
fn render_memory(m: &MemoryUsageLine) -> String {
    match m {
        MemoryUsageLine::Ok { used_percent } => format!(
            "  {g}✓{n} Memory       {pct}% used\n",
            g = GREEN,
            n = NC,
            pct = used_percent
        ),
        MemoryUsageLine::High { used_percent } => format!(
            "  {gray}{icon}{n} Memory       {y}{pct}% used{n}, High\n",
            gray = GRAY,
            icon = ICON_WARNING,
            n = NC,
            y = YELLOW,
            pct = used_percent
        ),
        MemoryUsageLine::Critical { used_percent } => format!(
            "  {r}✗{n} Memory       {r2}{pct}% used{n}, Critical\n",
            r = RED,
            r2 = RED,
            n = NC,
            pct = used_percent
        ),
        MemoryUsageLine::Unknown => format!(
            "  {gray}-{n} Memory       Unable to determine\n",
            gray = GRAY,
            n = NC
        ),
    }
}

#[cfg(target_os = "macos")]
fn render_swap(s: &SwapUsageLine) -> String {
    match s {
        SwapUsageLine::Ok { display } => format!(
            "  {g}✓{n} Swap Usage   {d}\n",
            g = GREEN,
            n = NC,
            d = display
        ),
        SwapUsageLine::High { display } => format!(
            "  {gray}{icon}{n} Swap Usage   {y}{d}{n}, High\n",
            gray = GRAY,
            icon = ICON_WARNING,
            n = NC,
            y = YELLOW,
            d = display
        ),
    }
}

#[cfg(target_os = "macos")]
fn render_login_items(l: &LoginItemsLine) -> String {
    match l {
        LoginItemsLine::None => format!("  {g}✓{n} Login Items  None\n", g = GREEN, n = NC),
        LoginItemsLine::Ok { count, preview } => {
            let mut s = format!("  {g}✓{n} Login Items  {count} apps\n", g = GREEN, n = NC);
            s.push_str(&format!(
                "    {gray}{p}{n}\n",
                gray = GRAY,
                p = preview,
                n = NC
            ));
            s
        }
        LoginItemsLine::Many { count, preview } => {
            let mut s = format!(
                "  {gray}{icon}{n} Login Items  {y}{count} apps{n}\n",
                gray = GRAY,
                icon = ICON_WARNING,
                n = NC,
                y = YELLOW
            );
            s.push_str(&format!(
                "    {gray}{p}{n}\n",
                gray = GRAY,
                p = preview,
                n = NC
            ));
            s
        }
    }
}

#[cfg(target_os = "macos")]
fn render_disk_smart(d: &DiskSmartLine) -> String {
    match d {
        DiskSmartLine::Verified => {
            format!("  {g}✓{n} Disk Health  SMART Verified\n", g = GREEN, n = NC)
        }
        DiskSmartLine::Failing => format!(
            "  {r}✗{n} Disk Health  {r2}SMART Failing — back up immediately{n}\n",
            r = RED,
            r2 = RED,
            n = NC
        ),
        DiskSmartLine::Other { status } => format!(
            "  {gray}{icon}{n} Disk Health  {y}SMART: {st}{n}\n",
            gray = GRAY,
            icon = ICON_WARNING,
            n = NC,
            y = YELLOW,
            st = status
        ),
    }
}

#[cfg(target_os = "macos")]
fn render_orphan(x: &OrphanLaunchAgentsLine) -> String {
    match x {
        OrphanLaunchAgentsLine::None => {
            format!("  {g}✓{n} Launch Agents None orphaned\n", g = GREEN, n = NC)
        }
        OrphanLaunchAgentsLine::Some { count, preview } => {
            let s = if *count > 1 { "s" } else { "" };
            let mut o = format!(
                "  {gray}{icon}{n} Launch Agents {y}{count} orphan{s}{n}\n",
                gray = GRAY,
                icon = ICON_WARNING,
                n = NC,
                y = YELLOW
            );
            o.push_str(&format!(
                "    {gray}{p}{n}\n",
                gray = GRAY,
                p = preview,
                n = NC
            ));
            o
        }
    }
}

#[cfg(target_os = "macos")]
fn render_brew(b: &BrewHealthLine) -> String {
    match b {
        BrewHealthLine::AllTapsInUse => format!(
            "  {g}✓{n} Brew Taps    All taps in use\n",
            g = GREEN,
            n = NC
        ),
        BrewHealthLine::UnusedTaps { count, preview } => {
            let s = if *count > 1 { "s" } else { "" };
            let mut o = format!(
                "  {gray}{icon}{n} Brew Taps    {y}{count} unused tap{s}{n}\n",
                gray = GRAY,
                icon = ICON_WARNING,
                n = NC,
                y = YELLOW
            );
            o.push_str(&format!(
                "    {gray}{p}{n}\n",
                gray = GRAY,
                p = preview,
                n = NC
            ));
            o
        }
    }
}

#[cfg(target_os = "macos")]
fn render_nonstandard(p: &NonstandardAppsLine) -> String {
    match p {
        NonstandardAppsLine::None => format!(
            "  {g}✓{n} Pkg Apps     None in non-standard paths\n",
            g = GREEN,
            n = NC
        ),
        NonstandardAppsLine::Some { count, preview } => {
            let s = if *count > 1 { "s" } else { "" };
            let mut o = format!(
                "  {gray}{icon}{n} Pkg Apps     {b}{count} app{s}{n} in /usr/local or /opt\n",
                gray = GRAY,
                icon = ICON_INFO,
                n = NC,
                b = BLUE
            );
            o.push_str(&format!(
                "    {gray}{p}{n}\n",
                gray = GRAY,
                p = preview,
                n = NC
            ));
            o.push_str(&format!(
                "    {gray}Run 'rmo uninstall' to manage these apps{n}\n",
                gray = GRAY,
                n = NC
            ));
            o
        }
    }
}

#[cfg(target_os = "macos")]
fn render_cache(c: &CacheSizeLine) -> String {
    match c {
        CacheSizeLine::Ok { size_gb } => format!(
            "  {g}✓{n} Cache Size   {sg}GB\n",
            g = GREEN,
            n = NC,
            sg = size_gb
        ),
        CacheSizeLine::Warning { size_gb } => format!(
            "  {gray}{icon}{n} Cache Size   {y}{sg}GB{n} cleanable\n",
            gray = GRAY,
            icon = ICON_WARNING,
            n = NC,
            y = YELLOW,
            sg = size_gb
        ),
    }
}

#[cfg(target_os = "macos")]
fn render_brew_outdated(bo: &BrewOutdatedLine) -> String {
    match bo {
        BrewOutdatedLine::NotInstalled => format!(
            "  {gray}{icon}{n} {:<12} {}\n",
            "Homebrew",
            "Not installed",
            gray = GRAY,
            icon = ICON_EMPTY,
            n = NC
        ),
        BrewOutdatedLine::UpToDate => format!(
            "  {g}✓{n} {:<12} {}\n",
            "Homebrew",
            "Up to date",
            g = GREEN,
            n = NC
        ),
        BrewOutdatedLine::Outdated { detail, .. } => format!(
            "  {gray}{icon}{n} {:<12} {y}{detail}{n}\n",
            "Homebrew",
            gray = GRAY,
            icon = ICON_WARNING,
            n = NC,
            y = YELLOW
        ),
        BrewOutdatedLine::TimedOut => format!(
            "  {gray}{icon}{n} {:<12} {y}Check timed out{n}\n",
            "Homebrew",
            gray = GRAY,
            icon = ICON_WARNING,
            n = NC,
            y = YELLOW
        ),
        BrewOutdatedLine::CheckFailed => format!(
            "  {gray}{icon}{n} {:<12} {y}Check failed{n}\n",
            "Homebrew",
            gray = GRAY,
            icon = ICON_WARNING,
            n = NC,
            y = YELLOW
        ),
    }
}

#[cfg(target_os = "macos")]
fn render_macos_update(mu: &MacOSUpdateLine) -> String {
    match mu {
        MacOSUpdateLine::UpToDate => format!(
            "  {g}✓{n} {:<12} {}\n",
            "macOS",
            "System up to date",
            g = GREEN,
            n = NC
        ),
        MacOSUpdateLine::UpdateAvailable { summary } => {
            format!(
                "  {gray}{icon}{n} {:<12} {y}{summary}{n}\n",
                "macOS",
                gray = GRAY,
                icon = ICON_WARNING,
                n = NC,
                y = YELLOW
            )
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_path_env() -> String {
    let tail = std::env::var("PATH").unwrap_or_default();
    format!("/usr/bin:/bin:/usr/sbin:/sbin:{tail}")
}

#[cfg(target_os = "macos")]
fn mole_skip_applescript() -> bool {
    matches!(std::env::var("MOLE_TEST_MODE").as_deref(), Ok("1"))
        || matches!(std::env::var("MOLE_TEST_NO_AUTH").as_deref(), Ok("1"))
}

#[cfg(target_os = "macos")]
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Option<std::process::Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().ok()?;
    match child.wait_timeout(timeout).ok()? {
        Some(_) => child.wait_with_output().ok(),
        None => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

#[cfg(target_os = "macos")]
fn check_disk_space_line() -> DiskSpaceLine {
    let out = Command::new("/bin/sh")
        .env("PATH", macos_path_env())
        .args(["-c", "df -k / | awk 'NR==2 {print $4}'"])
        .output()
        .ok();
    let free_kb: u64 = out
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let free_num = (free_kb / 1_048_576) as i64;
    let free_gb = format!("{:.1}", free_kb as f64 / 1_048_576.0);
    // 对齐 SH 第 594 行:`export DISK_FREE_GB=$free_num`(整数 GB)。
    std::env::set_var("DISK_FREE_GB", free_num.to_string());
    if free_num < 20 {
        DiskSpaceLine::Critical { free_gb }
    } else if free_num < 50 {
        DiskSpaceLine::Low { free_gb }
    } else {
        DiskSpaceLine::Ok { free_gb }
    }
}

#[cfg(target_os = "macos")]
fn check_memory_line() -> MemoryUsageLine {
    let mem_total: u64 = Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    if mem_total == 0 {
        return MemoryUsageLine::Unknown;
    }
    let vm_out = Command::new("/usr/bin/vm_stat")
        .output()
        .map(|o| o.stdout)
        .unwrap_or_default();
    let vm = String::from_utf8_lossy(&vm_out);
    let page_size: u64 = vm
        .lines()
        .find(|l| l.contains("page size of"))
        .and_then(|l| l.split_whitespace().nth(7))
        .and_then(|s| s.parse().ok())
        .unwrap_or(4096);
    let parse_pages = |key: &str| -> u64 {
        vm.lines()
            .find(|l| l.contains(key))
            .and_then(|l| l.split_whitespace().nth(2))
            .map(|s| s.trim_end_matches('.').to_string().parse().unwrap_or(0))
            .unwrap_or(0)
    };
    let free_pages = parse_pages("Pages free");
    let inactive_pages = parse_pages("Pages inactive");
    let spec_pages = parse_pages("Pages speculative");
    let total_pages = mem_total / page_size;
    let free_total = free_pages + inactive_pages + spec_pages;
    let used_pages = total_pages.saturating_sub(free_total);
    let mut used_percent = ((used_pages as f64 / total_pages as f64) * 100.0).round() as u32;
    if used_percent > 100 {
        used_percent = 100;
    }
    if used_percent > 90 {
        MemoryUsageLine::Critical { used_percent }
    } else if used_percent > 80 {
        MemoryUsageLine::High { used_percent }
    } else {
        MemoryUsageLine::Ok { used_percent }
    }
}

#[cfg(target_os = "macos")]
fn check_swap_line() -> Option<SwapUsageLine> {
    let out = Command::new("/usr/sbin/sysctl")
        .args(["vm.swapusage"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout);
    if text.trim().is_empty() {
        return None;
    }
    let used = text
        .lines()
        .find(|l| l.contains("used ="))?
        .split("used =")
        .nth(1)?
        .split_whitespace()
        .next()?
        .trim()
        .to_string();
    if used.contains('G') {
        let num = used.trim_end_matches('G').parse::<f64>().unwrap_or(0.0);
        let gb_floor = num as i32;
        if gb_floor > 2 {
            return Some(SwapUsageLine::High { display: used });
        }
    }
    Some(SwapUsageLine::Ok { display: used })
}

#[cfg(target_os = "macos")]
fn check_login_items_line(wl: &[String], home: &Path) -> Option<LoginItemsLine> {
    if whitelist_optimize::is_whitelisted_optimize("check_login_items", wl, home) {
        return None;
    }
    let mut items: Vec<String> = Vec::new();
    if std::io::stdin().is_terminal() && !mole_skip_applescript() {
        if Command::new("/bin/sh")
            .env("PATH", macos_path_env())
            .args(["-c", "command -v osascript >/dev/null 2>&1"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            let script = "tell application \"System Events\" to get the name of every login item";
            if let Ok(out) = Command::new("/usr/bin/osascript")
                .args(["-e", script])
                .output()
            {
                let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !raw.is_empty() && raw != "missing value" {
                    items = raw
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
        }
    }
    let count = items.len() as u32;
    if count == 0 {
        return Some(LoginItemsLine::None);
    }
    let preview = preview_csv(&items, 3);
    if count > 15 {
        Some(LoginItemsLine::Many { count, preview })
    } else {
        Some(LoginItemsLine::Ok { count, preview })
    }
}

#[cfg(target_os = "macos")]
fn preview_csv(items: &[String], limit: usize) -> String {
    let n = limit.min(items.len());
    if n == 0 {
        return String::new();
    }
    let parts: Vec<&str> = items.iter().take(n).map(String::as_str).collect();
    let mut s = parts.join(", ");
    if items.len() > n {
        s.push_str(&format!(" +{}", items.len() - n));
    }
    s
}

#[cfg(target_os = "macos")]
fn check_disk_smart_line(wl: &[String], home: &Path) -> Option<DiskSmartLine> {
    if whitelist_optimize::is_whitelisted_optimize("check_disk_smart", wl, home) {
        return None;
    }
    if !Command::new("/bin/sh")
        .env("PATH", macos_path_env())
        .args(["-c", "command -v diskutil >/dev/null 2>&1"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return None;
    }
    let root_out = Command::new("/usr/sbin/diskutil")
        .args(["info", "/"])
        .output()
        .ok()?;
    if !root_out.status.success() {
        return None;
    }
    let root_info = String::from_utf8_lossy(&root_out.stdout);
    let boot = root_info.lines().find(|l| l.contains("Part of Whole:"))?;
    let disk = boot.split(':').nth(1)?.trim();
    if disk.is_empty() {
        return None;
    }
    let disk_out = Command::new("/usr/sbin/diskutil")
        .args(["info", disk])
        .output()
        .ok()?;
    if !disk_out.status.success() {
        return None;
    }
    let disk_info = String::from_utf8_lossy(&disk_out.stdout);
    let smart_line = disk_info.lines().find(|l| l.contains("SMART Status:"))?;
    let st = smart_line.split(':').nth(1)?.trim();
    if st.is_empty() {
        return None;
    }
    Some(if st == "Verified" {
        DiskSmartLine::Verified
    } else if st == "Failing" {
        DiskSmartLine::Failing
    } else {
        DiskSmartLine::Other {
            status: st.to_string(),
        }
    })
}

#[cfg(target_os = "macos")]
fn launch_agent_dirs(home: &Path) -> Vec<PathBuf> {
    let default = format!(
        "{}:/Library/LaunchAgents",
        home.join("Library/LaunchAgents").display()
    );
    let raw = std::env::var("MOLE_LAUNCH_AGENT_DIRS").unwrap_or(default);
    raw.split(':').map(PathBuf::from).collect()
}

#[cfg(target_os = "macos")]
fn check_orphan_agents_line(wl: &[String], home: &Path) -> Option<OrphanLaunchAgentsLine> {
    if whitelist_optimize::is_whitelisted_optimize("check_orphan_launch_agents", wl, home) {
        return None;
    }
    let mut orphans: Vec<String> = Vec::new();
    for dir in launch_agent_dirs(home) {
        if !dir.is_dir() {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for ent in rd.flatten() {
            let path = ent.path();
            if !path.extension().map(|e| e == "plist").unwrap_or(false) {
                continue;
            }
            let label = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if label.starts_with("com.apple.") {
                continue;
            }
            if let Some(p) = plutil_program(&path) {
                if p.starts_with('/') && !Path::new(&p).exists() {
                    orphans.push(label);
                }
            }
        }
    }
    orphans.sort();
    orphans.dedup();
    if orphans.is_empty() {
        return Some(OrphanLaunchAgentsLine::None);
    }
    let count = orphans.len() as u32;
    let preview = preview_csv(
        &orphans.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        3,
    );
    Some(OrphanLaunchAgentsLine::Some { count, preview })
}

/// 与 `all.sh` `check_orphan_launch_agents` 一致：仅在**第一次** `plutil` 失败时才尝试 `ProgramArguments.0`，
/// 第一次 exit 0 时即使 stdout 为空也不再回退（避免误用 `.0` 把存在的路径当成孤儿）。
#[cfg(target_os = "macos")]
fn plutil_program(plist: &Path) -> Option<String> {
    fn raw_output(o: &std::process::Output) -> String {
        let s = String::from_utf8_lossy(&o.stdout);
        // bash `$(plutil …)` 只去掉末尾换行，不 trim 中间空格
        let s = s.trim_end_matches(|c| c == '\n' || c == '\r');
        if s == "null" {
            String::new()
        } else {
            s.to_string()
        }
    }
    let o = Command::new("/usr/bin/plutil")
        .args(["-extract", "Program", "raw", "-o", "-"])
        .arg(plist)
        .output()
        .ok()?;
    if o.status.success() {
        return Some(raw_output(&o));
    }
    let o2 = Command::new("/usr/bin/plutil")
        .args(["-extract", "ProgramArguments.0", "raw", "-o", "-"])
        .arg(plist)
        .output()
        .ok()?;
    if o2.status.success() {
        return Some(raw_output(&o2));
    }
    None
}

#[cfg(target_os = "macos")]
fn check_brew_health_line(wl: &[String], home: &Path) -> Option<BrewHealthLine> {
    if whitelist_optimize::is_whitelisted_optimize("check_brew_health", wl, home) {
        return None;
    }
    if !Command::new("/bin/sh")
        .env("PATH", macos_path_env())
        .args(["-c", "command -v brew >/dev/null 2>&1"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return None;
    }
    let installed = {
        let mut cmd = Command::new("brew");
        cmd.env("PATH", macos_path_env())
            .args(["list", "--full-name"])
            .stderr(Stdio::null());
        run_with_timeout(cmd, Duration::from_secs(5))
    }
    .filter(|o| o.status.success())
    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
    .unwrap_or_default();
    let tap_stdout = {
        let mut cmd = Command::new("brew");
        cmd.env("PATH", macos_path_env())
            .args(["tap"])
            .stderr(Stdio::null());
        run_with_timeout(cmd, Duration::from_secs(5))
    }
    .filter(|o| o.status.success())
    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
    .unwrap_or_default();
    let mut stale: Vec<String> = Vec::new();
    for line in tap_stdout.lines() {
        let tap = line.trim();
        if tap.is_empty() {
            continue;
        }
        if tap == "homebrew/core" || tap == "homebrew/cask" {
            continue;
        }
        let prefix = format!("{tap}/");
        if !installed.lines().any(|l| l.starts_with(prefix.as_str())) {
            stale.push(tap.to_string());
        }
    }
    if stale.is_empty() {
        return Some(BrewHealthLine::AllTapsInUse);
    }
    let count = stale.len() as u32;
    let preview = preview_csv(&stale, 2);
    Some(BrewHealthLine::UnusedTaps { count, preview })
}

#[cfg(target_os = "macos")]
fn check_nonstandard_apps_line(wl: &[String], home: &Path) -> Option<NonstandardAppsLine> {
    if whitelist_optimize::is_whitelisted_optimize("check_nonstandard_apps", wl, home) {
        return None;
    }
    if !Command::new("/bin/sh")
        .env("PATH", macos_path_env())
        .args(["-c", "command -v pkgutil >/dev/null 2>&1"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return None;
    }
    let names = pkg_nonstandard_app_basenames();
    if names.is_empty() {
        return Some(NonstandardAppsLine::None);
    }
    let preview = preview_csv(&names, 3);
    Some(NonstandardAppsLine::Some {
        count: names.len() as u32,
        preview,
    })
}

#[cfg(target_os = "macos")]
fn pkg_nonstandard_app_basenames() -> Vec<String> {
    let scan_timeout = std::env::var("MOLE_PKG_RECEIPT_SCAN_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(8);
    let list_timeout = std::env::var("MOLE_PKG_RECEIPT_LIST_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(3);
    let start = Instant::now();
    let pkgs_out = {
        let mut cmd = Command::new("/usr/sbin/pkgutil");
        cmd.args(["--pkgs"]).stderr(Stdio::null());
        run_with_timeout(cmd, Duration::from_secs(list_timeout))
    };
    let Some(pkgs_out) = pkgs_out else {
        return Vec::new();
    };
    if !pkgs_out.status.success() {
        return Vec::new();
    }
    let pkgs: Vec<String> = String::from_utf8_lossy(&pkgs_out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut names: Vec<String> = Vec::new();
    'pkgs: for pkg_id in pkgs {
        if start.elapsed().as_secs() >= scan_timeout {
            break;
        }
        if pkg_id.starts_with("com.apple.") {
            continue;
        }
        let files_out = Command::new("/usr/sbin/pkgutil")
            .args(["--files", &pkg_id])
            .stderr(Stdio::null())
            .output()
            .ok();
        let Some(files_out) = files_out else {
            continue;
        };
        if !files_out.status.success() {
            continue;
        }
        for rel in String::from_utf8_lossy(&files_out.stdout).lines() {
            if start.elapsed().as_secs() >= scan_timeout {
                break 'pkgs;
            }
            let line = rel.trim();
            if !line.contains(".app") {
                continue;
            }
            let stripped = line.strip_prefix('/').unwrap_or(line);
            if !stripped.starts_with("usr/local/") && !stripped.starts_with("opt/") {
                continue;
            }
            let app_path = normalize_pkg_app_path(stripped);
            let Some(app_path) = app_path else { continue };
            if !app_path.is_dir() {
                continue;
            }
            if seen.insert(app_path.clone()) {
                let base = app_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("App")
                    .to_string();
                names.push(base);
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

#[cfg(target_os = "macos")]
fn normalize_pkg_app_path(rel: &str) -> Option<PathBuf> {
    let stripped = rel.trim().trim_start_matches('/');
    if stripped.is_empty() {
        return None;
    }
    let candidate = PathBuf::from("/").join(stripped);
    let s = candidate.to_string_lossy();
    if s.ends_with(".app") && candidate.is_dir() {
        return Some(candidate);
    }
    let idx = s.find(".app/")?;
    let root = PathBuf::from(format!("{}.app", &s[..idx]));
    if root.is_dir() { Some(root) } else { None }
}

/// 对齐 `file_ops.sh` `get_path_size_kb`：macOS 上 `du` 对部分受保护子目录会非零退出，但仍会在首行输出目录总计。
#[cfg(target_os = "macos")]
fn du_sk_path(path: &Path) -> u64 {
    let Some(out) = Command::new("/usr/bin/du")
        .args(["-skP", &path.to_string_lossy()])
        .output()
        .ok()
    else {
        return 0;
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
fn check_cache_size_line(home: &Path) -> CacheSizeLine {
    let paths = [home.join("Library/Caches"), home.join("Library/Logs")];
    let mut cache_size_kb: u64 = 0;
    for p in paths {
        if p.is_dir() {
            cache_size_kb += du_sk_path(&p);
        }
    }
    let cache_size_gb = cache_size_kb as f64 / 1024.0 / 1024.0;
    let gb_str = format!("{:.1}", cache_size_gb);
    // 对齐 SH 第 728 行:`export CACHE_SIZE_GB=$cache_size_gb`(保留一位小数的字符串)。
    std::env::set_var("CACHE_SIZE_GB", &gb_str);
    // 对齐 SH 第 736-744 行:`cache_size_int=$(echo "$cache_size_gb" | cut -d'.' -f1)`,
    // 然后只在 `> 10` 或 `> 5` 时报警(语义上等价 `> 5`)。
    let cache_int: i64 = gb_str
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if cache_int > 5 {
        CacheSizeLine::Warning { size_gb: gb_str }
    } else {
        CacheSizeLine::Ok { size_gb: gb_str }
    }
}

/// 对齐 SH 第 324-428 行 `check_homebrew_updates`:
/// - 缓存文件 `$HOME/.cache/mole/brew_updates`,TTL 600s,内容 `<formula> <cask>`;
/// - 并发执行两次 `brew outdated`(对齐 SH 第 369-378 行 `&` + `wait`);
/// - 仅当**两侧都成功**才写缓存(对齐 SH 第 396-399 行的注释);
/// - 任一侧 timeout(124) 整体返回 `TimedOut`(对齐 SH 第 400-402);
/// - 同步 export `BREW_FORMULA_OUTDATED_COUNT` / `BREW_CASK_OUTDATED_COUNT` /
///   `BREW_OUTDATED_COUNT`(对齐 SH 第 410-412),供 `manage/update.rs` 直接读取。
#[cfg(target_os = "macos")]
fn check_brew_outdated_line(wl: &[String], home: &Path) -> Option<BrewOutdatedLine> {
    if whitelist_optimize::is_whitelisted_optimize("check_brew_outdated", wl, home) {
        return None;
    }
    // 进入函数即重置三个 env,对齐 SH 第 328-330 行。
    std::env::set_var("BREW_OUTDATED_COUNT", "0");
    std::env::set_var("BREW_FORMULA_OUTDATED_COUNT", "0");
    std::env::set_var("BREW_CASK_OUTDATED_COUNT", "0");

    if !Command::new("/bin/sh")
        .env("PATH", macos_path_env())
        .args(["-c", "command -v brew >/dev/null 2>&1"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Some(BrewOutdatedLine::NotInstalled);
    }

    let cache_file = home.join(".cache/mole/brew_updates");

    // 1) 命中文件缓存:直接复用结果,避免再跑 `brew outdated`(SH 第 343-352)。
    if is_cache_file_valid(&cache_file, 600) {
        if let Some((f, c)) = read_brew_cache(&cache_file) {
            return Some(finalize_brew_counts(f, c));
        }
    }

    // 2) 并发跑 formula / cask outdated(SH 第 369-378)。
    let path_env = macos_path_env();
    let path_env_a = path_env.clone();
    let formula_handle = std::thread::spawn(move || {
        let mut cmd = Command::new("brew");
        cmd.env("PATH", &path_env_a)
            .args(["outdated", "--formula", "--quiet"])
            .stderr(Stdio::null());
        run_with_timeout(cmd, Duration::from_secs(8))
    });
    let cask_handle = std::thread::spawn(move || {
        let mut cmd = Command::new("brew");
        cmd.env("PATH", &path_env)
            .args(["outdated", "--cask", "--quiet"])
            .stderr(Stdio::null());
        run_with_timeout(cmd, Duration::from_secs(8))
    });
    let formula_out = formula_handle.join().ok().flatten();
    let cask_out = cask_handle.join().ok().flatten();

    let formula_ok = formula_out.as_ref().is_some_and(|o| o.status.success());
    let cask_ok = cask_out.as_ref().is_some_and(|o| o.status.success());

    // SH 第 391 行:formula_status 或 cask_status 任一为 0 即继续解析;
    // 都失败时再检查是否 124 超时。
    if !formula_ok && !cask_ok {
        let timed_out = formula_out
            .as_ref()
            .is_some_and(|o| o.status.code() == Some(124))
            || cask_out
                .as_ref()
                .is_some_and(|o| o.status.code() == Some(124))
            || formula_out.is_none()
            || cask_out.is_none();
        if timed_out {
            return Some(BrewOutdatedLine::TimedOut);
        }
        return Some(BrewOutdatedLine::CheckFailed);
    }

    let formula_count = formula_out
        .as_ref()
        .filter(|o| o.status.success())
        .map(|o| count_nonempty_lines(&o.stdout))
        .unwrap_or(0);
    let cask_count = cask_out
        .as_ref()
        .filter(|o| o.status.success())
        .map(|o| count_nonempty_lines(&o.stdout))
        .unwrap_or(0);

    // 仅在两侧都成功时写缓存(SH 第 396-399 行的注释:partial 结果不能写)。
    if formula_ok && cask_ok {
        if let Some(parent) = cache_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&cache_file, format!("{formula_count} {cask_count}\n"));
    }

    Some(finalize_brew_counts(formula_count, cask_count))
}

#[cfg(target_os = "macos")]
fn count_nonempty_lines(bytes: &[u8]) -> u32 {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count() as u32
}

#[cfg(target_os = "macos")]
fn is_cache_file_valid(p: &Path, ttl_secs: u64) -> bool {
    if !p.exists() {
        return false;
    }
    let mtime = crate::core::base::get_file_mtime(&p.to_string_lossy());
    let now = crate::core::base::get_epoch_seconds();
    now.saturating_sub(mtime) < ttl_secs
}

#[cfg(target_os = "macos")]
fn read_brew_cache(p: &Path) -> Option<(u32, u32)> {
    let raw = std::fs::read_to_string(p).ok()?;
    let line = raw.lines().next()?.trim();
    let mut it = line.split_whitespace();
    let f = it.next()?.parse().ok()?;
    let c = it.next()?.parse().ok()?;
    Some((f, c))
}

#[cfg(target_os = "macos")]
fn finalize_brew_counts(formula_count: u32, cask_count: u32) -> BrewOutdatedLine {
    let total = formula_count + cask_count;
    std::env::set_var("BREW_FORMULA_OUTDATED_COUNT", formula_count.to_string());
    std::env::set_var("BREW_CASK_OUTDATED_COUNT", cask_count.to_string());
    std::env::set_var("BREW_OUTDATED_COUNT", total.to_string());
    if total == 0 {
        return BrewOutdatedLine::UpToDate;
    }
    let mut parts = Vec::new();
    if formula_count > 0 {
        parts.push(format!("{formula_count} formula"));
    }
    if cask_count > 0 {
        parts.push(format!("{cask_count} cask"));
    }
    let detail = parts.join(", ");
    BrewOutdatedLine::Outdated {
        formula_count,
        cask_count,
        detail,
    }
}

#[cfg(target_os = "macos")]
fn check_macos_update_line(wl: &[String], home: &Path) -> Option<MacOSUpdateLine> {
    if whitelist_optimize::is_whitelisted_optimize("check_macos_updates", wl, home) {
        // 与 SH 一致:进白名单不打印也不 export(下游 manage 用 `false` 默认值即可)。
        return None;
    }
    // 对齐 SH 第 440-456 行的 update 判定:
    // 1. 必须出现 `* Label:` 行;
    // 2. summary 非空 OR 整段文本被 `is_macos_software_update_text` 匹配。
    let sw_output = get_softwareupdate_list(home);
    let summary = get_first_macos_update_summary(&sw_output);
    let updates_available = software_update_has_entries(&sw_output)
        && (!summary.is_empty() || is_macos_software_update_text(&sw_output));
    std::env::set_var(
        "MACOS_UPDATE_AVAILABLE",
        if updates_available { "true" } else { "false" },
    );
    if !updates_available {
        return Some(MacOSUpdateLine::UpToDate);
    }
    Some(MacOSUpdateLine::UpdateAvailable {
        summary: if summary.is_empty() {
            "Update available".to_string()
        } else {
            summary
        },
    })
}

/// 对齐 SH 第 244-255 行 `is_macos_software_update_text`:
/// 整段文本(小写)若包含 `macos` / `background security improvement` /
/// `rapid security response` / `security response` 任一,则视为 macOS 系统更新。
#[cfg(target_os = "macos")]
fn is_macos_software_update_text(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("macos")
        || lower.contains("background security improvement")
        || lower.contains("rapid security response")
        || lower.contains("security response")
}

#[cfg(target_os = "macos")]
fn softwareupdate_cache_file(home: &Path) -> PathBuf {
    home.join(".cache/mole/softwareupdate_list")
}

#[cfg(target_os = "macos")]
fn softwareupdate_cache_ttl() -> u64 {
    600
}

#[cfg(target_os = "macos")]
fn is_softwareupdate_cache_valid(home: &Path) -> bool {
    let cf = softwareupdate_cache_file(home);
    if !cf.exists() {
        return false;
    }
    let mtime = crate::core::base::get_file_mtime(&cf.to_string_lossy());
    let now = crate::core::base::get_epoch_seconds();
    let age = now.saturating_sub(mtime);
    age < softwareupdate_cache_ttl()
}

/// 对齐 SH 第 278-322 行 `get_software_updates`:
/// - 进程内只缓存「本次成功取到的内容」一次(SH 用 `SOFTWARE_UPDATE_LIST_LOADED`),
///   避免同一次 `mo check` 内重复调用 `softwareupdate -l`(慢)。
/// - 文件缓存 TTL 600s,失败时回退到旧缓存(SH 第 305-310 行)。
/// - **不再用 `OnceLock<String>`**:它一旦写入就再也无法刷新,
///   导致一次失败缓存空串后整个进程都拿不到结果(原 Rust 的 bug)。
///   这里改用 `Mutex<Option<String>>` + `AtomicBool`-语义的 loaded 标志。
#[cfg(target_os = "macos")]
fn get_softwareupdate_list(home: &Path) -> String {
    // 进程内 memo:SH 通过两个 shell 变量控制,这里用单一 Mutex<Option> 表达
    //   `None`        => 尚未加载,需要走 cache/subprocess
    //   `Some(text)`  => 已加载本次结果(可能为空字符串,若失败且无旧缓存)
    static SW_LIST: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    let slot = SW_LIST.get_or_init(|| Mutex::new(None));

    if let Ok(guard) = slot.lock() {
        if let Some(ref s) = *guard {
            return s.clone();
        }
    }

    let cf = softwareupdate_cache_file(home);

    // 命中文件缓存(TTL 内)直接读
    if is_softwareupdate_cache_valid(home) {
        if let Ok(content) = std::fs::read_to_string(&cf) {
            if let Ok(mut guard) = slot.lock() {
                *guard = Some(content.clone());
            }
            return content;
        }
    }

    // 调用 softwareupdate -l --no-scan(对齐 SH 第 301)
    let mut cmd = Command::new("softwareupdate");
    cmd.args(["-l", "--no-scan"]).stderr(Stdio::null());
    let output = run_with_timeout(cmd, Duration::from_secs(10));
    let success_text = output
        .as_ref()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string());

    let final_text = if let Some(text) = success_text {
        // 写入文件缓存(对齐 SH 第 302-304)
        if !text.is_empty() {
            if let Some(parent) = cf.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&cf, &text);
        }
        text
    } else {
        // 失败:回退到磁盘旧缓存(对齐 SH 第 305-310 行)。
        std::fs::read_to_string(&cf).unwrap_or_default()
    };

    if let Ok(mut guard) = slot.lock() {
        *guard = Some(final_text.clone());
    }
    final_text
}

#[cfg(target_os = "macos")]
fn software_update_has_entries(text: &str) -> bool {
    text.lines().any(|l| l.trim().starts_with("* Label:"))
}

#[cfg(target_os = "macos")]
fn get_first_macos_update_summary(text: &str) -> String {
    let mut label = String::new();
    let mut in_target = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(l) = trimmed.strip_prefix("* Label:") {
            label = l.trim().to_string();
            in_target = false;
        }
        if let Some(t) = trimmed.strip_prefix("Title:") {
            let title = t.trim();
            let title_no_ver = title
                .split(", Version:")
                .next()
                .unwrap_or(title)
                .split(", Size:")
                .next()
                .unwrap_or(title)
                .trim();
            let combined = format!("{label} {title_no_ver}").to_lowercase();
            if combined.contains("macos")
                || combined.contains("background security improvement")
                || combined.contains("rapid security response")
                || combined.contains("security response")
            {
                return title_no_ver.to_string();
            }
            in_target = true;
        }
        if in_target {
            continue;
        }
    }
    String::new()
}
