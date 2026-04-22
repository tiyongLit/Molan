//! 对齐 `lib/optimize/diagnostics.sh`。
//!
//! GUI 端不需要 `read_key` TUI 交互,因此 `offer_detach_candidates` 不会真的提示按键,
//! 实际确认/调用 `hdiutil detach` 由前端弹窗 + Tauri command 触发(参考 SH 第 301-341 行)。
//! 其它纯数据/计算函数严格按 SH 翻译。

use std::process::Command;

use serde::Serialize;

use crate::core::app_protection::{is_path_whitelisted_from_global, should_protect_path};
use crate::core::base::{BLUE, GRAY, GREEN, NC, YELLOW};
use crate::core::base::{ICON_INFO, ICON_LIST, ICON_REVIEW, ICON_SUCCESS, ICON_WARNING};
use crate::core::timeout::run_with_timeout_capture;

/// 对齐 SH 第 6 行常量。
pub const MOLE_OPTIMIZE_DIAG_CPU_THRESHOLD_DEFAULT: f64 = 25.0;
/// 对齐 SH 第 7 行常量。
pub const MOLE_OPTIMIZE_DIAG_SAMPLE_DELAY_DEFAULT: f64 = 1.0;

// ============================================================================
// 内存 / 虚拟机 / 失控进程诊断（对齐 SH L452-694）
// ============================================================================

/// 对齐 SH L462 `MOLE_OPTIMIZE_SWAP_PCT_DEFAULT`。
const SWAP_PCT_DEFAULT: u32 = 50;
/// 对齐 SH L463 `MOLE_OPTIMIZE_FREE_PCT_DEFAULT`。
const FREE_PCT_DEFAULT: u32 = 15;
/// 对齐 SH L464 `MOLE_OPTIMIZE_IDLE_VM_GB_DEFAULT`。
const IDLE_VM_GB_DEFAULT: u64 = 2;
/// 对齐 SH L465 `MOLE_OPTIMIZE_RUNAWAY_PCT_DEFAULT`。
const RUNAWAY_PCT_DEFAULT: u32 = 25;
/// 对齐 SH L466 `MOLE_OPTIMIZE_RUNAWAY_MIN_HOURS_DEFAULT`。
const RUNAWAY_MIN_HOURS_DEFAULT: u64 = 12;

