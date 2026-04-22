//! 开发者工具清理 — 严格对齐 `Mole/lib/clean/dev.sh` 与 `clean_developer_tools` 调用顺序。
//!
//! 不臆造路径:所有目标与 SH 中 `safe_clean` / `clean_tool_cache` / `safe_find_delete` 等一致。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::core::app_protection::{
    get_global_whitelist, is_path_whitelisted_from_global, set_global_whitelist,
    should_protect_path,
};
use crate::core::base::{
    MOLE_LOG_AGE_DAYS, bytes_to_human, command_available, get_file_mtime, get_path_size_kb,
    home_dir, is_dry_run, note_activity, pgrep_x,
};
use crate::core::dry_run_registry::dry_run_register_cleanup_target;
use crate::core::file_ops::{MOLE_OK, safe_clean, safe_find_delete, safe_remove, safe_sudo_remove};
use crate::core::log::{debug_log, log_info, log_warning};
use crate::core::sudo::{ensure_admin_session, is_admin_authorized, sudo_output};
use crate::core::timeout::{
    run_with_timeout, run_with_timeout_capture, run_with_timeout_capture_lossy,
    run_with_timeout_capture_silent,
};

use super::app_caches::{clean_code_editors, clean_xcode_tools, simctl_available};
use super::brew::clean_homebrew;
use super::caches::{clean_project_caches, clean_service_worker_cache};
use super::maven::clean_maven_repository;

// ----------------------------------------------------------------------------- helpers

fn normalize_dir_key(path: &str) -> String {
    let p = path.trim_end_matches('/');
    Path::new(p)
        .canonicalize()
        .map(|c| c.to_string_lossy().to_string())
        .unwrap_or_else(|_| p.to_string())
}

/// Shell `clean_tool_cache` — 对齐 SH clean_tool_cache() 第 10-41 行。
/// 返回 (释放的 KB, 释放的条目数)。
fn run_clean_tool_cache(
    description: &str,
    cache_path: Option<&str>,
    cmd: &str,
    args: &[&str],
) -> (u64, u64) {
    if let Some(cp) = cache_path {
        if !cp.is_empty() && is_path_whitelisted_from_global(cp) {
            if is_dry_run() {
                log_info(&format!("{description} · would skip (whitelist)"));
            } else {
                log_info(&format!("{description} · skipped (whitelist)"));
            }
            return (0, 0);
        }
    }
    let size_before = cache_path
        .filter(|p| !p.is_empty() && Path::new(p).is_dir())
        .map(|p| get_path_size_kb(p))
        .unwrap_or(0);
    if is_dry_run() {
        log_info(&format!("{description} · would clean"));
        return (size_before, if size_before > 0 { 1 } else { 0 });
    }
    let code = run_with_timeout(300.0, cmd, args);
    let size_after = cache_path
        .filter(|p| !p.is_empty() && Path::new(p).is_dir())
        .map(|p| get_path_size_kb(p))
        .unwrap_or(0);
    let freed = size_before.saturating_sub(size_after);
    if code == 0 {
        log_info(description);
    }
    (freed, if freed > 0 { 1 } else { 0 })
}

/// npm `cache clean --force` 的包装：屏蔽 stderr 避免 EACCES 错误输出到终端。
fn run_with_timeout_npm_cache_clean(_cache_path: &str) -> i32 {
    let timeout_bin = crate::core::timeout::detect_timeout_bin();
    let mut cmd = std::process::Command::new("npm");
    cmd.args(["cache", "clean", "--force"]);
    cmd.stderr(std::process::Stdio::null()); // 屏蔽 npm 的 EACCES 错误输出
    cmd.stdout(std::process::Stdio::null());

    // 用系统 timeout 包装
    if let Some(tb) = timeout_bin {
        let mut timeout_cmd = std::process::Command::new(&tb);
        timeout_cmd.arg("300"); // 5 min
        timeout_cmd.arg("npm");
        timeout_cmd.args(["cache", "clean", "--force"]);
        timeout_cmd.stderr(std::process::Stdio::null());
        timeout_cmd.stdout(std::process::Stdio::null());
        return timeout_cmd
            .status()
            .ok()
            .and_then(|s| s.code())
            .unwrap_or(124);
    }

    cmd.status().ok().and_then(|s| s.code()).unwrap_or(1)
}

/// 检查目录中是否有 root 所有权的文件（用于检测 npm EACCES 场景）。
fn has_root_owned_files(dir: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    let uid = unsafe { libc::getuid() };
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.uid() != uid {
                    return true;
                }
            }
        }
    }
    false
}

