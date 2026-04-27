use std::fs;
use std::path::Path;
use std::process::Command;

use super::app_protection::{get_global_whitelist, is_path_whitelisted, should_protect_path};
use super::base::{bytes_human_from_kb, get_epoch_seconds, get_path_size_kb, home_dir, run_cmd};
use super::dry_run_registry::dry_run_register_cleanup_target;
use super::log::{debug_file_action, debug_log, log_error, log_operation, oplog_enabled};
use super::sudo::sudo_output;

pub const MOLE_ERR_SIP_PROTECTED: i32 = 10;
pub const MOLE_ERR_AUTH_FAILED: i32 = 11;
pub const MOLE_ERR_READONLY_FS: i32 = 12;
pub const MOLE_ERR_MUTABLE_PARENT: i32 = 15;

/// 通用退出码:0 成功,1 普通失败,10/11/12/15 见上面常量,>=128 信号中断
/// 与 `safe_remove`/`safe_sudo_remove`/`safe_remove_symlink` 的返回值对齐
pub const MOLE_OK: i32 = 0;
pub const MOLE_ERR_GENERIC: i32 = 1;

/// 对齐 file_ops.sh 第 1269-1278 行 `_batch_selected_app_identity` 的 stat 部分:
/// `stat -f%d:%i:%m path` → `dev:ino:mode`(mode 为八进制权限位字符串,如 755)。
/// 返回 None 表示 stat 失败(此时调用方应 fail-closed)。
pub fn stat_path_identity(path: &str) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let md = std::fs::symlink_metadata(path).ok()?;
    Some(format!(
        "{}:{}:{:o}",
        md.dev(),
        md.ino(),
        md.mode() & 0o7777
    ))
}

/// 对齐 file_ops.sh 第 1163-1215 行 `_mole_privileged_path_has_mutable_ancestor`。
///
/// 特权删除(needs_sudo)的路径若存在"调用用户可写的祖先目录",则预览时绑定的
/// 路径无法绑定到 root 最终删除的对象(TOCTOU):用户在确认后替换目录内容,
/// sudo rm/mv 就会删错东西。逐级检查:符号链接、非 root 属主、022 写位、
/// ACL 写权限、不可读元数据,任一命中即判定可变(fail-closed)。
///
/// 返回 true = 存在可变祖先(调用方应拒绝删除)。
pub fn _mole_privileged_path_has_mutable_ancestor(path: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;

    // probe = dirname(path);SH 端 `${path%/*}` 为空时兜底 "/"
    let mut probe = Path::new(path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("/"));
    let invoking_uid = unsafe { libc::getuid() };
    let euid = unsafe { libc::geteuid() };

    loop {
        let md = match std::fs::symlink_metadata(&probe) {
            Ok(m) => m,
            // 元数据不可读 → 无法证明安全,失败闭合
            Err(_) => return true,
        };
        if md.file_type().is_symlink() {
            return true;
        }
        // owner 非 root 或 group/other 有写位 → 可变
        if md.uid() != 0 || (md.mode() & 0o022) != 0 {
            return true;
        }
        if invoking_uid != 0 {
            if euid == invoking_uid {
                // GUI 常态:进程即调用用户,直接 access W_OK
                use std::ffi::CString;
                let c = match CString::new(probe.to_string_lossy().as_bytes()) {
                    Ok(c) => c,
                    Err(_) => return true,
                };
                if unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0 {
                    return true;
                }
            } else if euid == 0 {
                // `sudo mo` 形态:降权到调用用户探测 ACL 写权限。
                // rc==0 可写 → 可变;rc==1 确证不可写 → 继续;其它(超时/鉴权失败)
                // 一律视为未知 → fail-closed 可变(对齐 SH 第 1190-1204 行)。
                let out = Command::new("sudo")
                    .args(["-n", "-u", &format!("#{invoking_uid}"), "/bin/test", "-w"])
                    .arg(&probe)
                    .stdin(std::process::Stdio::null())
                    .output();
                match out.map(|o| o.status.code()) {
                    Ok(Some(0)) => return true,
                    Ok(Some(1)) => {}
                    _ => return true,
                }
            } else {
                // 调用用户与进程身份不符且非 root → 无法证明安全
                return true;
            }
        }

        if probe == Path::new("/") {
            break;
        }
        probe = probe
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("/"));
    }
    false
}

pub fn format_duration_human(seconds: u64) -> String {
    let days = seconds / 86400;
    match days {
        0 => "today".to_string(),
        1 => "1 day".to_string(),
        2..=6 => format!("{days} days"),
        7..=29 => {
            let weeks = days / 7;
            if weeks == 1 {
                "1 week".to_string()
            } else {
                format!("{weeks} weeks")
            }
        }
        30..=364 => {
            let months = days / 30;
            if months == 1 {
                "1 month".to_string()
            } else {
                format!("{months} months")
            }
        }
        _ => {
            let years = days / 365;
            if years == 1 {
                "1 year".to_string()
            } else {
                format!("{years} years")
            }
        }
    }
}

