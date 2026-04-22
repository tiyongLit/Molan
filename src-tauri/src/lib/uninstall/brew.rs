//! 对齐 `lib/uninstall/brew.sh`。
//!
//! 关键差异回滚:之前 Rust 版 `brew_uninstall_cask` 在 brew 失败后会直接
//! `sudo rm -rf $app_path` 兜底,这是 SH 完全没有的危险行为(绕过
//! `validate_path_for_deletion` / Trash / safe_remove),已删除。
//! 真正的失败兜底由 `batch.rs::batch_uninstall_applications` 根据
//! `is_brew_cask_installed` 的三态码做。

use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::core::base::get_path_size_kb;
use crate::core::log::debug_log;
use crate::core::timeout::{run_with_timeout, run_with_timeout_capture};

/// 对齐 SH 第 38-51 行 `is_brew_cask_installed` 的三态码:
/// `0` 已安装 / `1` 未安装 / `2` 状态无法确定。
/// 之前 Rust 把 1/2 都映射到 `false`,破坏 batch.sh 第 619-625 行的分支语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaskInstallState {
    Installed = 0,
    NotInstalled = 1,
    Unknown = 2,
}

impl CaskInstallState {
    pub fn is_installed(self) -> bool {
        matches!(self, CaskInstallState::Installed)
    }
}

/// brew 可执行文件是否可用（原 `which brew` 的原生等价）：扫描 PATH 中可执行的 brew。
/// 保持与原语义一致（只认 PATH，不探测标准前缀）：GUI 环境 PATH 精简时行为不变。
pub fn is_homebrew_available() -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path)
        .any(|dir| !dir.as_os_str().is_empty() && is_executable(&dir.join("brew")))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else {
        return false;
    };
    unsafe { libc::access(c.as_ptr(), libc::X_OK) == 0 }
}

/// 对齐 SH 第 16-30 行 `resolve_path`（原 `realpath` 子进程改 `std::fs::canonicalize` 纯系统调用）。
pub fn resolve_path(path: &str) -> Option<String> {
    if !Path::new(path).exists() {
        return None;
    }
    std::fs::canonicalize(path)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

/// `brew list --cask` 结果缓存：卸载列表扫描会对每个 app 逐次探测 cask，
/// 逐次 spawn brew（Ruby 冷启动数百毫秒）会拖垮扫描；TTL 内复用一次结果。
/// brew 卸载 / autoremove 后主动失效，保证后续验证读到新状态。
const CASK_LIST_TTL: Duration = Duration::from_secs(30);

static CASK_LIST_CACHE: Mutex<Option<(Instant, Vec<String>)>> = Mutex::new(None);

fn cask_list_lock() -> std::sync::MutexGuard<'static, Option<(Instant, Vec<String>)>> {
    // 与 macos_running_apps 一致：毒化后取回内部值继续，而非永久失败
    CASK_LIST_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 取 `brew list --cask` 名字列表（TTL 缓存）；brew 不可用 / 执行失败 → None（不缓存失败）。
fn brew_cask_names_cached() -> Option<Vec<String>> {
    {
        let guard = cask_list_lock();
        if let Some((at, names)) = guard.as_ref() {
            if at.elapsed() < CASK_LIST_TTL {
                return Some(names.clone());
            }
        }
    }
    let names = brew_list_casks_uncached()?;
    let mut guard = cask_list_lock();
    *guard = Some((Instant::now(), names.clone()));
    Some(names)
}

/// 失效 cask 列表缓存：brew 变更操作（卸载 / autoremove）后调用。
fn invalidate_cask_list_cache() {
    *cask_list_lock() = None;
}

/// 实际执行 `brew list --cask`（无缓存；行内容保持原样，调用侧按整行精确比对）。
fn brew_list_casks_uncached() -> Option<Vec<String>> {
    let output = Command::new("brew")
        .env("HOMEBREW_NO_ENV_HINTS", "1")
        .args(["list", "--cask"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(String::from)
            .collect(),
    )
}

/// 对齐 SH 第 38-51 行。`brew` 不可用 / `brew list --cask` 失败 → `Unknown`。
pub fn is_brew_cask_installed(cask_name: &str) -> CaskInstallState {
    if cask_name.is_empty() {
        return CaskInstallState::Unknown;
    }
    if !is_homebrew_available() {
        return CaskInstallState::Unknown;
    }
    let Some(names) = brew_cask_names_cached() else {
        return CaskInstallState::Unknown;
    };
    let installed = names.iter().any(|l| l == cask_name);
    if installed {
        CaskInstallState::Installed
    } else {
        CaskInstallState::NotInstalled
    }
}

/// 对齐 SH 第 57-79 行 `_extract_cask_token_from_path`。
pub fn _extract_cask_token_from_path(path: &str) -> Option<String> {
    let caskroom_markers = ["/opt/homebrew/Caskroom/", "/usr/local/Caskroom/"];
    let marker = caskroom_markers.iter().find(|m| path.starts_with(*m))?;
    let rest = &path[marker.len()..];
    let token = rest.split('/').next().unwrap_or("");
    if token.is_empty() {
        return None;
    }
    let mut chars = token.chars();
    let first = chars.next()?;
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return None;
    }
    let valid_rest = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if valid_rest {
        Some(token.to_string())
    } else {
        None
    }
}

/// 对齐 SH 第 83-90 行。
pub fn _detect_cask_via_resolved_path(app_path: &str) -> Option<String> {
    let resolved = resolve_path(app_path)?;
    _extract_cask_token_from_path(&resolved)
}

/// 对齐 SH 第 95-126 行。
pub fn _detect_cask_via_caskroom_search(app_bundle_name: &str) -> Option<String> {
    if app_bundle_name.is_empty() {
        return None;
    }
    let mut tokens: Vec<String> = Vec::new();
    for room in ["/opt/homebrew/Caskroom", "/usr/local/Caskroom"] {
        if !Path::new(room).is_dir() {
            continue;
        }
        // 原 SH `find <room> -maxdepth 3 -name <app_bundle_name>` 的原生等价
        for found in find_named_maxdepth3(room, app_bundle_name) {
            if let Some(token) = _extract_cask_token_from_path(&found) {
                tokens.push(token);
            }
        }
    }
    if tokens.is_empty() {
        return None;
    }
    tokens.sort();
    tokens.dedup();
    if tokens.len() != 1 {
        return None;
    }
    let token = tokens.into_iter().next().unwrap();
    if token.is_empty() {
        return None;
    }
    // SH 第 120 行:还要再用 `brew list --cask` 验证(grep -qxF)。
    if is_brew_cask_installed(&token).is_installed() {
        Some(token)
    } else {
        None
    }
}

/// `find <root> -maxdepth 3 -name <target>` 的原生等价：收集深度 ≤3、
/// 文件名精确等于 target 的条目路径（不跟随符号链接，对齐 find 默认行为）。
fn find_named_maxdepth3(root: &str, target: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut stack: Vec<(std::path::PathBuf, usize)> = vec![(std::path::PathBuf::from(root), 0)];
    while let Some((current, depth)) = stack.pop() {
        if depth >= 3 {
            continue; // 再深入将超过 maxdepth 3
        }
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue; // 不可读目录跳过（find 打印错误后继续）
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if entry.file_name().to_string_lossy() == target {
                out.push(path.to_string_lossy().to_string());
            }
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                stack.push((path, depth + 1));
            }
        }
    }
    out
}