fn dir_is_writable(dir: &str) -> bool {
    Command::new("test")
        .args(["-w", dir])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn lock_dir_has_entries(dir: &str) -> bool {
    std::fs::read_dir(dir)
        .map(|mut rd| rd.next().is_some())
        .unwrap_or(false)
}

/// 对齐 SH `clean_corepack_cache()` 第 42-58 行。
pub fn clean_corepack_cache() -> (u64, u64) {
    let h = home_dir();
    let corepack_home =
        std::env::var("COREPACK_HOME").unwrap_or_else(|_| format!("{h}/.cache/node/corepack"));

    let t = corepack_home.trim();
    if t.is_empty() || !t.starts_with('/') {
        return (0, 0);
    }

    let unsafe_paths = [
        "/",
        &h,
        &format!("{h}/"),
        &format!("{h}/Library"),
        &format!("{h}/Library/"),
    ];
    if unsafe_paths.contains(&t) {
        debug_log(&format!("Skipping unsafe Corepack cache path: {t}"));
        return (0, 0);
    }

    if command_available("corepack")
        && run_with_timeout(2.0, "bash", &["-lc", "corepack --version >/dev/null 2>&1"]) == 0
    {
        run_clean_tool_cache("Corepack cache", Some(t), "corepack", &["cache", "clean"])
    } else {
        safe_clean(&[&format!("{t}/*")], "Corepack cache")
    }
}

pub fn clean_dev_npm() -> (u64, u64) {
    let h = home_dir();
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    let npm_default = format!("{h}/.npm");
    let mut npm_cache_path = npm_default.clone();

    if command_available("npm") {
        if let Some(p) =
            run_with_timeout_capture(2.0, "bash", &["-lc", "npm config get cache 2>/dev/null"])
        {
            let t = p.trim();
            if !t.is_empty() && t.starts_with('/') {
                npm_cache_path = t.to_string();
            }
        }

        // 优先用 npm cache clean 清理，但屏蔽 stderr（npm 已知 bug 会导致 EACCES）。
        // SH 中相同行为：stderr 默认继承到终端，但 GUI 环境下不应展示 raw Node 错误。
        // dry-run（扫描期）绝不执行真实清理：仅由下方 fallback 的 safe_clean 预估大小并注册目标；
        // 只有执行期才真正运行 `npm cache clean --force`。
        let cleaned_via_npm = if !Path::new(&npm_cache_path).is_dir() {
            false
        } else if is_dry_run() {
            log_info("npm cache · would clean");
            false
        } else {
            let size_before = get_path_size_kb(&npm_cache_path);
            let code = run_with_timeout_npm_cache_clean(&npm_cache_path);
            let size_after = get_path_size_kb(&npm_cache_path);
            let freed = size_before.saturating_sub(size_after);
            if code == 0 {
                log_info("npm cache");
                note_activity();
            }
            if freed > 0 {
                total_kb = total_kb.saturating_add(freed);
                total_cnt += 1;
            }
            freed > 0
        };
        // npm cache clean 失败后尝试直接删除 _cacache（含 root-owned 文件时需 sudo）。
        // dry-run 下 safe_clean 只预估体积并注册目标，不做真实删除；
        // root-owned（EACCES）场景仅提示用户手动 `sudo chown`，代码不代为修复所有权。
        if !cleaned_via_npm {
            let cacache = format!("{npm_cache_path}/_cacache");
            if Path::new(&cacache).is_dir() {
                let (kb, cnt) =
                    safe_clean(&[&format!("{cacache}/*")], "npm cache directory (direct)");
                total_kb = total_kb.saturating_add(kb);
                total_cnt = total_cnt.saturating_add(cnt);
                // 如果直接删除也失败（EACCES），提示用户修复所有权。
                if kb == 0 && cnt == 0 {
                    // check if dir still has root-owned files
                    if has_root_owned_files(&cacache) {
                        let uid = unsafe { libc::getuid() };
                        let gid = unsafe { libc::getgid() };
                        log_warning(&format!(
                            "npm cache contains root-owned files. To fix: sudo chown -R {uid}:{gid} \"{npm_cache_path}\"",
                        ));
                    }
                }
            }
        }
    }

    // 残留目录兜底（dry-run 下 safe_clean 仅预估/注册，不产生真实副作用）。
    let residuals = ["_cacache", "_npx", "_logs", "_prebuilds"];
    let labels = [
        "npm cache directory",
        "npm npx cache",
        "npm logs",
        "npm prebuilds",
    ];
    for (sub, label) in residuals.iter().zip(labels.iter()) {
        let g = format!("{npm_default}/{sub}/*");
        let (kb, cnt) = safe_clean(&[&g], label);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }

    let norm_custom = normalize_dir_key(&npm_cache_path);
    let norm_default = normalize_dir_key(&npm_default);
    if norm_custom != norm_default {
        for (sub, label) in residuals.iter().zip(labels.iter()) {
            let g = format!("{npm_cache_path}/{sub}/*");
            let (kb, cnt) = safe_clean(&[&g], &format!("{label} (custom path)"));
            total_kb = total_kb.saturating_add(kb);
            total_cnt = total_cnt.saturating_add(cnt);
        }
    }

    let pnpm_default = format!("{h}/Library/pnpm/store");
    let pnpm_ok = command_available("pnpm")
        && run_with_timeout(
            2.0,
            "bash",
            &[
                "-lc",
                "COREPACK_ENABLE_DOWNLOAD_PROMPT=0 pnpm --version >/dev/null 2>&1",
            ],
        ) == 0;

    if pnpm_ok {
        let mut pnpm_cache_check = pnpm_default.clone();
        if let Some(p) = run_with_timeout_capture(
            2.0,
            "bash",
            &[
                "-lc",
                "COREPACK_ENABLE_DOWNLOAD_PROMPT=0 pnpm store path 2>/dev/null",
            ],
        ) {
            let t = p.trim();
            if !t.is_empty() && t.starts_with('/') {
                pnpm_cache_check = t.to_string();
            }
        }
        let (kb, cnt) = run_clean_tool_cache(
            "pnpm cache",
            Some(&pnpm_cache_check),
            "bash",
            &[
                "-lc",
                "COREPACK_ENABLE_DOWNLOAD_PROMPT=0 pnpm store prune >/dev/null 2>&1",
            ],
        );
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
        if normalize_dir_key(&pnpm_cache_check) != normalize_dir_key(&pnpm_default) {
            let (kb, cnt) = safe_clean(&[&format!("{pnpm_default}/*")], "Orphaned pnpm store");
            total_kb = total_kb.saturating_add(kb);
            total_cnt = total_cnt.saturating_add(cnt);
        }
    } else {
        debug_log(&format!(
            "pnpm is unavailable, leaving global pnpm store for manual review: {pnpm_default}"
        ));
    }
    let (kb, cnt) = clean_corepack_cache();
    total_kb = total_kb.saturating_add(kb);
    total_cnt = total_cnt.saturating_add(cnt);

    let bun_default = format!("{h}/.bun/install/cache");
    let mut bun_cache_path = bun_default.clone();
    let mut bun_before = 0u64;

    if command_available("bun")
        && run_with_timeout(2.0, "bash", &["-lc", "bun --version >/dev/null 2>&1"]) == 0
    {
        if let Some(p) = run_with_timeout_capture(2.0, "bash", &["-lc", "bun pm cache 2>/dev/null"])
        {
            let t = p.trim();
            if !t.is_empty() && t.starts_with('/') {
                bun_cache_path = t.to_string();
            }
        }
        if Path::new(&bun_cache_path).is_dir() {
            bun_before = get_path_size_kb(&bun_cache_path);
        }
        let bun_cleaned = if is_path_whitelisted_from_global(&bun_cache_path) {
            if is_dry_run() {
                log_info("bun cache · would skip (whitelist)");
            } else {
                log_info("bun cache · skipped (whitelist)");
            }
            true
        } else if !is_dry_run() {
            let ok =
                run_with_timeout(10.0, "bash", &["-lc", "bun pm cache rm >/dev/null 2>&1"]) == 0;
            if ok {
                let bun_after = if Path::new(&bun_cache_path).is_dir() {
                    get_path_size_kb(&bun_cache_path)
                } else {
                    0
                };
                let freed = bun_before.saturating_sub(bun_after);
                if freed > 0 {
                    total_kb = total_kb.saturating_add(freed);
                    total_cnt += 1;
                }
                log_info("bun cache");
            }
            ok
        } else {
            log_info(&format!(
                "bun cache · would clean ({})",
                bytes_to_human(bun_before.saturating_mul(1024))
            ));
            total_kb = total_kb.saturating_add(bun_before);
            if bun_before > 0 {
                total_cnt += 1;
            }
            true
        };

        if normalize_dir_key(&bun_cache_path) != normalize_dir_key(&bun_default) {
            let (kb, cnt) = safe_clean(&[&format!("{bun_default}/*")], "Orphaned bun cache");
            total_kb = total_kb.saturating_add(kb);
            total_cnt = total_cnt.saturating_add(cnt);
        }
        if !bun_cleaned {
            let (kb, cnt) = safe_clean(&[&format!("{bun_cache_path}/*")], "Bun cache");
            total_kb = total_kb.saturating_add(kb);
            total_cnt = total_cnt.saturating_add(cnt);
        }
    } else {
        let (kb, cnt) = safe_clean(&[&format!("{bun_default}/*")], "Bun cache");
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }

    note_activity();
    for (g, l) in [
        (&format!("{h}/.tnpm/_cacache/*"), "tnpm cache directory"),
        (&format!("{h}/.tnpm/_logs/*"), "tnpm logs"),
        (&format!("{h}/.yarn/cache/*"), "Yarn cache"),
        (&format!("{h}/Library/Caches/Yarn/*"), "Yarn v1 cache"),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }

    (total_kb, total_cnt)
}

/// 对齐 SH `clean_uv_cache()` 第 60-72 行。
pub fn clean_uv_cache() -> (u64, u64) {
    let h = home_dir();
    let mut uv_cache_path = format!("{h}/.cache/uv");

    if command_available("uv")
        && run_with_timeout(2.0, "bash", &["-lc", "uv --version >/dev/null 2>&1"]) == 0
    {
        if let Some(p) = run_with_timeout_capture(2.0, "bash", &["-lc", "uv cache dir 2>/dev/null"])
        {
            let t = p.trim();
            if !t.is_empty() && t.starts_with('/') {
                uv_cache_path = t.to_string();
            }
        }
        run_clean_tool_cache("uv cache", Some(&uv_cache_path), "uv", &["cache", "prune"])
    } else {
        safe_clean(&[&format!("{uv_cache_path}/*")], "uv cache")
    }
}

/// 对齐 SH `conda_cache_whitelisted()` 第 74-83 行。
fn conda_cache_whitelisted(roots: &[&str]) -> bool {
    for root in roots {
        if !root.is_empty()
            && (is_path_whitelisted_from_global(root)
                || is_path_whitelisted_from_global(&format!("{root}/.mole-cache-guard")))
        {
            return true;
        }
    }
    false
}

pub fn clean_conda_metadata_caches() -> (u64, u64) {
    let h = home_dir();
    let conda_pkg_roots = [
        format!("{h}/.conda/pkgs"),
        format!("{h}/anaconda3/pkgs"),
        format!("{h}/miniconda3/pkgs"),
        format!("{h}/miniforge3/pkgs"),
        format!("{h}/mambaforge/pkgs"),
    ];

    if conda_cache_whitelisted(
        &conda_pkg_roots
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>(),
    ) {
        if is_dry_run() {
            log_info("conda index/tarball/log caches · would skip (whitelist)");
        } else {
            log_info("conda index/tarball/log caches · skipped (whitelist)");
        }
        return (0, 0);
    }

    let conda_cache_hint = format!("{h}/.conda/pkgs");
    if command_available("conda")
        && run_with_timeout(2.0, "bash", &["-lc", "conda --version >/dev/null 2>&1"]) == 0
    {
        let result = run_clean_tool_cache(
            "conda index/tarball/log caches",
            Some(&conda_cache_hint),
            "bash",
            &[
                "-lc",
                "run_with_timeout 30 conda clean --yes --index-cache --tarballs --logfiles",
            ],
        );
        note_activity();
        return result;
    }

    for root in &conda_pkg_roots {
        if Path::new(root).is_dir() {
            debug_log(&format!(
                "Conda package cache present but conda is unavailable, leaving for manual review: {root}"
            ));
        }
    }
    (0, 0)
}

pub fn clean_dev_python() -> (u64, u64) {
    let h = home_dir();
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    if command_available("pip3")
        && run_with_timeout(2.0, "bash", &["-lc", "pip3 --version >/dev/null 2>&1"]) == 0
    {
        let mut pip_cache = format!("{h}/Library/Caches/pip");
        if let Some(p) =
            run_with_timeout_capture(2.0, "bash", &["-lc", "pip3 cache dir 2>/dev/null"])
        {
            let t = p.trim();
            if !t.is_empty() && t.starts_with('/') {
                pip_cache = t.to_string();
            }
        }
        let (kb, cnt) = run_clean_tool_cache(
            "pip cache",
            Some(&pip_cache),
            "bash",
            &["-lc", "pip3 cache purge >/dev/null 2>&1 || true"],
        );
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
        note_activity();
    }

    for (g, l) in [
        ("~/.pyenv/cache/*", "pyenv cache"),
        ("~/.cache/poetry/*", "Poetry cache"),
        ("~/.cache/ruff/*", "Ruff cache"),
        ("~/.cache/mypy/*", "MyPy cache"),
        ("~/.pytest_cache/*", "Pytest cache"),
        ("~/.jupyter/runtime/*", "Jupyter runtime cache"),
        ("~/.cache/huggingface/*", "Hugging Face cache"),
        ("~/.cache/torch/*", "PyTorch cache"),
        ("~/.cache/tensorflow/*", "TensorFlow cache"),
        ("~/.cache/wandb/*", "Weights & Biases cache"),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    let (kb, cnt) = clean_uv_cache();
    total_kb = total_kb.saturating_add(kb);
    total_cnt = total_cnt.saturating_add(cnt);
    let (kb, cnt) = clean_conda_metadata_caches();
    total_kb = total_kb.saturating_add(kb);
    total_cnt = total_cnt.saturating_add(cnt);
    (total_kb, total_cnt)
}

// ----------------------------------------------------------------------------- go

pub fn clean_dev_go() -> (u64, u64) {
    if !command_available("go") {
        return (0, 0);
    }
    let h = home_dir();
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    let go_build_cache =
        run_with_timeout_capture(2.0, "bash", &["-lc", "go env GOCACHE 2>/dev/null"])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.starts_with('/'))
            .unwrap_or_else(|| format!("{h}/Library/Caches/go-build"));
    let go_mod_cache =
        run_with_timeout_capture(2.0, "bash", &["-lc", "go env GOMODCACHE 2>/dev/null"])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.starts_with('/'))
            .unwrap_or_else(|| format!("{h}/go/pkg/mod"));

    let build_prot = is_path_whitelisted_from_global(&go_build_cache);
    let mod_prot = is_path_whitelisted_from_global(&go_mod_cache);

    if build_prot && mod_prot {
        if is_dry_run() {
            log_info("Go cache · would skip (whitelist)");
        } else {
            log_info("Go cache · skipped (whitelist)");
        }
        return (0, 0);
    }

    if !build_prot && !mod_prot {
        let b_before = if Path::new(&go_build_cache).is_dir() {
            get_path_size_kb(&go_build_cache)
        } else {
            0
        };
        let m_before = if Path::new(&go_mod_cache).is_dir() {
            get_path_size_kb(&go_mod_cache)
        } else {
            0
        };
        run_clean_tool_cache(
            "Go cache",
            None,
            "bash",
            &[
                "-lc",
                "go clean -modcache >/dev/null 2>&1 || true; go clean -cache >/dev/null 2>&1 || true",
            ],
        );
        let b_after = if Path::new(&go_build_cache).is_dir() {
            get_path_size_kb(&go_build_cache)
        } else {
            0
        };
        let m_after = if Path::new(&go_mod_cache).is_dir() {
            get_path_size_kb(&go_mod_cache)
        } else {
            0
        };
        let freed_kb = b_before
            .saturating_sub(b_after)
            .saturating_add(m_before.saturating_sub(m_after));
        total_kb = total_kb.saturating_add(freed_kb);
        if freed_kb > 0 {
            total_cnt += 1;
        }
    } else if build_prot {
        let (kb, cnt) = run_clean_tool_cache(
            "Go module cache",
            Some(&go_mod_cache),
            "bash",
            &["-lc", "go clean -modcache >/dev/null 2>&1 || true"],
        );
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
        log_info("Go build cache · skipped (whitelist)");
    } else {
        let (kb, cnt) = run_clean_tool_cache(
            "Go build cache",
            Some(&go_build_cache),
            "bash",
            &["-lc", "go clean -cache >/dev/null 2>&1 || true"],
        );
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
        log_info("Go module cache · skipped (whitelist)");
    }
    note_activity();
    (total_kb, total_cnt)
}

// ----------------------------------------------------------------------------- mise

/// 对齐 SH `get_mise_cache_path()` 第 237-253 行。
pub fn get_mise_cache_path() -> String {
    if let Ok(v) = std::env::var("MISE_CACHE_DIR") {
        let t = v.trim();
        if !t.is_empty() && t.starts_with('/') {
            return t.to_string();
        }
    }
    if command_available("mise") {
        if let Some(p) =
            run_with_timeout_capture(2.0, "bash", &["-lc", "mise cache path 2>/dev/null"])
        {
            let t = p.trim();
            if !t.is_empty() && t.starts_with('/') {
                return t.to_string();
            }
        }
    }
    format!("{}/Library/Caches/mise", home_dir())
}

/// 对齐 SH `clean_dev_mise()` 第 330-348 行。
///
/// SH 中 `safe_clean "$mise_cache_path"/*` 在 `if command -v mise` 块**外面**，
/// 无论 mise 是否可用、是否 dry_run 都会执行，做兜底文件系统级清理。
pub fn clean_dev_mise() -> (u64, u64) {
    let mise_cache_path = get_mise_cache_path();
    let mut tool_kb = 0u64;
    let mut tool_cnt = 0u64;

    if command_available("mise") {
        if !is_dry_run() {
            let result = run_clean_tool_cache(
                "mise cache",
                Some(&mise_cache_path),
                "bash",
                &["-lc", "mise cache clear >/dev/null 2>&1 || true"],
            );
            tool_kb = result.0;
            tool_cnt = result.1;
            note_activity();
        } else if is_path_whitelisted_from_global(&mise_cache_path) {
            log_info("mise cache · would skip (whitelist)");
            note_activity();
        } else {
            log_info("mise cache · would clean");
            note_activity();
        }
    }

    let (kb, cnt) = safe_clean(&[&format!("{mise_cache_path}/*")], "mise cache");
    (tool_kb.saturating_add(kb), tool_cnt.saturating_add(cnt))
}

// ----------------------------------------------------------------------------- rust