pub fn validate_path_for_deletion(path: &str) -> bool {
    if path.is_empty() {
        log_error("Path validation failed: empty path");
        return false;
    }

    // 1. 如果是符号链接,先解析目标并阻止指向系统目录的链接
    if Path::new(path).is_symlink() {
        match std::fs::read_link(path) {
            Ok(target) => {
                let resolved = resolve_symlink_target(path, &target);
                if let Some(resolved) = resolved {
                    let protected = [
                        "/",
                        "/System",
                        "/bin",
                        "/sbin",
                        "/usr",
                        "/usr/bin",
                        "/usr/lib",
                        "/etc",
                        "/private/etc",
                        "/Library/Extensions",
                    ];
                    for p in &protected {
                        if resolved == *p || resolved.starts_with(&format!("{p}/")) {
                            log_error(&format!(
                                "Symlink points to protected system path: {path} -> {resolved}"
                            ));
                            return false;
                        }
                    }
                }
            }
            Err(_) => {
                log_error(&format!("Cannot read symlink: {path}"));
                return false;
            }
        }
    }

    // 2. 必须是绝对路径
    if !path.starts_with('/') {
        log_error(&format!(
            "Path validation failed: path must be absolute: {path}"
        ));
        return false;
    }

    // 3. 阻止 .. 路径穿越(必须是完整路径分量,允许 "name..files" 这种合法目录名)
    let parts: Vec<&str> = path.split('/').collect();
    if parts.iter().any(|p| *p == "..") {
        log_error(&format!(
            "Path validation failed: path traversal not allowed: {path}"
        ));
        return false;
    }

    // 4. 阻止控制字符
    if path.contains('\n') || path.chars().any(|c| c.is_control()) {
        log_error(&format!(
            "Path validation failed: contains control characters: {path}"
        ));
        return false;
    }

    // 5. 已知白名单:coresymbolicationd cache 可清理
    if path == "/System/Library/Caches/com.apple.coresymbolicationd/data"
        || path.starts_with("/System/Library/Caches/com.apple.coresymbolicationd/data/")
    {
        return true;
    }

    // 6. 已知白名单:/private 下的安全子目录
    let safe_private = [
        "/private/tmp",
        "/private/var/tmp",
        "/private/var/log",
        "/private/var/folders",
        "/private/var/db/diagnostics",
        "/private/var/db/DiagnosticPipeline",
        "/private/var/db/powerlog",
        "/private/var/db/reportmemoryexception",
    ];
    for sp in &safe_private {
        if path == *sp || path.starts_with(&format!("{sp}/")) {
            return true;
        }
    }
    if path.starts_with("/private/var/db/receipts/") {
        let name = Path::new(path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if name.ends_with(".bom") || name.ends_with(".plist") {
            return true;
        }
    }

    // 7. 关键系统目录黑名单
    let denied = [
        "/",
        "/bin",
        "/sbin",
        "/usr",
        "/usr/bin",
        "/usr/sbin",
        "/usr/lib",
        "/System",
        "/Library/Extensions",
        "/private",
        "/etc",
        "/private/etc",
        "/var",
        "/var/db",
        "/private/var",
        "/private/var/db",
    ];
    for d in &denied {
        if path == *d || path.starts_with(&format!("{d}/")) {
            log_error(&format!(
                "Path validation failed: critical system directory: {path}"
            ));
            return false;
        }
    }

    // 8. 接入 should_protect_path:对齐 file_ops.sh 第 168-176 行
    // 这是 SH 端 "if declare -f should_protect_path > /dev/null 2>&1; then" 那段缺失的桥接,
    // 用来把 app_protection 中的所有保护规则透传到所有 safe_remove 入口。
    if should_protect_path(path) {
        if std::env::var("MO_DEBUG").unwrap_or_default() == "1" {
            super::log::log_warning(&format!("Path validation: protected path skipped: {path}"));
        }
        return false;
    }

    true
}

/// 解析符号链接目标到绝对路径。对相对链接会用 parent + target 拼接后规范化掉中间的 `..` 和 `.`。
/// 失败返回 None。
fn resolve_symlink_target(link_path: &str, target: &std::path::Path) -> Option<String> {
    let candidate: std::path::PathBuf = if target.is_absolute() {
        target.to_path_buf()
    } else {
        let parent = Path::new(link_path).parent()?;
        parent.join(target)
    };

    // 优先用 canonicalize 取真实路径;失败时退回到组件级规范化
    if let Ok(canon) = candidate.canonicalize() {
        return Some(canon.to_string_lossy().to_string());
    }

    let mut out: Vec<&std::ffi::OsStr> = Vec::new();
    for comp in candidate.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            std::path::Component::RootDir => {
                out.clear();
                out.push(comp.as_os_str());
            }
            std::path::Component::Prefix(_) => {
                out.push(comp.as_os_str());
            }
            std::path::Component::Normal(s) => {
                out.push(s);
            }
        }
    }
    let mut acc = String::new();
    for (i, c) in out.iter().enumerate() {
        if i == 0 && c.to_string_lossy() == "/" {
            acc.push('/');
            continue;
        }
        if !acc.ends_with('/') && !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(&c.to_string_lossy());
    }
    if acc.is_empty() {
        return None;
    }
    Some(acc)
}

/// 全局 permission-denied 计数器:对齐 SH 中 MOLE_PERMISSION_DENIED_COUNT,
/// GUI 端可在结束时读取并提示用户开启 Full Disk Access。
static PERMISSION_DENIED_COUNT: std::sync::OnceLock<std::sync::Mutex<u32>> =
    std::sync::OnceLock::new();

pub fn permission_denied_count() -> u32 {
    let cell = PERMISSION_DENIED_COUNT.get_or_init(|| std::sync::Mutex::new(0));
    cell.lock().map(|g| *g).unwrap_or(0)
}

pub fn reset_permission_denied_count() {
    let cell = PERMISSION_DENIED_COUNT.get_or_init(|| std::sync::Mutex::new(0));
    if let Ok(mut g) = cell.lock() {
        *g = 0;
    }
}

fn inc_permission_denied_count() {
    let cell = PERMISSION_DENIED_COUNT.get_or_init(|| std::sync::Mutex::new(0));
    if let Ok(mut g) = cell.lock() {
        *g = g.saturating_add(1);
    }
}

