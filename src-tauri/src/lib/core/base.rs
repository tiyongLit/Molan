use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use std::{io::Write, mem};

use tauri::AppHandle;

use crate::events::{SpinnerUpdatePayload, emit_cleanup_spinner_update};

pub const ESC: &str = "\u{001b}";
pub const GREEN: &str = "\u{001b}[0;32m";
pub const BLUE: &str = "\u{001b}[1;34m";
pub const CYAN: &str = "\u{001b}[0;36m";
pub const YELLOW: &str = "\u{001b}[0;33m";
pub const PURPLE: &str = "\u{001b}[0;35m";
pub const PURPLE_BOLD: &str = "\u{001b}[1;35m";
pub const RED: &str = "\u{001b}[0;31m";
pub const GRAY: &str = "\u{001b}[0;90m";
pub const NC: &str = "\u{001b}[0m";

pub const ICON_CONFIRM: &str = "◎";
pub const ICON_ADMIN: &str = "⚙";
pub const ICON_SUCCESS: &str = "✓";
pub const ICON_ERROR: &str = "☻";
pub const ICON_WARNING: &str = "◎";
pub const ICON_EMPTY: &str = "○";
pub const ICON_SOLID: &str = "●";
pub const ICON_LIST: &str = "•";
pub const ICON_SUBLIST: &str = "↳";
pub const ICON_ARROW: &str = "➤";
pub const ICON_DRY_RUN: &str = "→";
pub const ICON_REVIEW: &str = "☞";
pub const ICON_NAV_UP: &str = "↑";
pub const ICON_NAV_DOWN: &str = "↓";
pub const ICON_INFO: &str = "ℹ";

pub const MOLE_TEMP_FILE_AGE_DAYS: u32 = 7;
pub const MOLE_ORPHAN_AGE_DAYS: u32 = 30;
pub const MOLE_MAX_PARALLEL_JOBS: u32 = 15;
pub const MOLE_MAIL_DOWNLOADS_MIN_KB: u64 = 5120;
pub const MOLE_MAIL_AGE_DAYS: u32 = 30;
pub const MOLE_LOG_AGE_DAYS: u32 = 7;
pub const MOLE_CRASH_REPORT_AGE_DAYS: u32 = 7;
pub const MOLE_SAVED_STATE_AGE_DAYS: u32 = 30;
pub const MOLE_TM_BACKUP_SAFE_HOURS: u32 = 48;
pub const MOLE_MAX_DS_STORE_FILES: u32 = 500;
pub const MOLE_MAX_ORPHAN_ITERATIONS: u32 = 100;
pub const MOLE_ONE_GIB_KB: u64 = 1024 * 1024;
pub const MOLE_ONE_GB_BYTES: u64 = 1_000_000_000;
/// 对齐 `system.sh` `gpu_cache_dir_is_stale` 默认保留天数。
pub const MOLE_GPU_CACHE_AGE_DAYS: u32 = 1;
pub const FINDER_METADATA_SENTINEL: &str = "FINDER_METADATA";
pub const STAT_BSD: &str = "/usr/bin/stat";

static ARCH_CACHE: OnceLock<String> = OnceLock::new();
static DARWIN_MAJOR_CACHE: OnceLock<u32> = OnceLock::new();
static CPU_CORES_CACHE: OnceLock<u32> = OnceLock::new();
static INVOKING_USER_CACHE: OnceLock<String> = OnceLock::new();
static RESOLVED_TMPDIR: OnceLock<String> = OnceLock::new();
static ANSI_SUPPORTED_CACHE: OnceLock<bool> = OnceLock::new();
static TEMP_FILES: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
static TEMP_DIRS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
static TRACK_SECTION: OnceLock<Mutex<i32>> = OnceLock::new();
static SECTION_ACTIVITY: OnceLock<Mutex<i32>> = OnceLock::new();
static CURRENT_SECTION: OnceLock<Mutex<String>> = OnceLock::new();
/// 时序埋点（卡顿分析）：当前 section 起点（`start_section` 写入、`end_section` 取走并打印耗时）。
static SECTION_STARTED_AT: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();
static EXPORT_LIST_FILE: OnceLock<Mutex<String>> = OnceLock::new();
/// GUI 推送目标：由 controller 在 `mole_clean` 等任务开始时注入，结束时清空。
static SPINNER_APP: OnceLock<Mutex<Option<AppHandle>>> = OnceLock::new();
/// Analyze 推送目标：由 controller 在 `mole_analyze` 开始时注入，结束时清空。
static ANALYZE_APP: OnceLock<Mutex<Option<AppHandle>>> = OnceLock::new();
/// 当前小节 spinner（与 shell 一致：先 stop 再 start 会结束上一段并推送 duration）。
static SPINNER_PHASE: OnceLock<Mutex<Option<SpinnerPhase>>> = OnceLock::new();
static IS_CHINESE_SYSTEM: OnceLock<bool> = OnceLock::new();

