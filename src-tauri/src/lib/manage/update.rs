//! 对齐 `lib/manage/update.sh`。
//!
//! GUI 端没有 TTY,因此 `ask_for_updates` 不会真正读取按键(由前端对话框代替)。
//! 数据计算、env var 导出、`format_brew_update_*` 等业务逻辑严格按 SH 翻译。

use std::path::Path;

use crate::core::base::home_dir;
use crate::core::timeout::run_with_timeout_capture;

/// 对齐 SH 中 `reset_mole_cache`(定义在 `lib/check/all.sh:219-221`)的语义:
/// 仅清空 `$CACHE_DIR/mole_version`。
pub fn reset_mole_cache() {
    let home = home_dir();
    let _ = std::fs::remove_file(format!("{home}/.cache/mole/mole_version"));
}

fn read_count_env(name: &str) -> Option<u32> {
    std::env::var(name)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

fn brew_available() -> bool {
    run_with_timeout_capture(2.0, "which", &["brew"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

fn count_outdated_lines(out: &str) -> u32 {
    out.lines().filter(|l| !l.trim().is_empty()).count() as u32
}

/// 对齐 `update.sh:38-65` 中的 `populate_brew_update_counts_if_unset`。
///
/// 当任意一个 BREW_* 环境变量未设置时,运行 `brew outdated` 探测,并把
/// `BREW_FORMULA_OUTDATED_COUNT` / `BREW_CASK_OUTDATED_COUNT` / `BREW_OUTDATED_COUNT`
/// 写回到环境变量。
pub fn populate_brew_update_counts_if_unset() {
    let need_probe = std::env::var("BREW_OUTDATED_COUNT").is_err()
        || std::env::var("BREW_FORMULA_OUTDATED_COUNT").is_err()
        || std::env::var("BREW_CASK_OUTDATED_COUNT").is_err();
    if !need_probe {
        return;
    }

    let mut formula_count = read_count_env("BREW_FORMULA_OUTDATED_COUNT").unwrap_or(0);
    let mut cask_count = read_count_env("BREW_CASK_OUTDATED_COUNT").unwrap_or(0);

    if brew_available() {
        if let Some(out) =
            run_with_timeout_capture(8.0, "brew", &["outdated", "--formula", "--quiet"])
        {
            formula_count = count_outdated_lines(&out);
        }
        if let Some(out) = run_with_timeout_capture(8.0, "brew", &["outdated", "--cask", "--quiet"])
        {
            cask_count = count_outdated_lines(&out);
        }
    }

    let total = formula_count + cask_count;
    unsafe {
        std::env::set_var("BREW_FORMULA_OUTDATED_COUNT", formula_count.to_string());
        std::env::set_var("BREW_CASK_OUTDATED_COUNT", cask_count.to_string());
        std::env::set_var("BREW_OUTDATED_COUNT", total.to_string());
    }
}

/// 对齐 `update.sh:8-29` 中的 `format_brew_update_detail`。
///
/// 注意:不带 `Homebrew, ` 前缀(前缀属于 label,见 `format_brew_update_label`)。
/// 输入读自 `BREW_OUTDATED_COUNT` / `BREW_FORMULA_OUTDATED_COUNT` /
/// `BREW_CASK_OUTDATED_COUNT` 环境变量。
pub fn format_brew_update_detail() -> String {
    let total = read_count_env("BREW_OUTDATED_COUNT").unwrap_or(0);
    if total == 0 {
        return String::new();
    }
    let formulas = read_count_env("BREW_FORMULA_OUTDATED_COUNT").unwrap_or(0);
    let casks = read_count_env("BREW_CASK_OUTDATED_COUNT").unwrap_or(0);

    let mut details: Vec<String> = Vec::new();
    if formulas > 0 {
        details.push(format!("{formulas} formula"));
    }
    if casks > 0 {
        details.push(format!("{casks} cask"));
    }

    if details.is_empty() {
        format!("{total} updates")
    } else {
        details.join(", ")
    }
}

/// 对齐 `update.sh:32-36` 中的 `format_brew_update_label`,保留旧调用方/测试兼容。
pub fn format_brew_update_label() -> String {
    let detail = format_brew_update_detail();
    if detail.is_empty() {
        String::new()
    } else {
        format!("Homebrew, {detail}")
    }
}

/// 对齐 `update.sh:67-76` 中的 `brew_has_outdated`。
/// `kind = "cask"` 仅看 cask,其它默认看全部 outdated。
pub fn brew_has_outdated(kind: &str) -> bool {
    if !brew_available() {
        return false;
    }
    let out = if kind == "cask" {
        run_with_timeout_capture(8.0, "brew", &["outdated", "--cask", "--quiet"])
    } else {
        run_with_timeout_capture(8.0, "brew", &["outdated", "--quiet"])
    };
    out.map(|s| s.lines().any(|l| !l.trim().is_empty()))
        .unwrap_or(false)
}

/// 对齐 `update.sh:80-130` 中的 `ask_for_updates`。
///
/// SH 端会通过 `read_key` 读取回车 / ESC 来确认 Mole 更新;GUI 端无 TTY,
/// 这里只判断"是否存在任意一种待更新源"——具体确认行为由前端弹窗承担。
pub fn ask_for_updates() -> bool {
    populate_brew_update_counts_if_unset();

    let mut has_updates = false;
    if read_count_env("BREW_OUTDATED_COUNT").unwrap_or(0) > 0 {
        has_updates = true;
    }
    if read_count_env("APPSTORE_UPDATE_COUNT").unwrap_or(0) > 0 {
        has_updates = true;
    }
    if std::env::var("MACOS_UPDATE_AVAILABLE").unwrap_or_default() == "true" {
        has_updates = true;
    }
    if std::env::var("MOLE_UPDATE_AVAILABLE").unwrap_or_default() == "true" {
        has_updates = true;
    }

    if !has_updates {
        return false;
    }

    // SH 第 104-117 行:仅 Mole 走交互确认。GUI 没有 TTY,直接返回 false,
    // 由前端单独通过 Tauri command 触发 perform_updates。
    false
}

/// 对齐 `update.sh:134-169` 中的 `perform_updates`。
///
/// 仅处理 Mole 自身更新(brew / App Store / macOS 都是手动)。
/// 返回 true 表示存在更新且全部成功;无更新或失败均返回 false——与 SH
/// 中 `return 0/1` 的语义保持一致(0 = 全部成功且无错误)。
pub fn perform_updates() -> bool {
    let mut updated_count = 0u32;
    let mut total_count = 0u32;

    if std::env::var("MOLE_UPDATE_AVAILABLE").unwrap_or_default() == "true" {
        total_count = 1;

        // SH 第 141-142 行:优先 ${SCRIPT_DIR}/../../mole,其次 `command -v mole`。
        // Rust 端没有 SCRIPT_DIR,只能走 PATH 查找。
        let mole_bin = run_with_timeout_capture(2.0, "command", &["-v", "mole"])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && Path::new(s).exists());

        if let Some(bin) = mole_bin {
            let out = run_with_timeout_capture(60.0, &bin, &["update"]).unwrap_or_default();
            if out.contains("Updated") || out.contains("latest version") {
                reset_mole_cache();
                updated_count += 1;
            }
        }
    }

    if total_count == 0 {
        return true;
    }
    updated_count == total_count
}