/// 安全删除,返回退出码:
///  - 0  成功
///  - 1  普通失败
///  - >=128 信号中断(目前 Rust 实现中较少见,主要靠 sudo rm 路径触发)
///
/// `precomputed_size_kb`:对齐 SH 第 3 参数,调用方已经测过体积时可传入,避免重复 du
pub fn safe_remove_ex(path: &str, silent: bool, precomputed_size_kb: Option<u64>) -> i32 {
    if !validate_path_for_deletion(path) {
        return MOLE_ERR_GENERIC;
    }
    if !Path::new(path).exists() && !Path::new(path).is_symlink() {
        return MOLE_OK;
    }

    if std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1" {
        if std::env::var("MO_DEBUG").unwrap_or_default() == "1" {
            let mut file_size = String::new();
            let mut file_age = String::new();
            if Path::new(path).exists() {
                let size_kb = precomputed_size_kb.unwrap_or_else(|| get_path_size_kb(path));
                if size_kb > 0 {
                    file_size = bytes_human_from_kb(size_kb);
                }
                if !Path::new(path).is_symlink()
                    && (Path::new(path).is_file() || Path::new(path).is_dir())
                {
                    let mtime = super::base::get_file_mtime(path);
                    let now = get_epoch_seconds();
                    if mtime > 0 && now > 0 {
                        file_age = format_duration_human(now.saturating_sub(mtime));
                    }
                }
            }
            debug_file_action(
                "[DRY RUN] Would remove",
                path,
                if file_size.is_empty() {
                    None
                } else {
                    Some(&file_size)
                },
                if file_age.is_empty() {
                    None
                } else {
                    Some(&file_age)
                },
            );
        } else {
            debug_log(&format!("[DRY RUN] Would remove: {path}"));
        }
        return MOLE_OK;
    }

    debug_log(&format!("Removing: {path}"));

    // oplog 启用时才量体积,避免无谓 du(对齐 SH oplog_enabled 守卫)
    let size_kb = if oplog_enabled() {
        precomputed_size_kb.unwrap_or_else(|| get_path_size_kb(path))
    } else {
        0
    };
    let size_human = if size_kb > 0 {
        bytes_human_from_kb(size_kb)
    } else {
        String::new()
    };

    // 用 `rm -rf` 与 SH 完全对齐:
    //   - 顶层是符号链接到目录时,只断链接而不递归进去
    //   - 错误信息可控,便于检测 SIP/RO/auth 等
    //   - 退出码语义稳定
    let out = Command::new("rm").args(["-rf", path]).output();
    match out {
        Ok(o) if o.status.success() => {
            log_operation(
                &std::env::var("MOLE_CURRENT_COMMAND").unwrap_or_else(|_| "clean".to_string()),
                "REMOVED",
                path,
                if size_human.is_empty() {
                    None
                } else {
                    Some(&size_human)
                },
            );
            MOLE_OK
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let exit_code = o.status.code().unwrap_or(1);

            if stderr.contains("Permission denied") || stderr.contains("Operation not permitted") {
                inc_permission_denied_count();
                debug_log(&format!(
                    "Permission denied: {path}, may need Full Disk Access"
                ));
                log_operation(
                    &std::env::var("MOLE_CURRENT_COMMAND").unwrap_or_else(|_| "clean".to_string()),
                    "FAILED",
                    path,
                    Some("permission denied"),
                );
            } else {
                if !silent {
                    log_error(&format!("Failed to remove: {path}"));
                }
                log_operation(
                    &std::env::var("MOLE_CURRENT_COMMAND").unwrap_or_else(|_| "clean".to_string()),
                    "FAILED",
                    path,
                    Some("error"),
                );
            }

            // 信号中断保留原始退出码,让上层可以正确响应 Ctrl-C
            if exit_code >= 128 {
                exit_code
            } else {
                MOLE_ERR_GENERIC
            }
        }
        Err(_) => {
            // rm 可执行文件不可用或被沙盒拦截 → 兜底使用 std::fs
            let res = if Path::new(path).is_dir() && !Path::new(path).is_symlink() {
                fs::remove_dir_all(path)
            } else {
                fs::remove_file(path)
            };
            match res {
                Ok(()) => {
                    log_operation(
                        &std::env::var("MOLE_CURRENT_COMMAND")
                            .unwrap_or_else(|_| "clean".to_string()),
                        "REMOVED",
                        path,
                        if size_human.is_empty() {
                            None
                        } else {
                            Some(&size_human)
                        },
                    );
                    MOLE_OK
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("Permission denied") || msg.contains("Operation not permitted")
                    {
                        inc_permission_denied_count();
                    }
                    if !silent {
                        log_error(&format!("Failed to remove: {path}"));
                    }
                    log_operation(
                        &std::env::var("MOLE_CURRENT_COMMAND")
                            .unwrap_or_else(|_| "clean".to_string()),
                        "FAILED",
                        path,
                        Some("error"),
                    );
                    MOLE_ERR_GENERIC
                }
            }
        }
    }
}

/// 兼容包装:保留原 bool 接口给老调用方
pub fn safe_remove(path: &str, silent: bool) -> bool {
    safe_remove_ex(path, silent, None) == MOLE_OK
}

/// 删除符号链接,支持 sudo。返回 0 成功 / 1 失败,对齐 file_ops.sh:safe_remove_symlink()
pub fn safe_remove_symlink_ex(path: &str, use_sudo: bool) -> i32 {
    if !Path::new(path).is_symlink() {
        return MOLE_ERR_GENERIC;
    }
    // 注:SH 端 safe_remove_symlink 自身不调 validate_path_for_deletion,
    // 由调用方(mole_delete)负责。这里保留同样语义,以避免误拒一些合法的符号链接清理。
    if std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1" {
        debug_log(&format!("[DRY RUN] Would remove symlink: {path}"));
        return MOLE_OK;
    }

    let result = if use_sudo {
        sudo_output(&["/bin/rm", path]).status.success()
    } else {
        fs::remove_file(path).is_ok()
    };

    if result {
        log_operation(
            &std::env::var("MOLE_CURRENT_COMMAND").unwrap_or_else(|_| "clean".to_string()),
            "REMOVED",
            path,
            Some("symlink"),
        );
        MOLE_OK
    } else {
        log_operation(
            &std::env::var("MOLE_CURRENT_COMMAND").unwrap_or_else(|_| "clean".to_string()),
            "FAILED",
            path,
            Some("symlink removal failed"),
        );
        MOLE_ERR_GENERIC
    }
}

