//! Hint notices for `mo clean` — 严格对齐 lib/clean/hints.sh
//!
//! 关键约束:
//!   - 所有 `du` 调用必须接 `run_with_timeout_capture_lossy`(SH 用 0.8s);
//!     未接超时会让一次 hint 探测被慢路径拖死
//!   - `probe_project_artifact_hints` 必须扫到 nested_dir 第二层(SH 第 256-292 行)
//!   - `plutil -extract` 才是 plist 的正确读法,SH 用它处理 binary plist;
//!     **不要**自行用字符串扫 `<key>...</key>`,会在 binary plist 上完全失效
//!   - `show_user_launch_agent_hint_notice` 必须遵守 `max_hits=3`(SH 第 430 行)

use std::path::Path;
use std::process::Command;

use super::purge_shared::{
    MOLE_PURGE_DEFAULT_SEARCH_PATHS, mole_purge_is_project_root,
    mole_purge_quick_hint_target_names, mole_purge_read_paths_config,
};
use crate::core::app_protection::is_path_whitelisted_from_global;
use crate::core::base::{get_epoch_seconds, get_file_mtime, home_dir, note_activity};
use crate::core::bundle_resolver::bundle_has_installed_app;
use crate::core::timeout::run_with_timeout_capture_lossy;
use crate::events::{CleanupHintItem, CleanupHintsResultPayload};

pub struct ProjectArtifactHints {
    pub detected: bool,
    pub count: usize,
    pub truncated: bool,
    pub examples: Vec<String>,
    pub estimated_kb: u64,
    pub estimate_samples: usize,
    pub estimate_partial: bool,
}

/// SH `load_quick_purge_hint_paths` (第 13-28 行)
pub fn load_quick_purge_hint_paths() -> Vec<String> {
    let config_file = format!("{}/.config/mole/purge_paths", home_dir());
    let paths = mole_purge_read_paths_config(&config_file);
    if paths.is_empty() {
        MOLE_PURGE_DEFAULT_SEARCH_PATHS
            .iter()
            .map(|s| s.replacen('~', &home_dir(), 1))
            .collect()
    } else {
        paths
    }
}

/// SH `hint_get_path_size_kb_with_timeout` (第 31-55 行)
///
/// 调用 `du -skP <path>`,默认 0.8s 超时。失败/超时返回 None。
pub fn hint_get_path_size_kb_with_timeout(path: &str, timeout_seconds: f64) -> Option<u64> {
    let secs = if timeout_seconds <= 0.0 {
        0.8
    } else {
        timeout_seconds
    };
    let out = run_with_timeout_capture_lossy(secs, "du", &["-skP", path])?;
    let first = out.lines().next()?;
    let size_str = first.split_whitespace().next()?;
    size_str.parse::<u64>().ok()
}

// =============================================================================
// LaunchAgent plist 解析 — SH 第 57-101 行
// =============================================================================

/// 调一次 `plutil -extract <key> raw <plist>`,只关心 stdout。
/// 退出码非零(key 不存在)时返回空串,与 SH 的 `|| echo ""` 等价。
fn plutil_extract(key: &str, plist: &str) -> String {
    match Command::new("plutil")
        .args(["-extract", key, "raw", plist])
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => String::new(),
    }
}

/// SH `hint_extract_launch_agent_program_path` (第 58-68 行)
pub fn hint_extract_launch_agent_program_path(plist: &str) -> String {
    let mut program = plutil_extract("ProgramArguments.0", plist);
    if program.is_empty() {
        program = plutil_extract("Program", plist);
    }
    program
}

/// SH `hint_extract_launch_agent_associated_bundle` (第 71-84 行)
pub fn hint_extract_launch_agent_associated_bundle(plist: &str) -> String {
    let mut associated = plutil_extract("AssociatedBundleIdentifiers.0", plist);
    if associated.is_empty() || associated == "1" {
        associated = plutil_extract("AssociatedBundleIdentifiers", plist);
        if associated.starts_with('{') || associated.starts_with('[') {
            associated.clear();
        }
    }
    associated
}

