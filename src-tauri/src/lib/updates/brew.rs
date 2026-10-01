//! Homebrew 更新（brew outdated 捕获 + brew upgrade 流式）。
//! 行为基线见 `controllers/updates.md` §2.2。
//!
//! 管道纪律（防死锁规范）：捕获走 stdout/stderr 双临时文件；流式走逐行 reader 线程，
//! 等待期间并发排空，绝不 wait-then-read。

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use wait_timeout::ChildExt;

/// brew 行条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrewOutdatedItem {
    pub name: String,
    pub installed: String,
    pub latest: String,
    /// "formula" | "cask"
    pub kind: String,
}

/// 流式升级结果。
#[derive(Debug, Clone, Serialize)]
pub struct BrewUpgradeOutcome {
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
}

/// brew 可执行路径（可执行位检查，两个标准前缀）。
pub fn brew_path() -> Option<String> {
    for p in ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"] {
        if is_executable(p) {
            return Some(p.to_string());
        }
    }
    None
}

fn is_executable(p: &str) -> bool {
    let c = std::ffi::CString::new(p).ok();
    match c {
        Some(c) => unsafe { libc::access(c.as_ptr(), libc::X_OK) == 0 },
        None => false,
    }
}

/// PATH 注入：brew 目录前置，保证其内部 git/curl 可找到。
fn brew_env(brew: &str) -> (String, String) {
    let dir = std::path::Path::new(brew)
        .parent()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let old = std::env::var("PATH").unwrap_or_default();
    (
        "PATH".to_string(),
        format!("{dir}:/usr/bin:/bin:/usr/sbin:/sbin:{old}"),
    )
}

/// `brew outdated --json=v2` → 行条目。brew 不存在 / 执行失败 / 超时 → 空。
/// 超时 120s。
pub fn brew_outdated() -> Vec<BrewOutdatedItem> {
    let Some(brew) = brew_path() else {
        return Vec::new();
    };
    let out = brew_capture(&brew, &["outdated", "--json=v2"], 120.0);
    match out {
        Some(text) => parse_outdated(&text),
        None => Vec::new(),
    }
}