/// 兼容包装
pub fn safe_remove_symlink(path: &str, use_sudo: bool) -> bool {
    safe_remove_symlink_ex(path, use_sudo) == MOLE_OK
}

/// Safe sudo removal with symlink protection
pub fn safe_sudo_remove(path: &str, precomputed_size_kb: Option<u64>) -> i32 {
    if !validate_path_for_deletion(path) {
        if should_protect_path(path) {
            debug_log(&format!("Skipped sudo remove for protected path: {path}"));
        } else {
            log_error(&format!("Path validation failed for sudo remove: {path}"));
        }
        return MOLE_ERR_GENERIC;
    }
    let path_exists = sudo_output(&["/bin/test", "-e", path]).status.success();
    if !path_exists {
        return MOLE_OK;
    }
    if Path::new(path).is_symlink() {
        log_error(&format!("Refusing to sudo remove symlink: {path}"));
        return MOLE_ERR_GENERIC;
    }

    if std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1" {
        if std::env::var("MO_DEBUG").unwrap_or_default() == "1" {
            let mut file_size = String::new();
            let mut file_age = String::new();
            let size_kb = precomputed_size_kb.unwrap_or_else(|| {
                String::from_utf8_lossy(&sudo_output(&["/usr/bin/du", "-skP", path]).stdout)
                    .split_whitespace()
                    .next()
                    .unwrap_or("0")
                    .parse::<u64>()
                    .unwrap_or(0)
            });
            if size_kb > 0 {
                file_size = bytes_human_from_kb(size_kb);
            }
            let mtime =
                String::from_utf8_lossy(&sudo_output(&["/usr/bin/stat", "-f%m", path]).stdout)
                    .split_whitespace()
                    .next()
                    .unwrap_or("0")
                    .parse::<u64>()
                    .unwrap_or(0);
            let now = get_epoch_seconds();
            if mtime > 0 && now > 0 {
                file_age = format_duration_human(now.saturating_sub(mtime));
            }
            debug_file_action(
                "[DRY RUN] Would sudo remove",
                path,
                if file_size.is_empty() {
                    None
                } else {
                    Some(&file_size)
                },
                if file_age.is_empty() {
                    None
                } else {
                    Some(&file_age)
                },
            );
        } else {
            debug_log(&format!("[DRY RUN] Would sudo remove: {path}"));
        }
        return MOLE_OK;
    }

    let size_kb = if oplog_enabled() {
        precomputed_size_kb.unwrap_or_else(|| {
            String::from_utf8_lossy(&sudo_output(&["/usr/bin/du", "-skP", path]).stdout)
                .split_whitespace()
                .next()
                .unwrap_or("0")
                .parse::<u64>()
                .unwrap_or(0)
        })
    } else {
        0
    };
    let size_human = if size_kb > 0 {
        bytes_human_from_kb(size_kb)
    } else {
        String::new()
    };

    let out = sudo_output(&["/bin/rm", "-rf", path]);
    let (ok, output) = (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    );

    if ok {
        log_operation(
            &std::env::var("MOLE_CURRENT_COMMAND").unwrap_or_else(|_| "clean".to_string()),
            "REMOVED",
            path,
            if size_human.is_empty() {
                None
            } else {
                Some(&size_human)
            },
        );
        return MOLE_OK;
    }

    let cmd = std::env::var("MOLE_CURRENT_COMMAND").unwrap_or_else(|_| "clean".to_string());
    if output.contains("Operation not permitted") {
        log_operation(&cmd, "FAILED", path, Some("sip/mdm protected"));
        return MOLE_ERR_SIP_PROTECTED;
    }
    if output.contains("Read-only file system") {
        log_operation(&cmd, "FAILED", path, Some("readonly filesystem"));
        return MOLE_ERR_READONLY_FS;
    }
    if output.contains("Sorry, try again")
        || output.contains("incorrect passphrase")
        || output.contains("incorrect credentials")
    {
        log_operation(&cmd, "FAILED", path, Some("auth failed"));
        return MOLE_ERR_AUTH_FAILED;
    }

    log_error(&format!("Failed to remove, sudo: {path}"));
    log_operation(&cmd, "FAILED", path, Some("sudo error"));
    MOLE_ERR_GENERIC
}

/// Trash fallback 警告标志(对齐 SH _MOLE_TRASH_FALLBACK_WARNED:每会话只警告一次)
static TRASH_FALLBACK_WARNED: std::sync::OnceLock<std::sync::Mutex<bool>> =
    std::sync::OnceLock::new();

fn warn_trash_fallback_once() {
    let cell = TRASH_FALLBACK_WARNED.get_or_init(|| std::sync::Mutex::new(false));
    if let Ok(mut g) = cell.lock() {
        if !*g {
            *g = true;
            eprintln!(
                "Warning: Trash unavailable, removing permanently. Subsequent files this session also bypass Trash."
            );
        }
    }
}

/// 计算 path 的 size。失败时返回 "unknown" 而非 "0",对齐 SH 注释:
/// "size_kb is \"unknown\" when du could not measure ... never silently coerced to 0KB"
fn measure_size_kb(path: &str, needs_sudo: bool) -> String {
    if !Path::new(path).exists() {
        return "unknown".to_string();
    }

    if needs_sudo {
        match run_cmd("sudo", &["du", "-skP", path]) {
            Some(s) => match s
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok())
            {
                Some(v) => v.to_string(),
                None => "unknown".to_string(),
            },
            None => "unknown".to_string(),
        }
    } else {
        // get_path_size_kb 内部对量不到的也返回 0,这里要区分 "测得 0" 和 "测不到"
        // 因此再走一次 du,失败才用 unknown
        match run_cmd("du", &["-skP", path]) {
            Some(s) => match s
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok())
            {
                Some(v) => v.to_string(),
                None => "unknown".to_string(),
            },
            None => "unknown".to_string(),
        }
    }
}