/// SH `hint_is_app_scoped_launch_target` (第 87-101 行)
pub fn hint_is_app_scoped_launch_target(program: &str) -> bool {
    let home = home_dir();
    // SH glob 形如 /Applications/Setapp/*.app/* 这种;Rust 用 contains + starts_with 等价表达
    if program.starts_with("/Applications/Setapp/") && program.contains(".app/") {
        return true;
    }
    if program.starts_with("/Applications/") && program.contains(".app/") {
        return true;
    }
    let user_apps = format!("{home}/Applications/");
    if program.starts_with(&user_apps) && program.contains(".app/") {
        return true;
    }
    if program.starts_with("/Library/Input Methods/") && program.contains(".app/") {
        return true;
    }
    if program.starts_with("/Library/PrivilegedHelperTools/") {
        return true;
    }
    false
}

/// SH `hint_is_system_binary` (第 103-114 行)
pub fn hint_is_system_binary(program: &str) -> bool {
    program.starts_with("/bin/")
        || program.starts_with("/sbin/")
        || program.starts_with("/usr/bin/")
        || program.starts_with("/usr/sbin/")
        || program.starts_with("/usr/libexec/")
}

/// SH `hint_launch_agent_bundle_exists` (第 116-125 行)
pub fn hint_launch_agent_bundle_exists(bundle_id: &str) -> bool {
    if bundle_id.is_empty() {
        return false;
    }
    bundle_has_installed_app(bundle_id)
}

// =============================================================================
// 项目工件探测 — SH 第 127-315 行
// =============================================================================

/// SH `record_project_artifact_hint` (第 128-157 行)
pub fn record_project_artifact_hint(path: &str, hints: &mut ProjectArtifactHints) {
    hints.count += 1;
    if hints.examples.len() < 2 {
        let display = path.replacen(&home_dir(), "~", 1);
        hints.examples.push(display);
    }
    let sample_max = 3;
    if hints.estimate_samples >= sample_max {
        hints.estimate_partial = true;
        return;
    }
    if let Some(size_kb) = hint_get_path_size_kb_with_timeout(path, 0.8) {
        hints.estimated_kb += size_kb;
        hints.estimate_samples += 1;
    } else {
        hints.estimate_partial = true;
    }
}

/// SH `is_quick_purge_project_root` (第 160-162 行)
pub fn is_quick_purge_project_root(dir: &str) -> bool {
    mole_purge_is_project_root(dir)
}

