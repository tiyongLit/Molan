use super::base::{
    ICON_ERROR, ICON_SUCCESS, ICON_WARNING, bytes_to_human, ensure_user_file, get_epoch_seconds,
    get_file_size, home_dir,
};
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

pub const LOG_MAX_SIZE_DEFAULT: u64 = 1_048_576;
pub const OPLOG_MAX_SIZE_DEFAULT: u64 = 5_242_880;

static LOG_ROTATED: OnceLock<bool> = OnceLock::new();
static SYS_INFO_LOGGED: AtomicBool = AtomicBool::new(false);

fn debug_enabled() -> bool {
    std::env::var("MO_DEBUG").unwrap_or_default() == "1"
}

/// 同时把一行写到主日志和(MO_DEBUG=1 时)调试日志,对齐 SH 第 101-103/112-114/123-125/134-136 行
fn append_main_with_debug_mirror(line: &str) {
    append_log_line(&log_file(), line);
    if debug_enabled() {
        append_log_line(&debug_log_file(), line);
    }
}

pub fn log_file() -> String {
    format!("{}/Library/Logs/mole/mole.log", home_dir())
}

pub fn operations_log_file() -> String {
    format!("{}/Library/Logs/mole/operations.log", home_dir())
}

pub fn debug_log_file() -> String {
    format!("{}/Library/Logs/mole/mole_debug_session.log", home_dir())
}

pub fn append_log_line(file_path: &str, line: &str) {
    ensure_user_file(file_path);
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file_path)
        .and_then(|mut f| {
            use std::io::Write;
            writeln!(f, "{line}")
        });
}

pub fn append_log_lines(file_path: &str, lines: &[String]) {
    ensure_user_file(file_path);
    for line in lines {
        append_log_line(file_path, line);
    }
}

pub fn rotate_log_once() {
    if LOG_ROTATED.get().is_some() {
        return;
    }
    let _ = LOG_ROTATED.set(true);
    rotate_if_needed(&log_file(), LOG_MAX_SIZE_DEFAULT);
    rotate_if_needed(&operations_log_file(), OPLOG_MAX_SIZE_DEFAULT);
}

pub fn get_timestamp() -> String {
    let secs = get_epoch_seconds() as i64;
    if let Some(dt) = chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0) {
        dt.with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    } else {
        "1970-01-01 00:00:00".to_string()
    }
}

pub fn log_info(msg: &str) {
    eprintln!("{msg}");
    append_main_with_debug_mirror(&format!("[{}] INFO: {}", get_timestamp(), msg));
}

pub fn log_success(msg: &str) {
    eprintln!("  {ICON_SUCCESS} {msg}");
    append_main_with_debug_mirror(&format!("[{}] SUCCESS: {}", get_timestamp(), msg));
}

pub fn log_warning(msg: &str) {
    eprintln!("{ICON_WARNING} {msg}");
    append_main_with_debug_mirror(&format!("[{}] WARNING: {}", get_timestamp(), msg));
}

pub fn log_error(msg: &str) {
    // 对齐 SH 第 130 行 `${ICON_ERROR} $1 >&2`,GUI 后端依然走 stderr 让 logger plugin 抓
    eprintln!("{ICON_ERROR} {msg}");
    append_main_with_debug_mirror(&format!("[{}] ERROR: {}", get_timestamp(), msg));
}

/// 对齐 SH 第 140-147 行:DEBUG 同时输出到 stderr(便于实时观察)+ 调试日志文件
pub fn debug_log(msg: &str) {
    if debug_enabled() {
        eprintln!("[DEBUG] {msg}");
        append_log_line(
            &debug_log_file(),
            &format!("[{}] DEBUG: {}", get_timestamp(), msg),
        );
    }
}

pub fn oplog_enabled() -> bool {
    std::env::var("MO_NO_OPLOG").unwrap_or_default() != "1"
}

pub fn log_operation(command: &str, action: &str, path: &str, detail: Option<&str>) {
    if !oplog_enabled() || path.is_empty() {
        return;
    }
    let mut line = format!("[{}] [{}] {} {}", get_timestamp(), command, action, path);
    if let Some(d) = detail {
        if !d.is_empty() {
            line.push_str(&format!(" ({d})"));
        }
    }
    append_log_line(&operations_log_file(), &line);
}