pub fn mole_delete(path: &str, needs_sudo: bool, expected_identity: Option<&str>) -> i32 {
    let mode = std::env::var("MOLE_DELETE_MODE").unwrap_or_else(|_| "permanent".to_string());

    if path.is_empty() {
        return MOLE_ERR_GENERIC;
    }

    if !Path::new(path).exists() && !Path::new(path).is_symlink() {
        return MOLE_OK;
    }

    // 软链接不走 validate_path_for_deletion(SH 同样跳过,以允许清理某些合法软链)
    if !Path::new(path).is_symlink() && !validate_path_for_deletion(path) {
        _mole_delete_log(&mode, "0", "rejected", path);
        return MOLE_ERR_GENERIC;
    }

    // 对齐 SH 第 1557-1566 行:特权删除前拒绝"存在调用用户可写祖先"的路径。
    // sudo rm/mv 收到的路径名可能在验证后被非 root 用户替换,无法绑定预览对象。
    if needs_sudo && _mole_privileged_path_has_mutable_ancestor(path) {
        _mole_delete_log(&mode, "unknown", "mutable-parent", path);
        debug_log(&format!(
            "Refusing privileged delete below mutable parent: {path}"
        ));
        return MOLE_ERR_MUTABLE_PARENT;
    }

    let size_kb = measure_size_kb(path, needs_sudo);

    // 对齐 SH 第 1600-1628 行:expected_identity 非空时重查 dev:ino:mode,
    // 预览与执行之间路径被换成新对象则拒绝删除。
    if let Some(expected) = expected_identity {
        if stat_path_identity(path).as_deref() != Some(expected) {
            _mole_delete_log(&mode, &size_kb, "identity-changed", path);
            debug_log(&format!(
                "Refusing deletion after selected path identity changed: {path}"
            ));
            return MOLE_ERR_GENERIC;
        }
    }

    if std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1" {
        debug_log(&format!("[DRY RUN] Would delete ({mode}): {path}"));
        _mole_delete_log(&mode, &size_kb, "dry-run", path);
        return MOLE_OK;
    }

    if mode == "trash" {
        if _mole_move_to_trash(path, needs_sudo) {
            _mole_delete_log("trash", &size_kb, "ok", path);
            log_operation(
                &std::env::var("MOLE_CURRENT_COMMAND").unwrap_or_else(|_| "uninstall".to_string()),
                "TRASHED",
                path,
                Some(&format!("{size_kb}KB")),
            );
            return MOLE_OK;
        }
        // trash 失败:警告用户(每会话一次)并回退到永久删除
        warn_trash_fallback_once();
        debug_log(&format!(
            "Trash move failed, falling back to permanent delete: {path}"
        ));
    }

    // 把已经测得的体积透传给 safe_*_ex,避免 du 第二次
    let precomputed = size_kb.parse::<u64>().ok();
    let rc = if Path::new(path).is_symlink() {
        // 注意:SH 端 mole_delete 在 trash 失败回退时,symlink 用 safe_remove_symlink "$path" "$needs_sudo"
        // 这里把 use_sudo 透传过去
        safe_remove_symlink_ex(path, needs_sudo)
    } else if needs_sudo {
        safe_sudo_remove(path, precomputed)
    } else {
        // SH 用 silent=true,因为 mole_delete 自己已经在 forensic log 里记录了错误
        safe_remove_ex(path, true, precomputed)
    };

    let status_label = if rc == MOLE_OK {
        if mode == "trash" {
            "trash-fallback-rm"
        } else {
            "ok"
        }
    } else {
        "error"
    };
    _mole_delete_log(&mode, &size_kb, status_label, path);
    rc
}

/// 生成一个不易碰撞的后缀(随机 + epoch),用于测试 trash 目录文件名
fn unique_trash_suffix() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        .hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// 解析 trash CLI 的绝对路径：仅允许 Homebrew 标准安装位置（与
/// `sudo::TRUSTED_ROOT_BINARIES` 白名单一致），防止未知路径被 root 执行。
fn resolve_trash_cli_path() -> Option<String> {
    let out = Command::new("/usr/bin/which").arg("trash").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    matches!(
        p.as_str(),
        "/opt/homebrew/bin/trash" | "/usr/local/bin/trash"
    )
    .then_some(p)
}

pub fn _mole_move_to_trash(path: &str, needs_sudo: bool) -> bool {
    // 测试 hook:bats 测试用的 trash 目录(仅 mv,完全不碰 Finder/osascript)
    if let Ok(test_dir) = std::env::var("MOLE_TEST_TRASH_DIR") {
        if fs::create_dir_all(&test_dir).is_err() {
            return false;
        }
        let dest = format!(
            "{}/{}.{}.{}",
            test_dir,
            Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("item"),
            std::process::id(),
            unique_trash_suffix()
        );
        return fs::rename(path, dest).is_ok();
    }
    if std::env::var("MOLE_TEST_NO_AUTH").unwrap_or_default() == "1" {
        return false;
    }

    // 优先用 trash CLI(Homebrew 安装的),无需 Finder 在跑
    let has_trash_cli = Command::new("trash")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if has_trash_cli {
        let cmd_result: Option<std::process::Output> = if needs_sudo {
            // 统一走 sudo_output 白名单（对齐 Burrow trustedExecutable）：
            // 只允许 Homebrew 标准安装位置的 trash 被 root 执行，
            // 未授权时静默失败并回退下方 osascript / trash crate，
            // 绝不在终端弹 Password。
            resolve_trash_cli_path().map(|bin| sudo_output(&[&bin, path]))
        } else {
            Command::new("trash").arg(path).output().ok()
        };
        if cmd_result.map(|o| o.status.success()).unwrap_or(false) {
            return true;
        }
    }

    // AppleScript fallback:macOS 自带,但对 root-owned 目标会再要权限
    let script = r#"on run argv
    set p to POSIX file (item 1 of argv)
    tell application "Finder"
        delete p
    end tell
end run"#;
    let osa = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .arg("--")
        .arg(path)
        .output();
    if osa.map(|o| o.status.success()).unwrap_or(false) {
        return true;
    }

    // 终极兜底:trash crate
    trash::delete(path).is_ok()
}