/// SH `probe_project_artifact_hints` (第 165-315 行)
pub fn probe_project_artifact_hints() -> ProjectArtifactHints {
    let mut hints = ProjectArtifactHints {
        detected: false,
        count: 0,
        truncated: false,
        examples: Vec::new(),
        estimated_kb: 0,
        estimate_samples: 0,
        estimate_partial: false,
    };

    let max_projects = 200usize;
    let max_nested_per_project = 120usize;
    let max_matches = 12usize;

    let target_names = mole_purge_quick_hint_target_names();
    let scan_roots = load_quick_purge_hint_paths();
    if scan_roots.is_empty() {
        return hints;
    }

    // SH 第 192-196 行:每个 root 至少分到 max_projects/N(向上取整),保底 25
    let mut max_projects_per_root = (max_projects + scan_roots.len() - 1) / scan_roots.len();
    if max_projects_per_root < 25 {
        max_projects_per_root = 25;
    }
    if max_projects_per_root > max_projects {
        max_projects_per_root = max_projects;
    }

    let mut scanned_projects: usize = 0;

    // 用 labeled break 翻译 SH 的多层 `[[ "$stop_scan" == "true" ]] && break`
    'outer: for root in &scan_roots {
        if !Path::new(root).is_dir() {
            continue;
        }
        let mut root_projects_scanned: usize = 0;

        // SH 第 212-227 行:root 自身就是 project root 的情况
        if is_quick_purge_project_root(root) {
            scanned_projects += 1;
            root_projects_scanned += 1;
            if scanned_projects > max_projects {
                hints.truncated = true;
                break 'outer;
            }
            for tn in &target_names {
                let candidate = format!("{root}/{tn}");
                if Path::new(&candidate).is_dir() {
                    record_project_artifact_hint(&candidate, &mut hints);
                }
            }
        }

        if root_projects_scanned >= max_projects_per_root {
            hints.truncated = true;
            continue;
        }

        // SH 第 235-295 行:遍历 $root/*/(project_dir)
        let entries = match std::fs::read_dir(root) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let project_dir = entry.path();
            if !project_dir.is_dir() {
                continue;
            }
            let project_name = project_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if project_name.starts_with('.') {
                continue;
            }
            if root_projects_scanned >= max_projects_per_root {
                hints.truncated = true;
                break;
            }

            scanned_projects += 1;
            root_projects_scanned += 1;
            if scanned_projects > max_projects {
                hints.truncated = true;
                break 'outer;
            }

            let pd_str = project_dir.to_string_lossy().to_string();
            for tn in &target_names {
                let candidate = format!("{pd_str}/{tn}");
                if Path::new(&candidate).is_dir() {
                    record_project_artifact_hint(&candidate, &mut hints);
                }
            }

            // SH 第 264-292 行:nested_dir 第二层
            let mut nested_count: usize = 0;
            let nested_iter = match std::fs::read_dir(&project_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for nested_entry in nested_iter.flatten() {
                let nested_dir = nested_entry.path();
                if !nested_dir.is_dir() {
                    continue;
                }
                let nested_name = nested_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                if nested_name.starts_with('.') {
                    continue;
                }
                if matches!(
                    nested_name,
                    "node_modules" | "target" | "build" | "dist" | "DerivedData" | "Pods"
                ) {
                    continue;
                }
                nested_count += 1;
                if nested_count > max_nested_per_project {
                    break;
                }
                let nd_str = nested_dir.to_string_lossy().to_string();
                for tn in &target_names {
                    let candidate = format!("{nd_str}/{tn}");
                    if Path::new(&candidate).is_dir() {
                        record_project_artifact_hint(&candidate, &mut hints);
                    }
                }
            }
        }
    }

    if hints.count > 0 {
        hints.detected = true;
    }
    // SH 第 304-312 行:hint_count 超阈值时也设 truncated(只影响显示,不停止扫描)
    if hints.count > max_matches {
        hints.truncated = true;
    }
    hints
}

// =============================================================================
// 通知输出 — SH 第 318-481 行
// =============================================================================

pub fn show_system_data_hint_notice() -> Vec<(String, u64, String)> {
    let home = home_dir();
    let min_gb: u64 = 2;
    let timeout_seconds: f64 = 0.8;
    let max_hits: usize = 3;
    let threshold_kb = min_gb * 1024 * 1024;

    let labels = [
        "Xcode DerivedData",
        "Xcode Archives",
        "iPhone backups",
        "Simulator data",
        "Docker Desktop data",
        "Mail data",
    ];
    let paths = [
        format!("{home}/Library/Developer/Xcode/DerivedData"),
        format!("{home}/Library/Developer/Xcode/Archives"),
        format!("{home}/Library/Application Support/MobileSync/Backup"),
        format!("{home}/Library/Developer/CoreSimulator/Devices"),
        format!("{home}/Library/Containers/com.docker.docker/Data"),
        format!("{home}/Library/Mail"),
    ];
    let mut clues: Vec<(String, u64, String)> = Vec::new();
    for i in 0..labels.len() {
        if !Path::new(&paths[i]).is_dir() {
            continue;
        }
        if let Some(sz) = hint_get_path_size_kb_with_timeout(&paths[i], timeout_seconds) {
            if sz >= threshold_kb {
                let display = paths[i].replacen(&home, "~", 1);
                clues.push((labels[i].to_string(), sz, display));
                if clues.len() >= max_hits {
                    break;
                }
            }
        }
    }
    note_activity();
    clues
}

/// SH `show_project_artifact_hint_notice` (第 381-423 行)
///
/// GUI 版：不 println!，直接返回探测结果供 controller 发事件给前端。
pub fn show_project_artifact_hint_notice() -> ProjectArtifactHints {
    let hints = probe_project_artifact_hints();
    if hints.detected {
        note_activity();
    }
    hints
}