struct SpinnerPhase {
    section: String,
    started: Instant,
}

/// 在阻塞清理任务内 `enter`，Drop 时先 [`stop_section_spinner`] 再断开 [`set_spinner_app_handle`]，
/// 避免前端残留 loading；与 shell 入口先 `stop_section_spinner` 语义一致。
pub struct SpinnerAppGuard;

impl SpinnerAppGuard {
    pub fn enter(app: AppHandle) -> Self {
        set_spinner_app_handle(Some(app));
        Self
    }
}

impl Drop for SpinnerAppGuard {
    fn drop(&mut self) {
        stop_section_spinner();
        set_spinner_app_handle(None);
    }
}

pub fn set_spinner_app_handle(app: Option<AppHandle>) {
    if let Ok(mut g) = SPINNER_APP.get_or_init(|| Mutex::new(None)).lock() {
        *g = app;
    }
}

pub fn current_spinner_app_handle() -> Option<AppHandle> {
    SPINNER_APP
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|g| g.clone())
}

pub fn set_analyze_app_handle(app: Option<AppHandle>) {
    if let Ok(mut g) = ANALYZE_APP.get_or_init(|| Mutex::new(None)).lock() {
        *g = app;
    }
}

pub fn current_analyze_app_handle() -> Option<AppHandle> {
    ANALYZE_APP
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|g| g.clone())
}

fn spinner_try_emit(payload: SpinnerUpdatePayload) {
    let app = SPINNER_APP
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|g| g.clone());
    if let Some(app) = app {
        emit_cleanup_spinner_update(&app, &payload);
    }
}

pub fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_default()
}

