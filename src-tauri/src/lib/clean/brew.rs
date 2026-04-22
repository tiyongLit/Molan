//! Homebrew 清理 — 严格对齐 lib/clean/brew.sh
//!
//! 行为对齐项:
//!   - 白名单短路:`~/Library/Caches/Homebrew` 在白名单时跳过(对齐 SH 第 9-20 行)
//!   - 7 天冷却窗:重复运行时跳过 cleanup(对齐 SH 第 22-37 行)
//!   - 缓存 < 50MB 时跳过 `brew cleanup`,但仍跑 `brew autoremove`(对齐 SH 第 39-47 行)
//!   - 两个子命令并行启动 + 各自 120s 超时(对齐 SH 第 56-74 行)
//!   - 解析输出:统计 "Removing:" 行数与 "X freed" 字段(对齐 SH 第 91-119 行)
//!
//! 返回值:`(removed_items, autoremoved_packages, freed_human)` 给 GUI 直接展示。

use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

use crate::core::app_protection::is_path_whitelisted_from_global;
use crate::core::base::{ensure_user_file, get_epoch_seconds, home_dir};
use crate::core::log::{debug_log, log_warning};
use crate::core::timeout::run_with_timeout_capture;

#[derive(Debug, Default, Clone)]
pub struct HomebrewCleanResult {
    pub skipped: bool,
    pub skip_reason: Option<String>,
    /// `brew cleanup` 报告的 "X freed" 文字(原文,可能为空)
    pub freed_space: Option<String>,
    /// `brew cleanup` 删除条目数(grep "Removing:")
    pub removed_count: u64,
    /// `brew autoremove` 卸载的孤立包数(grep "^Uninstalling")
    pub autoremoved_packages: u64,
    pub cleanup_timed_out: bool,
    pub autoremove_timed_out: bool,
}