/// SH `show_user_launch_agent_hint_notice` (第 426-481 行)
pub fn show_user_launch_agent_hint_notice() -> CleanupHintsResultPayload {
    let home = home_dir();
    let noop = CleanupHintsResultPayload {
        section: "App leftovers".into(),
        phase: "orphaned-launch-agents".into(),
        title: "User LaunchAgents check".into(),
        detected: false,
        review_hint: String::new(),
        items: Vec::new(),
    };
    let la_dir = format!("{home}/Library/LaunchAgents");
    if !Path::new(&la_dir).is_dir() {
        return noop;
    }
    let max_hits: usize = 3;
    let mut items: Vec<CleanupHintItem> = Vec::new();

    let entries = match std::fs::read_dir(&la_dir) {
        Ok(e) => e,
        Err(_) => return noop,
    };
    for entry in entries.flatten() {
        if items.len() >= max_hits {
            break;
        }
        let p = entry.path();
        let filename = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if filename.starts_with("com.apple.") || !filename.ends_with(".plist") {
            continue;
        }
        let plist = p.to_string_lossy().to_string();

        let program = hint_extract_launch_agent_program_path(&plist);
        if !program.is_empty() && hint_is_system_binary(&program) {
            continue;
        }

        let mut reason = String::new();
        let mut target = String::new();
        if !program.is_empty()
            && hint_is_app_scoped_launch_target(&program)
            && !Path::new(&program).exists()
        {
            reason = "Missing app/helper target".to_string();
            target = program.replacen(&home, "~", 1);
        } else {
            let associated = hint_extract_launch_agent_associated_bundle(&plist);
            if !associated.is_empty() && !hint_launch_agent_bundle_exists(&associated) {
                reason = "Associated app not found".to_string();
                target = associated;
            }
        }

        if !reason.is_empty() {
            items.push(CleanupHintItem {
                label: filename,
                size_bytes: 0,
                size_human: String::new(),
                path: p.to_string_lossy().to_string(),
                detail: Some(format!("{reason}: {target}")),
            });
        }
    }

    if !items.is_empty() {
        note_activity();
        CleanupHintsResultPayload {
            section: "App leftovers".into(),
            phase: "orphaned-launch-agents".into(),
            title: "User LaunchAgents check".into(),
            detected: true,
            review_hint: "Review: open ~/Library/LaunchAgents and remove only items you recognize"
                .into(),
            items,
        }
    } else {
        noop
    }
}

