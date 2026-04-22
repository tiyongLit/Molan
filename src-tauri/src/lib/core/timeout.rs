//! Mole timeout helpers — 对齐 lib/core/timeout.sh
//!
//! 选择策略 (priority high → low):
//!   1. gtimeout / timeout (coreutils)         — 推荐
//!   2. perl helper(setsid + 进程组 kill)    — fallback,POSIX 自带
//!   3. wait_timeout + 进程组 kill             — Rust 自身实现
//!
//! 退出码语义:
//!   - 命令正常退出 → 命令自己的 exit code
//!   - 超时         → 124 (与 GNU `timeout` 一致)
//!   - 被信号中断   → 128 + signo

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;
use wait_timeout::ChildExt;

static TIMEOUT_BIN: OnceLock<Option<String>> = OnceLock::new();
static PERL_BIN: OnceLock<Option<String>> = OnceLock::new();

fn debug_enabled() -> bool {
    std::env::var("MO_DEBUG").unwrap_or_default() == "1"
}

pub fn detect_timeout_bin() -> Option<String> {
    TIMEOUT_BIN
        .get_or_init(|| {
            for candidate in ["gtimeout", "timeout"] {
                if Command::new(candidate)
                    .arg("--version")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
                {
                    if debug_enabled() {
                        eprintln!("[TIMEOUT] Using command: {candidate}");
                    }
                    return Some(candidate.to_string());
                }
            }
            None
        })
        .clone()
}

fn detect_perl_bin() -> Option<String> {
    PERL_BIN
        .get_or_init(|| {
            Command::new("perl")
                .arg("-e")
                .arg("exit 0")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .ok()
                .filter(|s| s.success())
                .map(|_| "perl".to_string())
        })
        .clone()
}

/// 用 timeout 包装一条命令。失败时尽力返回有意义的 exit code。
///
/// `duration_secs` <= 0 时禁用超时,直接同步运行。
pub fn run_with_timeout(duration_secs: f64, cmd: &str, args: &[&str]) -> i32 {
    if duration_secs <= 0.0 {
        return Command::new(cmd)
            .args(args)
            .status()
            .ok()
            .and_then(|s| s.code())
            .unwrap_or(1);
    }

    if debug_enabled() {
        eprintln!(
            "[TIMEOUT] Running with {}s timeout: {} {}",
            duration_secs,
            cmd,
            args.join(" ")
        );
    }

    // 1. 系统 timeout / gtimeout(最稳)
    if let Some(timeout_bin) = detect_timeout_bin() {
        return Command::new(timeout_bin)
            .arg(format!("{duration_secs}"))
            .arg(cmd)
            .args(args)
            .status()
            .ok()
            .and_then(|s| s.code())
            .unwrap_or(124);
    }

    // 2. perl fallback(对齐 SH 第 116-175 行;setsid + 进程组 SIGTERM/SIGKILL)
    if let Some(perl_bin) = detect_perl_bin() {
        if debug_enabled() {
            eprintln!(
                "[TIMEOUT] Perl fallback, {}s: {} {}",
                duration_secs,
                cmd,
                args.join(" ")
            );
        }
        let script = r#"
use strict;
use warnings;
use POSIX qw(:sys_wait_h setsid);
use Time::HiRes qw(time sleep);
my $duration = 0 + shift @ARGV;
$duration = 1 if $duration <= 0;
my $pid = fork();
defined $pid or exit 125;
if ($pid == 0) {
    setsid() or exit 125;
    exec @ARGV;
    exit 127;
}
my $deadline = time() + $duration;
while (1) {
    my $result = waitpid($pid, WNOHANG);
    if ($result == $pid) {
        if (WIFEXITED($?)) { exit WEXITSTATUS($?); }
        if (WIFSIGNALED($?)) { exit 128 + WTERMSIG($?); }
        exit 1;
    }
    if (time() >= $deadline) {
        kill "TERM", -$pid;
        sleep 0.5;
        for (1 .. 6) {
            $result = waitpid($pid, WNOHANG);
            if ($result == $pid) { exit 124; }
            sleep 0.25;
        }
        kill "KILL", -$pid;
        waitpid($pid, 0);
        exit 124;
    }
    sleep 0.1;
}
"#;
        return Command::new(perl_bin)
            .arg("-e")
            .arg(script)
            .arg(format!("{duration_secs}"))
            .arg(cmd)
            .args(args)
            .status()
            .ok()
            .and_then(|s| s.code())
            .unwrap_or(124);
    }

    // 3. Rust fallback:setsid 让子进程独立成进程组,超时按进程组杀
    if debug_enabled() {
        eprintln!(
            "[TIMEOUT] Shell fallback, {}s: {} {}",
            duration_secs,
            cmd,
            args.join(" ")
        );
    }
    let mut command = Command::new(cmd);
    command.args(args);
    unsafe {
        command.pre_exec(|| {
            // setsid: 子进程成为新 session leader,这样 -pgid 一发就能杀全家
            if libc::setsid() == -1 {
                // 已经是 session leader 时 setsid 会失败,可忽略
            }
            Ok(())
        });
    }
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(_) => return 1,
    };
    let pid = child.id() as i32;

    let timeout = Duration::from_millis((duration_secs * 1000.0) as u64);
    match child.wait_timeout(timeout).ok().flatten() {
        Some(status) => status.code().unwrap_or_else(|| {
            // 被信号杀:对齐 GNU timeout 的 128 + signo 语义
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if let Some(sig) = status.signal() {
                    return 128 + sig;
                }
            }
            1
        }),
        None => {
            // 进程组 SIGTERM
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
            std::thread::sleep(Duration::from_millis(500));
            for _ in 0..6 {
                if let Ok(Some(status)) = child.try_wait().map(|s| s) {
                    let _ = status;
                    if debug_enabled() {
                        eprintln!("[TIMEOUT] Command timed out after {duration_secs}s");
                    }
                    return 124;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let _ = child.wait();
            if debug_enabled() {
                eprintln!("[TIMEOUT] Command timed out after {duration_secs}s");
            }
            124
        }
    }
}

