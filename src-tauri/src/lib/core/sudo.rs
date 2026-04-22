//! macOS 管理员会话 — 统一入口。
//!
//! 公共 API：
//! - `ensure_admin_session()`  请求管理员权限（首次弹原生认证面板，后续直接返回 true）
//! - `ensure_admin_session_detailed()` 同上，但返回 `AdminAuthResult` 区分用户取消/失败
//! - `is_admin_authorized()`    只查询不弹窗
//! - `revoke_admin_session()`   释放权限
//! - `sudo_output(args)`        代替 `Command::new("sudo")`，自动路由到原生执行或普通 sudo
//!
//! 安全加固（对齐 Burrow trustedExecutable）：
//! - AEWP root 执行只允许 `TRUSTED_ROOT_BINARIES` 白名单内的绝对路径二进制
//! - GUI（无 TTY）场景禁止回退裸 sudo，避免密码提示打到控制台

use std::io::{IsTerminal, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

// ── Security.framework FFI：原生认证面板 ──────────────────────────────
mod auth_ffi {
    #![allow(non_camel_case_types)]

    pub type AuthorizationRef = *mut std::ffi::c_void;
    pub type OSStatus = i32;

    pub const ERR_SUCCESS: OSStatus = 0;
    pub const ERR_CANCELED: OSStatus = -60005; // errAuthorizationCanceled

    pub const FLAG_DEFAULTS: u32 = 0;
    pub const FLAG_INTERACTION_ALLOWED: u32 = 1; // kAuthorizationFlagInteractionAllowed
    pub const FLAG_EXTEND_RIGHTS: u32 = 1 << 1; // kAuthorizationFlagExtendRights

    #[repr(C)]
    pub struct AuthorizationItem {
        pub name: *const std::ffi::c_char,
        pub value_length: usize,
        pub value: *mut std::ffi::c_void,
        pub flags: u32,
    }

    #[repr(C)]
    pub struct AuthorizationRights {
        pub count: u32,
        pub items: *mut AuthorizationItem,
    }

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        pub fn AuthorizationCreate(
            rights: *const AuthorizationRights,
            environment: *const AuthorizationItem,
            flags: u32,
            authorization: *mut AuthorizationRef,
        ) -> OSStatus;

        pub fn AuthorizationCopyRights(
            authorization: AuthorizationRef,
            rights: *const AuthorizationRights,
            environment: *const AuthorizationItem,
            flags: u32,
            authorized_rights: *mut *mut AuthorizationRights,
        ) -> OSStatus;

        pub fn AuthorizationFree(authorization: AuthorizationRef, flags: u32) -> OSStatus;

        pub fn AuthorizationExecuteWithPrivileges(
            authorization: AuthorizationRef,
            path_to_tool: *const std::ffi::c_char,
            options: u32,
            arguments: *const *const std::ffi::c_char,
            communications_pipe: *mut *mut libc::FILE,
        ) -> OSStatus;
    }
}

use auth_ffi as auth;

/// 全局 auth ref：原生认证面板通过后保留，不释放。
/// 所有后续 sudo_output() 调用直接用 AuthorizationExecuteWithPrivileges 执行 root 命令。
struct AuthRef(*mut std::ffi::c_void);
unsafe impl Send for AuthRef {}
unsafe impl Sync for AuthRef {}

static MOLE_AUTH_REF: OnceLock<Mutex<Option<AuthRef>>> = OnceLock::new();
static MOLE_SUDO_KEEPALIVE_PID: OnceLock<Mutex<Option<u32>>> = OnceLock::new();
static MOLE_SUDO_CLEANUP_REGISTERED: AtomicBool = AtomicBool::new(false);

/// 可信 root 二进制白名单（对齐 Burrow `trustedExecutable`）。
/// AEWP 只允许这些硬编码绝对路径被 root 执行；args[0] 命中白名单之外的
/// 路径（相对名 / 用户可控路径）一律拒绝，防止上游拼接漏洞劫持 root。
/// 新增调用方时把绝对路径加入此表，禁止传裸命令名。
const TRUSTED_ROOT_BINARIES: &[&str] = &[
    "/bin/bash",
    "/bin/launchctl",
    "/bin/rm",
    "/bin/sh",
    "/bin/test",
    "/sbin/route",
    "/usr/bin/du",
    "/usr/bin/dscacheutil",
    "/usr/bin/find",
    "/usr/bin/killall",
    "/usr/bin/mdutil",
    "/usr/bin/pkill",
    "/usr/bin/sfltool",
    "/usr/bin/stat",
    "/usr/bin/true",
    "/usr/local/bin/trash",
    "/opt/homebrew/bin/trash",
    "/usr/sbin/arp",
    "/usr/sbin/periodic",
];