/// Shell 中 `ORPHAN_DOTDIR_KNOWN_SAFE` 数组常量 — 这些 ~/.<dir> 不会被提示为孤立目录。
const ORPHAN_DOTDIR_KNOWN_SAFE: &[&str] = &[
    ".bash_history",
    ".bash_profile",
    ".bash_sessions",
    ".bashrc",
    ".zshrc",
    ".zsh_history",
    ".zsh_sessions",
    ".zprofile",
    ".zshenv",
    ".zlogout",
    ".zcompdump",
    ".profile",
    ".inputrc",
    ".hushlogin",
    ".oh-my-zsh",
    ".zinit",
    ".zplug",
    ".antigen",
    ".p10k.zsh",
    ".config",
    ".local",
    ".cache",
    ".ssh",
    ".gnupg",
    ".gpg",
    ".password-store",
    ".gitconfig",
    ".gitignore_global",
    ".git-credentials",
    ".gitattributes_global",
    ".pyenv",
    ".rbenv",
    ".nvm",
    ".nodenv",
    ".goenv",
    ".jenv",
    ".rustup",
    ".cargo",
    ".ghcup",
    ".stack",
    ".cabal",
    ".sdkman",
    ".jabba",
    ".asdf",
    ".mise",
    ".rtx",
    ".volta",
    ".fnm",
    ".deno",
    ".bun",
    ".npm",
    ".yarn",
    ".pnpm",
    ".bundle",
    ".gem",
    ".composer",
    ".nuget",
    ".pub-cache",
    ".m2",
    ".gradle",
    ".sbt",
    ".ivy2",
    ".lein",
    ".hex",
    ".mix",
    ".opam",
    ".cpan",
    ".cpanm",
    ".conda",
    ".virtualenvs",
    ".pipx",
    ".docker",
    ".kube",
    ".minikube",
    ".helm",
    ".aws",
    ".azure",
    ".terraform",
    ".vagrant",
    ".vim",
    ".vimrc",
    ".viminfo",
    ".emacs",
    ".emacs.d",
    ".doom.d",
    ".nano",
    ".nanorc",
    ".vscode",
    ".cursor",
    ".atom",
    ".claude",
    ".copilot",
    ".ollama",
    ".Trash",
    ".Trashes",
    ".CFUserTextEncoding",
    ".DS_Store",
    ".cups",
    ".dropbox",
    ".android",
    ".cocoapods",
    ".fastlane",
    ".expo",
    ".react-native",
    ".swiftpm",
    ".tmux",
    ".screen",
    ".wget-hsts",
    ".curlrc",
    ".netrc",
    ".wgetrc",
    ".putty",
    ".lesshst",
    ".python_history",
    ".node_repl_history",
    ".irb_history",
    ".pry_history",
    ".jupyter",
    ".ipython",
    ".matplotlib",
    ".keras",
    ".torch",
    ".psql_history",
    ".mysql_history",
    ".sqlite_history",
    ".rediscli_history",
    ".mongo",
    ".dbshell",
    ".homebrew",
    ".hg",
    ".hgrc",
    ".svn",
    ".bazaar",
    ".fly",
    ".gemini",
];

/// SH `hint_normalize_app_match_text` — lowercase + strip non-alnum.
fn hint_normalize_app_match_text(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

/// SH `dotdir_has_owning_gui_app` with 5-min caching.
fn dotdir_has_owning_gui_app(name: &str) -> bool {
    if name.is_empty() || name.len() < 4 {
        return false;
    }
    let home = home_dir();
    let cache_dir = format!("{home}/.cache/mole");
    let cache_file = format!("{cache_dir}/installed_app_tokens_cache");
    let cache_ttl: u64 = 300; // 5 min

    let rebuild = if Path::new(&cache_file).is_file() {
        match std::fs::metadata(&cache_file) {
            Ok(meta) => {
                let mtime = meta
                    .modified()
                    .map(|t| {
                        t.duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                    })
                    .unwrap_or(0);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                now.saturating_sub(mtime) >= cache_ttl
            }
            Err(_) => true,
        }
    } else {
        true
    };

    if rebuild {
        let _ = std::fs::create_dir_all(&cache_dir);
        let mut tokens: Vec<String> = Vec::new();
        let app_roots: [&str; 7] = [
            "/Applications",
            "/Applications/Setapp",
            "/Applications/Utilities",
            &format!("{home}/Applications"),
            "/Library/Input Methods",
            &format!("{home}/Library/Input Methods"),
            &format!("{home}/Library/Application Support/Setapp/Applications"),
        ];
        for root in &app_roots {
            if !Path::new(root).is_dir() {
                continue;
            }
            let cmd = format!("find \"{}\" -maxdepth 2 -name \"*.app\" 2>/dev/null", root);
            if let Some(out) = run_with_timeout_capture_lossy(2.0, "sh", &["-c", &cmd]) {
                for line in out.lines() {
                    if line.is_empty() {
                        continue;
                    }
                    let app_path = Path::new(line.trim());
                    if let Some(app_name) = app_path.file_stem().and_then(|s| s.to_str()) {
                        tokens.push(app_name.to_string());
                        let info = app_path.join("Contents/Info.plist");
                        if info.is_file() {
                            for key in
                                &["CFBundleIdentifier", "CFBundleName", "CFBundleDisplayName"]
                            {
                                if let Some(v) =
                                    crate::core::bundle_id_anchor::plist_string_key(&info, key)
                                        .map(|s| s.trim().to_string())
                                        .filter(|s| !s.is_empty() && s != "(null)")
                                {
                                    tokens.push(v);
                                }
                            }
                        }
                    }
                }
            }
        }
        // brew casks
        for cask_root in &["/opt/homebrew/Caskroom", "/usr/local/Caskroom"] {
            if Path::new(cask_root).is_dir() {
                if let Some(out) = run_with_timeout_capture_lossy(
                    1.0,
                    "find",
                    &[cask_root, "-mindepth", "1", "-maxdepth", "1", "-type", "d"],
                ) {
                    for line in out.lines() {
                        if !line.trim().is_empty() {
                            tokens.push(
                                Path::new(line.trim())
                                    .file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("")
                                    .to_string(),
                            );
                        }
                    }
                }
            }
        }
        // normalize: lowercase, alnum, ≥4 chars, dedup
        let mut normalized: Vec<String> = tokens
            .iter()
            .map(|t| hint_normalize_app_match_text(t))
            .filter(|t| t.len() >= 4)
            .collect();
        normalized.sort();
        normalized.dedup();
        let content = normalized.join("\n");
        let _ = std::fs::write(&cache_file, &content);
    }

    if !Path::new(&cache_file).is_file() {
        return false;
    }
    let cache = match std::fs::read_to_string(&cache_file) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let name_lower = name.to_lowercase();
    // Tokenize the dotdir name into alnum runs of ≥4 chars
    let tokens: Vec<String> = name_lower
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '\n' })
        .collect::<String>()
        .split('\n')
        .filter(|s| s.len() >= 4)
        .map(|s| s.to_string())
        .collect();

    for tok in tokens {
        if cache.lines().any(|l| l.trim() == tok) {
            return true;
        }
    }
    false
}