pub fn log_operation_session_start(command: &str) {
    if !oplog_enabled() {
        return;
    }
    append_log_lines(
        &operations_log_file(),
        &[
            "".to_string(),
            format!(
                "# ========== {} session started at {} ==========",
                command,
                get_timestamp()
            ),
        ],
    );
}

/// session 结束行,对齐 SH 第 235-243 行:把 size 转成人类格式("1.23GB" / "0B" 等)
pub fn log_operation_session_end(command: &str, items: u64, size_kb: u64) {
    if !oplog_enabled() {
        return;
    }
    let size_human = if size_kb > 0 {
        bytes_to_human(size_kb.saturating_mul(1024))
    } else {
        "0B".to_string()
    };
    append_log_line(
        &operations_log_file(),
        &format!(
            "# ========== {} session ended at {}, {} items, {} ==========",
            command,
            get_timestamp(),
            items,
            size_human
        ),
    );
}

pub fn debug_operation_start(operation_name: &str, operation_desc: Option<&str>) {
    if !debug_enabled() {
        return;
    }
    eprintln!("[DEBUG] === {operation_name} ===");
    if let Some(desc) = operation_desc {
        if !desc.is_empty() {
            eprintln!("[DEBUG] {desc}");
        }
    }
    let mut lines = vec!["".to_string(), format!("=== {operation_name} ===")];
    if let Some(desc) = operation_desc {
        if !desc.is_empty() {
            lines.push(format!("Description: {desc}"));
        }
    }
    append_log_lines(&debug_log_file(), &lines);
}

pub fn debug_operation_detail(detail_type: &str, detail_value: &str) {
    if !debug_enabled() {
        return;
    }
    eprintln!("[DEBUG] {detail_type}: {detail_value}");
    append_log_line(&debug_log_file(), &format!("{detail_type}: {detail_value}"));
}

/// 对齐 log.sh:debug_file_action() 第 287-304 行,格式为:
///   "<action>:   * <path>, <size>, <age> days old"
/// 注意:age 字段后面要带 " days old" 后缀,之前 Rust 版本漏了。
pub fn debug_file_action(
    action: &str,
    file_path: &str,
    file_size: Option<&str>,
    file_age: Option<&str>,
) {
    if !debug_enabled() {
        return;
    }
    let mut msg = format!("  * {file_path}");
    if let Some(size) = file_size {
        if !size.is_empty() {
            msg.push_str(&format!(", {size}"));
        }
    }
    if let Some(age) = file_age {
        if !age.is_empty() {
            msg.push_str(&format!(", {age} days old"));
        }
    }
    eprintln!("[DEBUG] {action}: {msg}");
    append_log_line(&debug_log_file(), &format!("{action}: {msg}"));
}

pub fn debug_risk_level(risk_level: &str, reason: &str) {
    if !debug_enabled() {
        return;
    }
    eprintln!("[DEBUG] Risk Level: {risk_level}, {reason}");
    append_log_line(
        &debug_log_file(),
        &format!("Risk Level: {risk_level}, {reason}"),
    );
}