/// 对齐 SH 第 129-136 行。
pub fn _detect_cask_via_symlink_check(app_path: &str) -> Option<String> {
    let p = Path::new(app_path);
    if !p.is_symlink() {
        return None;
    }
    let target = std::fs::read_link(p).ok()?;
    _extract_cask_token_from_path(&target.to_string_lossy())
}

/// 对齐 SH 第 139-151 行 `_detect_cask_via_brew_list`。
pub fn _detect_cask_via_brew_list(app_path: &str, app_bundle_name: &str) -> Option<String> {
    let stripped = app_bundle_name
        .strip_suffix(".app")
        .unwrap_or(app_bundle_name);
    // SH `LC_ALL=C tr '[:upper:]' '[:lower:]'`:这里直接 ascii_lowercase
    let needle = stripped.to_ascii_lowercase();

    // `brew list --cask` 走 TTL 缓存（扫描期逐 app 调用不再逐次 spawn brew）
    let names = brew_cask_names_cached()?;
    // SH `grep -Fix` 取整行匹配的第一个 cask
    let cask_name = names
        .iter()
        .find(|l| l.eq_ignore_ascii_case(&needle))?
        .clone();

    // 用 `brew info --cask` 校验该 cask 真正包含 app_path
    let info = Command::new("brew")
        .env("HOMEBREW_NO_ENV_HINTS", "1")
        .args(["info", "--cask", &cask_name])
        .output()
        .ok()?;
    if !info.status.success() {
        return None;
    }
    if String::from_utf8_lossy(&info.stdout).contains(app_path) {
        Some(cask_name)
    } else {
        None
    }
}