/// SH `hint_collect_claude_plugin_tokens`
fn hint_collect_claude_plugin_tokens() -> Vec<String> {
    let home = home_dir();
    let settings = format!("{home}/.claude/settings.json");
    let installed = format!("{home}/.claude/plugins/installed_plugins.json");
    let mut tokens: Vec<String> = Vec::new();

    for file in [&settings, &installed] {
        if !Path::new(file).is_file() {
            continue;
        }
        let content = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(_) => continue,
        };
        // Match "token@marketplace" patterns
        if let Ok(re) = regex::Regex::new(r#""[A-Za-z0-9._-]+@[A-Za-z0-9._-]+""#) {
            for caps in re.captures_iter(&content) {
                if let Some(m) = caps.get(0) {
                    let raw = m.as_str();
                    let trimmed = &raw[1..raw.len() - 1]; // strip quotes
                    if let Some(at_pos) = trimmed.find('@') {
                        let token = &trimmed[..at_pos];
                        if token.len() >= 4 {
                            tokens.push(token.to_string());
                        }
                    }
                }
            }
        }
    }
    tokens.sort();
    tokens.dedup();
    tokens
}

/// SH `hint_dotdir_owned_by_claude_plugin`
fn hint_dotdir_owned_by_claude_plugin(dotdir_name: &str, claude_tokens: &[String]) -> bool {
    if claude_tokens.is_empty() {
        return false;
    }
    let stripped = dotdir_name.trim_start_matches('.');
    for token in claude_tokens {
        if token.len() < 4 {
            continue;
        }
        if stripped.contains(token.as_str()) {
            return true;
        }
    }
    false
}