/// 批量移动到废纸篓,对齐 file_ops.sh:_mole_move_to_trash_batch()。
/// 真批量调用 trash CLI / 一次 osascript,避免 100 次 fork 导致 GUI 卡顿。
pub fn _mole_move_to_trash_batch(paths: &[String]) -> bool {
    if paths.is_empty() {
        return true;
    }

    // 测试 hook
    if let Ok(test_dir) = std::env::var("MOLE_TEST_TRASH_DIR") {
        if fs::create_dir_all(&test_dir).is_err() {
            return false;
        }
        for p in paths {
            let dest = format!(
                "{}/{}.{}.{}",
                test_dir,
                Path::new(p)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("item"),
                std::process::id(),
                unique_trash_suffix()
            );
            if fs::rename(p, dest).is_err() {
                return false;
            }
        }
        return true;
    }
    if std::env::var("MOLE_TEST_NO_AUTH").unwrap_or_default() == "1" {
        return false;
    }

    // 优先 trash CLI 一次性传所有路径
    let has_trash_cli = Command::new("trash")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if has_trash_cli {
        let mut cmd = Command::new("trash");
        for p in paths {
            cmd.arg(p);
        }
        if cmd.output().map(|o| o.status.success()).unwrap_or(false) {
            return true;
        }
    }

    // AppleScript 一次构建路径列表 + 一次 Finder 调用
    let script = r#"on run argv
    set posixList to {}
    repeat with a in argv
        set end of posixList to POSIX file (a as text)
    end repeat
    tell application "Finder" to delete posixList
end run"#;
    let mut osa = Command::new("osascript");
    osa.arg("-e").arg(script).arg("--");
    for p in paths {
        osa.arg(p);
    }
    if osa.output().map(|o| o.status.success()).unwrap_or(false) {
        return true;
    }

    // 兜底:逐个用 trash crate
    let mut all_ok = true;
    for p in paths {
        if trash::delete(p).is_err() {
            all_ok = false;
        }
    }
    all_ok
}

pub fn _mole_delete_log(mode: &str, size: &str, status: &str, path: &str) {
    let log_file = std::env::var("MOLE_DELETE_LOG")
        .unwrap_or_else(|_| format!("{}/Library/Logs/mole/deletions.log", home_dir()));
    let log_dir = Path::new(&log_file)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| Path::new("/tmp").to_path_buf());
    if fs::create_dir_all(&log_dir).is_err() {
        return;
    }
    let ts = chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%z")
        .to_string();
    let line = format!("{ts}\t{mode}\t{size}\t{status}\t{path}\n");
    if std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(line.as_bytes())
        })
        .is_err()
    {}
}

pub fn diagnose_removal_failure(exit_code: i32, _app_name: &str) -> (String, String) {
    let reason = match exit_code {
        MOLE_ERR_SIP_PROTECTED => "protected by macOS (SIP/MDM)".to_string(),
        MOLE_ERR_AUTH_FAILED => "authentication failed".to_string(),
        MOLE_ERR_READONLY_FS => "filesystem is read-only".to_string(),
        MOLE_ERR_MUTABLE_PARENT => {
            "Mole cannot safely use elevated deletion below a user-writable parent".to_string()
        }
        _ => "permission denied".to_string(),
    };
    let suggestion = match exit_code {
        MOLE_ERR_AUTH_FAILED => {
            "Check your credentials or try 'mole touchid' to enable fingerprint auth".to_string()
        }
        MOLE_ERR_READONLY_FS => "Check if disk needs repair".to_string(),
        MOLE_ERR_MUTABLE_PARENT => {
            "Move the app to Trash in Finder; Mole will leave protected containers and app data untouched".to_string()
        }
        _ => "Try running again or check file ownership".to_string(),
    };
    (reason, suggestion)
}

pub fn _mole_warn_log_broken(reason: &str) {
    eprintln!(
        "Warning: deletions audit log unavailable ({reason}). Forensic trail incomplete this session."
    );
}

/// 对齐 file_ops.sh:safe_find_delete()
/// 重要:迭代每个匹配项时必须同时尊重 should_protect_path 和 is_path_whitelisted,
/// 否则等同于把 #710/#724/#738/#744/#757 这一连串回归 bug 重新引入。
pub fn safe_find_delete(base_dir: &str, pattern: &str, age_days: u32, type_filter: &str) {
    if !Path::new(base_dir).is_dir() || Path::new(base_dir).is_symlink() {
        return;
    }
    if type_filter != "f" && type_filter != "d" {
        return;
    }
    let mut cmd = Command::new("find");
    let mut args: Vec<String> = vec![
        base_dir.to_string(),
        "-maxdepth".to_string(),
        "5".to_string(),
    ];
    if pattern != "*" {
        args.push("-name".to_string());
        args.push(pattern.to_string());
    }
    args.push("-type".to_string());
    args.push(type_filter.to_string());
    if age_days > 0 {
        args.push("-mtime".to_string());
        args.push(format!("+{age_days}"));
    }
    cmd.args(&args).arg("-print0");
    let whitelist_patterns = get_global_whitelist();
    if let Ok(out) = cmd.output() {
        for entry in out.stdout.split(|&b| b == 0) {
            let p = String::from_utf8_lossy(entry);
            if p.is_empty() {
                continue;
            }
            let p_str = p.as_ref();
            if should_protect_path(p_str) {
                continue;
            }
            if !whitelist_patterns.is_empty() && is_path_whitelisted(p_str, &whitelist_patterns) {
                continue;
            }
            safe_remove(p_str, true);
        }
    }
}