/// 类似 `run_with_timeout_capture`,但**不过滤** exit status,
/// 只要不是超时就返回 stdout(可能为空字符串)。
///
/// 适用场景:`find` 在权限错时 exit 非零、`du` 在跨权限子树时 exit 非零、
/// `plutil -extract` 在 key 缺失时 exit 非零,但 stdout 仍然是有效的。
/// SH 端用 `2> /dev/null` 吞掉 stderr 后取 stdout,这条路径与之对齐。
///
/// 超时(无论是 GNU `timeout`、perl 还是自身 wait_timeout 触发)统一返回 None。
pub fn run_with_timeout_capture_lossy(
    duration_secs: f64,
    cmd: &str,
    args: &[&str],
) -> Option<String> {
    if duration_secs <= 0.0 {
        return Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string());
    }

    if let Some(timeout_bin) = detect_timeout_bin() {
        let out = Command::new(timeout_bin)
            .arg(format!("{duration_secs}"))
            .arg(cmd)
            .args(args)
            .output()
            .ok()?;
        // GNU timeout: 124 表示超时
        if out.status.code() == Some(124) {
            return None;
        }
        return Some(String::from_utf8_lossy(&out.stdout).to_string());
    }

    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(|| {
            let _ = libc::setsid();
            Ok(())
        });
    }
    let mut child = command.spawn().ok()?;
    let pid = child.id() as i32;
    // 等待期间必须并发排空 stdout：macOS 管道缓冲约 64KB，子进程输出超过该值
    // 就会写阻塞，而父进程在 wait_timeout 等退出 → 死锁 → 超时被杀 → 返回 None。
    // reader 线程持走读端持续读取，子进程退出（或被杀）后 read_to_end 自然 EOF。
    let stdout_reader = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = Vec::new();
            let _ = out.read_to_end(&mut buf);
            buf
        })
    });
    let timeout = Duration::from_millis((duration_secs * 1000.0) as u64);
    match child.wait_timeout(timeout).ok().flatten() {
        Some(_status) => {
            let buf = stdout_reader
                .and_then(|h| h.join().ok())
                .unwrap_or_default();
            Some(String::from_utf8_lossy(&buf).to_string())
        }
        None => {
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
            std::thread::sleep(Duration::from_millis(300));
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let _ = child.wait();
            None
        }
    }
}