/// SH `show_orphan_dotdir_hint_notice` — 探测 ~/.<dir> 中可能属于已卸载 CLI 工具的目录。
/// 对齐 `lib/clean/hints.sh` 第 530-619 行。
pub fn show_orphan_dotdir_hint_notice() -> CleanupHintsResultPayload {
    let home = home_dir();
    let max_hits: usize = 5;
    let age_days: u64 = std::env::var("MOLE_DOTDIR_ORPHAN_AGE_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let now = get_epoch_seconds();

    let find_cmd = format!(
        "find \"{}\" -maxdepth 1 -mindepth 1 -type d -name '.*' 2>/dev/null | LC_ALL=C sort",
        home
    );
    let output = run_with_timeout_capture_lossy(3.0, "sh", &["-c", &find_cmd]).unwrap_or_default();

    let mut labels: Vec<String> = Vec::new();
    let mut details: Vec<String> = Vec::new();

    let claude_plugin_tokens = hint_collect_claude_plugin_tokens();

    for dotdir in output.lines() {
        let dotdir = dotdir.trim();
        if dotdir.is_empty() || !Path::new(dotdir).is_dir() {
            continue;
        }
        let basename = Path::new(dotdir)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if basename.is_empty() {
            continue;
        }

        let is_safe = ORPHAN_DOTDIR_KNOWN_SAFE.contains(&basename);
        if is_safe {
            continue;
        }

        let stripped = basename.strip_prefix('.').unwrap_or(&basename);
        if dotdir_has_owning_gui_app(stripped) {
            continue;
        }
        if hint_dotdir_owned_by_claude_plugin(stripped, &claude_plugin_tokens) {
            continue;
        }

        if is_path_whitelisted_from_global(dotdir) {
            continue;
        }

        let mtime = get_file_mtime(dotdir);
        if mtime == 0 {
            continue;
        }
        let age_d = (now.saturating_sub(mtime)) / 86400;
        if age_d < age_days {
            continue;
        }

        let name = basename.strip_prefix('.').unwrap_or(basename);
        let mut candidates = vec![name.to_string()];
        let dehyphen = name.replace('-', "_");
        if dehyphen != name {
            candidates.push(dehyphen);
        }
        let stripped = name.replace('-', "");
        let last = candidates.last().unwrap();
        if stripped != name && stripped != *last {
            candidates.push(stripped);
        }
        if let Some(no_suffix) = name.strip_suffix("-cli") {
            candidates.push(no_suffix.to_string());
        }
        if let Some(no_suffix) = name.strip_suffix("-temp") {
            candidates.push(no_suffix.to_string());
        }
        if let Some(no_suffix) = name.strip_suffix("-data") {
            candidates.push(no_suffix.to_string());
        }

        let has_binary = candidates.iter().any(|c| {
            Command::new("which")
                .arg(c)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        });
        if has_binary {
            continue;
        }

        let la_dir = format!("{home}/Library/LaunchAgents");
        if Path::new(&la_dir).is_dir() {
            if run_with_timeout_capture_lossy(2.0, "grep", &["-rlc", basename, &la_dir])
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
            {
                continue;
            }
        }

        let size_human = if let Some(size_kb) = hint_get_path_size_kb_with_timeout(dotdir, 0.8) {
            format!(" ({})", bytes_to_human_kb(size_kb))
        } else {
            String::new()
        };

        labels.push(format!("~/{basename}{size_human}"));
        details.push(format!(
            "No matching binary in PATH, last modified {age_d} days ago"
        ));

        if labels.len() >= max_hits {
            break;
        }
    }

    if labels.is_empty() {
        return CleanupHintsResultPayload {
            section: "App leftovers".into(),
            phase: "orphaned-dotdir".into(),
            title: "Orphaned dotdir check".into(),
            detected: false,
            review_hint: String::new(),
            items: Vec::new(),
        };
    }

    let mut items: Vec<CleanupHintItem> = Vec::new();
    for i in 0..labels.len() {
        items.push(CleanupHintItem {
            label: labels[i].clone(),
            size_bytes: 0,
            size_human: String::new(),
            path: String::new(),
            detail: Some(details[i].clone()),
        });
    }
    note_activity();
    CleanupHintsResultPayload {
        section: "App leftovers".into(),
        phase: "orphaned-dotdir".into(),
        title: "Orphaned dotdir check".into(),
        detected: true,
        review_hint: "Review manually before removing any ~/.<dir> directory".into(),
        items,
    }
}

fn bytes_to_human_kb(kb: u64) -> String {
    if kb >= 1024 * 1024 {
        format!("{:.1}GB", kb as f64 / (1024.0 * 1024.0))
    } else if kb >= 1024 {
        format!("{:.1}MB", kb as f64 / 1024.0)
    } else {
        format!("{kb}KB")
    }
}