/// 对齐 file_ops.sh:safe_sudo_find_delete()
/// 同样必须在删除每个 match 前过保护表与白名单。
/// 返回 `(total_size_kb, total_count)`,size 通过 `sudo du -skP` 逐文件实打实测量。
pub fn safe_sudo_find_delete(
    base_dir: &str,
    pattern: &str,
    age_days: u32,
    type_filter: &str,
) -> (u64, u64) {
    let ok = sudo_output(&["/bin/test", "-d", base_dir]).status.success();
    if !ok {
        debug_log(&format!("Directory does not exist, skipping: {}", base_dir));
        return (0, 0);
    }

    let is_symlink = sudo_output(&["/bin/test", "-L", base_dir]).status.success();
    if is_symlink {
        log_error(&format!(
            "Refusing to search symlinked directory: {base_dir}"
        ));
        return (0, 0);
    }

    if type_filter != "f" && type_filter != "d" {
        log_error(&format!(
            "Invalid type filter: {}, must be 'f' or 'd'",
            type_filter
        ));
        return (0, 0);
    }

    debug_log(&format!(
        "Finding, sudo, in {}: {}, age: {}d, type: {}",
        base_dir, pattern, age_days, type_filter
    ));

    let mut cmd_args: Vec<&str> = vec!["/usr/bin/find"];
    let mut args: Vec<String> = vec![
        base_dir.to_string(),
        "-maxdepth".to_string(),
        "5".to_string(),
    ];
    if pattern != "*" {
        args.push("-name".to_string());
        args.push(pattern.to_string());
    }
    args.push("-type".to_string());
    args.push(type_filter.to_string());
    if age_days > 0 {
        args.push("-mtime".to_string());
        args.push(format!("+{age_days}"));
    }
    cmd_args.extend(args.iter().map(|s| s.as_str()));
    cmd_args.push("-print0");

    let whitelist_patterns = get_global_whitelist();
    let mut paths: Vec<String> = Vec::new();
    let out = sudo_output(&cmd_args);
    for entry in out.stdout.split(|&b| b == 0) {
        let p = String::from_utf8_lossy(entry);
        if p.is_empty() {
            continue;
        }
        let p_str = p.as_ref();
        if should_protect_path(p_str) {
            continue;
        }
        if !whitelist_patterns.is_empty() && is_path_whitelisted(p_str, &whitelist_patterns) {
            continue;
        }
        paths.push(p.to_string());
    }

    if paths.is_empty() {
        return (0, 0);
    }

    let mut total_kb: u64 = 0;
    for p in &paths {
        let size_kb = String::from_utf8_lossy(&sudo_output(&["/usr/bin/du", "-skP", p]).stdout)
            .split_whitespace()
            .next()
            .unwrap_or("0")
            .parse::<u64>()
            .unwrap_or(0);
        total_kb = total_kb.saturating_add(size_kb);
    }

    let mut total_count: u64 = 0;
    for p in &paths {
        if safe_sudo_remove(p, None) == MOLE_OK {
            total_count = total_count.saturating_add(1);
        }
    }

    (total_kb, total_count)
}
pub fn calculate_total_size(files: &str) -> u64 {
    let mut total = 0u64;
    for file in files.lines() {
        let f = file.trim();
        if !f.is_empty() && Path::new(f).exists() {
            total += get_path_size_kb(f);
        }
    }
    total
}

/// 把可能含 glob (`*`, `?`, `[...]`) 的路径展开成具体的命中列表。
/// 不存在时返回空 vec(对齐 SH `nullglob` + `safe_clean` 中的"父目录不存在则跳过"快速路径)。
/// `~` 会被替换成 $HOME。
pub fn expand_glob_paths(pattern: &str) -> Vec<String> {
    if pattern.is_empty() {
        return Vec::new();
    }
    let expanded = if let Some(rest) = pattern.strip_prefix("~/") {
        format!("{}/{}", home_dir(), rest)
    } else if pattern == "~" {
        home_dir()
    } else {
        pattern.to_string()
    };

    let has_glob = expanded.contains('*') || expanded.contains('?') || expanded.contains('[');
    if !has_glob {
        return if Path::new(&expanded).exists() || Path::new(&expanded).is_symlink() {
            vec![expanded]
        } else {
            Vec::new()
        };
    }

    // 父目录不存在 → 直接放弃,避免 glob crate 无谓 stat
    // 对齐 bin/clean.sh:safe_clean() 第 405-418 行
    if let Some(star_pos) = expanded.find(|c: char| c == '*' || c == '?' || c == '[') {
        let base = &expanded[..star_pos];
        let parent = if let Some(pos) = base.rfind('/') {
            &base[..pos]
        } else {
            "."
        };
        if !parent.is_empty() && !Path::new(parent).is_dir() {
            return Vec::new();
        }
    }

    let mut out: Vec<String> = match glob::glob(&expanded) {
        Ok(it) => it
            .filter_map(|r| r.ok())
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort();
    out.dedup();
    out
}

/// 对齐 `bin/clean.sh:normalize_paths_for_cleanup()` 第 228-318 行：
///
/// 1. 去除尾随 `/`；
/// 2. 精确去重；
/// 3. **父子路径去重**：当 `/a/b` 已在列表中时，移除 `/a/b/c`、`/a/b/c/d` 等子路径。
///    这一步确保删除目录时不重复统计其内部文件，与 Shell 行为一致。
///
/// 不处理包含换行符的路径（Shell 版也跳过此类路径的排序流水线）。
fn normalize_paths_for_cleanup(paths: &[String]) -> Vec<String> {
    // Step 1: 去除尾随 `/` + 精确去重
    let mut seen = std::collections::HashSet::new();
    let mut normalized: Vec<String> = Vec::new();
    for p in paths {
        let trimmed = p.trim_end_matches('/').to_string();
        if trimmed.is_empty() || !seen.insert(trimmed.clone()) {
            continue;
        }
        normalized.push(trimmed);
    }

    // Step 2: 按长度升序排序，保证父路径出现在子路径之前
    normalized.sort_by(|a, b| a.len().cmp(&b.len()));

    // Step 3: 过滤子路径（Shell 版 O(n²) 内层循环）
    let mut result: Vec<String> = Vec::new();
    'outer: for p in &normalized {
        for kept in &result {
            if p == kept {
                continue 'outer;
            }
            // p 以 kept 为前缀且紧跟 `/` → kept 是父目录
            if p.starts_with(kept) && p[kept.len()..].starts_with('/') {
                continue 'outer;
            }
        }
        result.push(p.clone());
    }
    result
}