/// 对齐 SH 第 163-178 行 `get_brew_cask_name`。
pub fn get_brew_cask_name(app_path: &str) -> Option<String> {
    if app_path.is_empty() || !Path::new(app_path).exists() {
        return None;
    }
    if !is_homebrew_available() {
        return None;
    }
    let app_bundle_name = Path::new(app_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    _detect_cask_via_resolved_path(app_path)
        .or_else(|| _detect_cask_via_caskroom_search(app_bundle_name))
        .or_else(|| _detect_cask_via_symlink_check(app_path))
        .or_else(|| _detect_cask_via_brew_list(app_path, app_bundle_name))
}

/// 对齐 SH 第 200-210 行:按 app 体积选 timeout。
fn timeout_for_app(app_path: Option<&str>) -> u64 {
    let Some(p) = app_path else { return 300 };
    if p.is_empty() || !Path::new(p).is_dir() {
        return 300;
    }
    let size_gb = get_path_size_kb(p) / 1_048_576;
    if size_gb > 15 {
        900
    } else if size_gb > 5 {
        600
    } else {
        300
    }
}

/// 对齐 SH 第 183-256 行 `brew_uninstall_cask`。
///
/// 与 SH 完全对齐:
/// - 动态 timeout(<=5GB:300s,5-15GB:600s,>15GB:900s)经 `run_with_timeout` 包裹;
/// - 若 `SUDO_USER` 非空,切到 `sudo -u $SUDO_USER env ...` 执行(避免 brew 在 root 身份下出问题);
/// - 始终设置 `HOMEBREW_NO_ENV_HINTS=1 HOMEBREW_NO_AUTO_UPDATE=1 NONINTERACTIVE=1`;
/// - 退出码 124(超时)立即失败,**不做验证**(避免不一致状态被误判成功);
/// - 验证 `is_brew_cask_installed` + `app_path -e`,**两条都得 gone** 才算成功。
///
/// 失败时 **不会** 调用 `sudo rm -rf`(SH 也不会)。手工兜底由调用方按
/// `is_brew_cask_installed` 三态码决定。
pub fn brew_uninstall_cask(cask_name: &str, app_path: Option<&str>, zap_mode: &str) -> bool {
    if cask_name.is_empty() {
        return false;
    }
    // nozap = 不带 --zap 的 plain uninstall（sibling guard 时用，避免删共享 config）。
    let zap = zap_mode != "nozap";
    if std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1" {
        debug_log(&format!(
            "[DRY RUN] Would brew uninstall --cask{} {cask_name}",
            if zap { " --zap" } else { "" }
        ));
        return true;
    }
    if !is_homebrew_available() {
        return false;
    }

    debug_log(&format!(
        "Attempting brew uninstall --cask{} {cask_name}",
        if zap { " --zap" } else { "" }
    ));

    let timeout = timeout_for_app(app_path) as f64;
    debug_log(&format!(
        "App size-based timeout: {}s for cask {}",
        timeout as u64, cask_name
    ));

    // SH 第 213-226 行:SUDO_USER 非空时切到目标用户身份。
    let sudo_user = std::env::var("SUDO_USER").unwrap_or_default();
    let rc = if !sudo_user.is_empty() {
        // sudo -u $SUDO_USER env <ENVS> brew uninstall --cask --zap <cask>
        let envs = [
            "HOMEBREW_NO_ENV_HINTS=1",
            "HOMEBREW_NO_AUTO_UPDATE=1",
            "NONINTERACTIVE=1",
        ];
        let mut args: Vec<&str> = vec!["-u", sudo_user.as_str(), "env"];
        args.extend_from_slice(&envs);
        args.extend_from_slice(&["brew", "uninstall", "--cask"]);
        if zap {
            args.push("--zap");
        }
        args.push(cask_name);
        run_with_timeout(timeout, "sudo", &args)
    } else {
        // 直接走 brew,但要在子进程环境里设置三个变量。
        // run_with_timeout 不支持 env,这里手工组合 `env` 命令。
        let mut args: Vec<&str> = vec![
            "HOMEBREW_NO_ENV_HINTS=1",
            "HOMEBREW_NO_AUTO_UPDATE=1",
            "NONINTERACTIVE=1",
            "brew",
            "uninstall",
            "--cask",
        ];
        if zap {
            args.push("--zap");
        }
        args.push(cask_name);
        run_with_timeout(timeout, "env", &args)
    };

    let uninstall_ok = rc == 0;
    // brew 副作用已发生：失效 cask 列表缓存，保证下方验证读到新状态
    invalidate_cask_list_cache();
    if !uninstall_ok {
        debug_log(&format!(
            "brew uninstall timeout or failed with exit code: {rc}"
        ));
        if rc == 124 {
            debug_log(&format!(
                "brew uninstall timed out after {}s, returning failure",
                timeout as u64
            ));
            return false;
        }
    }

    // 验证 cask 是否还在
    let cask_state = is_brew_cask_installed(cask_name);
    let cask_gone = matches!(cask_state, CaskInstallState::NotInstalled);
    let app_gone = match app_path {
        Some(p) if !p.is_empty() => !Path::new(p).exists(),
        _ => true,
    };

    if cask_gone && app_gone {
        debug_log(&format!("Successfully uninstalled cask '{cask_name}'"));
        return true;
    }
    debug_log(&format!(
        "brew uninstall failed: cask_gone={cask_gone} app_gone={app_gone}"
    ));
    false
}

