//! 对齐 `lib/optimize/diagnostics.sh`。
//!
//! GUI 端不需要 `read_key` TUI 交互,因此 `offer_detach_candidates` 不会真的提示按键,
//! 实际确认/调用 `hdiutil detach` 由前端弹窗 + Tauri command 触发(参考 SH 第 301-341 行)。
//! 其它纯数据/计算函数严格按 SH 翻译。

use std::process::Command;

use crate::core::app_protection::{is_path_whitelisted_from_global, should_protect_path};
use crate::core::base::{BLUE, GRAY, GREEN, NC, YELLOW};
use crate::core::base::{ICON_INFO, ICON_LIST, ICON_REVIEW, ICON_SUCCESS, ICON_WARNING};
use crate::core::timeout::run_with_timeout_capture;

/// 对齐 SH 第 6 行常量。
pub const MOLE_OPTIMIZE_DIAG_CPU_THRESHOLD_DEFAULT: f64 = 25.0;
/// 对齐 SH 第 7 行常量。
pub const MOLE_OPTIMIZE_DIAG_SAMPLE_DELAY_DEFAULT: f64 = 1.0;

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
}