/// 带退出码的 stdout 捕获:`(rc, stdout)`。rc=124 表示超时;被信号杀死时 rc>=128。
/// 供需要区分"命令正常失败/超时"与"信号中断"的 fail-closed 路径使用
/// (对齐 SH 26f4d47a 加固:探测超时不得当作正常结果静默继续)。
pub fn run_with_timeout_capture_rc(
    duration_secs: f64,
    cmd: &str,
    args: &[&str],
) -> (i32, Option<String>) {
    if duration_secs <= 0.0 {
        return match Command::new(cmd).args(args).output() {
            Ok(o) => (
                o.status.code().unwrap_or(1),
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string()),
            ),
            Err(_) => (1, None),
        };
    }

    if let Some(timeout_bin) = detect_timeout_bin() {
        return match Command::new(timeout_bin)
            .arg(format!("{duration_secs}"))
            .arg(cmd)
            .args(args)
            .output()
        {
            Ok(o) => (
                o.status.code().unwrap_or(124),
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string()),
            ),
            Err(_) => (124, None),
        };
    }

    // Fallback:与新进程组并发排空 stdout(同 capture 版),超时杀全家
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    unsafe {
        command.pre_exec(|| {
            let _ = libc::setsid();
            Ok(())
        });
    }
    let Ok(mut child) = command.spawn() else {
        return (1, None);
    };
    let pid = child.id() as i32;
    let stdout_reader = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = Vec::new();
            let _ = out.read_to_end(&mut buf);
            buf
        })
    });
    let timeout = Duration::from_millis((duration_secs * 1000.0) as u64);

    match child.wait_timeout(timeout).ok().flatten() {
        Some(status) => {
            let buf = stdout_reader
                .and_then(|h| h.join().ok())
                .unwrap_or_default();
            (
                status.code().unwrap_or(1),
                Some(String::from_utf8_lossy(&buf).trim().to_string()),
            )
        }
        None => {
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
            std::thread::sleep(Duration::from_millis(300));
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let _ = child.wait();
            (124, None)
        }
    }
}

/// 类似 `run_with_timeout`,但捕获 stdout 返回。stderr 透传。
/// 失败或超时时返回 None。GUI 侧短超时(2s)的 mdfind/pkgutil 都用这条路径。
pub fn run_with_timeout_capture(duration_secs: f64, cmd: &str, args: &[&str]) -> Option<String> {
    run_with_timeout_capture_impl(duration_secs, cmd, args, false)
}

/// 与 [`run_with_timeout_capture`] 行为一致,但屏蔽 stderr(不继承到父进程终端)。
///
/// 适用场景:失败属于预期路径的探测命令(如无完整 Xcode 时的 `xcrun simctl`),
/// 不应把 raw stderr 喷到 GUI/终端。超时与失败语义同 capture 版。
pub fn run_with_timeout_capture_silent(
    duration_secs: f64,
    cmd: &str,
    args: &[&str],
) -> Option<String> {
    run_with_timeout_capture_impl(duration_secs, cmd, args, true)
}

fn run_with_timeout_capture_impl(
    duration_secs: f64,
    cmd: &str,
    args: &[&str],
    silent_stderr: bool,
) -> Option<String> {
    if duration_secs <= 0.0 {
        return Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    }

    if let Some(timeout_bin) = detect_timeout_bin() {
        return Command::new(timeout_bin)
            .arg(format!("{duration_secs}"))
            .arg(cmd)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    }

    // Fallback:在新进程组里跑,超时杀全家
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(if silent_stderr {
            Stdio::null()
        } else {
            Stdio::inherit()
        });
    unsafe {
        command.pre_exec(|| {
            let _ = libc::setsid();
            Ok(())
        });
    }
    let mut child = command.spawn().ok()?;
    let pid = child.id() as i32;
    // 与 lossy 版同理：等待期间并发排空 stdout，避免大输出（如 pkgutil --files）
    // 写满管道缓冲后与 wait_timeout 形成死锁。
    let stdout_reader = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = Vec::new();
            let _ = out.read_to_end(&mut buf);
            buf
        })
    });
    let timeout = Duration::from_millis((duration_secs * 1000.0) as u64);

    match child.wait_timeout(timeout).ok().flatten() {
        Some(status) if status.success() => {
            let buf = stdout_reader
                .and_then(|h| h.join().ok())
                .unwrap_or_default();
            Some(String::from_utf8_lossy(&buf).trim().to_string())
        }
        Some(_) => None,
        None => {
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
            std::thread::sleep(Duration::from_millis(300));
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let _ = child.wait();
            None
        }
    }
}