/// 对齐 SH `clean_dev_rust()` 第 275-279 行。
pub fn clean_dev_rust() -> (u64, u64) {
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    for (g, l) in [
        ("~/.cargo/registry/cache/*", "Rust cargo cache"),
        ("~/.cargo/git/*", "Cargo git cache"),
        ("~/.rustup/downloads/*", "Rust downloads cache"),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    (total_kb, total_cnt)
}

/// 对齐 SH `clean_dev_ruby()` 第 280-285 行。
pub fn clean_dev_ruby() -> (u64, u64) {
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    for (g, l) in [
        ("~/.rbenv/cache/*", "rbenv download cache"),
        ("~/.gem/specs/*", "gem spec cache"),
        ("~/.gem/ruby/*/cache/*.gem", "gem package cache"),
        ("~/.bundle/cache/*", "Ruby Bundler cache"),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    (total_kb, total_cnt)
}

/// 对齐 SH `clean_dev_perl()` 第 287-290 行。
pub fn clean_dev_perl() -> (u64, u64) {
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    for (g, l) in [
        ("~/.cpan/build/*", "CPAN build artifacts"),
        ("~/.cpan/sources/*", "CPAN source cache"),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    (total_kb, total_cnt)
}

// ----------------------------------------------------------------------------- checks

/// 对齐 SH `check_multiple_versions()` 第 283-304 行。
pub fn check_multiple_versions(dir: &str, tool_name: &str, list_cmd: &str, _remove_cmd: &str) {
    if !Path::new(dir).is_dir() {
        return;
    }
    let count = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .count()
        })
        .unwrap_or(0);
    if count > 1 {
        note_activity();
        let hint = if list_cmd.is_empty() {
            String::new()
        } else {
            format!(" · {list_cmd}")
        };
        log_info(&format!("{tool_name}: {count} found{hint}"));
    }
}

/// 对齐 SH `check_rust_toolchains()` 第 307-314 行。
pub fn check_rust_toolchains() {
    if !command_available("rustup") {
        return;
    }
    let dir = format!("{}/.rustup/toolchains", home_dir());
    check_multiple_versions(&dir, "Rust toolchains", "rustup toolchain list", "");
}

/// 对齐 SH `check_android_ndk()` 第 362-367 行。
pub fn check_android_ndk() {
    let dir = format!("{}/Library/Android/sdk/ndk", home_dir());
    check_multiple_versions(
        &dir,
        "Android NDK versions",
        "Android Studio → SDK Manager",
        "",
    );
}

// ----------------------------------------------------------------------------- docker / nix / cloud / frontend

/// 对齐 SH `clean_dev_docker()` 第 316-325 行。
pub fn clean_dev_docker() -> (u64, u64) {
    if command_available("docker") {
        note_activity();
        log_warning("Docker unused data · skipped by default");
        log_info("Review: docker system df");
        log_info("Prune:  docker system prune --filter until=720h");
        debug_log("Docker daemon-managed cleanup skipped by default");
    }
    safe_clean(&["~/.docker/buildx/cache/*"], "Docker BuildX cache")
}

/// 对齐 SH `clean_dev_nix()` 第 327-338 行。
pub fn clean_dev_nix() -> (u64, u64) {
    if !command_available("nix-collect-garbage") {
        return (0, 0);
    }
    if !is_dry_run() {
        let result = run_clean_tool_cache(
            "Nix garbage collection",
            Some("/nix/store"),
            "nix-collect-garbage",
            &["--delete-older-than", "30d"],
        );
        note_activity();
        result
    } else if is_path_whitelisted_from_global("/nix/store") {
        log_info("Nix garbage collection · would skip (whitelist)");
        note_activity();
        (0, 0)
    } else {
        log_info("Nix garbage collection · would clean");
        note_activity();
        (0, 0)
    }
}

/// 对齐 SH `clean_dev_cloud()` 第 340-346 行。
pub fn clean_dev_cloud() -> (u64, u64) {
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    for (g, l) in [
        ("~/.kube/cache/*", "Kubernetes cache"),
        (
            "~/.local/share/containers/storage/tmp/*",
            "Container storage temp",
        ),
        ("~/.aws/cli/cache/*", "AWS CLI cache"),
        ("~/.config/gcloud/logs/*", "Google Cloud logs"),
        ("~/.azure/logs/*", "Azure CLI logs"),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    (total_kb, total_cnt)
}

/// 对齐 SH `clean_dev_frontend()` 第 348-360 行。
pub fn clean_dev_frontend() -> (u64, u64) {
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    for (g, l) in [
        ("~/.cache/typescript/*", "TypeScript cache"),
        ("~/.cache/electron/*", "Electron cache"),
        ("~/.cache/node-gyp/*", "node-gyp cache"),
        ("~/.node-gyp/*", "node-gyp build cache"),
        ("~/.turbo/cache/*", "Turbo cache"),
        ("~/.vite/cache/*", "Vite cache"),
        ("~/.cache/vite/*", "Vite global cache"),
        ("~/.cache/webpack/*", "Webpack cache"),
        ("~/.parcel-cache/*", "Parcel cache"),
        ("~/.cache/eslint/*", "ESLint cache"),
        ("~/.cache/prettier/*", "Prettier cache"),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    (total_kb, total_cnt)
}

// ----------------------------------------------------------------------------- Xcode documentation / device support / sim runtime

fn documentation_cache_root() -> String {
    std::env::var("MOLE_XCODE_DOCUMENTATION_CACHE_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/Library/Developer/Xcode/DocumentationCache".to_string())
}

fn find_doc_index_entries(root: &str) -> Vec<String> {
    let mut v = Vec::new();
    let rp = Path::new(root);
    if !rp.is_dir() {
        return v;
    }
    if let Ok(rd) = std::fs::read_dir(rp) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name == "DeveloperDocumentation.index"
                || (name.starts_with("DeveloperDocumentation") && name.ends_with(".index"))
            {
                v.push(p.to_string_lossy().to_string());
            }
        }
    }
    v
}

/// 对齐 SH `clean_xcode_documentation_cache()` 第 369-456 行。
pub fn clean_xcode_documentation_cache() {
    let doc_cache_root = documentation_cache_root();
    if !Path::new(&doc_cache_root).is_dir() {
        return;
    }
    if pgrep_x("Xcode") {
        log_warning("Xcode is running, skipping documentation cache cleanup");
        note_activity();
        return;
    }

    let mut index_entries = find_doc_index_entries(&doc_cache_root);
    if index_entries.len() <= 1 {
        return;
    }

    index_entries.sort_by_key(|p| std::cmp::Reverse(get_file_mtime(p)));
    let stale: Vec<String> = index_entries.into_iter().skip(1).collect();
    if stale.is_empty() {
        return;
    }

    if is_dry_run() {
        let refs: Vec<&str> = stale.iter().map(|s| s.as_str()).collect();
        let _ = safe_clean(&refs, "Xcode documentation cache (old indexes)");
        note_activity();
        return;
    }

    if !ensure_admin_session() {
        log_warning("Xcode documentation cache cleanup skipped (sudo denied)");
        note_activity();
        return;
    }

    let mut removed = 0u32;
    let mut skipped = 0u32;
    for stale_entry in &stale {
        if should_protect_path(stale_entry) || is_path_whitelisted_from_global(stale_entry) {
            skipped += 1;
            continue;
        }
        if safe_sudo_remove(stale_entry, None) == MOLE_OK {
            removed += 1;
        }
    }
    if removed > 0 {
        log_info(&format!(
            "Xcode documentation cache · removed {removed} old indexes"
        ));
        if skipped > 0 {
            log_warning(&format!(
                "Xcode documentation cache · skipped {skipped} protected items"
            ));
        }
        note_activity();
    } else if skipped > 0 {
        log_info("Xcode documentation cache · nothing to clean");
        log_warning(&format!(
            "Xcode documentation cache · skipped {skipped} protected items"
        ));
        note_activity();
    } else {
        log_warning("Xcode documentation cache · no items removed");
        note_activity();
    }
}

/// 对齐 SH `clean_xcode_device_support()` 第 462-530 行。
fn clean_xcode_device_support_dir(ds_dir: &str, display_name: &str) {
    let keep: usize = std::env::var("MOLE_XCODE_DEVICE_SUPPORT_KEEP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);

    if !Path::new(ds_dir).is_dir() {
        return;
    }

    let mut version_dirs: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(ds_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                version_dirs.push(p);
            }
        }
    }
    if version_dirs.is_empty() {
        return;
    }

    version_dirs.sort_by_key(|p| std::cmp::Reverse(get_file_mtime(&p.to_string_lossy())));
    let stale: Vec<PathBuf> = version_dirs.into_iter().skip(keep).collect();

    if !stale.is_empty() {
        let mut stale_kb: u64 = 0;
        for p in &stale {
            let s = p.to_string_lossy().to_string();
            if is_dry_run() && !dry_run_register_cleanup_target(&s) {
                continue;
            }
            stale_kb = stale_kb.saturating_add(get_path_size_kb(&s));
        }
        let human = bytes_to_human(stale_kb.saturating_mul(1024));
        if is_dry_run() {
            log_info(&format!(
                "{display_name} · would remove {} old versions ({human}), keeping {keep} most recent",
                stale.len()
            ));
            note_activity();
            return;
        }
        let mut removed = 0u32;
        for p in &stale {
            let s = p.to_string_lossy().to_string();
            if should_protect_path(&s) || is_path_whitelisted_from_global(&s) {
                continue;
            }
            if safe_remove(&s, true) {
                removed += 1;
            }
        }
        if removed > 0 {
            log_info(&format!(
                "{display_name} · removed {removed} old versions, {human}"
            ));
            note_activity();
        }
    }

    let sym_glob = format!("{ds_dir}/*/Symbols/System/Library/Caches/*");
    let _ = safe_clean(&[&sym_glob], &format!("{display_name} symbol cache"));
    let log_glob = format!("{ds_dir}/*.log");
    let _ = safe_clean(&[&log_glob], &format!("{display_name} logs"));
}

/// 对齐 SH 对三平台 DeviceSupport 的调用(第 911-913 行)。
pub fn clean_xcode_device_support() {
    let h = home_dir();
    clean_xcode_device_support_dir(
        &format!("{h}/Library/Developer/Xcode/iOS DeviceSupport"),
        "iOS DeviceSupport",
    );
    clean_xcode_device_support_dir(
        &format!("{h}/Library/Developer/Xcode/watchOS DeviceSupport"),
        "watchOS DeviceSupport",
    );
    clean_xcode_device_support_dir(
        &format!("{h}/Library/Developer/Xcode/tvOS DeviceSupport"),
        "tvOS DeviceSupport",
    );
}

fn sim_runtime_mount_points() -> Vec<String> {
    if let Ok(env) = std::env::var("MOLE_XCODE_SIM_RUNTIME_MOUNT_POINTS") {
        let v: Vec<String> = env
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        if !v.is_empty() {
            return v;
        }
    }
    let out = Command::new("mount")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let mut pts = Vec::new();
    for line in out.lines() {
        let mut it = line.split_whitespace();
        let _dev = it.next();
        let _on = it.next();
        if let Some(mp) = it.next() {
            if !mp.is_empty() {
                pts.push(mp.to_string());
            }
        }
    }
    pts
}

fn sim_runtime_is_path_in_use(target: &str, mounts: &[String]) -> bool {
    for m in mounts {
        if m.is_empty() {
            continue;
        }
        if m == target || target.starts_with(&format!("{m}/")) {
            return true;
        }
    }
    false
}

fn sim_runtime_size_kb(path: &str) -> u64 {
    if is_admin_authorized() {
        String::from_utf8_lossy(&sudo_output(&["/usr/bin/du", "-skP", path]).stdout)
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    } else {
        get_path_size_kb(path)
    }
}

fn sim_runtime_volumes_root() -> String {
    std::env::var("MOLE_XCODE_SIM_RUNTIME_VOLUMES_ROOT")
        .unwrap_or_else(|_| "/Library/Developer/CoreSimulator/Volumes".to_string())
}

fn sim_runtime_cryptex_root() -> String {
    std::env::var("MOLE_XCODE_SIM_RUNTIME_CRYPTEX_ROOT")
        .unwrap_or_else(|_| "/Library/Developer/CoreSimulator/Cryptex".to_string())
}

/// 对齐 SH `clean_xcode_simulator_runtime_volumes()` 第 566-752 行。
pub fn clean_xcode_simulator_runtime_volumes() {
    let volumes_root = sim_runtime_volumes_root();
    let cryptex_root = sim_runtime_cryptex_root();
    let mut candidates: Vec<String> = Vec::new();
    for root in [&volumes_root, &cryptex_root] {
        if !Path::new(root).is_dir() {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(root) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    candidates.push(p.to_string_lossy().to_string());
                }
            }
        }
    }
    if candidates.is_empty() {
        return;
    }
    candidates.sort();
    let mount_points = sim_runtime_mount_points();
    let mut statuses: Vec<&'static str> = Vec::with_capacity(candidates.len());
    let mut in_use = 0u64;
    let mut unused = 0u64;
    for c in &candidates {
        if !mount_points.is_empty() && sim_runtime_is_path_in_use(c, &mount_points) {
            statuses.push("IN_USE");
            in_use += 1;
        } else {
            statuses.push("UNUSED");
            unused += 1;
        }
    }

    if is_dry_run() {
        let mut size_vals: Vec<u64> = Vec::new();
        let mut in_use_kb = 0u64;
        let mut unused_kb = 0u64;
        for (i, c) in candidates.iter().enumerate() {
            let sz_raw = sim_runtime_size_kb(c);
            let sz = if dry_run_register_cleanup_target(c) {
                sz_raw
            } else {
                0
            };
            size_vals.push(sz);
            if statuses.get(i).copied() == Some("IN_USE") {
                in_use_kb = in_use_kb.saturating_add(sz);
            } else {
                unused_kb = unused_kb.saturating_add(sz);
            }
        }
        log_info(&format!(
            "Xcode runtime volumes · {unused} unused, {in_use} in use"
        ));
        let total_kb = unused_kb.saturating_add(in_use_kb);
        log_info(&format!(
            "Runtime volumes total: {} (unused {}, in-use {})",
            bytes_to_human(total_kb.saturating_mul(1024)),
            bytes_to_human(unused_kb.saturating_mul(1024)),
            bytes_to_human(in_use_kb.saturating_mul(1024)),
        ));
        let max_items: usize = std::env::var("MOLE_SIM_RUNTIME_DRYRUN_MAX_ITEMS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&n: &usize| n > 0)
            .unwrap_or(20);
        let mut rows: Vec<(u64, String, String)> = Vec::new();
        for (i, c) in candidates.iter().enumerate() {
            let sz = size_vals.get(i).copied().unwrap_or(0);
            let st = statuses.get(i).copied().unwrap_or("UNUSED").to_string();
            rows.push((sz, st, c.clone()));
        }
        rows.sort_by(|a, b| b.0.cmp(&a.0));
        for (j, (sz, st, path)) in rows.iter().enumerate() {
            if j >= max_items {
                break;
            }
            log_info(&format!(
                "    {st} {} · {path}",
                bytes_to_human(sz.saturating_mul(1024))
            ));
        }
        if candidates.len() > max_items {
            log_info(&format!(
                "... and {} more runtime volume entries",
                candidates.len() - max_items
            ));
        }
        note_activity();
        return;
    }

    let mut selected: Vec<String> = Vec::new();
    let mut skipped_prot = 0u32;
    for (i, c) in candidates.iter().enumerate() {
        if statuses.get(i).copied() == Some("IN_USE") {
            continue;
        }
        if should_protect_path(c) || is_path_whitelisted_from_global(c) {
            skipped_prot += 1;
            continue;
        }
        selected.push(c.clone());
    }
    if selected.is_empty() {
        log_info("Xcode runtime volumes · already clean");
        note_activity();
        return;
    }
    if !ensure_admin_session() {
        log_warning("Xcode runtime volumes · skipped (sudo denied)");
        note_activity();
        return;
    }
    let mut removed = 0u32;
    let mut removed_kb = 0u64;
    for p in selected {
        let sz = sim_runtime_size_kb(&p);
        if safe_sudo_remove(&p, Some(sz)) == MOLE_OK {
            removed += 1;
            removed_kb = removed_kb.saturating_add(sz);
        }
    }
    if removed > 0 {
        let h = bytes_to_human(removed_kb.saturating_mul(1024));
        if skipped_prot > 0 {
            log_info(&format!(
                "Xcode runtime volumes · removed {removed} ({h}), skipped {skipped_prot} protected"
            ));
        } else {
            log_info(&format!("Xcode runtime volumes · removed {removed} ({h})"));
        }
        note_activity();
    } else if skipped_prot > 0 {
        log_warning(&format!(
            "Xcode runtime volumes · skipped {skipped_prot} protected, none removed"
        ));
        note_activity();
    } else {
        log_info("Xcode runtime volumes · already clean");
        note_activity();
    }
}