pub fn run_cmd(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn run_cmd_with_stderr(bin: &str, args: &[&str]) -> Option<(String, String)> {
    let out = Command::new(bin).args(args).output().ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Some((stdout, stderr))
}

pub fn pgrep_x(name: &str) -> bool {
    Command::new("pgrep")
        .args(["-x", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn command_available(bin_name: &str) -> bool {
    Command::new("which")
        .arg(bin_name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn get_lsregister_path() -> String {
    let candidates = [
        "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister",
        "/System/Library/CoreServices/Frameworks/LaunchServices.framework/Support/lsregister",
    ];
    for candidate in candidates {
        if Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }
    String::new()
}

/// 取文件 size。先按 lstat,失败再 follow 软链接(对齐 SH 第 71-73 行 stat / stat -L)。
pub fn get_file_size(file: &str) -> u64 {
    if let Some(v) = run_cmd(STAT_BSD, &["-f%z", file]) {
        if let Ok(n) = v.trim().parse::<u64>() {
            return n;
        }
    }
    run_cmd(STAT_BSD, &["-Lf%z", file])
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

/// 取文件 mtime;同样有 -L fallback
pub fn get_file_mtime(file: &str) -> u64 {
    if file.is_empty() {
        return 0;
    }
    if let Some(v) = run_cmd(STAT_BSD, &["-f%m", file]) {
        if let Ok(n) = v.trim().parse::<u64>() {
            return n;
        }
    }
    run_cmd(STAT_BSD, &["-Lf%m", file])
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

pub fn get_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn get_file_owner(file: &str) -> String {
    run_cmd(STAT_BSD, &["-f%Su", file]).unwrap_or_default()
}

pub fn is_sip_enabled() -> bool {
    if run_cmd("which", &["csrutil"]).is_none() {
        return true;
    }
    run_cmd("csrutil", &["status"])
        .map(|s| s.to_lowercase().contains("enabled"))
        .unwrap_or(true)
}

pub fn detect_architecture() -> String {
    ARCH_CACHE
        .get_or_init(|| {
            let arch = run_cmd("uname", &["-m"]).unwrap_or_default();
            if arch.trim() == "arm64" {
                "Apple Silicon".to_string()
            } else {
                "Intel".to_string()
            }
        })
        .clone()
}

pub fn get_free_space() -> String {
    let target = if Path::new("/System/Volumes/Data").is_dir() {
        "/System/Volumes/Data"
    } else {
        "/"
    };
    let out = run_cmd("df", &["-h", target]).unwrap_or_default();
    out.lines()
        .nth(1)
        .and_then(|line| line.split_whitespace().nth(3))
        .unwrap_or("")
        .to_string()
}

pub fn get_darwin_major() -> u32 {
    *DARWIN_MAJOR_CACHE.get_or_init(|| {
        let kernel = run_cmd("uname", &["-r"]).unwrap_or_default();
        kernel
            .split('.')
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(999)
    })
}

pub fn is_darwin_ge(minimum: u32) -> bool {
    get_darwin_major() >= minimum
}

pub fn get_optimal_parallel_jobs(operation_type: &str) -> u32 {
    let cores = *CPU_CORES_CACHE.get_or_init(|| {
        run_cmd("sysctl", &["-n", "hw.ncpu"])
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(4)
    });
    match operation_type {
        "scan" | "io" => cores.saturating_mul(2),
        "compute" => cores,
        _ => cores.saturating_add(2),
    }
}

pub fn is_root_user() -> bool {
    run_cmd("id", &["-u"])
        .map(|s| s.trim() == "0")
        .unwrap_or(false)
}

pub fn get_invoking_user() -> String {
    INVOKING_USER_CACHE
        .get_or_init(|| {
            let sudo_user = std::env::var("SUDO_USER").unwrap_or_default();
            if !sudo_user.is_empty() && sudo_user != "root" {
                sudo_user
            } else {
                std::env::var("USER").unwrap_or_default()
            }
        })
        .clone()
}

pub fn get_invoking_uid() -> String {
    if let Ok(v) = std::env::var("SUDO_UID") {
        return v;
    }
    run_cmd("id", &["-u"]).unwrap_or_default()
}

pub fn get_invoking_gid() -> String {
    if let Ok(v) = std::env::var("SUDO_GID") {
        return v;
    }
    run_cmd("id", &["-g"]).unwrap_or_default()
}

pub fn get_user_home(user: &str) -> String {
    if user.is_empty() {
        return String::new();
    }
    let out = run_cmd(
        "dscl",
        &[".", "-read", &format!("/Users/{user}"), "NFSHomeDirectory"],
    );
    if let Some(v) = out {
        let mut it = v.split_whitespace();
        let _ = it.next();
        if let Some(home) = it.next() {
            if !home.starts_with('~') {
                return home.to_string();
            }
        }
    }
    if let Some(v) = run_cmd("id", &["-P", user]) {
        let fields: Vec<&str> = v.split(':').collect();
        if fields.len() >= 9 {
            let home = fields[8].trim();
            if !home.starts_with('~') {
                return home.to_string();
            }
        }
    }
    String::new()
}

pub fn get_invoking_home() -> String {
    let sudo_user = std::env::var("SUDO_USER").unwrap_or_default();
    if !sudo_user.is_empty() && sudo_user != "root" {
        return get_user_home(&sudo_user);
    }
    std::env::var("HOME").unwrap_or_default()
}

/// 确保目录存在,且当前进程是 root 时把所有权交还给 invoking user。
/// 对齐 base.sh:ensure_user_dir() — 重要,否则 sudo mole … 创建的目录后续以普通用户运行时会读不进去。
pub fn ensure_user_dir(path: &str) {
    if path.is_empty() {
        return;
    }
    let target = if path.starts_with('~') {
        path.replacen('~', &std::env::var("HOME").unwrap_or_default(), 1)
    } else {
        path.to_string()
    };
    let _ = std::fs::create_dir_all(&target);
    chown_to_invoking_user(&target);
}

/// 确保文件存在,父目录就绪,root 模式下回填所有权
pub fn ensure_user_file(path: &str) {
    if let Some(parent) = Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
        if let Some(parent_str) = parent.to_str() {
            chown_to_invoking_user(parent_str);
        }
    }
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path);
    chown_to_invoking_user(path);
}

/// 当本进程实际以 root 身份运行(EUID=0)时,把指定路径 chown 给 invoking user。
/// 普通进程下完全 noop,因此调用是安全的。对齐 SH `if is_root_user; then chown ...; fi`。
fn chown_to_invoking_user(path: &str) {
    if path.is_empty() {
        return;
    }
    // unsafe geteuid 比 spawn `id -u` 快很多,且不会触发副作用
    let euid = unsafe { libc::geteuid() };
    if euid != 0 {
        return;
    }
    let uid = std::env::var("SUDO_UID").unwrap_or_default();
    let gid = std::env::var("SUDO_GID").unwrap_or_default();
    if uid.is_empty() || gid.is_empty() {
        return;
    }
    let _ = Command::new("chown")
        .args(["-R", &format!("{uid}:{gid}"), path])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

pub fn get_brand_name(name: &str) -> String {
    let is_chinese = *IS_CHINESE_SYSTEM.get_or_init(|| {
        std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLanguages"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("zh"))
            .unwrap_or(false)
    });
    if is_chinese {
        match name {
            "qiyimac" | "iQiyi" => "爱奇艺".to_string(),
            "wechat" | "WeChat" => "微信".to_string(),
            "QQ" => "QQ".to_string(),
            "VooV Meeting" => "腾讯会议".to_string(),
            "dingtalk" | "DingTalk" => "钉钉".to_string(),
            "NeteaseMusic" | "NetEase Music" => "网易云音乐".to_string(),
            "BaiduNetdisk" | "Baidu NetDisk" => "百度网盘".to_string(),
            "alipay" | "Alipay" => "支付宝".to_string(),
            "taobao" | "Taobao" => "淘宝".to_string(),
            "futunn" | "Futu NiuNiu" => "富途牛牛".to_string(),
            "tencent lemon" | "Tencent Lemon Cleaner" | "Tencent Lemon" => {
                "腾讯柠檬清理".to_string()
            }
            _ => name.to_string(),
        }
    } else {
        match name {
            "qiyimac" | "爱奇艺" => "iQiyi".to_string(),
            "wechat" | "微信" => "WeChat".to_string(),
            "QQ" => "QQ".to_string(),
            "腾讯会议" => "VooV Meeting".to_string(),
            "dingtalk" | "钉钉" => "DingTalk".to_string(),
            "网易云音乐" => "NetEase Music".to_string(),
            "百度网盘" => "Baidu NetDisk".to_string(),
            "alipay" | "支付宝" => "Alipay".to_string(),
            "taobao" | "淘宝" => "Taobao".to_string(),
            "富途牛牛" => "Futu NiuNiu".to_string(),
            "腾讯柠檬清理" | "Tencent Lemon Cleaner" => "Tencent Lemon".to_string(),
            "keynote" | "Keynote" => "Keynote".to_string(),
            "pages" | "Pages" => "Pages".to_string(),
            "numbers" | "Numbers" => "Numbers".to_string(),
            _ => name.to_string(),
        }
    }
}

pub fn bytes_to_human(bytes: u64) -> String {
    let k = crate::constants::SIZE_BASE;
    let mb = k.saturating_mul(k);
    let gb = mb.saturating_mul(k);
    if bytes >= gb {
        let scaled = (bytes * 100 + gb / 2) / gb;
        format!("{}.{:02}GB", scaled / 100, scaled % 100)
    } else if bytes >= mb {
        let scaled = (bytes * 10 + mb / 2) / mb;
        format!("{}.{:01}MB", scaled / 10, scaled % 10)
    } else if bytes >= k {
        format!("{}KB", (bytes + k / 2) / k)
    } else {
        format!("{bytes}B")
    }
}

pub fn bytes_to_human_kb(kb: u64) -> String {
    bytes_to_human(kb.saturating_mul(1024))
}

pub fn bytes_human_from_kb(kb: u64) -> String {
    bytes_to_human(kb.saturating_mul(1024))
}

pub fn cleanup_result_color_kb() -> &'static str {
    GREEN
}

/// iCloud「优化 Mac 存储」下未下载到本地的占位文件（dataless）标记位（`st_flags`）。
#[cfg(target_os = "macos")]
pub const SF_DATALESS: u32 = 0x4000_0000;

/// 判断文件元数据是否为 iCloud dataless（云端占位、本地未 materialize）。
///
/// 这类文件 `metadata.len()` 返回完整逻辑大小，但本地实际占用为 0（`st_blocks == 0`）。
/// 逻辑口径下若计入会高估本地可回收空间，故尺寸统计一律跳过。非 macOS 平台恒 `false`。
pub fn is_dataless(meta: &std::fs::Metadata) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        meta.st_flags() & SF_DATALESS != 0
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = meta;
        false
    }
}

pub fn get_path_size_kb(path: &str) -> u64 {
    let p = Path::new(path);
    if !p.exists() {
        return 0;
    }
    let path_str = path;
    if path_str.ends_with(".app") || path_str.ends_with(".app/") {
        if let Some(out) = run_cmd("mdls", &["-name", "kMDItemLogicalSize", "-raw", path_str]) {
            if let Ok(val) = out.trim().parse::<u64>() {
                if val > 0 {
                    return val / 1024;
                }
            }
        }
    }
    // Fast path for regular files and symlinks: use lstat (syscall, no fork).
    // Aligns with Shell stat -f%z path. du -skP is reserved for directories.
    if p.is_file() || p.is_symlink() {
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            // iCloud dataless（云端占位）本地占用为 0，逻辑口径下不计入，避免高估可回收空间
            if is_dataless(&meta) {
                return 0;
            }
            let bytes = meta.len();
            if bytes > 0 {
                return (bytes + 1023) / 1024;
            }
        }
    }
    run_cmd("du", &["-skP", path_str])
        .and_then(|s| {
            s.split_whitespace()
                .next()
                .unwrap_or("0")
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0)
}

pub fn normalize_slashes(path: &str) -> String {
    let mut s = path.to_string();
    while s.contains("//") {
        s = s.replace("//", "/");
    }
    s
}

/// Bash 风格的 glob 匹配,对齐 `[[ "$x" == $pattern ]]`。
/// 支持的元字符:
///   `*`       任意长度任意字符(包括空)
///   `?`       恰好一个任意字符
///   `[abc]`   字符集
///   `[a-z]`   字符范围
///   `[!abc]`  反向字符集(对齐 bash extglob 行为)
pub fn wildcard_match(input: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    // 仅含字面量的快速路径
    if !pattern.contains('*') && !pattern.contains('?') && !pattern.contains('[') {
        return input == pattern;
    }
    let in_chars: Vec<char> = input.chars().collect();
    let pat_chars: Vec<char> = pattern.chars().collect();
    glob_match(&in_chars, 0, &pat_chars, 0)
}

fn glob_match(input: &[char], mut i: usize, pattern: &[char], mut p: usize) -> bool {
    while p < pattern.len() {
        match pattern[p] {
            '*' => {
                // 跳过连续的 *,然后尝试在剩余 input 上每个偏移都做一次匹配
                while p < pattern.len() && pattern[p] == '*' {
                    p += 1;
                }
                if p == pattern.len() {
                    return true;
                }
                while i <= input.len() {
                    if glob_match(input, i, pattern, p) {
                        return true;
                    }
                    if i == input.len() {
                        return false;
                    }
                    i += 1;
                }
                return false;
            }
            '?' => {
                if i >= input.len() {
                    return false;
                }
                i += 1;
                p += 1;
            }
            '[' => {
                if i >= input.len() {
                    return false;
                }
                // 寻找配对的 ]
                let mut end = p + 1;
                while end < pattern.len() && pattern[end] != ']' {
                    end += 1;
                }
                if end >= pattern.len() {
                    // 未闭合:按字面量匹配
                    if input[i] != '[' {
                        return false;
                    }
                    i += 1;
                    p += 1;
                    continue;
                }
                let mut set_start = p + 1;
                let mut negate = false;
                if set_start < end && (pattern[set_start] == '!' || pattern[set_start] == '^') {
                    negate = true;
                    set_start += 1;
                }
                let ch = input[i];
                let mut matched = false;
                let mut k = set_start;
                while k < end {
                    if k + 2 < end && pattern[k + 1] == '-' {
                        if ch >= pattern[k] && ch <= pattern[k + 2] {
                            matched = true;
                        }
                        k += 3;
                    } else {
                        if ch == pattern[k] {
                            matched = true;
                        }
                        k += 1;
                    }
                }
                if matched == negate {
                    return false;
                }
                i += 1;
                p = end + 1;
            }
            _ => {
                if i >= input.len() || input[i] != pattern[p] {
                    return false;
                }
                i += 1;
                p += 1;
            }
        }
    }
    i == input.len()
}

pub fn extract_bundle_from_path(path: &str) -> Option<String> {
    let marker1 = "/Library/Containers/";
    if let Some(pos) = path.find(marker1) {
        let rest = &path[(pos + marker1.len())..];
        let token = rest.split('/').next().unwrap_or_default();
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    let marker2 = "/Library/Group Containers/";
    if let Some(pos) = path.find(marker2) {
        let rest = &path[(pos + marker2.len())..];
        let token = rest.split('/').next().unwrap_or_default();
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    None
}

pub fn normalize_temp_root(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let mut v = if path.starts_with('~') {
        path.replacen('~', &std::env::var("HOME").unwrap_or_default(), 1)
    } else {
        path.to_string()
    };
    while v.len() > 1 && v.ends_with('/') {
        v.pop();
    }
    Some(v)
}

pub fn probe_temp_root(raw_path: &str, allow_create: bool) -> Option<String> {
    let path = normalize_temp_root(raw_path)?;
    if allow_create {
        ensure_user_dir(&path);
    }
    let p = Path::new(&path);
    if !p.is_dir() {
        return None;
    }
    let probe = p.join(format!("mole.probe.{}", get_epoch_seconds()));
    if std::fs::File::create(&probe).is_err() {
        return None;
    }
    let _ = std::fs::remove_file(&probe);
    Some(path)
}

pub fn ensure_mole_temp_root() {
    let _ = get_mole_temp_root();
}

pub fn prepare_mole_tmpdir() -> String {
    let tmp = get_mole_temp_root();
    unsafe { std::env::set_var("TMPDIR", &tmp) };
    tmp
}

pub fn get_mole_temp_root() -> String {
    RESOLVED_TMPDIR
        .get_or_init(|| {
            let candidate = std::env::var("TMPDIR").ok().filter(|v| !v.is_empty());
            if let Some(ref c) = candidate {
                if let Some(resolved) = probe_temp_root(c, false) {
                    return resolved;
                }
            }
            let invoking_home = get_invoking_home();
            if !invoking_home.is_empty() {
                let cache_tmp = format!("{invoking_home}/.cache/mole/tmp");
                if let Some(resolved) = probe_temp_root(&cache_tmp, true) {
                    return resolved;
                }
            }
            probe_temp_root("/tmp", false).unwrap_or_else(|| "/tmp".to_string())
        })
        .clone()
}

pub fn mole_temp_path_template(prefix: &str) -> String {
    format!("{}/{}.XXXXXX", get_mole_temp_root(), prefix)
}

/// 用 system mktemp 生成唯一文件,完全对齐 SH `mktemp "$tmpdir/mole.XXXXXX"`。
/// 失败时退回到 epoch + pid + 计数器,避免并发碰撞。
pub fn create_temp_file() -> Option<PathBuf> {
    let template = format!("{}/mole.XXXXXX", get_mole_temp_root());
    let path = mktemp_via_command(&template, false)?;
    register_temp_file(path.clone());
    Some(path)
}

/// 用 system mktemp -d 生成唯一目录,对齐 SH `mktemp -d "$tmpdir/mole.XXXXXX"`
pub fn create_temp_dir() -> Option<PathBuf> {
    let template = format!("{}/mole.XXXXXX", get_mole_temp_root());
    let path = mktemp_via_command(&template, true)?;
    register_temp_dir(path.clone());
    Some(path)
}

/// 自定义前缀的临时文件,对齐 SH `mktemp "$prefix.XXXXXX"`
pub fn mktemp_file(prefix: &str) -> Option<PathBuf> {
    let template = format!("{}/{}.XXXXXX", get_mole_temp_root(), prefix);
    let path = mktemp_via_command(&template, false)?;
    register_temp_file(path.clone());
    Some(path)
}

fn mktemp_via_command(template: &str, is_dir: bool) -> Option<PathBuf> {
    let mut cmd = Command::new("mktemp");
    if is_dir {
        cmd.arg("-d");
    }
    cmd.arg(template);
    if let Ok(output) = cmd.output() {
        if output.status.success() {
            let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !raw.is_empty() {
                return Some(PathBuf::from(raw));
            }
        }
    }
    // mktemp 不可用时的 fallback:用单调递增计数器 + epoch + pid
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
    let suffix = format!("{}.{}.{}", std::process::id(), get_epoch_seconds(), counter);
    let final_path = template.replace("XXXXXX", &suffix);
    let p = PathBuf::from(&final_path);
    if is_dir {
        std::fs::create_dir_all(&p).ok()?;
    } else if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&p)
            .ok()?;
    }
    Some(p)
}

pub fn register_temp_file(path: impl Into<PathBuf>) {
    let files = TEMP_FILES.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut guard) = files.lock() {
        guard.push(path.into());
    }
}

pub fn register_temp_dir(path: impl Into<PathBuf>) {
    let dirs = TEMP_DIRS.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut guard) = dirs.lock() {
        guard.push(path.into());
    }
}

pub fn cleanup_temp_files() {
    if let Some(files) = TEMP_FILES.get() {
        if let Ok(mut g) = files.lock() {
            for f in g.iter() {
                let _ = std::fs::remove_file(f);
            }
            g.clear();
        }
    }
    if let Some(dirs) = TEMP_DIRS.get() {
        if let Ok(mut g) = dirs.lock() {
            for d in g.iter() {
                let _ = std::fs::remove_dir_all(d);
            }
            g.clear();
        }
    }
}

pub fn is_dry_run() -> bool {
    std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1"
        || std::env::var("DRY_RUN").unwrap_or_default() == "true"
}

fn get_export_list_path() -> std::path::PathBuf {
    let export_file = EXPORT_LIST_FILE.get_or_init(|| Mutex::new(String::new()));
    if let Ok(guard) = export_file.lock() {
        if !guard.is_empty() {
            return PathBuf::from(&*guard);
        }
    }
    let path = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("mole/clean-list.txt");
    if let Ok(mut export) = export_file.lock() {
        *export = path.to_string_lossy().to_string();
    }
    path
}

pub fn start_section(title: &str) {
    let track = TRACK_SECTION.get_or_init(|| Mutex::new(0));
    let activity = SECTION_ACTIVITY.get_or_init(|| Mutex::new(0));
    let current_section = CURRENT_SECTION.get_or_init(|| Mutex::new(String::new()));

    if let Ok(mut t) = track.lock() {
        *t = 1;
    }
    if let Ok(mut a) = activity.lock() {
        *a = 0;
    }
    if let Ok(mut cs) = current_section.lock() {
        *cs = title.to_string();
    }

    // 时序埋点（卡顿分析）：记录本段起点，供 end_section 打印耗时
    let started = SECTION_STARTED_AT.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = started.lock() {
        *g = Some((title.to_string(), Instant::now()));
    }

    println!();
    println!("{PURPLE_BOLD}{ICON_ARROW} {title}{NC}");

    // 推送给前端
    spinner_try_emit(SpinnerUpdatePayload {
        section: title.to_string(),
        message: format!("Scanning {}...", title),
        is_active: true,
        duration_ms: None,
    });

    if is_dry_run() {
        let export_path = get_export_list_path();
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&export_path)
        {
            let _ = writeln!(file);
            let _ = writeln!(file, "=== {} ===", title);
        }
    }
}

pub fn end_section() {
    let track = TRACK_SECTION.get_or_init(|| Mutex::new(0));
    let activity = SECTION_ACTIVITY.get_or_init(|| Mutex::new(0));
    let current_section = CURRENT_SECTION.get_or_init(|| Mutex::new(String::new()));

    let t = track.lock().map(|v| *v).unwrap_or(0);
    let a = activity.lock().map(|v| *v).unwrap_or(0);

    // 获取当前 section 名称用于推送
    let section_name = current_section
        .lock()
        .map(|cs| cs.clone())
        .unwrap_or_default();

    if t == 1 && a == 0 {
        println!("  {GREEN}{ICON_SUCCESS}{NC} Nothing to tidy");
    }
    if let Ok(mut t) = track.lock() {
        *t = 0;
    }
    if let Ok(mut cs) = current_section.lock() {
        cs.clear();
    }

    // 推送给前端（结束当前阶段）
    spinner_try_emit(SpinnerUpdatePayload {
        section: section_name,
        message: "Complete".to_string(),
        is_active: false,
        duration_ms: None,
    });

    // 时序埋点（卡顿分析）：本段耗时（配合 start_section 写入的起点；未 begin 时静默）
    let started = SECTION_STARTED_AT.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = started.lock() {
        if let Some((title, at)) = g.take() {
            log::info!(
                "[section] \"{}\" took {:.0}ms",
                title,
                at.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}

pub fn note_activity() {
    let track = TRACK_SECTION.get_or_init(|| Mutex::new(0));
    let activity = SECTION_ACTIVITY.get_or_init(|| Mutex::new(0));
    if track.lock().map(|v| *v).unwrap_or(0) == 1 {
        if let Ok(mut a) = activity.lock() {
            *a = 1;
        }
    }
}

/// 开始一段“是否发生清理动作”的探针；配合 [`note_activity`] 与 [`take_activity_probe_result`] 使用。
pub fn begin_activity_probe() {
    let track = TRACK_SECTION.get_or_init(|| Mutex::new(0));
    let activity = SECTION_ACTIVITY.get_or_init(|| Mutex::new(0));
    if let Ok(mut t) = track.lock() {
        *t = 1;
    }
    if let Ok(mut a) = activity.lock() {
        *a = 0;
    }
}

/// 结束探针并返回本段是否发生过清理动作。
pub fn take_activity_probe_result() -> bool {
    let track = TRACK_SECTION.get_or_init(|| Mutex::new(0));
    let activity = SECTION_ACTIVITY.get_or_init(|| Mutex::new(0));
    let cleaned = activity.lock().map(|v| *v == 1).unwrap_or(false);
    if let Ok(mut t) = track.lock() {
        *t = 0;
    }
    cleaned
}

/// 开始阶段 spinner：先 [`stop_section_spinner`]（结束上一段并推送 `is_active: false`），再推送本段。
/// `section` 供前端区分模块（如 `system`、`scan`）；`message` 为展示文案。
pub fn start_section_spinner(section: &str, message: &str) {
    stop_section_spinner();
    let final_message = if message.trim().is_empty() {
        "Scanning...".to_string()
    } else {
        message.to_string()
    };
    let section = section.to_string();
    if let Ok(mut guard) = SPINNER_PHASE.get_or_init(|| Mutex::new(None)).lock() {
        *guard = Some(SpinnerPhase {
            section: section.clone(),
            started: Instant::now(),
        });
    }
    spinner_try_emit(SpinnerUpdatePayload {
        section,
        message: final_message,
        is_active: true,
        duration_ms: None,
    });
    if is_ansi_supported() {
        let line = if message.trim().is_empty() {
            "Scanning..."
        } else {
            message
        };
        eprint!("\r\u{001b}[2K  {line}");
        let _ = std::io::stderr().flush();
    }
}

pub fn stop_section_spinner() {
    let ended = SPINNER_PHASE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|mut g| mem::take(&mut *g));
    if let Some(phase) = ended {
        spinner_try_emit(SpinnerUpdatePayload {
            section: phase.section,
            message: String::new(),
            is_active: false,
            duration_ms: Some(phase.started.elapsed().as_millis() as u64),
        });
        if is_ansi_supported() {
            eprint!("\r\u{001b}[2K");
            let _ = std::io::stderr().flush();
        }
    }
}

pub fn safe_clear_lines(_lines: usize, _tty_device: Option<&str>) -> bool {
    is_ansi_supported()
}

pub fn safe_clear_line(_tty_device: Option<&str>) -> bool {
    is_ansi_supported()
}

pub fn update_progress_if_needed(
    completed: u64,
    total: u64,
    last_time: &mut u64,
    interval: u64,
) -> bool {
    let current = get_epoch_seconds();
    if current.saturating_sub(*last_time) >= interval {
        start_section_spinner("scan", &format!("Scanning items... {completed}/{total}"));
        *last_time = current;
        return true;
    }
    false
}

/// ANSI 颜色是否可用。对齐 SH:同时要求 TERM 非 dumb 且 stdout 是 tty。
/// GUI 后端默认非 tty,因此 ANSI 转义会被正确关闭,日志文件里不会留下乱码。
pub fn is_ansi_supported() -> bool {
    *ANSI_SUPPORTED_CACHE.get_or_init(|| {
        let term = std::env::var("TERM").unwrap_or_default();
        if term.is_empty() || term == "dumb" || term == "unknown" {
            return false;
        }
        // unsafe isatty 比 spawn `tty` 快得多
        unsafe { libc::isatty(libc::STDOUT_FILENO) == 1 }
    })
}

static CLEAN_CANCELLED: AtomicBool = AtomicBool::new(false);

pub fn set_clean_cancelled() {
    CLEAN_CANCELLED.store(true, Ordering::SeqCst);
}

pub fn is_clean_cancelled() -> bool {
    CLEAN_CANCELLED.load(Ordering::SeqCst)
}

pub fn reset_clean_cancelled() {
    CLEAN_CANCELLED.store(false, Ordering::SeqCst);
}