fn is_trusted_root_binary(path: &str) -> bool {
    TRUSTED_ROOT_BINARIES.contains(&path)
}

/// 管理员授权结果三态（对齐 Burrow AuthCancel 分类）。
/// `UserCanceled` 与 `Failed` 分开，上层可展示不同文案。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminAuthResult {
    Authorized,
    UserCanceled,
    Failed,
}

// ============================================================================
// Touch ID and Clamshell Detection
// ============================================================================

pub fn check_touchid_support() -> bool {
    // 优先看 sudo_local(macOS Sonoma+),否则看 sudo
    for file in ["/etc/pam.d/sudo_local", "/etc/pam.d/sudo"] {
        if let Ok(content) = std::fs::read_to_string(file) {
            if content.contains("pam_tid.so") {
                return true;
            }
            // 第一个文件存在就以它为准(对齐 SH 行为:命中即返回)
            if file == "/etc/pam.d/sudo_local" {
                return false;
            }
        }
    }
    false
}

pub fn is_clamshell_mode() -> bool {
    // ioreg 不存在时按 lid open 处理
    if !Command::new("which")
        .arg("ioreg")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return false;
    }
    let out = Command::new("ioreg")
        .args(["-r", "-k", "AppleClamshellState", "-d", "4"])
        .output();
    match out {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            // 严格匹配:对齐 SH `=~ \"AppleClamshellState\"\ =\ Yes`
            // 之前用宽松 contains("Yes") 会被 NotKey: Yes 之类的误命中
            stdout.lines().any(|l| {
                let l = l.trim();
                l.contains("\"AppleClamshellState\" = Yes")
            })
        }
        _ => false,
    }
}

// ============================================================================
// Password / Touch ID prompt
// ============================================================================

fn detect_tty_path() -> Option<String> {
    if let Ok(tty) = Command::new("tty").output() {
        let path = String::from_utf8_lossy(&tty.stdout).trim().to_string();
        if !path.is_empty() && path.starts_with("/dev/") {
            return Some(path);
        }
    }
    if std::path::Path::new("/dev/tty").exists() {
        return Some("/dev/tty".to_string());
    }
    None
}

/// Save current termios so we can restore after a botched sudo prompt.
/// 对齐 SH `stty_orig=$(stty -g < "$tty_path")` + `trap '... stty "$stty_orig"' RETURN`
struct TtyGuard {
    tty: String,
    saved: Option<String>,
}

impl TtyGuard {
    fn new(tty: &str) -> Self {
        let saved = Command::new("sh")
            .args(["-c", &format!("stty -g < {tty} 2>/dev/null")])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                } else {
                    None
                }
            });
        Self {
            tty: tty.to_string(),
            saved,
        }
    }
}

impl Drop for TtyGuard {
    fn drop(&mut self) {
        if let Some(s) = &self.saved {
            if !s.is_empty() {
                let _ = Command::new("sh")
                    .args(["-c", &format!("stty {s} < {} 2>/dev/null", self.tty)])
                    .status();
            }
        }
    }
}