fn brew_available() -> bool {
    Command::new("which")
        .arg("brew")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 提取 "X.YGB freed" / "150MB freed" 之类的最后一段
fn extract_freed_space(output: &str) -> Option<String> {
    let mut last: Option<String> = None;
    for line in output.lines() {
        // 仅在含 " freed" 的行里抠数字 + 单位
        let Some(idx) = line.find(" freed") else {
            continue;
        };
        let prefix = &line[..idx];
        // 倒着扫描,找到一段 [0-9.]+[KMGT]B
        let bytes = prefix.as_bytes();
        let mut end = bytes.len();
        if end == 0 {
            continue;
        }
        // 跳过尾部空白
        while end > 0 && bytes[end - 1] == b' ' {
            end -= 1;
        }
        // 必须以 B 结尾
        if end < 2 || bytes[end - 1] != b'B' {
            continue;
        }
        let unit_byte = bytes[end - 2];
        if !matches!(unit_byte, b'K' | b'M' | b'G' | b'T') {
            continue;
        }
        let mut start = end - 2;
        while start > 0 {
            let c = bytes[start - 1];
            if c.is_ascii_digit() || c == b'.' {
                start -= 1;
            } else {
                break;
            }
        }
        if start < end {
            last = Some(String::from_utf8_lossy(&bytes[start..end]).to_string() + " freed");
        }
    }
    last
}

fn count_lines_starting_with(output: &str, prefix: &str) -> u64 {
    output
        .lines()
        .filter(|l| l.trim_start().starts_with(prefix))
        .count() as u64
}

/// 主入口。对齐 SH `clean_homebrew()` 第 5-127 行。
pub fn clean_homebrew() -> HomebrewCleanResult {
    let mut result = HomebrewCleanResult::default();

    if !brew_available() {
        result.skipped = true;
        result.skip_reason = Some("brew not installed".to_string());
        return result;
    }
    let home = home_dir();
    let dry_run = std::env::var("DRY_RUN").unwrap_or_default() == "true"
        || std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1";

    let brew_cache_dir = format!("{home}/Library/Caches/Homebrew");

    // 白名单短路(SH 第 9-20 行)
    if is_path_whitelisted_from_global(&brew_cache_dir) {
        result.skipped = true;
        result.skip_reason = Some("whitelist".to_string());
        return result;
    }

    if dry_run {
        result.skipped = true;
        result.skip_reason = Some("dry-run".to_string());
        return result;
    }

    // 7 天冷却窗(SH 第 22-37 行)
    let brew_cache_file = format!("{home}/.cache/mole/brew_last_cleanup");
    let cache_valid_days: u64 = 7;
    if Path::new(&brew_cache_file).is_file() {
        if let Ok(ts) = std::fs::read_to_string(&brew_cache_file) {
            if let Ok(last) = ts.trim().parse::<u64>() {
                let now = get_epoch_seconds();
                let days_diff = now.saturating_sub(last) / 86400;
                if days_diff < cache_valid_days {
                    result.skipped = true;
                    result.skip_reason = Some(format!("cleaned {days_diff}d ago"));
                    return result;
                }
            }
        }
    }

    // 缓存 < 50MB 时跳过 `brew cleanup` 但仍跑 autoremove(SH 第 39-47 行)
    let mut skip_cleanup = false;
    if Path::new(&brew_cache_dir).is_dir() {
        // run_with_timeout_capture 限定 3s,与 SH `run_with_timeout 3 du -skP` 一致
        if let Some(out) = run_with_timeout_capture(3.0, "du", &["-skP", &brew_cache_dir]) {
            let kb: u64 = out
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            if kb > 0 && kb < 51200 {
                skip_cleanup = true;
            }
        }
    }

    // 并行启动 cleanup + autoremove,各自 120s 超时(SH 第 56-74 行)
    let timeout_seconds = 120.0;
    let (tx, rx) = mpsc::channel::<(String, Option<String>)>();

    let mut handles: Vec<thread::JoinHandle<()>> = Vec::new();
    if !skip_cleanup {
        let txc = tx.clone();
        handles.push(thread::spawn(move || {
            // 用 capture 接全部输出,timeout 内部已实现进程组 KILL
            let out = run_brew_capture(timeout_seconds, &["cleanup", "--prune=30"]);
            let _ = txc.send(("cleanup".to_string(), out));
        }));
    }
    {
        let txc = tx.clone();
        handles.push(thread::spawn(move || {
            let out = run_brew_capture(timeout_seconds, &["autoremove"]);
            let _ = txc.send(("autoremove".to_string(), out));
        }));
    }
    drop(tx);

    let mut cleanup_output: Option<String> = None;
    let mut autoremove_output: Option<String> = None;
    while let Ok((kind, out)) = rx.recv() {
        match kind.as_str() {
            "cleanup" => cleanup_output = Some(out.unwrap_or_default()),
            "autoremove" => autoremove_output = Some(out.unwrap_or_default()),
            _ => {}
        }
    }
    for h in handles {
        let _ = h.join();
    }

    // 解析 cleanup 输出(SH 第 91-105 行)
    if skip_cleanup {
        result.skip_reason = Some("cache <50MB, cleanup skipped".to_string());
    } else if let Some(out) = cleanup_output.as_deref() {
        if out.is_empty() {
            // 大概率超时
            result.cleanup_timed_out = true;
            log_warning("Homebrew cleanup timed out");
        } else {
            result.removed_count = count_lines_starting_with(out, "Removing:");
            result.freed_space = extract_freed_space(out);
        }
    }

    // 解析 autoremove 输出(SH 第 109-119 行)
    if let Some(out) = autoremove_output.as_deref() {
        if out.is_empty() {
            result.autoremove_timed_out = true;
            log_warning("Homebrew autoremove timed out");
        } else {
            result.autoremoved_packages = count_lines_starting_with(out, "Uninstalling");
        }
    }

    // 任意一段成功就更新 cache 时间戳(SH 第 123-126 行)
    let any_success = skip_cleanup
        || (!result.cleanup_timed_out && cleanup_output.is_some())
        || (!result.autoremove_timed_out && autoremove_output.is_some());
    if any_success {
        ensure_user_file(&brew_cache_file);
        let _ = std::fs::write(&brew_cache_file, get_epoch_seconds().to_string());
        debug_log(&format!(
            "Homebrew cleanup completed, removed={}, autoremoved={}, freed={:?}",
            result.removed_count, result.autoremoved_packages, result.freed_space
        ));
    }
    result
}

/// 用 timeout 包一层 `brew <args>`,失败/超时 → None,成功 → 命令的 stdout+stderr。
fn run_brew_capture(timeout_secs: f64, args: &[&str]) -> Option<String> {
    // run_with_timeout_capture 只取 stdout;brew 的进度信息会写 stderr,
    // 因此这里我们手动 spawn:用一个 sh 包装把 stderr 合并到 stdout(对齐 SH `2>&1`)。
    let joined_args = args
        .iter()
        .map(|a| shell_escape(a))
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!("brew {joined_args} 2>&1");
    run_with_timeout_capture(timeout_secs, "sh", &["-lc", &script])
}

fn shell_escape(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    if arg
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '=' | ',' | ':'))
    {
        return arg.to_string();
    }
    let mut s = String::with_capacity(arg.len() + 2);
    s.push('\'');
    for c in arg.chars() {
        if c == '\'' {
            s.push_str("'\\''");
        } else {
            s.push(c);
        }
    }
    s.push('\'');
    s
}