/// 系统信息只写一次(对齐 SH 第 330-331 行 MOLE_SYS_INFO_LOGGED 哨兵),并且会先把 debug 日志清空
/// 让每个 GUI 后端 session 起步时拿到干净的调试上下文。
pub fn log_system_info() {
    if !debug_enabled() {
        return;
    }
    if SYS_INFO_LOGGED.swap(true, Ordering::SeqCst) {
        return;
    }
    let dbg_path = debug_log_file();
    ensure_user_file(&dbg_path);
    if std::fs::write(&dbg_path, b"").is_err() {
        eprintln!("{ICON_WARNING} Debug log not writable: {dbg_path}");
    }

    let hostname = std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let arch = std::process::Command::new("uname")
        .arg("-m")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let kernel = std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let macos_version = std::process::Command::new("sw_vers")
        .output()
        .ok()
        .map(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            let pv = out
                .lines()
                .find(|l| l.contains("ProductVersion"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim())
                .unwrap_or("");
            let bv = out
                .lines()
                .find(|l| l.contains("BuildVersion"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim())
                .unwrap_or("");
            if bv.is_empty() {
                pv.to_string()
            } else {
                format!("{pv}, {bv}")
            }
        })
        .unwrap_or_else(|| "unknown".to_string());
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".to_string());
    let term = std::env::var("TERM").unwrap_or_else(|_| "unknown".to_string());
    let sudo_status = if std::env::var("MOLE_TEST_MODE").unwrap_or_default() == "1"
        || std::env::var("MOLE_TEST_NO_AUTH").unwrap_or_default() == "1"
    {
        "Skipped (test mode)".to_string()
    } else if crate::core::sudo::sudo_output(&["/usr/bin/true"])
        .status
        .success()
    {
        "Active".to_string()
    } else {
        "Required".to_string()
    };

    let lines = vec![
        "----------------------------------------------------------------------".to_string(),
        format!("Mole Debug Session, {}", get_timestamp()),
        "----------------------------------------------------------------------".to_string(),
        format!("User: {}", std::env::var("USER").unwrap_or_default()),
        format!("Hostname: {hostname}"),
        format!("Architecture: {arch}"),
        format!("Kernel: {kernel}"),
        format!("macOS: {macos_version}"),
        format!("Shell: {shell}, {term}"),
        format!("Sudo Access: {sudo_status}"),
        "----------------------------------------------------------------------".to_string(),
    ];
    append_log_lines(&dbg_path, &lines);
    eprintln!("[DEBUG] Debug logging enabled. Session log: {dbg_path}");
}

pub fn run_silent(command: &[String]) {
    if command.is_empty() {
        return;
    }
    let mut cmd = std::process::Command::new(&command[0]);
    if command.len() > 1 {
        cmd.args(&command[1..]);
    }
    let _ = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
}

pub fn run_logged(command: &[String]) -> bool {
    if command.is_empty() {
        return false;
    }
    let mut cmd = std::process::Command::new(&command[0]);
    if command.len() > 1 {
        cmd.args(&command[1..]);
    }
    let out = cmd.output();
    let ok = out.as_ref().map(|o| o.status.success()).unwrap_or(false);
    if let Ok(o) = out {
        let mut merged = String::new();
        merged.push_str(&String::from_utf8_lossy(&o.stdout));
        merged.push_str(&String::from_utf8_lossy(&o.stderr));
        if !merged.trim().is_empty() {
            append_log_line(&log_file(), merged.trim_end());
        }
        if std::env::var("MO_DEBUG").unwrap_or_default() == "1" && !merged.trim().is_empty() {
            append_log_line(&debug_log_file(), merged.trim_end());
        }
    }
    if !ok {
        log_warning(&format!("Command failed: {}", command[0]));
        return false;
    }
    true
}

pub fn print_summary_block(heading: &str, details: &[String]) -> String {
    let mut width = 70usize;
    if let Ok(cols) = std::process::Command::new("tput").arg("cols").output() {
        if cols.status.success() {
            if let Ok(v) = String::from_utf8_lossy(&cols.stdout)
                .trim()
                .parse::<usize>()
            {
                width = v.min(70);
            }
        }
    }
    let divider = "=".repeat(width);
    let mut out = String::new();
    out.push('\n');
    out.push_str(&divider);
    out.push('\n');
    if !heading.is_empty() {
        out.push_str(heading);
        out.push('\n');
    }
    for d in details {
        if !d.is_empty() {
            out.push_str(d);
            out.push('\n');
        }
    }
    out.push_str(&divider);
    out.push('\n');
    if std::env::var("MO_DEBUG").unwrap_or_default() == "1" {
        out.push_str(&format!(
            "Debug session log saved to: {}\n",
            debug_log_file()
        ));
    }
    out
}

fn rotate_if_needed(file: &str, max_size: u64) {
    if Path::new(file).exists() && get_file_size(file) > max_size {
        let _ = std::fs::rename(file, format!("{file}.old"));
        ensure_user_file(file);
    }
}