/// 静默触发 `brew autoremove`,对齐 batch.sh 第 962-968 行的后台清理。
/// GUI 后端无 TTY,这里同步执行 30s 超时,捕获不输出。
pub fn brew_autoremove_silent() {
    if std::env::var("MOLE_DRY_RUN").unwrap_or_default() == "1" {
        return;
    }
    if !is_homebrew_available() {
        return;
    }
    let _ = run_with_timeout_capture(
        30.0,
        "env",
        &[
            "HOMEBREW_NO_ENV_HINTS=1",
            "HOMEBREW_NO_AUTO_UPDATE=1",
            "NONINTERACTIVE=1",
            "brew",
            "autoremove",
        ],
    );
    // autoremove 可能移除无人引用的 cask：失效列表缓存
    invalidate_cask_list_cache();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_token_basic() {
        assert_eq!(
            _extract_cask_token_from_path("/opt/homebrew/Caskroom/visual-studio-code/1.2.3/app"),
            Some("visual-studio-code".to_string())
        );
        assert_eq!(
            _extract_cask_token_from_path("/usr/local/Caskroom/firefox/100/Firefox.app"),
            Some("firefox".to_string())
        );
    }

    #[test]
    fn extract_token_rejects_invalid() {
        // 大写
        assert!(_extract_cask_token_from_path("/opt/homebrew/Caskroom/Foo/1.0").is_none());
        // 以 `-` 起头
        assert!(_extract_cask_token_from_path("/opt/homebrew/Caskroom/-foo/1.0").is_none());
        // 不在 Caskroom 下
        assert!(_extract_cask_token_from_path("/Applications/Firefox.app").is_none());
    }

    #[test]
    fn cask_install_state_helpers() {
        assert!(CaskInstallState::Installed.is_installed());
        assert!(!CaskInstallState::NotInstalled.is_installed());
        assert!(!CaskInstallState::Unknown.is_installed());
    }

    /// 原生 `find -maxdepth 3 -name <target>` 等价物：与系统 find 结果集合一致。
    #[test]
    fn find_named_matches_system_find() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("token/1.2.3/App.app/Contents")).unwrap();
        std::fs::create_dir_all(root.join("other/App")).unwrap();
        std::fs::create_dir_all(root.join("a/b/c/App.app/Inner.app")).unwrap(); // 两者均 depth4 超界
        std::fs::write(root.join("token/App.app"), b"").unwrap(); // 同名普通文件（depth2）

        let root_str = root.to_string_lossy().to_string();
        let out = std::process::Command::new("find")
            .args([&root_str, "-maxdepth", "3", "-name", "App.app", "-print0"])
            .output()
            .expect("系统 find 不可用");
        let mut expected: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .split('\0')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(String::from)
            .collect();
        let mut actual = find_named_maxdepth3(&root_str, "App.app");
        expected.sort();
        actual.sort();
        assert_eq!(actual, expected);
    }

    /// resolve_path 解析符号链接到真实路径（原 realpath 语义）。
    #[test]
    fn resolve_path_follows_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();

        let resolved = resolve_path(&root.join("link").to_string_lossy()).expect("应能解析");
        assert_eq!(
            resolved,
            std::fs::canonicalize(root.join("real"))
                .unwrap()
                .to_string_lossy()
                .to_string()
        );
        assert!(resolve_path("/nonexistent-mole-probe-9f3a").is_none());
    }

    /// is_executable 按 X_OK 判定（原 `which brew` 的底层判据）。
    /// 不注入 PATH：避免与并行测试中的其他子进程调用互相干扰。
    #[test]
    fn is_executable_checks_x_ok() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("tool");
        std::fs::write(&f, b"x").unwrap();

        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!is_executable(&f));
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_executable(&f));
        assert!(!is_executable(Path::new("/nonexistent-mole-probe-9f3a")));
    }

    #[test]
    fn timeout_scales_with_size() {
        // 没传 app_path 一律 300s
        assert_eq!(timeout_for_app(None), 300);
        // 不存在的路径也是 300s
        assert_eq!(timeout_for_app(Some("/nope/does/not/exist")), 300);
    }
}
