//! 对齐 `lib/uninstall/brew.sh`。
//!
//! 关键差异回滚:之前 Rust 版 `brew_uninstall_cask` 在 brew 失败后会直接
//! `sudo rm -rf $app_path` 兜底,这是 SH 完全没有的危险行为(绕过
//! `validate_path_for_deletion` / Trash / safe_remove),已删除。
//! 真正的失败兜底由 `batch.rs::batch_uninstall_applications` 根据
//! `is_brew_cask_installed` 的三态码做。

use std::path::Path;
use std::process::Command;

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

pub fn is_homebrew_available() -> bool {
    Command::new("which")
        .arg("brew")
        .output()
        .map(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
}

/// 对齐 SH 第 16-30 行 `resolve_path`。
/// `realpath` 不可用时回退到 `cd -P "$(dirname)" && pwd` + basename(macOS < 12.3 / 自定义 PATH)。
pub fn resolve_path(path: &str) -> Option<String> {
    if !Path::new(path).exists() {
        return None;
    }
    if let Ok(out) = Command::new("realpath").arg(path).output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    if let Ok(canon) = std::fs::canonicalize(path) {
        return Some(canon.to_string_lossy().to_string());
    }
    None
}

/// 对齐 SH 第 38-51 行。`brew` 不可用 / `brew list --cask` 失败 → `Unknown`。
pub fn is_brew_cask_installed(cask_name: &str) -> CaskInstallState {
    if cask_name.is_empty() {
        return CaskInstallState::Unknown;
    }
    if !is_homebrew_available() {
        return CaskInstallState::Unknown;
    }
    let output = Command::new("brew")
        .env("HOMEBREW_NO_ENV_HINTS", "1")
        .args(["list", "--cask"])
        .output();
    let out = match output {
        Ok(o) if o.status.success() => o,
        _ => return CaskInstallState::Unknown,
    };
    let installed = String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|l| l == cask_name);
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
        let output = Command::new("find")
            .args([room, "-maxdepth", "3", "-name", app_bundle_name])
            .output();
        if let Ok(o) = output {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                if let Some(token) = _extract_cask_token_from_path(line.trim()) {
                    tokens.push(token);
                }
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

    let output = Command::new("brew")
        .env("HOMEBREW_NO_ENV_HINTS", "1")
        .args(["list", "--cask"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    // SH `grep -Fix` 取整行匹配的第一个 cask
    let cask_name = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|l| l.eq_ignore_ascii_case(&needle))?
        .to_string();

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

    #[test]
    fn timeout_scales_with_size() {
        // 没传 app_path 一律 300s
        assert_eq!(timeout_for_app(None), 300);
        // 不存在的路径也是 300s
        assert_eq!(timeout_for_app(Some("/nope/does/not/exist")), 300);
    }
}