/// 内存压力诊断结果（GUI 序列化给前端，SH 直接打印文本）。
#[derive(Serialize, Clone, Debug)]
pub struct MemoryPressure {
    pub swap_total_mb: u64,
    pub swap_used_mb: u64,
    pub swap_pct: u32,
    pub free_pct: u32,
    pub top_holders: Vec<ProcessHolder>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProcessHolder {
    pub name: String,
    pub rss_kb: u64,
}

/// 空闲虚拟机诊断结果。
#[derive(Serialize, Clone, Debug)]
pub struct IdleVm {
    pub vm_kb: u64,
    /// `None` = docker 不可用, `Some(0)` = 无运行中容器。
    pub docker_running: Option<u32>,
}

/// 失控进程诊断结果。
#[derive(Serialize, Clone, Debug)]
pub struct RunawayProcess {
    pub pid: u32,
    pub name: String,
    pub cpu_hours: u32,
    pub run_hours: u32,
    pub pct: u32,
}

/// 对齐 SH 第 9-15 行 `opt_diag_cpu_threshold`。
pub fn opt_diag_cpu_threshold() -> f64 {
    std::env::var("MOLE_OPTIMIZE_DIAG_CPU_THRESHOLD")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(MOLE_OPTIMIZE_DIAG_CPU_THRESHOLD_DEFAULT)
}

/// 对齐 SH 第 17-23 行 `opt_diag_sample_delay`。
pub fn opt_diag_sample_delay() -> f64 {
    std::env::var("MOLE_OPTIMIZE_DIAG_SAMPLE_DELAY")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(MOLE_OPTIMIZE_DIAG_SAMPLE_DELAY_DEFAULT)
}

/// 对齐 SH 第 43-58 行 `opt_diag_get_ps_sample`。
/// `index` ∈ {1, 2}。允许 env override(用于测试)。
pub fn opt_diag_get_ps_sample(index: u32) -> String {
    let env_key = match index {
        1 => "MOLE_OPTIMIZE_PS_SAMPLE_1",
        2 => "MOLE_OPTIMIZE_PS_SAMPLE_2",
        _ => "",
    };
    if !env_key.is_empty() {
        if let Ok(v) = std::env::var(env_key) {
            return v;
        }
    }
    Command::new("ps")
        .args(["-Aceo", "pcpu=,command="])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// 对齐 SH 第 60-67 行 `opt_diag_get_spctl_status`。
pub fn opt_diag_get_spctl_status() -> String {
    if let Ok(v) = std::env::var("MOLE_OPTIMIZE_SPCTL_STATUS") {
        return v;
    }
    Command::new("spctl")
        .arg("--status")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// 对齐 SH 第 69-76 行 `opt_diag_get_hdiutil_info`,带 8s 超时。
pub fn opt_diag_get_hdiutil_info() -> String {
    if let Ok(v) = std::env::var("MOLE_OPTIMIZE_HDIUTIL_INFO") {
        return v;
    }
    run_with_timeout_capture(8.0, "hdiutil", &["info"]).unwrap_or_default()
}

/// 命令名 → 进程家族,对齐 SH 第 81-89 行 `awk classify`。
fn classify_command(cmd: &str) -> &'static str {
    let lower = cmd.to_lowercase();
    if lower.contains("cloudshell") || lower.contains("alientsafe") || lower.contains("aliedr") {
        return "cloudshell";
    }
    if matches_word(&lower, "syspolicyd") {
        return "syspolicyd";
    }
    if matches_word(&lower, "windowserver") {
        return "windowserver";
    }
    if matches_word(&lower, "mds")
        || lower.contains("mdworker")
        || lower.contains("mds_stores")
        || lower.contains("mdbulkimport")
    {
        return "spotlight";
    }
    if lower.contains("diskimagesiod") || lower.contains("simdiskimaged") {
        return "coresim_disk_images";
    }
    ""
}

/// 对齐 SH awk regex `(^|\/)<name>([[:space:]]|$)`:
/// 在路径分隔/行首与空白/行尾边界匹配 `name`,避免误匹配子串。
fn matches_word(haystack: &str, needle: &str) -> bool {
    let hay = haystack.as_bytes();
    let need = needle.as_bytes();
    let mut i = 0usize;
    while i + need.len() <= hay.len() {
        if &hay[i..i + need.len()] == need {
            let left_ok = i == 0 || hay[i - 1] == b'/';
            let right_idx = i + need.len();
            let right_ok =
                right_idx == hay.len() || hay[right_idx] == b' ' || hay[right_idx] == b'\t';
            if left_ok && right_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// 对齐 SH 第 78-105 行 `opt_diag_family_totals`。
/// 输入是 `ps -Aceo pcpu=,command=` 的原始输出;返回 5 个家族 → CPU 累计百分比。
pub fn opt_diag_family_totals(raw: &str) -> [(&'static str, f64); 5] {
    let mut sums = [
        ("cloudshell", 0.0_f64),
        ("syspolicyd", 0.0_f64),
        ("windowserver", 0.0_f64),
        ("spotlight", 0.0_f64),
        ("coresim_disk_images", 0.0_f64),
    ];
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.splitn(2, |c: char| c.is_whitespace());
        let cpu_token = parts.next().unwrap_or("0");
        let rest = parts.next().unwrap_or("").trim_start();
        let cpu: f64 = cpu_token.parse().unwrap_or(0.0);
        let family = classify_command(rest);
        if family.is_empty() {
            continue;
        }
        for entry in sums.iter_mut() {
            if entry.0 == family {
                entry.1 += cpu;
                break;
            }
        }
    }
    sums
}

/// 对齐 SH 第 107-111 行 `opt_diag_family_total_for`。
pub fn opt_diag_family_total_for(totals: &[(&'static str, f64)], family: &str) -> f64 {
    totals
        .iter()
        .find(|(k, _)| *k == family)
        .map(|(_, v)| *v)
        .unwrap_or(0.0)
}

/// 对齐 SH 第 113-122 行 `opt_diag_family_label`。
pub fn opt_diag_family_label(family: &str) -> String {
    match family {
        "cloudshell" => "CloudShell / AliEntSafe",
        "syspolicyd" => "syspolicyd",
        "windowserver" => "WindowServer",
        "spotlight" => "Spotlight indexing",
        "coresim_disk_images" => "CoreSimulator disk images",
        other => return other.to_string(),
    }
    .to_string()
}

/// 对齐 SH 第 124-145 行 `opt_diag_family_note`。
pub fn opt_diag_family_note(family: &str) -> String {
    match family {
        "cloudshell" => "External enterprise agent pressure detected. Mole will not terminate enterprise security processes; restart or policy checks must happen outside Mole.",
        "syspolicyd" => "Gatekeeper and code-signature assessment activity is elevated.",
        "windowserver" => "Desktop composition is busy. When another family is higher, treat this as a likely symptom rather than the root cause.",
        "spotlight" => "Metadata indexing or import work is consuming CPU.",
        "coresim_disk_images" => "Simulator runtime disk-image services are active.",
        _ => "",
    }
    .to_string()
}

/// 对齐 SH 第 147-191 行 `opt_diag_parse_image_mount_pairs`。
///
/// 解析 `hdiutil info` 输出,把每个 `image-path:` 块下面的 `/...` 行提取成 mount path,
/// 与 image-path 配成 `(image, mount)` 对。`====` 行(等号 ≥1 个)作为块分隔符。
pub fn opt_diag_parse_image_mount_pairs(info: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut current_image: Option<String> = None;
    let mut current_mounts: Vec<String> = Vec::new();

    let flush =
        |image: &mut Option<String>, mounts: &mut Vec<String>, out: &mut Vec<(String, String)>| {
            if let Some(img) = image.take() {
                for m in mounts.drain(..) {
                    if !m.is_empty() {
                        out.push((img.clone(), m));
                    }
                }
            } else {
                mounts.clear();
            }
        };

    for raw in info.lines() {
        let line = raw;
        if !line.is_empty() && line.bytes().all(|b| b == b'=') {
            flush(&mut current_image, &mut current_mounts, &mut out);
            current_image = None;
            continue;
        }
        // image-path : <value>
        if let Some(rest) = strip_prefix_image_path(line) {
            current_image = Some(rest.to_string());
            continue;
        }
        // 行内含 mount(空白后跟 `/...`),对齐 SH `extract_mount`
        if let Some(mount) = extract_mount_from_line(line) {
            current_mounts.push(mount);
        }
    }
    flush(&mut current_image, &mut current_mounts, &mut out);
    out
}

fn strip_prefix_image_path(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let after = trimmed.strip_prefix("image-path")?;
    let after = after.trim_start();
    let after = after.strip_prefix(':')?;
    Some(after.trim())
}

fn extract_mount_from_line(line: &str) -> Option<String> {
    // SH extract_mount(#960): 仅 /dev/disk* 行才是真实挂载点;image-alias / icon-path /
    // shadow-path 等字段虽含绝对路径但不是挂载点,此前会误报幽灵 detach 建议。
    if !line.trim_start().starts_with("/dev/disk") {
        return None;
    }
    // SH awk: `if (line ~ /[[:space:]]\/.*/) { sub(/^.*[[:space:]]\//, "/", line); return line }`
    // 先确定存在"空白 + /"模式;若有,从最后一个"空白 + /"开始截取(贪婪 sub)。
    let bytes = line.as_bytes();
    let mut last_idx: Option<usize> = None;
    for i in 0..bytes.len().saturating_sub(1) {
        let b = bytes[i];
        if (b == b' ' || b == b'\t') && bytes[i + 1] == b'/' {
            last_idx = Some(i + 1);
        }
    }
    let idx = last_idx?;
    let mount = &line[idx..];
    if mount.starts_with('/') {
        Some(mount.to_string())
    } else {
        None
    }
}

/// 对齐 SH 第 193-210 行 `opt_diag_is_system_managed_mount`。
pub fn opt_diag_is_system_managed_mount(image_path: &str, mount_path: &str) -> bool {
    if image_path.starts_with("/System/")
        || image_path.starts_with("/Library/Apple/")
        || image_path.starts_with("/private/var/run/com.apple.security.cryptexd/")
    {
        return true;
    }
    if mount_path.starts_with("/Library/Developer/CoreSimulator/Volumes/")
        || mount_path.starts_with("/private/var/run/com.apple.security.cryptexd/")
    {
        return true;
    }
    false
}

fn has_image_extension(image_path: &str) -> bool {
    let lower = image_path.to_lowercase();
    [
        ".dmg",
        ".iso",
        ".img",
        ".cdr",
        ".sparseimage",
        ".sparsebundle",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

/// 对齐 SH 第 212-238 行 `opt_diag_is_mount_detach_candidate`。
pub fn opt_diag_is_mount_detach_candidate(image_path: &str, mount_path: &str) -> bool {
    if opt_diag_is_system_managed_mount(image_path, mount_path) {
        return false;
    }
    if !mount_path.starts_with("/Volumes/") {
        return false;
    }
    if !has_image_extension(image_path) {
        return false;
    }
    if should_protect_path(mount_path) || is_path_whitelisted_from_global(mount_path) {
        return false;
    }
    if !image_path.is_empty()
        && (should_protect_path(image_path) || is_path_whitelisted_from_global(image_path))
    {
        return false;
    }
    true
}

/// 对齐 SH 第 240-250 行 `opt_diag_collect_detach_candidates`。
pub fn opt_diag_collect_detach_candidates(pairs: &[(String, String)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .filter(|(img, mount)| {
            !img.is_empty() && !mount.is_empty() && opt_diag_is_mount_detach_candidate(img, mount)
        })
        .cloned()
        .collect()
}

/// 对齐 SH 第 252-274 行 `opt_diag_count_matches`。
/// `mode` ∈ {"system_managed", "coresim_only"}。
pub fn opt_diag_count_matches(pairs: &[(String, String)], mode: &str) -> u32 {
    let mut count = 0u32;
    for (img, mount) in pairs {
        if img.is_empty() || mount.is_empty() {
            continue;
        }
        match mode {
            "system_managed" => {
                if opt_diag_is_system_managed_mount(img, mount) {
                    count += 1;
                }
            }
            "coresim_only" => {
                if mount.starts_with("/Library/Developer/CoreSimulator/Volumes/") {
                    count += 1;
                }
            }
            _ => {}
        }
    }
    count
}

/// 对齐 SH 第 276-299 行 `opt_diag_detach_candidates`。
/// 实际调用 `hdiutil detach` 卸载。
pub fn opt_diag_detach_candidates(candidates: &[(String, String)]) {
    let mut detached = 0u32;
    let mut failed = 0u32;
    for (_image, mount) in candidates {
        if mount.is_empty() {
            continue;
        }
        let rc = crate::core::timeout::run_with_timeout(15.0, "hdiutil", &["detach", mount]);
        if rc == 0 {
            detached += 1;
            println!("  {GREEN}{ICON_SUCCESS}{NC} Detached {mount}");
        } else {
            failed += 1;
            println!("  {YELLOW}{ICON_WARNING}{NC} Failed to detach {mount}");
        }
    }
    if detached > 0 {
        println!("  {GRAY}{ICON_REVIEW}{NC} Detached {detached} mounted image(s)");
    }
    if failed > 0 {
        println!("  {GRAY}{ICON_REVIEW}{NC} {failed} mounted image(s) still need manual review");
    }
}

/// 对齐 SH 第 301-341 行 `opt_diag_offer_detach_candidates`。
///
/// SH 端通过 `read_key` 在 TTY 上确认是否 detach。GUI 端无 TTY,本函数:
/// 1. 列出候选(供前端展示);
/// 2. 当 `MOLE_DRY_RUN=1` 仅打印 "would detach" 提示;
/// 3. 否则:**不自动 detach,只打印复审提示**——实际 detach 由前端通过 Tauri command
///    显式调用 `opt_diag_detach_candidates(...)` 完成,避免在 GUI 后台静默卸载用户磁盘镜像。
pub fn opt_diag_offer_detach_candidates(candidates: &[(String, String)]) {
    if candidates.is_empty() {
        return;
    }

    let count = candidates.len();
    println!("  {GRAY}{ICON_LIST}{NC} Mounted image detach candidates:");
    for (image, mount) in candidates {
        println!("    {GRAY}{mount}{NC} ← {image}");
    }

    if std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1" {
        println!("  {YELLOW}→{NC} Would offer detach for {count} mounted image(s)");
        return;
    }

    println!(
        "  {GRAY}{ICON_REVIEW}{NC} Review these mounted images and detach any you no longer need"
    );
}

/// 对齐 SH 第 343-419 行 `run_optimize_diagnostics` 主入口。
///
/// 流程:
/// 1. 采两次 ps 样本(默认间隔 1s,可由 env 覆盖);
/// 2. 按家族聚合 CPU,识别"持续高 CPU"(两次都超过阈值);
/// 3. 对 syspolicyd 高 CPU 场景额外提示 Gatekeeper 状态 / 已挂载磁盘镜像;
/// 4. 提供 detach candidates(GUI 端只列出,不自动 detach,见 `offer_detach_candidates`)。
pub fn run_optimize_diagnostics() {
    let sample1 = opt_diag_get_ps_sample(1);
    let delay = opt_diag_sample_delay();
    let env1 = std::env::var("MOLE_OPTIMIZE_PS_SAMPLE_1").is_ok();
    let env2 = std::env::var("MOLE_OPTIMIZE_PS_SAMPLE_2").is_ok();
    if !env1 || !env2 {
        let dur = std::time::Duration::from_secs_f64(delay.max(0.0));
        std::thread::sleep(dur);
    }
    let sample2 = opt_diag_get_ps_sample(2);
    let totals1 = opt_diag_family_totals(&sample1);
    let totals2 = opt_diag_family_totals(&sample2);
    let threshold = opt_diag_cpu_threshold();

    println!();
    println!("{BLUE}PERFORMANCE DIAGNOSIS{NC}");

    let families = [
        "cloudshell",
        "syspolicyd",
        "windowserver",
        "spotlight",
        "coresim_disk_images",
    ];
    let mut sustained: Vec<(&'static str, f64, String)> = Vec::new();
    let mut primary_family: &'static str = "";
    let mut primary_avg: f64 = 0.0;

    for family in families.iter() {
        let cpu1 = opt_diag_family_total_for(&totals1, family);
        let cpu2 = opt_diag_family_total_for(&totals2, family);
        if cpu1 >= threshold && cpu2 >= threshold {
            let avg = (cpu1 + cpu2) / 2.0;
            let label = opt_diag_family_label(family);
            sustained.push((family, avg, label.clone()));
            if primary_family.is_empty() || avg > primary_avg {
                primary_family = family;
                primary_avg = avg;
            }
        }
    }

    if primary_family.is_empty() {
        println!("  {GREEN}{ICON_SUCCESS}{NC} No obvious sustained high-CPU bottleneck detected");
    } else {
        let label = opt_diag_family_label(primary_family);
        println!(
            "  {YELLOW}{ICON_WARNING}{NC} Likely bottleneck: {label} (~{:.1}% CPU sustained)",
            primary_avg
        );
        let note = opt_diag_family_note(primary_family);
        if !note.is_empty() {
            println!("  {GRAY}{ICON_REVIEW}{NC} {note}");
        }
        if sustained.len() > 1 {
            println!("  {GRAY}{ICON_LIST}{NC} Additional sustained pressure:");
            for (fam, avg, lbl) in sustained.iter() {
                if *fam == primary_family {
                    continue;
                }
                println!("    {GRAY}{lbl}{NC} ~{:.1}%", avg);
            }
        }
    }

    let spctl_status = opt_diag_get_spctl_status();
    let hdiutil_info = opt_diag_get_hdiutil_info();
    let image_pairs = opt_diag_parse_image_mount_pairs(&hdiutil_info);
    let detach_candidates = opt_diag_collect_detach_candidates(&image_pairs);

    let syspolicyd_in_sustained = sustained.iter().any(|(f, _, _)| *f == "syspolicyd");
    if primary_family == "syspolicyd" || syspolicyd_in_sustained {
        let managed_count = opt_diag_count_matches(&image_pairs, "system_managed");
        let coresim_count = opt_diag_count_matches(&image_pairs, "coresim_only");
        let detach_count = detach_candidates.len() as u32;

        if !spctl_status.is_empty() {
            println!("  {GRAY}{ICON_LIST}{NC} Gatekeeper status: {spctl_status}");
        }
        if managed_count > 0 && managed_count == coresim_count && detach_count == 0 {
            println!(
                "  {GRAY}{ICON_INFO}{NC} Only system-managed CoreSimulator images are mounted, informational only, not a detach target"
            );
        } else if detach_count > 0 {
            println!(
                "  {GRAY}{ICON_INFO}{NC} User-mounted disk images may contribute to assessment overhead"
            );
        }
    }

    opt_diag_offer_detach_candidates(&detach_candidates);

    // 对齐 SH L398-425：内存压力 / 空闲虚拟机 / 失控进程检测
    let mem_out = opt_diag_memory_pressure();
    let vm_out = opt_diag_idle_vm();
    let runaway_out = opt_diag_runaway_process();

    if let Some(ref mem) = mem_out {
        println!(
            "  {YELLOW}{ICON_WARNING}{NC} Memory pressure: swap {}MB of {}MB used ({}%), {}% free",
            mem.swap_used_mb, mem.swap_total_mb, mem.swap_pct, mem.free_pct
        );
        for h in &mem.top_holders {
            println!("    {GRAY}{}{}{NC}", h.name, h.rss_kb);
        }
        if mem.top_holders.is_empty() {
            println!("    {GRAY}pressure is spread across many small processes{NC}");
        }
    }

    if let Some(ref vm) = vm_out {
        let vm_gb = vm.vm_kb as f64 / 1_048_576.0;
        match vm.docker_running {
            Some(0) => {
                println!(
                    "  {YELLOW}{ICON_WARNING}{NC} Virtual machine holding {vm_gb:.1}GB with no running containers (likely Docker Desktop)"
                );
                println!(
                    "  {GRAY}{ICON_REVIEW}{NC} If this is Docker Desktop, quitting it reclaims all of it{NC}"
                );
            }
            Some(n) => {
                println!(
                    "  {GRAY}{ICON_LIST}{NC} Virtual machine using {vm_gb:.1}GB ({n} containers running){NC}"
                );
            }
            None => {
                println!("  {YELLOW}{ICON_WARNING}{NC} Virtual machine holding {vm_gb:.1}GB{NC}");
                println!(
                    "  {GRAY}{ICON_REVIEW}{NC} Check Docker Desktop, UTM, or other virtualization tools{NC}"
                );
            }
        }
    }

    for r in &runaway_out {
        println!(
            "  {YELLOW}{ICON_WARNING}{NC} {} has burned {}h CPU over {}h of runtime (~{}% sustained)",
            r.name, r.cpu_hours, r.run_hours, r.pct
        );
        println!(
            "  {GRAY}{ICON_REVIEW}{NC} Sustained for its whole lifetime; if not doing real work: kill -TERM {}{NC}",
            r.pid
        );
    }
}

// ============================================================================
// 内存压力 / 空闲虚拟机 / 失控进程 — 数据采集与分析
// ============================================================================

fn diag_int_env(key: &str, fallback: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(fallback)
}

/// 对齐 SH L472-478 `opt_diag_get_swapusage`。
fn opt_diag_get_swapusage() -> String {
    if let Ok(v) = std::env::var("MOLE_OPTIMIZE_SWAPUSAGE") {
        return v;
    }
    Command::new("sysctl")
        .args(["-n", "vm.swapusage"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// 对齐 SH L480-486 `opt_diag_get_mem_free_pct`。
fn opt_diag_get_mem_free_pct() -> u32 {
    if let Ok(v) = std::env::var("MOLE_OPTIMIZE_MEM_FREE_SAMPLE") {
        return v.trim().parse().unwrap_or(100);
    }
    Command::new("memory_pressure")
        .output()
        .ok()
        .and_then(|o| {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            for line in text.lines() {
                if line.contains("free percentage") {
                    if let Some(pos) = line.find(':') {
                        let val = line[pos + 1..].replace('%', "");
                        return val.trim().parse().ok();
                    }
                }
            }
            None
        })
        .unwrap_or(100)
}

/// 对齐 SH L489-495 `opt_diag_get_rss_sample`。
fn opt_diag_get_rss_sample() -> String {
    if let Ok(v) = std::env::var("MOLE_OPTIMIZE_RSS_SAMPLE") {
        return v;
    }
    Command::new("ps")
        .args(["-Ao", "rss,comm"])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .skip(1)
                .filter(|l| !l.contains("CoreSimulator"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// 对齐 SH L498-504 `opt_diag_get_proctime_sample`。
fn opt_diag_get_proctime_sample() -> String {
    if let Ok(v) = std::env::var("MOLE_OPTIMIZE_PROCTIME_SAMPLE") {
        return v;
    }
    Command::new("ps")
        .args(["-Ao", "pid,time,etime,comm"])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .skip(1)
                .filter(|l| !l.contains("CoreSimulator"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// 对齐 SH L507-513 `opt_diag_get_vm_sample`。
fn opt_diag_get_vm_sample() -> String {
    if let Ok(v) = std::env::var("MOLE_OPTIMIZE_VM_SAMPLE") {
        return v;
    }
    Command::new("ps")
        .args(["-Ao", "rss,command"])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .skip(1)
                .filter(|l| !l.contains("CoreSimulator"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// 对齐 SH L515-519 `opt_diag_int_env`（u64 版本）。
fn diag_int_env_u64(key: &str, fallback: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(fallback)
}

/// 对齐 SH L542-611 `opt_diag_memory_pressure`。
/// swap 占比 >= SWAP_PCT_DEFAULT 或空闲内存 < FREE_PCT_DEFAULT 时返回 Some。
pub fn opt_diag_memory_pressure() -> Option<MemoryPressure> {
    let warn_swap = diag_int_env("MOLE_OPTIMIZE_SWAP_PCT", SWAP_PCT_DEFAULT);
    let warn_free = diag_int_env("MOLE_OPTIMIZE_FREE_PCT", FREE_PCT_DEFAULT);

    let swap_line = opt_diag_get_swapusage();
    if swap_line.is_empty() {
        return None;
    }

    let (swap_total, swap_used) = parse_swap_mb(&swap_line);
    if swap_total == 0 && swap_used == 0 {
        return None;
    }

    let swap_pct = if swap_total > 0 {
        (swap_used * 100 / swap_total) as u32
    } else {
        0
    };
    let free_pct = opt_diag_get_mem_free_pct();

    if swap_pct < warn_swap && free_pct >= warn_free {
        return None;
    }

    // Top 4 内存消耗进程（> 256MB = 262144 KB）
    let rss_raw = opt_diag_get_rss_sample();
    let mut holders: Vec<(u64, String)> = rss_raw
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let mut parts = trimmed.splitn(2, |c: char| c.is_whitespace());
            let kb: u64 = parts.next()?.trim().parse().ok()?;
            let name = parts.next().unwrap_or("").trim();
            if kb <= 262_144 || name.is_empty() {
                return None;
            }
            // 取最后一段路径（sub(/^.*\//, "", name)）
            let short = name.rsplit('/').next().unwrap_or(name);
            Some((kb, short.to_string()))
        })
        .collect();
    holders.sort_by(|a, b| b.0.cmp(&a.0));
    let top_holders = holders
        .into_iter()
        .take(4)
        .map(|(kb, name)| ProcessHolder { name, rss_kb: kb })
        .collect();

    Some(MemoryPressure {
        swap_total_mb: swap_total,
        swap_used_mb: swap_used,
        swap_pct,
        free_pct,
        top_holders,
    })
}

/// 解析 `sysctl vm.swapusage` 输出，提取 total 和 used 的 MB 值。
fn parse_swap_mb(line: &str) -> (u64, u64) {
    // 典型输出: "total = 2048.00M  used = 512.00M  free = 1536.00M"
    let extract = |needle: &str| -> u64 {
        if let Some(pos) = line.find(needle) {
            let rest = &line[pos + needle.len()..];
            let num_str: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            num_str.parse::<f64>().unwrap_or(0.0) as u64
        } else {
            0
        }
    };
    (extract("total = "), extract("used = "))
}

/// 对齐 SH L616-651 `opt_diag_idle_vm`。
/// Virtualization.framework 进程占用 >= IDLE_VM_GB_DEFAULT GB 时返回 Some。
pub fn opt_diag_idle_vm() -> Option<IdleVm> {
    let min_gb = diag_int_env_u64("MOLE_OPTIMIZE_IDLE_VM_GB", IDLE_VM_GB_DEFAULT);

    let vm_raw = opt_diag_get_vm_sample();
    let vm_kb: u64 = vm_raw
        .lines()
        .filter_map(|line| {
            if !line.contains("Virtualization.framework") || !line.contains("VirtualMachine") {
                return None;
            }
            let trimmed = line.trim();
            let mut parts = trimmed.splitn(2, |c: char| c.is_whitespace());
            let kb: u64 = parts.next()?.trim().parse().ok()?;
            Some(kb)
        })
        .sum();

    if vm_kb <= min_gb * 1_048_576 {
        return None;
    }

    // 检查 docker 容器数量
    let docker_running = if command_available("docker") {
        match Command::new("docker").args(["ps", "-q"]).output() {
            Ok(out) if out.status.success() => {
                let count = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .count() as u32;
                Some(count)
            }
            _ => None,
        }
    } else {
        None
    };

    Some(IdleVm {
        vm_kb,
        docker_running,
    })
}

fn command_available(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 对齐 SH L526-538 `opt_secs`：将 ps 的 time/etime 格式（[[dd-]hh:]mm:ss）转为秒数。
fn parse_ps_time(s: &str) -> u64 {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return 0;
    }
    let first = parts[0];
    let (days, hours) = if let Some(dash_pos) = first.find('-') {
        let d: u64 = first[..dash_pos].parse().unwrap_or(0);
        let h: u64 = first[dash_pos + 1..].parse().unwrap_or(0);
        (d, h)
    } else if parts.len() == 3 {
        let h: u64 = first.parse().unwrap_or(0);
        (0, h)
    } else {
        (0, 0)
    };
    let minutes: u64 = if parts.len() == 3 {
        parts[1].parse().unwrap_or(0)
    } else {
        first.parse().unwrap_or(0)
    };
    let seconds_str = if parts.len() == 3 { parts[2] } else { parts[1] };
    let seconds: u64 = seconds_str.parse::<f64>().unwrap_or(0.0) as u64;

    // 对于 2 段格式 (mm:ss) 且无 dash，first 就是 minutes
    if parts.len() == 2 && first.find('-').is_none() {
        let m: u64 = first.parse().unwrap_or(0);
        return m * 60 + seconds;
    }

    days * 86400 + hours * 3600 + minutes * 60 + seconds
}

/// 对齐 SH L656-694 `opt_diag_runaway_process`。
/// 检测生命周期内持续高 CPU（>= 25%，>= 12h）的进程。
/// 排除 kernel_task/WindowServer/mds/mds_stores/syspolicyd/mdworker（已被家族检测覆盖）。
pub fn opt_diag_runaway_process() -> Vec<RunawayProcess> {
    let pct_floor = diag_int_env("MOLE_OPTIMIZE_RUNAWAY_PCT", RUNAWAY_PCT_DEFAULT) as u64;
    let min_hours = diag_int_env_u64("MOLE_OPTIMIZE_RUNAWAY_MIN_HOURS", RUNAWAY_MIN_HOURS_DEFAULT);

    let sample = opt_diag_get_proctime_sample();
    let mut result = Vec::new();

    for line in sample.lines().take(400) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // 格式: PID TIME ELAPSED COMMAND...
        let mut fields = trimmed.splitn(4, |c: char| c.is_whitespace());
        let pid: u32 = match fields.next().and_then(|s| s.trim().parse().ok()) {
            Some(p) => p,
            None => continue,
        };
        let time_str = match fields.next() {
            Some(s) => s.trim(),
            None => continue,
        };
        let etime_str = match fields.next() {
            Some(s) => s.trim(),
            None => continue,
        };
        let comm = match fields.next() {
            Some(s) => s.trim(),
            None => continue,
        };

        // 取短名
        let name = comm.rsplit('/').next().unwrap_or(comm);

        // 排除系统进程（已被家族 CPU 检测覆盖）
        if matches!(
            name,
            "kernel_task" | "WindowServer" | "mds" | "mds_stores" | "syspolicyd"
        ) || name.starts_with("mdworker")
        {
            continue;
        }

        let elapsed_secs = parse_ps_time(etime_str);
        if elapsed_secs <= min_hours * 3600 {
            continue;
        }

        let cpu_secs = parse_ps_time(time_str);
        if cpu_secs == 0 {
            continue;
        }

        let pct = cpu_secs * 100 / elapsed_secs;
        if pct < pct_floor {
            continue;
        }

        result.push(RunawayProcess {
            pid,
            name: name.to_string(),
            cpu_hours: (cpu_secs / 3600) as u32,
            run_hours: (elapsed_secs / 3600) as u32,
            pct: pct as u32,
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_basic() {
        assert_eq!(classify_command("/usr/sbin/syspolicyd -X"), "syspolicyd");
        assert_eq!(
            classify_command("/System/Library/.../WindowServer -daemon"),
            "windowserver"
        );
        assert_eq!(classify_command("mdworker_shared"), "spotlight");
        assert_eq!(
            classify_command("/usr/libexec/diskimagesiod"),
            "coresim_disk_images"
        );
        assert_eq!(classify_command("AliEntSafe-helper"), "cloudshell");
        assert_eq!(classify_command("Finder"), "");
    }

    #[test]
    fn matches_word_boundary() {
        // syspolicyd 在路径分隔/行尾边界匹配
        assert!(matches_word("/usr/sbin/syspolicyd", "syspolicyd"));
        assert!(matches_word("/usr/sbin/syspolicyd ", "syspolicyd"));
        // 不应匹配 "mysyspolicyd-foo" 这种子串
        assert!(!matches_word("/usr/sbin/mysyspolicyd-foo", "syspolicyd"));
    }

    #[test]
    fn family_totals_aggregates() {
        let raw = " 12.5 /usr/sbin/syspolicyd\n 7.0 /usr/sbin/syspolicyd\n 3.0 Finder\n";
        let totals = opt_diag_family_totals(raw);
        let v = opt_diag_family_total_for(&totals, "syspolicyd");
        assert!((v - 19.5).abs() < 1e-6);
        assert_eq!(opt_diag_family_total_for(&totals, "spotlight"), 0.0);
    }

    #[test]
    fn parse_image_mount_pairs_basic() {
        // hdiutil info 风格的迷你输出:`====` 分隔块,块内 image-path 加 mount 行
        let info = "framework      : 565.140.1\n\
                    driver         : 565.140.1\n\
                    ================================================\n\
                    image-path        : /Volumes/SomeDmg.dmg\n\
                    image-alias       : /Users/x/Downloads/SomeDmg.dmg\n\
                    /dev/disk5        Apple_HFS                  /Volumes/SomeDmg\n\
                    ================================================\n\
                    image-path        : /System/Library/Foo.dmg\n\
                    /dev/disk6        Apple_APFS                 /Library/Developer/CoreSimulator/Volumes/iOS_X\n";
        let pairs = opt_diag_parse_image_mount_pairs(info);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].0, "/Volumes/SomeDmg.dmg");
        assert_eq!(pairs[0].1, "/Volumes/SomeDmg");
        assert_eq!(pairs[1].0, "/System/Library/Foo.dmg");
        assert_eq!(pairs[1].1, "/Library/Developer/CoreSimulator/Volumes/iOS_X");
    }

    #[test]
    fn detach_candidate_filters() {
        // 系统管理的不算候选
        assert!(!opt_diag_is_mount_detach_candidate(
            "/System/Library/Foo.dmg",
            "/Volumes/Foo"
        ));
        // 非 /Volumes 不算
        assert!(!opt_diag_is_mount_detach_candidate(
            "/Users/me/x.dmg",
            "/Library/Developer/CoreSimulator/Volumes/iOS"
        ));
        // 非 image 后缀不算
        assert!(!opt_diag_is_mount_detach_candidate(
            "/Users/me/somefile.txt",
            "/Volumes/Foo"
        ));
        // 用户挂载的 .dmg 算
        assert!(opt_diag_is_mount_detach_candidate(
            "/Users/me/x.dmg",
            "/Volumes/X"
        ));
    }

    // ====================================================================
    // 新增诊断函数测试
    // ====================================================================

    #[test]
    fn parse_swap_mb_basic() {
        let line = "total = 2048.00M  used = 512.00M  free = 1536.00M";
        let (total, used) = parse_swap_mb(line);
        assert_eq!(total, 2048);
        assert_eq!(used, 512);
    }

    #[test]
    fn parse_swap_mb_empty() {
        let (t, u) = parse_swap_mb("");
        assert_eq!(t, 0);
        assert_eq!(u, 0);
    }

    #[test]
    fn parse_ps_time_seconds() {
        assert_eq!(parse_ps_time("00:05"), 5);
        assert_eq!(parse_ps_time("01:30"), 90);
    }

    #[test]
    fn parse_ps_time_hours() {
        assert_eq!(parse_ps_time("01:00:00"), 3600);
        assert_eq!(parse_ps_time("02:30:15"), 9015);
    }

    #[test]
    fn parse_ps_time_days() {
        assert_eq!(parse_ps_time("1-00:00:00"), 86400);
        assert_eq!(parse_ps_time("2-12:30:00"), 2 * 86400 + 12 * 3600 + 30 * 60);
    }

    #[test]
    fn parse_ps_time_garbage_returns_zero() {
        assert_eq!(parse_ps_time("garbage"), 0);
        assert_eq!(parse_ps_time(""), 0);
    }

    #[test]
    fn memory_pressure_env_injection() {
        // swap 低于阈值 + 空闲内存充足 → None
        std::env::set_var(
            "MOLE_OPTIMIZE_SWAPUSAGE",
            "total = 100.00M  used = 10.00M  free = 90.00M",
        );
        std::env::set_var("MOLE_OPTIMIZE_MEM_FREE_SAMPLE", "80");
        let result = opt_diag_memory_pressure();
        std::env::remove_var("MOLE_OPTIMIZE_SWAPUSAGE");
        std::env::remove_var("MOLE_OPTIMIZE_MEM_FREE_SAMPLE");
        assert!(result.is_none());
    }

    #[test]
    fn memory_pressure_high_swap() {
        // swap >= 50% → Some
        std::env::set_var(
            "MOLE_OPTIMIZE_SWAPUSAGE",
            "total = 100.00M  used = 60.00M  free = 40.00M",
        );
        std::env::set_var("MOLE_OPTIMIZE_MEM_FREE_SAMPLE", "80");
        std::env::set_var("MOLE_OPTIMIZE_RSS_SAMPLE", "");
        let result = opt_diag_memory_pressure();
        std::env::remove_var("MOLE_OPTIMIZE_SWAPUSAGE");
        std::env::remove_var("MOLE_OPTIMIZE_MEM_FREE_SAMPLE");
        std::env::remove_var("MOLE_OPTIMIZE_RSS_SAMPLE");
        assert!(result.is_some());
        let mem = result.unwrap();
        assert_eq!(mem.swap_pct, 60);
    }

    #[test]
    fn idle_vm_below_threshold() {
        // 无 Virtualization.framework 进程 → None
        std::env::set_var("MOLE_OPTIMIZE_VM_SAMPLE", "100 /usr/sbin/syspolicyd");
        let result = opt_diag_idle_vm();
        std::env::remove_var("MOLE_OPTIMIZE_VM_SAMPLE");
        assert!(result.is_none());
    }

    #[test]
    fn runaway_filters_kernel_task() {
        // kernel_task 被排除
        std::env::set_var(
            "MOLE_OPTIMIZE_PROCTIME_SAMPLE",
            "  1 100:00:00 100:00:00 kernel_task",
        );
        let result = opt_diag_runaway_process();
        std::env::remove_var("MOLE_OPTIMIZE_PROCTIME_SAMPLE");
        assert!(result.is_empty());
    }

    #[test]
    fn runaway_detects_stuck_process() {
        // 模拟一个运行 24h、CPU 占 50% 的进程
        std::env::set_var(
            "MOLE_OPTIMIZE_PROCTIME_SAMPLE",
            "12345 12:00:00 24:00:00 stuck_worker",
        );
        let result = opt_diag_runaway_process();
        std::env::remove_var("MOLE_OPTIMIZE_PROCTIME_SAMPLE");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].pid, 12345);
        assert_eq!(result[0].name, "stuck_worker");
        assert_eq!(result[0].pct, 50);
    }
}