fn is_uuid_segment(s: &str) -> bool {
    s.len() == 36
        && s.chars().filter(|c| *c == '-').count() == 4
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

fn simctl_unavailable_udids(listing: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in listing.lines() {
        if !line.contains("(unavailable") {
            continue;
        }
        for part in line.split('(').skip(1) {
            if let Some(close) = part.find(')') {
                let inner = &part[..close];
                if is_uuid_segment(inner) && !out.contains(&inner.to_string()) {
                    out.push(inner.to_string());
                }
            }
        }
    }
    out
}

fn count_unavailable_lines(listing: &str) -> u64 {
    listing
        .lines()
        .filter(|l| l.contains("(unavailable"))
        .count() as u64
}

/// 对齐 SH `clean_dev_mobile()` 第 754-932 行。
pub fn clean_dev_mobile() -> (u64, u64) {
    check_android_ndk();
    clean_xcode_documentation_cache();
    clean_xcode_simulator_runtime_volumes();

    let h = home_dir();
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    if command_available("xcrun") {
        // 先静默探测 simctl 可用性:无完整 Xcode 时 `xcrun simctl` 必失败(exit 72),
        // 原实现在扫描期会执行两次注定失败的探测并把 raw stderr 喷到控制台;
        // 不可用时静默跳过,仅留 debug 日志。
        if !simctl_available() {
            debug_log(
                "simctl unavailable · skip unavailable-simulator cleanup (full Xcode not found)",
            );
        } else {
            let listing = run_with_timeout_capture_silent(
                5.0,
                "xcrun",
                &["simctl", "list", "devices", "unavailable"],
            )
            .unwrap_or_default();
            let unavailable_before = count_unavailable_lines(&listing);
            let udids = simctl_unavailable_udids(&listing);
            let mut unavailable_size_kb = 0u64;
            for u in &udids {
                let p = format!("{h}/Library/Developer/CoreSimulator/Devices/{u}");
                if Path::new(&p).is_dir() {
                    let kb = get_path_size_kb(&p);
                    if is_dry_run() {
                        if dry_run_register_cleanup_target(&p) {
                            unavailable_size_kb = unavailable_size_kb.saturating_add(kb);
                        }
                    } else {
                        unavailable_size_kb = unavailable_size_kb.saturating_add(kb);
                    }
                }
            }
            let unavailable_size_human = bytes_to_human(unavailable_size_kb.saturating_mul(1024));

            if is_dry_run() {
                if unavailable_before > 0 {
                    log_info(&format!(
                        "Xcode unavailable simulators · would clean {unavailable_before}, {unavailable_size_human}"
                    ));
                } else {
                    log_info("Xcode unavailable simulators · already clean");
                }
            } else if unavailable_before == 0 {
                log_info("Xcode unavailable simulators · already clean");
                note_activity();
            } else {
                let del_result = Command::new("xcrun")
                    .args(["simctl", "delete", "unavailable"])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output();
                let delete_ok = del_result
                    .as_ref()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                let del_out = del_result
                    .map(|o| {
                        String::from_utf8_lossy(&o.stdout).to_string()
                            + &String::from_utf8_lossy(&o.stderr)
                    })
                    .unwrap_or_default();

                if delete_ok {
                    let listing2 = run_with_timeout_capture_silent(
                        5.0,
                        "xcrun",
                        &["simctl", "list", "devices", "unavailable"],
                    )
                    .unwrap_or_default();
                    let unavailable_after = count_unavailable_lines(&listing2);
                    let removed = unavailable_before.saturating_sub(unavailable_after);
                    if removed > 0 {
                        log_info(&format!(
                            "Xcode unavailable simulators · removed {removed}, {unavailable_size_human}"
                        ));
                    } else {
                        log_info(&format!(
                            "Xcode unavailable simulators · cleanup completed, {unavailable_size_human}"
                        ));
                    }
                } else {
                    let mut hint = String::new();
                    let lo = del_out.to_lowercase();
                    if lo.contains("permission denied") {
                        hint = " (permission denied)".to_string();
                    } else if lo.contains("in use") || lo.contains("busy") {
                        hint = " (device in use)".to_string();
                    } else if lo.contains("unable to boot") || lo.contains("failed to boot") {
                        hint = " (boot failure)".to_string();
                    } else if lo.contains("service") {
                        hint = " (CoreSimulator service issue)".to_string();
                    }
                    if !udids.is_empty() {
                        debug_log("Attempting fallback: manual deletion of unavailable simulators");
                        let mut manually_removed = 0u32;
                        let mut manual_failed = 0u32;
                        for udid in &udids {
                            if !is_uuid_segment(udid) {
                                manual_failed += 1;
                                continue;
                            }
                            let device_path =
                                format!("{h}/Library/Developer/CoreSimulator/Devices/{udid}");
                            if Path::new(&device_path).is_dir() {
                                if safe_remove(&device_path, true) {
                                    manually_removed += 1;
                                    debug_log(&format!("Manually removed simulator: {udid}"));
                                } else {
                                    manual_failed += 1;
                                    debug_log(&format!(
                                        "Failed to manually remove simulator: {udid}"
                                    ));
                                }
                            }
                        }
                        if manually_removed > 0 {
                            if manual_failed == 0 {
                                log_info(&format!(
                                    "Xcode unavailable simulators · removed {manually_removed} (fallback), {unavailable_size_human}"
                                ));
                            } else {
                                log_warning(&format!(
                                    "Xcode unavailable simulators · partially cleaned {manually_removed}/{}, {unavailable_size_human}",
                                    udids.len()
                                ));
                            }
                        } else {
                            log_warning(&format!(
                                "Xcode unavailable simulators cleanup failed{hint}"
                            ));
                            debug_log(&format!("simctl delete error: {del_out}"));
                        }
                    } else {
                        log_warning(&format!(
                            "Xcode unavailable simulators cleanup failed{hint}"
                        ));
                    }
                }
                note_activity();
            }
        }
    }

    clean_xcode_device_support();

    let jobs: &[(&str, &str)] = &[
        (
            "~/Library/Developer/CoreSimulator/Profiles/Runtimes/*/Contents/Resources/RuntimeRoot/System/Library/Caches/*",
            "Simulator runtime cache",
        ),
        (
            "~/Library/Caches/Google/AndroidStudio*/*",
            "Android Studio cache",
        ),
        ("~/.android/build-cache/*", "Android build cache"),
        ("~/.android/cache/*", "Android SDK cache"),
        (
            "~/Library/Developer/Xcode/UserData/IB Support/*",
            "Xcode Interface Builder cache",
        ),
        (
            "~/.cache/swift-package-manager/*",
            "Swift package manager cache",
        ),
        (
            "~/Library/Caches/org.swift.swiftpm/*",
            "Swift package manager library cache",
        ),
        ("~/.expo/expo-go/*", "Expo Go cache"),
        ("~/.expo/android-apk-cache/*", "Expo Android APK cache"),
        (
            "~/.expo/ios-simulator-app-cache/*",
            "Expo iOS simulator app cache",
        ),
        (
            "~/.expo/native-modules-cache/*",
            "Expo native modules cache",
        ),
        ("~/.expo/schema-cache/*", "Expo schema cache"),
        ("~/.expo/template-cache/*", "Expo template cache"),
        ("~/.expo/versions-cache/*", "Expo versions cache"),
    ];
    for (g, l) in jobs {
        let (kb, cnt) = safe_clean(&[*g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    (total_kb, total_cnt)
}

// ----------------------------------------------------------------------------- JVM / JetBrains / AI / misc

/// 对齐 SH `gradle_daemon_running()` 第 117-121 行。
fn gradle_daemon_running() -> bool {
    run_with_timeout(2.0, "pgrep", &["-f", "org.gradle.launcher.daemon"]) == 0
        || run_with_timeout(2.0, "pgrep", &["-f", "GradleDaemon"]) == 0
}

/// 对齐 SH `clean_dev_jvm()` 第 948-1051 行。
pub fn clean_dev_jvm() -> (u64, u64) {
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    let (kb, cnt) = clean_maven_repository();
    total_kb = total_kb.saturating_add(kb);
    total_cnt = total_cnt.saturating_add(cnt);
    for (g, l) in [
        ("~/.sbt/boot/*", "SBT boot cache"),
        ("~/.sbt/launchers/*", "SBT launcher cache"),
        ("~/.ivy2/cache/*", "Ivy cache"),
        ("~/.gradle/caches/build-cache-*/*", "Gradle build cache"),
        ("~/.gradle/notifications/*", "Gradle notifications cache"),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    if gradle_daemon_running() {
        log_warning("Gradle daemon is running · daemon/workers cleanup skipped");
    } else {
        for (g, l) in [
            ("~/.gradle/daemon/*", "Gradle daemon"),
            ("~/.gradle/workers/*", "Gradle workers"),
        ] {
            let (kb, cnt) = safe_clean(&[g], l);
            total_kb = total_kb.saturating_add(kb);
            total_cnt = total_cnt.saturating_add(cnt);
        }
    }
    (total_kb, total_cnt)
}

struct RestoreGlobalWhitelist(Vec<String>);

impl Drop for RestoreGlobalWhitelist {
    fn drop(&mut self) {
        set_global_whitelist(std::mem::take(&mut self.0));
    }
}

/// 对齐 SH `clean_dev_jetbrains_toolbox()` 第 948-1051 行。
pub fn clean_dev_jetbrains_toolbox() -> (u64, u64) {
    let toolbox_root = format!(
        "{}/Library/Application Support/JetBrains/Toolbox/apps",
        home_dir()
    );
    if !Path::new(&toolbox_root).is_dir() {
        return (0, 0);
    }

    let keep_previous: usize = std::env::var("MOLE_JETBRAINS_TOOLBOX_KEEP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);

    let original = get_global_whitelist();
    let toolbox_norm = toolbox_root.trim_end_matches('/');
    let filtered: Vec<String> = original
        .iter()
        .filter(|p| {
            let p = p.trim_end_matches('/');
            !(p == toolbox_norm || p.starts_with(&format!("{toolbox_norm}/")))
        })
        .cloned()
        .collect();
    set_global_whitelist(filtered);
    let _whitelist_guard = RestoreGlobalWhitelist(original);

    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;

    if let Ok(product_dirs) = std::fs::read_dir(&toolbox_root) {
        for prod in product_dirs.flatten() {
            let product_dir = prod.path();
            if !product_dir.is_dir() {
                continue;
            }
            let pd_str = product_dir.to_string_lossy().to_string();
            let channels = match Command::new("find")
                .args([
                    &pd_str,
                    "-mindepth",
                    "1",
                    "-maxdepth",
                    "1",
                    "-type",
                    "d",
                    "-name",
                    "ch-*",
                    "-print0",
                ])
                .output()
            {
                Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
                    .split('\0')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            };
            for channel_dir_s in channels {
                let channel_dir = PathBuf::from(&channel_dir_s);
                let mut current_real: Option<PathBuf> = None;
                let cur_link = channel_dir.join("current");
                if cur_link.is_symlink() {
                    if let Ok(t) = std::fs::read_link(&cur_link) {
                        current_real = Some(if t.is_absolute() {
                            t
                        } else {
                            channel_dir.join(t)
                        });
                    }
                } else if cur_link.is_dir() {
                    current_real = Some(cur_link.clone());
                }

                let mut version_dirs: Vec<PathBuf> = Vec::new();
                if let Ok(rd) = std::fs::read_dir(&channel_dir) {
                    for e in rd.flatten() {
                        let p = e.path();
                        if !p.is_dir() {
                            continue;
                        }
                        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                        if name == "current" || name.starts_with('.') {
                            continue;
                        }
                        if matches!(name, "plugins" | "plugins-lib" | "plugins-libs") {
                            continue;
                        }
                        if let Some(cr) = &current_real {
                            if p == *cr {
                                continue;
                            }
                        }
                        if !name
                            .chars()
                            .next()
                            .map(|c| c.is_ascii_digit())
                            .unwrap_or(false)
                        {
                            continue;
                        }
                        version_dirs.push(p);
                    }
                }
                if version_dirs.is_empty() {
                    continue;
                }
                version_dirs
                    .sort_by_key(|p| std::cmp::Reverse(get_file_mtime(&p.to_string_lossy())));
                if version_dirs.len() <= keep_previous {
                    continue;
                }
                for (idx, p) in version_dirs.iter().enumerate() {
                    if idx < keep_previous {
                        continue;
                    }
                    let s = p.to_string_lossy().to_string();
                    let (kb, cnt) = safe_clean(&[&s], "JetBrains Toolbox old IDE version");
                    total_kb = total_kb.saturating_add(kb);
                    total_cnt = total_cnt.saturating_add(cnt);
                    note_activity();
                }
            }
        }
    }
    (total_kb, total_cnt)
}

/// 对齐 SH `clean_dev_jetbrains_logs()` 第 1055-1057 行。
pub fn clean_dev_jetbrains_logs() -> (u64, u64) {
    safe_clean(&["~/Library/Logs/JetBrains/*"], "JetBrains IDE logs")
}

/// 对齐 SH `clean_dev_ai_agents()` 第 1065-1148 行。
pub fn clean_dev_ai_agents() -> (u64, u64) {
    let keep: usize = std::env::var("MOLE_AI_AGENTS_KEEP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);

    let h = home_dir();
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    let specs: [(&str, &str, Option<&str>); 3] = [
        (
            &format!("{h}/.local/share/claude/versions"),
            "Claude Code old version",
            Some(&format!("{h}/.local/bin/claude")),
        ),
        (
            &format!("{h}/.local/share/cursor-agent/versions"),
            "Cursor Agent old version",
            Some(&format!("{h}/.local/bin/cursor-agent")),
        ),
        (
            &format!("{h}/.copilot/pkg/universal"),
            "GitHub Copilot CLI old version",
            None,
        ),
    ];

    for (versions_root, label, active_symlink) in specs {
        if !Path::new(versions_root).is_dir() {
            continue;
        }
        let mut active_path: Option<PathBuf> = None;
        if let Some(sym) = active_symlink {
            let sp = Path::new(sym);
            if sp.is_symlink() {
                if !sp.exists() {
                    log_warning(&format!(
                        "{label} active symlink is broken · skipping cleanup"
                    ));
                    continue;
                }
                if let Ok(target) = std::fs::read_link(sp) {
                    let resolved = if target.is_absolute() {
                        target
                    } else {
                        sp.parent().map(|p| p.join(&target)).unwrap_or(target)
                    };
                    if let Ok(rd) = std::fs::read_dir(versions_root) {
                        for e in rd.flatten() {
                            let ep = e.path();
                            let rs = resolved.to_string_lossy();
                            let es = ep.to_string_lossy();
                            if rs.starts_with(&format!("{es}/")) || rs == es.as_ref() {
                                active_path = Some(ep);
                                break;
                            }
                        }
                    }
                }
            }
        }

        let mut entries: Vec<PathBuf> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(versions_root) {
            for e in rd.flatten() {
                let p = e.path();
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name.starts_with('.') {
                    continue;
                }
                if !name
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false)
                {
                    continue;
                }
                if p.is_file() || p.is_dir() {
                    entries.push(p);
                }
            }
        }
        if entries.len() <= keep {
            continue;
        }
        entries.sort_by_key(|p| std::cmp::Reverse(get_file_mtime(&p.to_string_lossy())));
        let mut slot = 0usize;
        for target in &entries {
            if let Some(ap) = &active_path {
                if target == ap {
                    continue;
                }
            }
            if slot < keep {
                slot += 1;
                continue;
            }
            let s = target.to_string_lossy().to_string();
            let (kb, cnt) = safe_clean(&[&s], label);
            total_kb = total_kb.saturating_add(kb);
            total_cnt = total_cnt.saturating_add(cnt);
            note_activity();
            slot += 1;
        }
    }
    (total_kb, total_cnt)
}

/// 对齐 SH `clean_dev_other_langs()` 第 1151-1160 行。
pub fn clean_dev_other_langs() -> (u64, u64) {
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    for (g, l) in [
        ("~/.composer/cache/*", "PHP Composer cache (legacy)"),
        ("~/Library/Caches/composer/*", "PHP Composer cache"),
        ("~/.nuget/packages/*", "NuGet packages cache"),
        ("~/.cache/bazel/*", "Bazel cache"),
        ("~/.cache/zig/*", "Zig cache"),
        ("~/Library/Caches/deno/*", "Deno cache"),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    (total_kb, total_cnt)
}

/// 对齐 SH `clean_dev_cicd()` 第 1162-1171 行。
pub fn clean_dev_cicd() -> (u64, u64) {
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    for (g, l) in [
        ("~/.cache/terraform/*", "Terraform cache"),
        ("~/.grafana/cache/*", "Grafana cache"),
        ("~/.prometheus/data/wal/*", "Prometheus WAL cache"),
        ("~/.jenkins/workspace/*/target/*", "Jenkins workspace cache"),
        ("~/.cache/gitlab-runner/*", "GitLab Runner cache"),
        ("~/.github/cache/*", "GitHub Actions cache"),
        ("~/.circleci/cache/*", "CircleCI cache"),
        ("~/.sonar/*", "SonarQube cache"),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    (total_kb, total_cnt)
}

/// 对齐 SH `clean_dev_database()` 第 1173-1180 行。
pub fn clean_dev_database() -> (u64, u64) {
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    for (g, l) in [
        (
            "~/Library/Caches/com.sequel-ace.sequel-ace/*",
            "Sequel Ace cache",
        ),
        (
            "~/Library/Caches/com.eggerapps.Sequel-Pro/*",
            "Sequel Pro cache",
        ),
        (
            "~/Library/Caches/redis-desktop-manager/*",
            "Redis Desktop Manager cache",
        ),
        ("~/Library/Caches/com.navicat.*", "Navicat cache"),
        ("~/Library/Caches/com.dbeaver.*", "DBeaver cache"),
        (
            "~/Library/Caches/com.redis.RedisInsight",
            "Redis Insight cache",
        ),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    (total_kb, total_cnt)
}

/// 对齐 SH `clean_dev_api_tools()` 第 1182-1189 行。
pub fn clean_dev_api_tools() -> (u64, u64) {
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    for (g, l) in [
        ("~/Library/Caches/com.postmanlabs.mac/*", "Postman cache"),
        ("~/Library/Caches/com.konghq.insomnia/*", "Insomnia cache"),
        (
            "~/Library/Caches/com.tinyapp.TablePlus/*",
            "TablePlus cache",
        ),
        ("~/Library/Caches/com.getpaw.Paw/*", "Paw API cache"),
        (
            "~/Library/Caches/com.charlesproxy.charles/*",
            "Charles Proxy cache",
        ),
        ("~/Library/Caches/com.proxyman.NSProxy/*", "Proxyman cache"),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    (total_kb, total_cnt)
}

/// 对齐 SH `clean_dev_misc()` 第 1191-1256 行。
pub fn clean_dev_misc() -> (u64, u64) {
    let h = home_dir();
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    for (g, l) in [
        ("~/Library/Caches/com.unity3d.*/*", "Unity cache"),
        (
            "~/Library/Caches/com.mongodb.compass/*",
            "MongoDB Compass cache",
        ),
        ("~/Library/Caches/com.figma.Desktop/*", "Figma cache"),
        (
            "~/Library/Caches/com.github.GitHubDesktop/*",
            "GitHub Desktop cache",
        ),
        ("~/Library/Caches/SentryCrash/*", "Sentry crash reports"),
        ("~/Library/Caches/KSCrash/*", "KSCrash reports"),
        (
            "~/Library/Caches/com.crashlytics.data/*",
            "Crashlytics data",
        ),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }

    let ant = format!("{h}/Library/Application Support/Antigravity");
    if Path::new(&ant).is_dir() {
        for (g, l) in [
            (
                "~/Library/Application Support/Antigravity/Cache/*",
                "Antigravity cache",
            ),
            (
                "~/Library/Application Support/Antigravity/Code Cache/*",
                "Antigravity code cache",
            ),
            (
                "~/Library/Application Support/Antigravity/GPUCache/*",
                "Antigravity GPU cache",
            ),
            (
                "~/Library/Application Support/Antigravity/DawnGraphiteCache/*",
                "Antigravity Dawn cache",
            ),
            (
                "~/Library/Application Support/Antigravity/DawnWebGPUCache/*",
                "Antigravity WebGPU cache",
            ),
        ] {
            let (kb, cnt) = safe_clean(&[g], l);
            total_kb = total_kb.saturating_add(kb);
            total_cnt = total_cnt.saturating_add(cnt);
        }
    }
    let filo = format!("{h}/Library/Application Support/Filo");
    if Path::new(&filo).is_dir() {
        for (g, l) in [
            (
                "~/Library/Application Support/Filo/production/Cache/*",
                "Filo cache",
            ),
            (
                "~/Library/Application Support/Filo/production/Code Cache/*",
                "Filo code cache",
            ),
            (
                "~/Library/Application Support/Filo/production/GPUCache/*",
                "Filo GPU cache",
            ),
            (
                "~/Library/Application Support/Filo/production/DawnGraphiteCache/*",
                "Filo Dawn cache",
            ),
            (
                "~/Library/Application Support/Filo/production/DawnWebGPUCache/*",
                "Filo WebGPU cache",
            ),
        ] {
            let (kb, cnt) = safe_clean(&[g], l);
            total_kb = total_kb.saturating_add(kb);
            total_cnt = total_cnt.saturating_add(cnt);
        }
    }
    let claude = format!("{h}/Library/Application Support/Claude");
    if Path::new(&claude).is_dir() {
        for (g, l) in [
            (
                "~/Library/Application Support/Claude/Cache/*",
                "Claude cache",
            ),
            (
                "~/Library/Application Support/Claude/Code Cache/*",
                "Claude code cache",
            ),
            (
                "~/Library/Application Support/Claude/GPUCache/*",
                "Claude GPU cache",
            ),
            (
                "~/Library/Application Support/Claude/DawnGraphiteCache/*",
                "Claude Dawn cache",
            ),
            (
                "~/Library/Application Support/Claude/DawnWebGPUCache/*",
                "Claude WebGPU cache",
            ),
            (
                "~/Library/Application Support/Claude/sentry/*",
                "Claude sentry cache",
            ),
            (
                "~/Library/Application Support/Claude/pending-uploads/*",
                "Claude pending uploads",
            ),
        ] {
            let (kb, cnt) = safe_clean(&[g], l);
            total_kb = total_kb.saturating_add(kb);
            total_cnt = total_cnt.saturating_add(cnt);
        }
    }
    let qoder = format!("{h}/Library/Application Support/Qoder");
    if Path::new(&qoder).is_dir() {
        for (g, l) in [
            ("~/Library/Application Support/Qoder/Cache/*", "Qoder cache"),
            (
                "~/Library/Application Support/Qoder/CachedData/*",
                "Qoder cached data",
            ),
            (
                "~/Library/Application Support/Qoder/CachedExtensionVSIXs/*",
                "Qoder extension cache",
            ),
            (
                "~/Library/Application Support/Qoder/Code Cache/*",
                "Qoder code cache",
            ),
            (
                "~/Library/Application Support/Qoder/GPUCache/*",
                "Qoder GPU cache",
            ),
            (
                "~/Library/Application Support/Qoder/DawnGraphiteCache/*",
                "Qoder Dawn cache",
            ),
            (
                "~/Library/Application Support/Qoder/DawnWebGPUCache/*",
                "Qoder WebGPU cache",
            ),
            ("~/Library/Application Support/Qoder/logs/*", "Qoder logs"),
        ] {
            let (kb, cnt) = safe_clean(&[g], l);
            total_kb = total_kb.saturating_add(kb);
            total_cnt = total_cnt.saturating_add(cnt);
        }
    }

    for (g, l) in [
        ("~/.cache/prisma/*", "Prisma cache"),
        ("~/.cache/opencode/*", "OpenCode cache"),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    let op = format!("{h}/.local/share/opencode");
    if Path::new(&op).is_dir() {
        for (g, l) in [
            ("~/.local/share/opencode/snapshot/*", "OpenCode snapshots"),
            ("~/.local/share/opencode/log/*", "OpenCode logs"),
        ] {
            let (kb, cnt) = safe_clean(&[g], l);
            total_kb = total_kb.saturating_add(kb);
            total_cnt = total_cnt.saturating_add(cnt);
        }
    }
    let (kb, cnt) = safe_clean(&["~/.cache/codex-runtimes/*"], "Codex CLI runtimes");
    total_kb = total_kb.saturating_add(kb);
    total_cnt = total_cnt.saturating_add(cnt);
    let ca = format!("{h}/.local/share/cursor-agent");
    if Path::new(&ca).is_dir() {
        safe_find_delete(&ca, "*.log", MOLE_LOG_AGE_DAYS, "f");
    }
    for (g, l) in [
        ("~/Library/Caches/ms-playwright/*", "Playwright browsers"),
        (
            "~/Library/Application Support/com.wondershare.Installer/*",
            "Wondershare installer payload",
        ),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    (total_kb, total_cnt)
}

/// 对齐 SH `clean_dev_shell()` 第 1258-1266 行。
pub fn clean_dev_shell() -> (u64, u64) {
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    for (g, l) in [
        ("~/.gitconfig.lock", "Git config lock"),
        ("~/.gitconfig.bak*", "Git config backup"),
        ("~/.oh-my-zsh/cache/*", "Oh My Zsh cache"),
        ("~/.config/fish/fish_history.bak*", "Fish shell backup"),
        ("~/.bash_history.bak*", "Bash history backup"),
        ("~/.zsh_history.bak*", "Zsh history backup"),
        ("~/.cache/pre-commit/*", "pre-commit cache"),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    (total_kb, total_cnt)
}

/// 对齐 SH `clean_dev_network()` 第 1268-1273 行。
pub fn clean_dev_network() -> (u64, u64) {
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    for (g, l) in [
        ("~/.cache/curl/*", "curl cache"),
        ("~/.cache/wget/*", "wget cache"),
        ("~/Library/Caches/curl/*", "macOS curl cache"),
        ("~/Library/Caches/wget/*", "macOS wget cache"),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    (total_kb, total_cnt)
}

/// 对齐 SH `clean_sqlite_temp_files()` 第 1275-1277 行。
pub fn clean_sqlite_temp_files() -> (u64, u64) {
    (0, 0)
}

/// 对齐 SH `clean_dev_elixir()` 第 1280-1282 行。
pub fn clean_dev_elixir() -> (u64, u64) {
    safe_clean(&["~/.hex/cache/*"], "Hex cache")
}

/// 对齐 SH `clean_dev_haskell()` 第 1285-1287 行。
pub fn clean_dev_haskell() -> (u64, u64) {
    safe_clean(&["~/.cabal/packages/*"], "Cabal install cache")
}

/// 对齐 SH `clean_dev_ocaml()` 第 1289-1291 行。
pub fn clean_dev_ocaml() -> (u64, u64) {
    safe_clean(&["~/.opam/download-cache/*"], "Opam cache")
}

/// 对齐 SH `clean_dev_editors()` 第 1294-1322 行。
pub fn clean_dev_editors() -> (u64, u64) {
    let h = home_dir();
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    for (g, l) in [
        (
            "~/Library/Caches/com.microsoft.VSCode/Cache/*",
            "VS Code cached data",
        ),
        (
            "~/Library/Application Support/Code/CachedData/*",
            "VS Code cached data",
        ),
        (
            "~/Library/Application Support/Code/DawnGraphiteCache/*",
            "VS Code Dawn cache",
        ),
        (
            "~/Library/Application Support/Code/DawnWebGPUCache/*",
            "VS Code WebGPU cache",
        ),
        (
            "~/Library/Application Support/Code/GPUCache/*",
            "VS Code GPU cache",
        ),
        (
            "~/Library/Application Support/Code/CachedExtensionVSIXs/*",
            "VS Code extension cache",
        ),
        (
            "~/Library/Application Support/Code/WebStorage/*",
            "VS Code WebStorage",
        ),
        ("~/Library/Caches/Zed/*", "Zed cache"),
        ("~/Library/Caches/copilot/*", "GitHub Copilot cache"),
        ("~/.cache/vscode-ripgrep/*", "VS Code ripgrep cache"),
    ] {
        let (kb, cnt) = safe_clean(&[g], l);
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }
    let sw = format!("{h}/Library/Application Support/Code/Service Worker/CacheStorage");
    clean_service_worker_cache("VS Code", &sw);
    if !pgrep_x("Code") {
        let (kb, cnt) = safe_clean(
            &["~/Library/Application Support/Code/Service Worker/ScriptCache/*"],
            "VS Code Service Worker ScriptCache",
        );
        total_kb = total_kb.saturating_add(kb);
        total_cnt = total_cnt.saturating_add(cnt);
    }

    let cursor_root = format!("{h}/Library/Application Support/Cursor");
    if Path::new(&cursor_root).is_dir() {
        for (g, l) in [
            ("~/Library/Caches/Cursor/*", "Cursor cache"),
            (
                "~/Library/Application Support/Cursor/CachedData/*",
                "Cursor cached data",
            ),
            (
                "~/Library/Application Support/Cursor/CachedExtensionVSIXs/*",
                "Cursor extension cache",
            ),
            (
                "~/Library/Application Support/Cursor/WebStorage/*",
                "Cursor WebStorage",
            ),
            (
                "~/Library/Application Support/Cursor/GPUCache/*",
                "Cursor GPU cache",
            ),
            (
                "~/Library/Application Support/Cursor/DawnGraphiteCache/*",
                "Cursor Dawn cache",
            ),
            (
                "~/Library/Application Support/Cursor/DawnWebGPUCache/*",
                "Cursor WebGPU cache",
            ),
        ] {
            let (kb, cnt) = safe_clean(&[g], l);
            total_kb = total_kb.saturating_add(kb);
            total_cnt = total_cnt.saturating_add(cnt);
        }
        let swc = format!("{h}/Library/Application Support/Cursor/Service Worker/CacheStorage");
        clean_service_worker_cache("Cursor", &swc);
        if !pgrep_x("Cursor") {
            let (kb, cnt) = safe_clean(
                &["~/Library/Application Support/Cursor/Service Worker/ScriptCache/*"],
                "Cursor Service Worker ScriptCache",
            );
            total_kb = total_kb.saturating_add(kb);
            total_cnt = total_cnt.saturating_add(cnt);
        }
    }
    (total_kb, total_cnt)
}

// ----------------------------------------------------------------------------- legacy / tool stub

/// 历史 API 占位(旧 GUI 可能调用)。`clean_developer_tools` 内已用 `run_clean_tool_cache` 对齐 SH。
pub fn clean_tool_cache(description: &str, _cache_path: Option<&str>) {
    if is_dry_run() {
        log_info(&format!("{description} · would clean"));
    }
}

// ----------------------------------------------------------------------------- main entry

/// 将 brew 输出中的 "X freed" 字符串(如 "350MB freed"、"1.2GB freed")解析为 KB。
fn parse_brew_freed_kb(freed: &str) -> u64 {
    let s = freed
        .trim_end_matches(" freed")
        .trim_end_matches(" freed.")
        .trim();
    let mut num_str = String::new();
    let mut unit = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            num_str.push(ch);
        } else {
            unit.push(ch);
        }
    }
    let val: f64 = num_str.parse().unwrap_or(0.0);
    match unit.as_str() {
        "TB" => (val * 1024.0 * 1024.0 * 1024.0) as u64,
        "GB" => (val * 1024.0 * 1024.0) as u64,
        "MB" => (val * 1024.0) as u64,
        "KB" => val as u64,
        _ => 0,
    }
}

/// 为 `clean_developer_tools` 的子项补充主文件系统路径，供白名单路径匹配使用。
/// 也供 controller 层构建 `CleanItem.real_path` 使用。
pub fn dev_item_primary_path(id: &str) -> Option<String> {
    let h = home_dir();
    match id {
        "dev_npm" => Some(format!("{h}/.npm")),
        "dev_python" => Some(format!("{h}/Library/Caches/pip")),
        "dev_go" => Some(format!("{h}/Library/Caches/go-build")),
        "dev_mise" => Some(format!("{h}/.local/share/mise")),
        "dev_rust" => Some(format!("{h}/.cargo")),
        "dev_ruby" => Some(format!("{h}/.gem")),
        "dev_perl" => Some(format!("{h}/Library/Caches/perl")),
        "dev_docker" => Some(format!("{h}/.docker")),
        "dev_cloud" => Some(format!("{h}/.aws")),
        "dev_nix" => Some("/nix/store".into()),
        "dev_shell" => Some(format!("{h}/.zsh_history")),
        "dev_frontend" => Some(format!("{h}/.cache/yarn")),
        "dev_project_caches" => Some(format!("{h}/Library/Caches")),
        "dev_mobile" => Some(format!("{h}/Library/Developer")),
        "dev_jvm" => Some(format!("{h}/.gradle")),
        "dev_jetbrains_toolbox" => Some(format!("{h}/Library/Caches/com.jetbrains.toolbox")),
        "dev_jetbrains_logs" => Some(format!("{h}/Library/Caches/JetBrains")),
        "dev_ai_agents" => Some(format!("{h}/Library/Caches")),
        "dev_other_langs" => Some(format!("{h}/Library/Caches")),
        "dev_cicd" => Some(format!("{h}/.cache")),
        "dev_database" => Some(format!("{h}/Library/Caches")),
        "dev_api_tools" => Some(format!("{h}/Library/Caches")),
        "dev_network" => Some(format!("{h}/Library/Caches")),
        "dev_misc" => Some(format!("{h}/Library/Caches")),
        "dev_elixir" => Some(format!("{h}/.hex")),
        "dev_haskell" => Some(format!("{h}/.stack")),
        "dev_ocaml" => Some(format!("{h}/.opam")),
        "dev_sqlite" => Some(format!("{h}/Library/SQLite")),
        "dev_xcode" => Some(format!("{h}/Library/Developer")),
        "dev_code_editors" => Some(format!("{h}/Library/Application Support/Code")),
        "dev_homebrew_cache" => Some(format!("{h}/Library/Caches/Homebrew")),
        "dev_homebrew_locks" => Some(format!("/opt/homebrew/var/homebrew/locks")),
        "dev_homebrew_cleanup" => Some(format!("{h}/Library/Caches/Homebrew")),
        "dev_homebrew_autoremove" => Some(format!("{h}/Library/Caches/Homebrew")),
        _ => None,
    }
}

// =============================================================================
// AI Agent 清理 — 对齐 Mole/lib/clean/dev.sh §clean_dev_ai_agents 扩展段
// =============================================================================

/// SH `clean_versioned_agent_root` (第 1305-1354 行)
fn clean_versioned_agent_root(
    versions_root: &str,
    label: &str,
    keep_previous: usize,
    active_path: Option<&str>,
) -> (u64, u64) {
    if !Path::new(versions_root).is_dir() {
        return (0, 0);
    }
    let mut entries: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(versions_root) {
        for e in rd.flatten() {
            let name = e.file_name();
            let name_s = name.to_string_lossy();
            if name_s.starts_with('.') || !name_s.starts_with(|c: char| c.is_ascii_digit()) {
                continue;
            }
            if let Ok(meta) = e.metadata() {
                entries.push((meta.modified().unwrap_or(std::time::UNIX_EPOCH), e.path()));
            }
        }
    }
    if entries.len() <= keep_previous {
        return (0, 0);
    }
    entries.sort_by(|a, b| b.0.cmp(&a.0)); // newest first
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    for (idx, (_mtime, entry_path)) in entries.iter().enumerate() {
        if let Some(ap) = active_path {
            if entry_path == Path::new(ap) {
                continue;
            }
        }
        if idx < keep_previous {
            continue;
        }
        let ep = entry_path.to_string_lossy().to_string();
        let (kb, cnt) = safe_clean(&[&ep], &format!("{label}"));
        total_kb += kb;
        total_cnt += cnt;
    }
    (total_kb, total_cnt)
}

/// SH `count_versioned_agent_entries` (第 1354-1375 行)
fn count_versioned_agent_entries(dir: &str) -> usize {
    let path = Path::new(dir);
    if !path.is_dir() {
        return 0;
    }
    let mut count = 0usize;
    if let Ok(rd) = std::fs::read_dir(path) {
        for e in rd.flatten() {
            let name = e.file_name();
            let name_s = name.to_string_lossy();
            if name_s.starts_with('.') || !name_s.starts_with(|c: char| c.is_ascii_digit()) {
                continue;
            }
            count += 1;
        }
    }
    count
}

/// SH `claude_desktop_running`
fn claude_desktop_running() -> bool {
    pgrep_x("Claude") || pgrep_f("/Claude.app/")
}

fn pgrep_f(pattern: &str) -> bool {
    std::process::Command::new("pgrep")
        .args(["-f", pattern])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// SH `claude_desktop_sdk_version` — read active SDK version from .sdk-version file.
fn claude_desktop_sdk_version(claude_support: &str) -> Option<String> {
    let sdk_file = format!("{claude_support}/claude-code-vm/.sdk-version");
    let content = std::fs::read_to_string(&sdk_file).ok()?;
    let sdk_version = content.lines().next()?.trim().to_string();
    if sdk_version.is_empty()
        || sdk_version.starts_with('.')
        || sdk_version.contains('/')
        || sdk_version.contains("..")
        || !sdk_version.starts_with(|c: char| c.is_ascii_digit())
    {
        return None;
    }
    Some(sdk_version)
}

/// SH `clean_claude_desktop_bundled_versions` (第 1406-1473 行)
pub fn clean_claude_desktop_bundled_versions(keep_previous: usize) -> (u64, u64) {
    let claude_support = format!("{}/Library/Application Support/Claude", home_dir());
    if !Path::new(&claude_support).is_dir() {
        return (0, 0);
    }
    let specs: [(&str, &str); 2] = [
        (
            &format!("{claude_support}/claude-code"),
            "Claude Desktop bundled Claude Code old version",
        ),
        (
            &format!("{claude_support}/claude-code-vm"),
            "Claude Desktop bundled Claude Code VM old version",
        ),
    ];
    let mut has_multiple = false;
    for (root, _) in &specs {
        if count_versioned_agent_entries(root) > 1 {
            has_multiple = true;
            break;
        }
    }
    if !has_multiple {
        return (0, 0);
    }
    if claude_desktop_running() {
        note_activity();
        log::info!(
            "Claude Desktop bundled Claude Code cleanup skipped · Claude Desktop is running"
        );
        return (0, 0);
    }
    let sdk_version = match claude_desktop_sdk_version(&claude_support) {
        Some(v) => v,
        None => {
            note_activity();
            log::info!(
                "Claude Desktop bundled Claude Code active version unknown · skipping cleanup"
            );
            return (0, 0);
        }
    };
    for (root, label) in &specs {
        let version_path = format!("{root}/{sdk_version}");
        if !Path::new(&version_path).exists() {
            note_activity();
            log::info!("{label} active version unknown · skipping cleanup");
            return (0, 0);
        }
    }
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    for (root, label) in &specs {
        let (kb, cnt) = clean_versioned_agent_root(
            root,
            label,
            keep_previous,
            Some(&format!("{root}/{sdk_version}")),
        );
        total_kb += kb;
        total_cnt += cnt;
    }
    (total_kb, total_cnt)
}

/// SH `clean_dev_ai_agents` extended (第 1473 行, includes Claude Desktop bundled)
pub fn clean_dev_ai_agents_extended() -> (u64, u64) {
    let keep: usize = std::env::var("MOLE_AI_AGENTS_KEEP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let h = home_dir();

    // Extended specs — including the standard 3 plus Claude Desktop bundle
    let specs: [(&str, &str, Option<&str>); 3] = [
        (
            &format!("{h}/.local/share/claude/versions"),
            "Claude Code old version",
            Some(&format!("{h}/.local/bin/claude")),
        ),
        (
            &format!("{h}/.local/share/cursor-agent/versions"),
            "Cursor Agent old version",
            Some(&format!("{h}/.local/bin/cursor-agent")),
        ),
        (
            &format!("{h}/.copilot/pkg/universal"),
            "GitHub Copilot CLI old version",
            None,
        ),
    ];

    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;

    for (root, label, active_symlink) in &specs {
        if !Path::new(root).is_dir() {
            continue;
        }
        let mut active_path: Option<PathBuf> = None;
        if let Some(sym) = active_symlink {
            let sp = Path::new(sym);
            if sp.is_symlink() {
                if !sp.exists() {
                    log_warning(&format!(
                        "{label} active symlink is broken · skipping cleanup"
                    ));
                    continue;
                }
                if let Ok(target) = std::fs::read_link(sp) {
                    let resolved = if target.is_absolute() {
                        target
                    } else {
                        sp.parent().map(|p| p.join(&target)).unwrap_or(target)
                    };
                    if let Ok(rd) = std::fs::read_dir(root) {
                        for e in rd.flatten() {
                            let ep = e.path();
                            let rs = resolved.to_string_lossy();
                            let es = ep.to_string_lossy();
                            if rs.starts_with(&format!("{es}/")) || rs == es.as_ref() {
                                active_path = Some(ep);
                                break;
                            }
                        }
                    }
                }
            }
        }
        let (kb, cnt) = clean_versioned_agent_root(
            root,
            label,
            keep,
            active_path
                .as_ref()
                .map(|p| p.to_string_lossy().as_ref().to_owned())
                .as_deref(),
        );
        total_kb += kb;
        total_cnt += cnt;
    }

    // Claude Desktop bundled versions
    let (ckb, ccnt) = clean_claude_desktop_bundled_versions(keep);
    total_kb += ckb;
    total_cnt += ccnt;

    (total_kb, total_cnt)
}

/// SH `clean_xcode_xctest_devices` (第 587-599 行)
pub fn clean_xcode_xctest_devices() -> (u64, u64) {
    let root = format!("{}/Library/Developer/XCTestDevices", home_dir());
    safe_clean(&[&root], "Xcode XCTest devices")
}

/// SH `clean_xcode_system_coresimulator_caches` (第 600-683 行)
pub fn clean_xcode_system_coresimulator_caches() -> (u64, u64) {
    let cache_root = "/Library/Developer/CoreSimulator/Caches";
    if !Path::new(cache_root).is_dir() {
        return (0, 0);
    }
    safe_clean(&[cache_root], "Xcode CoreSimulator caches")
}

/// SH `clean_codex_runtimes` (第 1638-1706 行)
pub fn clean_codex_runtimes() -> (u64, u64) {
    let runtime_root = format!("{}/.cache/codex-runtimes", home_dir());
    if !Path::new(&runtime_root).is_dir() {
        return (0, 0);
    }
    if is_path_whitelisted_from_global(&runtime_root) {
        if is_dry_run() {
            log_info("Codex runtimes · would skip (whitelist)");
        } else {
            log_info("Codex runtimes · skipped (whitelist)");
        }
        note_activity();
        return (0, 0);
    }
    if pgrep_x("Codex") || pgrep_f("/Codex.app/") {
        log_info("Codex runtimes · skipped (Codex running)");
        note_activity();
        return (0, 0);
    }
    // SH keeps this for manual review; we report size only
    note_activity();
    let total_kb = get_path_size_kb(&runtime_root);
    log_info(&format!(
        "Codex runtimes · manual review ({})",
        bytes_to_human(total_kb * 1024)
    ));
    (0, 0)
}

/// SH `clean_codex_cli` (第 1690-1706 行) — kept out of default cleanup.
pub fn clean_codex_cli() -> (u64, u64) {
    let codex_root = format!("{}/.codex", home_dir());
    if !Path::new(&codex_root).is_dir() {
        return (0, 0);
    }
    if pgrep_x("codex") || pgrep_x("Codex") || pgrep_f("/Codex.app/") {
        log_info("Codex CLI state · skipped (Codex running)");
        note_activity();
        return (0, 0);
    }
    log_info("Codex CLI state · skipped by default");
    note_activity();
    debug_log(&format!(
        "Codex CLI state left intact by default: {codex_root}"
    ));
    (0, 0)
}

/// SH `clean_chromium_default_caches` (第 1706-1722 行)
fn clean_chromium_default_caches(profile_root: &str, label: &str) -> (u64, u64) {
    if !Path::new(profile_root).is_dir() {
        return (0, 0);
    }
    let paths: [String; 5] = [
        format!("{profile_root}/Default/Cache/Cache_Data"),
        format!("{profile_root}/Default/Code Cache"),
        format!("{profile_root}/Default/GPUCache"),
        format!("{profile_root}/Default/DawnGraphiteCache"),
        format!("{profile_root}/Default/DawnWebGPUCache"),
    ];
    let refs: Vec<&str> = paths.iter().map(|s| s.as_ref()).collect();
    safe_clean(&refs, &format!("{label} browser cache"))
}

/// SH `clean_antigravity_caches` (第 1722-1741 行)
pub fn clean_antigravity_caches() -> (u64, u64) {
    let ag_profile = format!("{}/.gemini/antigravity-browser-profile", home_dir());
    if pgrep_x("Antigravity")
        || pgrep_f("/Antigravity.app/")
        || pgrep_x("gemini")
        || pgrep_f("antigravity-browser-profile")
    {
        log_info("Antigravity/Gemini caches · skipped (Antigravity or Gemini running)");
        note_activity();
        return (0, 0);
    }
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    if Path::new(&ag_profile).is_dir() {
        let (kb, cnt) = clean_chromium_default_caches(&ag_profile, "Antigravity");
        total_kb += kb;
        total_cnt += cnt;
        for sub in &[
            "GraphiteDawnCache",
            "component_crx_cache",
            "extensions_crx_cache",
        ] {
            let p = format!("{ag_profile}/{sub}");
            if Path::new(&p).is_dir() {
                let (k, c) = safe_clean(&[&p], &format!("Antigravity {sub}"));
                total_kb += k;
                total_cnt += c;
            }
        }
        let sw = format!("{ag_profile}/Default/Service Worker/CacheStorage");
        if Path::new(&sw).is_dir() {
            clean_service_worker_cache("Antigravity", &sw);
        }
    }
    let (k, c) = safe_clean(
        &[&format!("{}/.gemini/tmp", home_dir())],
        "Gemini CLI temp files",
    );
    total_kb += k;
    total_cnt += c;
    (total_kb, total_cnt)
}

/// SH `clean_chrome_devtools_mcp_caches` (第 1741-1767 行)
pub fn clean_chrome_devtools_mcp_caches() -> (u64, u64) {
    let mcp_profile = format!("{}/.cache/chrome-devtools-mcp/chrome-profile", home_dir());
    if pgrep_f("chrome-devtools-mcp") {
        log_info("Chrome DevTools MCP caches · skipped (server running)");
        note_activity();
        return (0, 0);
    }
    if !Path::new(&mcp_profile).is_dir() {
        return (0, 0);
    }
    let (kb, cnt) = clean_chromium_default_caches(&mcp_profile, "Chrome DevTools MCP");
    let (k2, c2) = safe_clean(
        &[
            &format!("{mcp_profile}/Default/DawnCache"),
            &format!("{mcp_profile}/Default/GrShaderCache"),
            &format!("{mcp_profile}/Default/GraphiteDawnCache"),
            &format!("{mcp_profile}/GraphiteDawnCache"),
            &format!("{mcp_profile}/component_crx_cache"),
            &format!("{mcp_profile}/extensions_crx_cache"),
        ],
        "Chrome DevTools MCP extra caches",
    );
    (kb + k2, cnt + c2)
}

/// SH `clean_dev_agent_worktrees` (第 1810-1878 行)
pub fn clean_dev_agent_worktrees() -> (u64, u64) {
    scan_agent_worktrees(false)
}

fn scan_agent_worktrees(force: bool) -> (u64, u64) {
    let roots: Vec<String> = std::env::var("MOLE_AGENT_WORKTREE_PATHS")
        .ok()
        .filter(|v| !v.is_empty())
        .map(|v| v.split(':').map(String::from).collect())
        .unwrap_or_else(|| {
            [
                "code",
                "Code",
                "dev",
                "Projects",
                "GitHub",
                "Workspace",
                "Repos",
                "Development",
                "www",
                "src",
            ]
            .iter()
            .map(|s| format!("{}/{}", home_dir(), s))
            .collect()
        });

    let scan_timeout = 20u64;
    let mut containers: Vec<String> = Vec::new();
    for root in &roots {
        if !Path::new(root).is_dir() {
            continue;
        }
        // find $root -maxdepth 6 -type d -path "*/.claude/worktrees"
        let cmd = format!(
            "find \"{}\" -maxdepth 6 -type d -path \"*/.claude/worktrees\" 2>/dev/null",
            root
        );
        if let Some(out) = run_with_timeout_capture_lossy(scan_timeout as f64, "sh", &["-c", &cmd])
        {
            for line in out.lines() {
                let line = line.trim();
                if !line.is_empty() {
                    containers.push(line.to_string());
                }
            }
        }
    }

    if containers.is_empty() {
        return (0, 0);
    }

    if !force {
        // Default: report size only, never delete.
        let mut total_kb = 0u64;
        let mut count = 0usize;
        for container in &containers {
            let cmd = format!(
                "find \"{}\" -mindepth 1 -maxdepth 1 -type d 2>/dev/null",
                container
            );
            if let Some(out) = run_with_timeout_capture_lossy(10.0, "sh", &["-c", &cmd]) {
                for line in out.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    count += 1;
                    total_kb += get_path_size_kb(line);
                }
            }
        }
        if count > 0 {
            note_activity();
            log_info(&format!(
                "AI agent worktrees · skipped by default ({count} in .claude/worktrees, {})",
                bytes_to_human(total_kb * 1024)
            ));
            debug_log(&format!(
                "AI agent worktrees left intact by default ({count} dirs, {total_kb} KB)"
            ));
        }
        return (0, 0);
    }

    // Opt-in: remove clean worktrees only.
    let mut total_kb = 0u64;
    let mut total_cnt = 0u64;
    let mut kept = 0usize;
    for container in &containers {
        let parent_repo = container.trim_end_matches("/.claude/worktrees");
        let cmd = format!(
            "find \"{}\" -mindepth 1 -maxdepth 1 -type d 2>/dev/null",
            container
        );
        let out = match run_with_timeout_capture_lossy(10.0, "sh", &["-c", &cmd]) {
            Some(o) => o,
            None => continue,
        };
        for line in out.lines() {
            let wt = line.trim();
            if wt.is_empty() || should_protect_path(wt) {
                continue;
            }
            if agent_worktree_is_disposable(wt) {
                let (kb, cnt) = safe_clean(&[wt], "AI agent worktree");
                total_kb += kb;
                total_cnt += cnt;
                // Tidy the parent repo's worktree registry
                let git_dir = format!("{parent_repo}/.git");
                if !Path::new(wt).is_dir() && Path::new(&git_dir).exists() {
                    let _ = run_with_timeout(
                        2.0,
                        "git",
                        &["-C", parent_repo, "worktree", "unlock", wt],
                    );
                    let _ = run_with_timeout(
                        2.0,
                        "git",
                        &["-C", parent_repo, "worktree", "prune", "--expire=now"],
                    );
                }
            } else {
                kept += 1;
                note_activity();
                let display = wt.replacen(&home_dir(), "~", 1);
                log_info(&format!("Kept agent worktree (unsaved work): {display}"));
            }
        }
    }
    if kept > 0 {
        debug_log(&format!(
            "Kept {kept} AI agent worktree(s) with unsaved work"
        ));
    }
    (total_kb, total_cnt)
}

fn agent_worktree_is_disposable(wt: &str) -> bool {
    let git = "git";
    if run_with_timeout_capture_lossy(2.0, git, &["-C", wt, "rev-parse", "--is-inside-work-tree"])
        .is_none()
    {
        return false;
    }
    // Has uncommitted/untracked changes?
    if let Some(status) =
        run_with_timeout_capture_lossy(2.0, git, &["-C", wt, "status", "--porcelain"])
    {
        if !status.trim().is_empty() {
            return false;
        }
    } else {
        return false;
    }
    // Has stashed work?
    if let Some(stash) = run_with_timeout_capture_lossy(2.0, git, &["-C", wt, "stash", "list"]) {
        if !stash.trim().is_empty() {
            return false;
        }
    }
    // Any commits not on remotes?
    if let Some(revs) = run_with_timeout_capture_lossy(
        2.0,
        git,
        &[
            "-C",
            wt,
            "rev-list",
            "--count",
            "HEAD",
            "--not",
            "--remotes",
        ],
    ) {
        let count: i64 = revs.trim().parse().unwrap_or(1);
        if count != 0 {
            return false;
        }
    }
    true
}

/// 对齐 SH `clean_developer_tools()` 第 1324-1376 行。
pub fn clean_developer_tools() -> super::ModuleScanResult {
    let h = home_dir();
    let mut items: Vec<super::SubItemResult> = Vec::new();

    macro_rules! collect {
        ($func:expr, $id:literal, $title:literal) => {
            let (kb, cnt) = $func;
            items.push(super::SubItemResult::new($id, $title, kb, cnt));
        };
    }

    collect!(clean_sqlite_temp_files(), "dev_sqlite", "SQLite temp files");
    collect!(clean_dev_npm(), "dev_npm", "npm cache");
    collect!(clean_dev_python(), "dev_python", "pip/Python cache");
    collect!(clean_dev_go(), "dev_go", "Go build cache");
    collect!(clean_dev_mise(), "dev_mise", "mise-en-place cache");
    collect!(clean_dev_rust(), "dev_rust", "Rust cargo cache");
    check_rust_toolchains();
    collect!(clean_dev_ruby(), "dev_ruby", "RubyGems cache");
    collect!(clean_dev_perl(), "dev_perl", "Perl/CPAN cache");
    collect!(clean_dev_docker(), "dev_docker", "Docker cache");
    collect!(clean_dev_cloud(), "dev_cloud", "Cloud CLI cache");
    collect!(clean_dev_nix(), "dev_nix", "Nix store cache");
    collect!(clean_dev_shell(), "dev_shell", "Shell history/cache");
    collect!(clean_dev_frontend(), "dev_frontend", "Frontend tooling");
    collect!(
        clean_project_caches(),
        "dev_project_caches",
        "Project caches"
    );
    collect!(clean_dev_mobile(), "dev_mobile", "Mobile dev cache");
    collect!(clean_dev_jvm(), "dev_jvm", "JVM/Gradle/Maven");
    collect!(
        clean_dev_jetbrains_toolbox(),
        "dev_jetbrains_toolbox",
        "JetBrains Toolbox"
    );
    collect!(
        clean_dev_jetbrains_logs(),
        "dev_jetbrains_logs",
        "JetBrains IDE logs"
    );
    collect!(
        clean_dev_ai_agents_extended(),
        "dev_ai_agents",
        "AI agent tools"
    );
    collect!(
        clean_xcode_xctest_devices(),
        "dev_xctest",
        "Xcode XCTest devices"
    );
    collect!(
        clean_xcode_system_coresimulator_caches(),
        "dev_coresimulator",
        "Xcode CoreSimulator caches"
    );
    collect!(
        clean_codex_runtimes(),
        "dev_codex_runtimes",
        "Codex runtimes"
    );
    collect!(clean_codex_cli(), "dev_codex_cli", "Codex CLI state");
    collect!(
        clean_antigravity_caches(),
        "dev_antigravity",
        "Antigravity/Gemini caches"
    );
    collect!(
        clean_chrome_devtools_mcp_caches(),
        "dev_chrome_mcp",
        "Chrome DevTools MCP caches"
    );
    collect!(
        clean_dev_agent_worktrees(),
        "dev_agent_wt",
        "AI agent worktrees"
    );
    collect!(
        clean_dev_other_langs(),
        "dev_other_langs",
        "Other language tools"
    );
    collect!(clean_dev_cicd(), "dev_cicd", "CI/CD tools");
    collect!(clean_dev_database(), "dev_database", "Database tools");
    collect!(clean_dev_api_tools(), "dev_api_tools", "API dev tools");
    collect!(clean_dev_network(), "dev_network", "Network tools");
    collect!(clean_dev_misc(), "dev_misc", "Misc dev tools");
    collect!(clean_dev_elixir(), "dev_elixir", "Elixir/Hex cache");
    collect!(clean_dev_haskell(), "dev_haskell", "Haskell/Stack cache");
    collect!(clean_dev_ocaml(), "dev_ocaml", "OCaml/opam cache");

    let (xkb, xcnt) = clean_xcode_tools();
    items.push(super::SubItemResult::new(
        "dev_xcode",
        "Xcode tools",
        xkb,
        xcnt,
    ));

    let (ekb, ecnt) = clean_code_editors();
    items.push(super::SubItemResult::new(
        "dev_code_editors",
        "Code editors",
        ekb,
        ecnt,
    ));

    let (hkb, hcnt) = safe_clean(
        &[&format!("{h}/Library/Caches/Homebrew/*")],
        "Homebrew cache",
    );
    items.push(super::SubItemResult::new(
        "dev_homebrew_cache",
        "Homebrew cache",
        hkb,
        hcnt,
    ));
    let mut homebrew_locks_kb = 0;
    let mut homebrew_locks_cnt = 0;
    for lock_dir in [
        "/opt/homebrew/var/homebrew/locks",
        "/usr/local/var/homebrew/locks",
    ] {
        if Path::new(lock_dir).is_dir() && dir_is_writable(lock_dir) {
            let (lkb, lcnt) = safe_clean(&[&format!("{lock_dir}/*")], "Homebrew lock files");
            homebrew_locks_kb += lkb;
            homebrew_locks_cnt += lcnt;
        } else if Path::new(lock_dir).is_dir() && lock_dir_has_entries(lock_dir) {
            debug_log(&format!("Skipping read-only Homebrew locks in {lock_dir}"));
        }
    }
    items.push(super::SubItemResult::new(
        "dev_homebrew_locks",
        "Homebrew locks",
        homebrew_locks_kb,
        homebrew_locks_cnt,
    ));
    let brew_result = clean_homebrew();
    if !brew_result.skipped {
        if brew_result.cleanup_timed_out {
            items.push(super::SubItemResult::new(
                "dev_homebrew_cleanup",
                "Homebrew cleanup · timed out",
                0,
                0,
            ));
        } else if let Some(freed) = &brew_result.freed_space {
            let freed_kb = parse_brew_freed_kb(freed);
            items.push(super::SubItemResult::new(
                "dev_homebrew_cleanup",
                &format!("Homebrew cleanup · {freed}"),
                freed_kb,
                brew_result.removed_count,
            ));
        } else if brew_result.removed_count > 0 {
            items.push(super::SubItemResult::new(
                "dev_homebrew_cleanup",
                &format!(
                    "Homebrew cleanup · {} items removed",
                    brew_result.removed_count
                ),
                0,
                brew_result.removed_count,
            ));
        }
        if brew_result.autoremove_timed_out {
            items.push(super::SubItemResult::new(
                "dev_homebrew_autoremove",
                "Homebrew autoremove · timed out",
                0,
                0,
            ));
        } else if brew_result.autoremoved_packages > 0 {
            items.push(super::SubItemResult::new(
                "dev_homebrew_autoremove",
                &format!(
                    "Homebrew autoremove · {} orphaned packages",
                    brew_result.autoremoved_packages
                ),
                0,
                brew_result.autoremoved_packages,
            ));
        }
    }

    for item in &mut items {
        if item.path.is_none() {
            item.path = dev_item_primary_path(&item.id);
        }
    }

    super::ModuleScanResult { items }
}

/// Item-level dispatch: 清理 Homebrew cache + locks（"dev_homebrew_cache"）
pub fn clean_dev_homebrew_cache_with_locks() -> (u64, u64) {
    let h = home_dir();
    let (mut kb, mut cnt) = safe_clean(
        &[&format!("{h}/Library/Caches/Homebrew/*")],
        "Homebrew cache",
    );
    for lock_dir in [
        "/opt/homebrew/var/homebrew/locks",
        "/usr/local/var/homebrew/locks",
    ] {
        if Path::new(lock_dir).is_dir() && dir_is_writable(lock_dir) {
            let (lkb, lcnt) = safe_clean(&[&format!("{lock_dir}/*")], "Homebrew lock files");
            kb = kb.saturating_add(lkb);
            cnt = cnt.saturating_add(lcnt);
        }
    }
    (kb, cnt)
}

/// Item-level dispatch: 运行 `brew cleanup`（"dev_homebrew_cleanup"）
pub fn clean_dev_homebrew_cleanup_item() -> (u64, u64) {
    let result = clean_homebrew();
    if result.cleanup_timed_out || result.freed_space.is_none() {
        return (0, result.removed_count);
    }
    let freed_kb = result
        .freed_space
        .as_deref()
        .map(parse_brew_freed_kb)
        .unwrap_or(0);
    (freed_kb, result.removed_count)
}

/// Item-level dispatch: 运行 `brew autoremove`（"dev_homebrew_autoremove"）
pub fn clean_dev_homebrew_autoremove_item() -> (u64, u64) {
    let result = clean_homebrew();
    if result.autoremove_timed_out {
        (0, 0)
    } else {
        (0, result.autoremoved_packages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_does_not_panic_and_preserves_invariants() {
        std::env::set_var("MOLE_DRY_RUN", "1");
        std::env::set_var("DRY_RUN", "true");

        let result = clean_developer_tools();

        let sum_kb: u64 = result.items.iter().map(|i| i.size_kb).sum();
        let sum_cnt: u64 = result.items.iter().map(|i| i.file_count).sum();
        assert_eq!(
            result.total_kb(),
            sum_kb,
            "total_kb() must equal sum of items' size_kb"
        );
        assert_eq!(
            result.total_count(),
            sum_cnt,
            "total_count() must equal sum of items' file_count"
        );

        for item in &result.items {
            assert!(!item.id.is_empty(), "item id must not be empty");
            assert!(
                !item.title.is_empty(),
                "item title must not be empty for id={}",
                item.id
            );
        }
    }
}