pub fn _request_password(_prompt_msg: &str) -> bool {
    let _ = Command::new("sudo").arg("-k").output();

    if let Some(tty) = detect_tty_path() {
        let _guard = TtyGuard::new(&tty);
        if check_touchid_support() {
            // 提示也走 tty 设备(对齐 SH `> "$tty_path"`)
            let _ = Command::new("sh")
                .args([
                    "-c",
                    &format!(
                        "echo \"\\033[0;90mNote: Touch ID dialog may appear once more, just cancel it\\033[0m\" > {tty}"
                    ),
                ])
                .status();
        }
        let _ = Command::new("sh")
            .args([
                "-c",
                &format!("echo \"\\033[0;35m\u{27a4}\\033[0m Enter your credentials:\" > {tty}"),
            ])
            .status();

        let status = Command::new("sh")
            .args(["-c", &format!("sudo -v < {tty} >/dev/null 2> {tty}")])
            .status();
        return status.map(|s| s.success()).unwrap_or(false);
    }

    let _ = Command::new("sudo").arg("-k").output();
    Command::new("sudo")
        .arg("-v")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn _sudo_with_password(password: &str) -> bool {
    let mut child = match Command::new("sudo")
        .args(["-S", "-p", "", "-v"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(format!("{password}\n").as_bytes());
    }

    child.wait().map(|s| s.success()).unwrap_or(false)
}

/// 通过 Security.framework 弹出原生认证面板。
///
/// 认证成功后 **auth_ref 保留在 MOLE_AUTH_REF 中不释放**。
/// 后续所有 sudo_output() 调用自动用 AuthorizationExecuteWithPrivileges 原生执行，
/// 完全不需要 sudo 票据，终端永不弹 Password。
///
/// 返回三态：`Some(Authorized)` / `Some(UserCanceled)` / `Some(Failed)`；
/// `None` = 无 GUI 环境（AuthorizationCreate 失败），调用方可回退 TTY。
fn request_password_via_authorization_services(prompt_msg: &str) -> Option<AdminAuthResult> {
    let _ = prompt_msg;
    use std::ffi::CString;

    unsafe {
        let mut auth_ref: auth::AuthorizationRef = std::ptr::null_mut();
        let status = auth::AuthorizationCreate(
            std::ptr::null(),
            std::ptr::null(),
            auth::FLAG_DEFAULTS,
            &mut auth_ref,
        );
        if status != auth::ERR_SUCCESS || auth_ref.is_null() {
            log::error!("AuthorizationCreate failed: {status}");
            return None;
        }

        let right_name = match CString::new("system.privilege.admin") {
            Ok(s) => s,
            Err(_) => {
                auth::AuthorizationFree(auth_ref, auth::FLAG_DEFAULTS);
                return Some(AdminAuthResult::Failed);
            }
        };
        let mut item = auth::AuthorizationItem {
            name: right_name.as_ptr(),
            value_length: 0,
            value: std::ptr::null_mut(),
            flags: 0,
        };
        let rights = auth::AuthorizationRights {
            count: 1,
            items: &mut item,
        };

        let status = auth::AuthorizationCopyRights(
            auth_ref,
            &rights,
            std::ptr::null(),
            auth::FLAG_INTERACTION_ALLOWED | auth::FLAG_EXTEND_RIGHTS,
            std::ptr::null_mut(),
        );

        if status != auth::ERR_SUCCESS {
            auth::AuthorizationFree(auth_ref, auth::FLAG_DEFAULTS);
            return if status == auth::ERR_CANCELED {
                log::info!("auth: user canceled");
                Some(AdminAuthResult::UserCanceled)
            } else {
                log::error!("auth: AuthorizationCopyRights failed ({status})");
                Some(AdminAuthResult::Failed)
            };
        }

        // 核心：保留 auth_ref，存到全局。后续 sudo_output() 用它跑 root 命令。
        let cell = MOLE_AUTH_REF.get_or_init(|| Mutex::new(None));
        if let Ok(mut g) = cell.lock() {
            if let Some(old) = g.take() {
                auth::AuthorizationFree(old.0, auth::FLAG_DEFAULTS);
            }
            *g = Some(AuthRef(auth_ref));
        } else {
            auth::AuthorizationFree(auth_ref, auth::FLAG_DEFAULTS);
            return Some(AdminAuthResult::Failed);
        }
    }

    log::info!("auth: native dialog succeeded, auth_ref kept for root execution");
    Some(AdminAuthResult::Authorized)
}

/// 对单个参数做 shell 安全转义：单引号包裹，内部单引号替换为 '\''。
/// 例如 "it's a path" → 'it'\''s a path'
fn shell_escape(arg: &str) -> String {
    let escaped = arg.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

/// AEWP 不返回子进程 pid，无法 waitpid；改用输出协议拿真实退出码：
/// 在 shell 命令尾部追加 `printf` 打印标记+退出码，父进程读完 stdout 后
/// 从尾部解析标记，剥离后还原干净 stdout。对齐 Burrow `do shell script`
/// 会抛真实退出码的语义，避免「spawn 成功即当成功」的退出码失真。
const EXIT_MARK: &str = "__MOLE_ROOT_RC__:";

/// 用保存的 auth_ref 以 root 身份执行命令。
///
/// 方案 A：所有命令统一走 /bin/sh -c，确保 AuthorizationExecuteWithPrivileges
/// 只看到一个工具路径（/bin/sh），避免 macOS TCC 对不同工具路径重复弹窗。
///
/// `Ok(output)` = AEWP 启动成功（exit code 为子进程真实退出码）；
/// `Err(())`    = AEWP 调用本身失败（auth_ref 过期/被吊销等），调用方可重授权后重试。
fn exec_root(args: &[&str]) -> Result<std::process::Output, ()> {
    use std::ffi::CString;

    let empty = std::process::Output {
        status: std::process::ExitStatus::from_raw(1),
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    if args.is_empty() {
        return Ok(empty);
    }
    // 可信路径白名单（对齐 Burrow trustedExecutable）：args[0] 必须是硬编码
    // 绝对路径且在白名单内，否则拒绝 root 执行。exec_root 内部拼给 /bin/sh -c
    // 的命令串本身已 shell_escape，这里卡的是「谁被 root 跑」。
    if !is_trusted_root_binary(args[0]) {
        log::error!(
            "exec_root: refused non-whitelisted binary '{}' (see TRUSTED_ROOT_BINARIES)",
            args[0]
        );
        return Ok(std::process::Output {
            status: std::process::ExitStatus::from_raw(1),
            stdout: Vec::new(),
            stderr: format!("exec_root: untrusted binary '{}'", args[0]).into_bytes(),
        });
    }
    let cell = match MOLE_AUTH_REF.get() {
        Some(c) => c,
        None => return Err(()),
    };
    let auth_ref = match cell.lock() {
        Ok(g) => match g.as_ref() {
            Some(r) => r.0,
            None => return Err(()),
        },
        Err(_) => return Err(()),
    };

    // 将所有参数 shell-escape 后拼接为一条命令，交给 /bin/sh -c 执行。
    // 这样 AEWP 始终只看到 /bin/sh 一个工具路径，不会触发多次 TCC 弹窗。
    let shell_cmd = args
        .iter()
        .map(|a| shell_escape(a))
        .collect::<Vec<_>>()
        .join(" ");
    // 尾部追加退出码协议：$? 是上一条命令（真实任务）的退出码。
    let shell_cmd = format!("{shell_cmd}; __mole_rc=$?; printf '\\n{EXIT_MARK}%d' \"$__mole_rc\"");

    let c_path = match CString::new("/bin/sh") {
        Ok(s) => s,
        Err(_) => return Ok(empty),
    };
    // -p（privileged）必传：AEWP 经 security_authtrampoline 只提权 euid（real uid
    // 仍是登录用户），而 /bin/sh（bash）在 euid≠ruid 且无 -p 时会把 euid 重置回
    // real uid（man bash），sh 内所有命令降为普通用户身份运行——killall -HUP
    // mDNSResponder 即因权限不足报 "No matching processes belonging to you"。
    // -p 保留 euid=0，命令才真正以 root 执行。
    let c_flag_p = match CString::new("-p") {
        Ok(s) => s,
        Err(_) => return Ok(empty),
    };
    let c_flag = match CString::new("-c") {
        Ok(s) => s,
        Err(_) => return Ok(empty),
    };
    let c_cmd = match CString::new(shell_cmd.as_str()) {
        Ok(s) => s,
        Err(_) => return Ok(empty),
    };
    // AEWP 的 arguments 数组是子进程的 argv[1..]：工具路径由 pathToTool 参数
    // 单独传入并由系统作为 argv[0]。曾误把 "/bin/sh" 也放进数组，子进程实际收到
    // ["/bin/sh", "/bin/sh", "-c", cmd]，sh 把第二个 "/bin/sh" 当脚本文件执行，
    // 报 "cannot execute binary file" —— 所有经 AEWP 的 root 命令静默失败。
    let c_ptrs: [*const std::ffi::c_char; 4] = [
        c_flag_p.as_ptr(),
        c_flag.as_ptr(),
        c_cmd.as_ptr(),
        std::ptr::null(),
    ];

    unsafe {
        let mut pipe: *mut libc::FILE = std::ptr::null_mut();
        let status = auth::AuthorizationExecuteWithPrivileges(
            auth_ref,
            c_path.as_ptr(),
            auth::FLAG_DEFAULTS,
            c_ptrs.as_ptr(),
            &mut pipe,
        );
        if status != auth::ERR_SUCCESS {
            // AEWP 启动失败（常见于 auth_ref 过期）→ 交给调用方重授权重试
            log::warn!("exec_root: AEWP failed with status {status}");
            return Err(());
        }
        let mut stdout = Vec::new();
        if !pipe.is_null() {
            let mut buf = [0u8; 8192];
            loop {
                let n = libc::fread(buf.as_mut_ptr() as *mut _, 1, buf.len(), pipe);
                if n == 0 {
                    break;
                }
                stdout.extend_from_slice(&buf[..n]);
            }
            libc::fclose(pipe);
        }
        // 从尾部解析真实退出码并剥离协议标记（只看末尾窗口，避免误匹配正文）
        let (stdout, exit_code) = parse_exit_mark(stdout);
        Ok(std::process::Output {
            status: std::process::ExitStatus::from_raw(exit_code),
            stdout,
            stderr: Vec::new(),
        })
    }
}

/// 从 stdout 尾部解析 `\n__MOLE_ROOT_RC__:<code>` 协议：
/// 命中 → 返回（剥离标记后的 stdout，真实退出码）；未命中 → 保守返回 1。
fn parse_exit_mark(mut stdout: Vec<u8>) -> (Vec<u8>, i32) {
    let mark = format!("\n{EXIT_MARK}");
    let tail_start = stdout.len().saturating_sub(64);
    if let Ok(tail) = std::str::from_utf8(&stdout[tail_start..]) {
        if let Some(pos) = tail.rfind(mark.as_str()) {
            let code_part = &tail[pos + mark.len()..];
            if let Some(code) = code_part.trim().split_whitespace().next() {
                if let Ok(rc) = code.parse::<i32>() {
                    stdout.truncate(tail_start + pos);
                    return (stdout, rc);
                }
            }
        }
    }
    (stdout, 1)
}

/// 代替 `Command::new("sudo")` 的统一入口。
///
/// - auth_ref 有效时 → 用 AuthorizationExecuteWithPrivileges 原生执行（不依赖 sudo 票据）
/// - 否则且 stdin 是 TTY → 回退普通 `sudo`（CLI 场景）
/// - 否则（GUI/无 TTY）→ 直接返回失败，**绝不**让 sudo 把密码提示打到控制台
pub fn sudo_output(args: &[&str]) -> std::process::Output {
    if MOLE_AUTH_REF
        .get()
        .and_then(|c| c.lock().ok().map(|g| g.is_some()))
        .unwrap_or(false)
    {
        match exec_root(args) {
            Ok(out) => return out,
            Err(()) => {
                // AEWP 启动失败：最常见原因是 system.privilege.admin 默认
                // 5 分钟超时后凭证过期。自动重弹一次原生认证面板并重试，
                // 避免「开着 app 一段时间后 root 操作静默失败」的体验断裂。
                // 先释放失效的 auth_ref，否则 ensure 会误判已授权直接返回；
                // 只重试一次，防止循环弹窗。
                log::info!("sudo_output: AEWP failed, re-authorizing once");
                revoke_admin_session();
                if ensure_admin_session() {
                    if let Ok(out) = exec_root(args) {
                        return out;
                    }
                }
                return std::process::Output {
                    status: std::process::ExitStatus::from_raw(1),
                    stdout: Vec::new(),
                    stderr: b"sudo_output: AEWP failed and re-authorization did not recover"
                        .to_vec(),
                };
            }
        }
    }
    if !std::io::stdin().is_terminal() {
        // GUI 场景：未授权时静默失败，由调用方降级/跳过；
        // 杜绝 dev 终端里出现意外的命令行密码输入。
        return std::process::Output {
            status: std::process::ExitStatus::from_raw(1),
            stdout: Vec::new(),
            stderr: b"sudo_output: no admin session and no TTY (GUI)".to_vec(),
        };
    }
    let mut cmd = Command::new("sudo");
    for arg in args {
        cmd.arg(arg);
    }
    cmd.output().unwrap_or_else(|e| std::process::Output {
        status: std::process::ExitStatus::from_raw(1),
        stdout: Vec::new(),
        stderr: e.to_string().into_bytes(),
    })
}

/// 旧版 osascript 密码对话框（回退路径）。
/// 当 AuthorizationCopyRights 的 sudo 票据建立失败时使用。
fn _request_password_via_osascript_fallback(prompt_msg: &str) -> bool {
    let _ = Command::new("sudo").arg("-k").output();

    let script = format!(
        "display dialog \"{prompt_msg}\" default answer \"\" with title \"MoleStudio\" with icon caution with hidden answer"
    );
    let password = Command::new("osascript")
        .args(["-e", &script, "-e", "text returned of result"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    match password {
        Some(pwd) => _sudo_with_password(&pwd),
        None => false,
    }
}

fn request_password_via_osascript(prompt_msg: &str) -> Option<AdminAuthResult> {
    request_password_via_authorization_services(prompt_msg)
}

pub fn request_sudo_access_with_prompt(prompt_msg: &str) -> bool {
    request_sudo_access_with_prompt_detailed(prompt_msg) == AdminAuthResult::Authorized
}

/// 三态版授权入口：区分用户取消与失败，供上层展示不同文案。
pub fn request_sudo_access_with_prompt_detailed(prompt_msg: &str) -> AdminAuthResult {
    if is_admin_authorized() {
        return AdminAuthResult::Authorized;
    }

    if std::env::var("MOLE_TEST_MODE").unwrap_or_default() == "1"
        || std::env::var("MOLE_TEST_NO_AUTH").unwrap_or_default() == "1"
    {
        return AdminAuthResult::Failed;
    }

    // GUI 应用：优先用 Security.framework 弹出原生认证面板
    // （显示 "MoleStudio 想要进行更改" + 用户名 + 密码框）。
    // 授权成功后利用系统缓存建立 sudo 票据，无需二次弹窗。
    match request_password_via_osascript(prompt_msg) {
        Some(r) => return r,
        // 无 GUI 环境（AuthorizationCreate 失败）→ 回退到终端
        None => {}
    }

    let tty = detect_tty_path();
    if tty.is_some() {
        let _ = Command::new("sudo").arg("-k").output();
        eprintln!("\x1b[0;35m\u{27a4}\x1b[0m {prompt_msg}");
        return if _request_password(prompt_msg) {
            AdminAuthResult::Authorized
        } else {
            AdminAuthResult::Failed
        };
    }

    // 没有任何可用的输入方式
    AdminAuthResult::Failed
}

pub fn request_sudo_access() -> bool {
    request_sudo_access_with_prompt("Admin access required")
}

// ============================================================================
// Internal helpers — keepalive
// ============================================================================

fn _start_sudo_keepalive() {
    let parent_pid = std::process::id();
    let script = format!(
        r#"sleep 2
retry_count=0
while true; do
    if ! sudo -n -v 2>/dev/null; then
        retry_count=$((retry_count + 1))
        if [ $retry_count -ge 3 ]; then
            exit 1
        fi
        sleep 5
        continue
    fi
    retry_count=0
    sleep 30
    kill -0 {parent_pid} 2>/dev/null || exit 0
done"#
    );
    if let Ok(mut child) = Command::new("bash")
        .arg("-lc")
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        let pid = child.id();
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        let cell = MOLE_SUDO_KEEPALIVE_PID.get_or_init(|| Mutex::new(None));
        if let Ok(mut g) = cell.lock() {
            *g = Some(pid);
        }
    }
}

fn _stop_sudo_keepalive() {
    let cell = MOLE_SUDO_KEEPALIVE_PID.get_or_init(|| Mutex::new(None));
    let pid = cell.lock().ok().and_then(|g| *g);
    if let Some(pid) = pid {
        let _ = Command::new("kill").arg(pid.to_string()).output();
        if let Ok(mut g) = cell.lock() {
            *g = None;
        }
    }
}

// ============================================================================
// Public API — 管理员会话统一入口
// ============================================================================

/// 检查是否已授权（只查询 MOLE_AUTH_REF，不弹窗）。
pub fn is_admin_authorized() -> bool {
    MOLE_AUTH_REF
        .get()
        .and_then(|cell| cell.lock().ok().map(|g| g.is_some()))
        .unwrap_or(false)
}

/// 确保管理员权限。
///
/// - 首次调用 → 弹原生认证面板 "MoleStudio 想要进行更改"
/// - 后续调用 → 检查 MOLE_AUTH_REF，已有权限直接返回 true
/// - 返回 false = 用户取消或认证失败
///
/// 调用方只需这一行，不用再关心 has_sudo / ensure_sudo / prompt 等细节。
pub fn ensure_admin_session() -> bool {
    ensure_admin_session_detailed() == AdminAuthResult::Authorized
}

/// 三态版：上层需要区分「用户主动取消」和「认证失败」时使用
/// （对齐 Burrow AuthCancel：取消 → 静默/温和提示；失败 → 错误文案）。
pub fn ensure_admin_session_detailed() -> AdminAuthResult {
    if is_admin_authorized() {
        return AdminAuthResult::Authorized;
    }
    _stop_sudo_keepalive();
    let r = request_sudo_access_with_prompt_detailed("Admin access required");
    if r != AdminAuthResult::Authorized {
        return r;
    }
    let _ = _start_sudo_keepalive();
    AdminAuthResult::Authorized
}

/// 释放管理员权限。
pub fn revoke_admin_session() -> bool {
    let _ = _stop_sudo_keepalive();
    if let Some(cell) = MOLE_AUTH_REF.get() {
        if let Ok(mut g) = cell.lock() {
            if let Some(old) = g.take() {
                unsafe {
                    auth::AuthorizationFree(old.0, auth::FLAG_DEFAULTS);
                }
            }
        }
    }
    true
}

// ── 向后兼容别名（逐步替换后清除） ──────────────────────────────────

#[deprecated = "使用 is_admin_authorized()"]
pub fn has_sudo_session() -> bool {
    is_admin_authorized()
}

#[deprecated = "使用 ensure_admin_session()"]
pub fn request_sudo() -> bool {
    request_sudo_access()
}

#[deprecated = "使用 ensure_admin_session()"]
pub fn ensure_sudo_session() -> bool {
    ensure_admin_session()
}

#[deprecated = "使用 ensure_admin_session()"]
pub fn ensure_sudo_session_with_prompt(_prompt_msg: &str) -> bool {
    ensure_admin_session()
}

#[deprecated = "使用 revoke_admin_session()"]
pub fn stop_sudo_session() -> bool {
    _stop_sudo_keepalive();
    if let Some(cell) = MOLE_AUTH_REF.get() {
        if let Ok(mut g) = cell.lock() {
            if let Some(old) = g.take() {
                unsafe {
                    auth::AuthorizationFree(old.0, auth::FLAG_DEFAULTS);
                }
            }
        }
    }
    true
}

/// 注册 SIGINT/SIGTERM 处理钩子,在 GUI 进程被强杀前停掉 keepalive 并清掉 sudo 凭据。
/// 对齐 SH `trap stop_sudo_session EXIT INT TERM`。
/// 这个函数是幂等的(只注册一次),且在错误时不 panic,因为 sudo 不是 GUI 启动的关键路径。
pub fn register_sudo_cleanup() -> bool {
    if MOLE_SUDO_CLEANUP_REGISTERED.swap(true, Ordering::SeqCst) {
        return true;
    }

    use signal_hook::consts::signal::*;
    use signal_hook::iterator::Signals;

    let signals = match Signals::new([SIGINT, SIGTERM, SIGHUP]) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let mut handle = signals;
    std::thread::spawn(move || {
        for sig in handle.forever() {
            let _ = revoke_admin_session();
            // 收到信号后退出进程,保留信号语义(用 128+sig 退出码)
            std::process::exit(128 + sig);
        }
    });
    true
}

// ============================================================================
// will_need_sudo:对齐 SH 第 316-326 行
// ============================================================================

/// 给定一组 operation 名称,判断是否需要管理员权限。
/// 与 SH 一致,任意一个匹配即返回 true。GUI 端在执行清理 / 卸载 batch
/// 之前应该用这个先决定要不要预先弹出授权对话框。
pub fn will_need_sudo(operations: &[&str]) -> bool {
    operations.iter().any(|op| {
        matches!(
            *op,
            "system_update"
                | "appstore_update"
                | "macos_update"
                | "firewall"
                | "touchid"
                | "rosetta"
                | "system_fix"
        )
    })
}