/// 捕获 brew 命令 stdout：stdout/stderr 双临时文件（防 64KB 管道死锁），超时杀进程组。
fn brew_capture(brew: &str, args: &[&str], timeout_secs: f64) -> Option<String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_out = std::env::temp_dir().join(format!("mole_brew_out_{nanos}.log"));
    let tmp_err = std::env::temp_dir().join(format!("mole_brew_err_{nanos}.log"));
    let out_file = std::fs::File::create(&tmp_out).ok()?;
    let err_file = std::fs::File::create(&tmp_err).ok()?;

    let (k, v) = brew_env(brew);
    let mut cmd = Command::new(brew);
    cmd.args(args)
        .env(k, v)
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file));
    unsafe {
        cmd.pre_exec(|| {
            let _ = libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn().ok()?;
    let pid = child.id() as i32;
    let timeout = std::time::Duration::from_millis((timeout_secs * 1000.0) as u64);
    if child.wait_timeout(timeout).ok().flatten().is_none() {
        unsafe {
            libc::kill(-pid, libc::SIGTERM);
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        let _ = child.wait();
    }
    let text = std::fs::read_to_string(&tmp_out).ok();
    let _ = std::fs::remove_file(&tmp_out);
    let _ = std::fs::remove_file(&tmp_err);
    text
}

/// `brew upgrade [name]` 流式执行：stdout/stderr 逐行回调（两流逐行 onLine，
/// 噪声由调用侧过滤）。
/// 单包 1800s / 全部 3600s 超时。
pub fn brew_upgrade_streaming<F>(
    name: Option<&str>,
    timeout_secs: f64,
    mut on_line: F,
) -> BrewUpgradeOutcome
where
    F: FnMut(&str) + Send,
{
    let Some(brew) = brew_path() else {
        return BrewUpgradeOutcome {
            ok: false,
            exit_code: None,
            error: Some("Homebrew 未安装".to_string()),
        };
    };

    let mut cmd = Command::new(&brew);
    cmd.arg("upgrade");
    if let Some(n) = name {
        cmd.arg(n);
    }
    let (k, v) = brew_env(&brew);
    cmd.env(k, v).stdout(Stdio::piped()).stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(|| {
            let _ = libc::setsid();
            Ok(())
        });
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return BrewUpgradeOutcome {
                ok: false,
                exit_code: None,
                error: Some(format!("启动 brew 失败: {e}")),
            };
        }
    };
    let pid = child.id() as i32;

    // reader 线程逐行排空两个管道 → channel；主循环 try_wait 轮询期间 drain 回调。
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    if let Some(out) = child.stdout.take() {
        spawn_line_reader(out, tx.clone());
    }
    if let Some(err) = child.stderr.take() {
        spawn_line_reader(err, tx);
    }

    let deadline = std::time::Duration::from_millis((timeout_secs * 1000.0) as u64);
    let start = std::time::Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                while let Ok(line) = rx.try_recv() {
                    on_line(&line);
                }
                if start.elapsed() >= deadline {
                    timed_out = true;
                    unsafe {
                        libc::kill(-pid, libc::SIGTERM);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    unsafe {
                        libc::kill(-pid, libc::SIGKILL);
                    }
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => break None,
        }
    };

    // 子进程已退出（或被回收）：等 reader 线程写完最后几行，channel 随 sender drop 关闭。
    for line in rx {
        on_line(&line);
    }

    match status {
        Some(st) if st.success() => BrewUpgradeOutcome {
            ok: true,
            exit_code: st.code(),
            error: None,
        },
        Some(st) => BrewUpgradeOutcome {
            ok: false,
            exit_code: st.code(),
            error: Some(format!("brew exit {}", st)),
        },
        None if timed_out => BrewUpgradeOutcome {
            ok: false,
            exit_code: None,
            error: Some("升级超时".to_string()),
        },
        None => BrewUpgradeOutcome {
            ok: false,
            exit_code: None,
            error: Some("brew 进程异常退出".to_string()),
        },
    }
}

fn spawn_line_reader<R: std::io::Read + Send + 'static>(
    pipe: R,
    tx: std::sync::mpsc::Sender<String>,
) {
    std::thread::spawn(move || {
        let reader = BufReader::new(pipe);
        for line in reader.lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
}

/// 进度短语提取：`==> ` 前缀行 → 进度短语，其余为噪声。
pub fn brew_progress_phrase(line: &str) -> Option<String> {
    let t = line.trim();
    let rest = t.strip_prefix("==> ")?;
    let phrase = rest.trim();
    if phrase.is_empty() {
        None
    } else {
        Some(phrase.to_string())
    }
}

/// 纯解析 `brew outdated --json=v2`：
/// formulae → kind "formula"，casks → kind "cask"；
/// installed 取 `installed_versions` **第一项**（缺省 "?"），latest 取 `current_version`（缺省 "?"）。
pub fn parse_outdated(json: &str) -> Vec<BrewOutdatedItem> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    add_outdated(&root, "formulae", "formula", &mut out);
    add_outdated(&root, "casks", "cask", &mut out);
    out
}

fn add_outdated(root: &serde_json::Value, key: &str, kind: &str, out: &mut Vec<BrewOutdatedItem>) {
    let Some(arr) = root.get(key).and_then(|x| x.as_array()) else {
        return;
    };
    for d in arr {
        let Some(name) = d.get("name").and_then(|x| x.as_str()) else {
            continue;
        };
        let installed = d
            .get("installed_versions")
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .and_then(|x| x.as_str())
            .unwrap_or("?")
            .to_string();
        let latest = d
            .get("current_version")
            .and_then(|x| x.as_str())
            .unwrap_or("?")
            .to_string();
        out.push(BrewOutdatedItem {
            name: name.to_string(),
            installed,
            latest,
            kind: kind.to_string(),
        });
    }
}