/// `safe_clean` 命令级 helper,对齐 bin/clean.sh:safe_clean() 第 387-820 行。
///
/// 流程(精简版,GUI 后端无 spinner/dry-run export 列表):
///   1. 对每条 target 做 glob 展开
///   2. 走 should_protect_path / is_path_whitelisted 过滤
///   3. 测体积、累加,然后 safe_remove
///   4. log_operation + note_activity
///
/// 返回 `(total_size_kb, total_count)`。调用方据此聚合 section 级总量。
///
/// 注意:**永远不要**绕过这个 helper 直接 `safe_remove`,否则会丢失白名单/保护检查,
/// 等同于把 SH 端长期沉淀的安全规则直接注释掉。
pub fn safe_clean(targets: &[&str], description: &str) -> (u64, u64) {
    if targets.is_empty() {
        return (0, 0);
    }

    if super::base::is_clean_cancelled() {
        return (0, 0);
    }

    let dry_run = std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1"
        || std::env::var("DRY_RUN").unwrap_or_default() == "true";
    let whitelist_patterns = get_global_whitelist();

    let mut existing_paths: Vec<String> = Vec::new();
    for raw in targets {
        for path in expand_glob_paths(raw) {
            // 防止重复(典型场景:目录里既有显式路径又有 glob)
            if existing_paths.contains(&path) {
                continue;
            }
            if should_protect_path(&path) {
                log_operation("clean", "SKIPPED", &path, Some("protected"));
                continue;
            }
            if !whitelist_patterns.is_empty() && is_path_whitelisted(&path, &whitelist_patterns) {
                log_operation("clean", "SKIPPED", &path, Some("whitelist"));
                continue;
            }
            if Path::new(&path).exists() || Path::new(&path).is_symlink() {
                if dry_run && !dry_run_register_cleanup_target(&path) {
                    continue;
                }
                existing_paths.push(path);
            }
        }
    }

    if existing_paths.is_empty() {
        return (0, 0);
    }

    // 与 bin/clean.sh:normalize_paths_for_cleanup 对齐：
    // 去尾随 `/` + 父子路径去重，避免删除目录时重复统计内部文件。
    existing_paths = normalize_paths_for_cleanup(&existing_paths);

    debug_log(&format!(
        "Cleaning: {description}, {} items",
        existing_paths.len()
    ));

    let mut total_size_kb: u64 = 0;
    let mut total_count: u64 = 0;

    for path in &existing_paths {
        let size_kb = get_path_size_kb(path);
        let removed = if dry_run {
            // dry-run 时只统计,真实保留 path
            true
        } else {
            safe_remove_ex(path, true, Some(size_kb)) == MOLE_OK
        };
        if removed {
            total_size_kb = total_size_kb.saturating_add(size_kb);
            total_count = total_count.saturating_add(1);
        }
    }

    if total_count > 0 {
        super::base::note_activity();
    }

    (total_size_kb, total_count)
}

/// `safe_clean` 的 sudo 变体,对齐 SH `safe_sudo_*` 习惯用法。
pub fn safe_sudo_clean(targets: &[&str], description: &str) -> (u64, u64) {
    if targets.is_empty() {
        return (0, 0);
    }
    if super::base::is_clean_cancelled() {
        return (0, 0);
    }
    let dry_run = std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1"
        || std::env::var("DRY_RUN").unwrap_or_default() == "true";
    let whitelist_patterns = get_global_whitelist();

    let mut existing_paths: Vec<String> = Vec::new();
    for raw in targets {
        for path in expand_glob_paths(raw) {
            if existing_paths.contains(&path) {
                continue;
            }
            if should_protect_path(&path) {
                log_operation("clean", "SKIPPED", &path, Some("protected"));
                continue;
            }
            if !whitelist_patterns.is_empty() && is_path_whitelisted(&path, &whitelist_patterns) {
                log_operation("clean", "SKIPPED", &path, Some("whitelist"));
                continue;
            }
            // sudo 路径用 sudo test -e 而不是 std::fs(避免误判 root-owned 文件不存在)
            let exists = sudo_output(&["/bin/test", "-e", &path]).status.success();
            if exists {
                if dry_run && !dry_run_register_cleanup_target(&path) {
                    continue;
                }
                existing_paths.push(path);
            }
        }
    }
    if existing_paths.is_empty() {
        return (0, 0);
    }

    // 与 bin/clean.sh:normalize_paths_for_cleanup 对齐
    existing_paths = normalize_paths_for_cleanup(&existing_paths);

    debug_log(&format!(
        "Sudo cleaning: {description}, {} items",
        existing_paths.len()
    ));

    let mut total_size_kb: u64 = 0;
    let mut total_count: u64 = 0;
    for path in &existing_paths {
        let size_kb = run_cmd("sudo", &["du", "-skP", path])
            .and_then(|s| {
                s.split_whitespace()
                    .next()
                    .unwrap_or("0")
                    .parse::<u64>()
                    .ok()
            })
            .unwrap_or(0);
        let removed = if dry_run {
            true
        } else {
            safe_sudo_remove(path, Some(size_kb)) == MOLE_OK
        };
        if removed {
            total_size_kb = total_size_kb.saturating_add(size_kb);
            total_count = total_count.saturating_add(1);
        }
    }
    if total_count > 0 {
        super::base::note_activity();
    }
    (total_size_kb, total_count)
}
