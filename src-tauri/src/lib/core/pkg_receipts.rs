use std::path::Path;
use std::time::Instant;

use super::timeout::run_with_timeout_capture;

/// 给定 pkgutil --files 输出的相对路径,返回其所属 .app 包根。
/// 仅对 /usr/local/*.app 与 /opt/*.app 这两类非标准目录生效,对齐 SH:_mole_pkg_receipt_app_root()
///
/// 重要:必须用完整 path 段匹配 `.app`(`.app` 后跟 `/` 或末尾),而不是简单 `contains(".app")`,
/// 否则会把 `/usr/local/share/foo.appdata.json` 这种路径误认成 .app bundle。
pub fn _mole_pkg_receipt_app_root(rel_path: &str) -> Option<String> {
    let trimmed = rel_path.trim_start_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let candidate = format!("/{trimmed}");

    // 1. 必须是 /usr/local/ 或 /opt/ 下面
    let is_usr_local = candidate.starts_with("/usr/local/");
    let is_opt = candidate.starts_with("/opt/");
    if !is_usr_local && !is_opt {
        return None;
    }

    // 2. 必须包含 ".app" 段(末尾或后跟 /)
    if let Some(idx) = candidate.find(".app") {
        let after_idx = idx + ".app".len();
        let next = candidate.as_bytes().get(after_idx).copied();
        if next.is_none() || next == Some(b'/') {
            return Some(candidate[..after_idx].to_string());
        }
    }
    None
}

/// 扫描 pkgutil receipts,找出 /usr/local/*.app 与 /opt/*.app 这类非标准位置安装的应用。
/// 对齐 pkg_receipts.sh:pkg_receipt_nonstandard_app_paths()
///
/// 关键改进:
///   - `pkgutil --pkgs` 套 3s 超时(SH 的 MOLE_PKG_RECEIPT_LIST_TIMEOUT)
///   - 整体扫描套 8s 截止时间(SH 的 MOLE_PKG_RECEIPT_SCAN_TIMEOUT)
///   - 单个 pkgutil --files 套 5s 超时,避免某个损坏的 receipt 卡死整个 GUI
///   - 内层路径过滤后再 dedup,避免 N^2 contains 检查
///
/// 返回 `(paths, complete)`,其中 complete=false 表示扫描因超时提前结束(结果不完整),
/// 供 sibling guard 做 fail-closed 判断。
fn scan_pkg_receipts_impl() -> (Vec<String>, bool) {
    let list_timeout = std::env::var("MOLE_PKG_RECEIPT_LIST_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| *v > 0.0)
        .unwrap_or(3.0);
    let scan_timeout_secs = std::env::var("MOLE_PKG_RECEIPT_SCAN_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(8);

    let pkgs_output = match run_with_timeout_capture(list_timeout, "pkgutil", &["--pkgs"]) {
        Some(s) => s,
        None => return (Vec::new(), false),
    };
    if pkgs_output.is_empty() {
        return (Vec::new(), true);
    }

    let scan_start = Instant::now();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut result: Vec<String> = Vec::new();
    let mut complete = true;

    for pkg_id in pkgs_output.lines() {
        if scan_start.elapsed().as_secs() >= scan_timeout_secs {
            complete = false;
            break;
        }
        let pkg_id = pkg_id.trim();
        if pkg_id.is_empty() || pkg_id.starts_with("com.apple.") {
            continue;
        }

        let pkg_files = match run_with_timeout_capture(5.0, "pkgutil", &["--files", pkg_id]) {
            Some(s) => s,
            None => continue,
        };
        if pkg_files.is_empty() {
            continue;
        }

        for rel_path in pkg_files.lines() {
            if scan_start.elapsed().as_secs() >= scan_timeout_secs {
                complete = false;
                return (result, complete);
            }
            let rel_path = rel_path.trim();
            if rel_path.is_empty() {
                continue;
            }
            // 预过滤:对齐 SH 第 57 行 `grep -E '^(/usr/local/|/opt/).*\.app(/|$)'`
            let stripped = rel_path.trim_start_matches('/');
            let candidate = format!("/{stripped}");
            if !(candidate.starts_with("/usr/local/") || candidate.starts_with("/opt/")) {
                continue;
            }
            if !candidate.contains(".app") {
                continue;
            }

            if let Some(app_path) = _mole_pkg_receipt_app_root(rel_path) {
                if Path::new(&app_path).is_dir() && seen.insert(app_path.clone()) {
                    result.push(app_path);
                }
            }
        }
    }

    (result, complete)
}

pub fn pkg_receipt_nonstandard_app_paths() -> Vec<String> {
    scan_pkg_receipts_impl().0
}

/// 同 `pkg_receipt_nonstandard_app_paths`，但额外返回扫描是否完整（未超时）。
/// sibling guard 用：receipt 扫描不完整时，不能据此证明"无同 bundle id 兄弟"。
pub fn pkg_receipt_nonstandard_app_paths_complete() -> (Vec<String>, bool) {
    scan_pkg_receipts_impl()
}
