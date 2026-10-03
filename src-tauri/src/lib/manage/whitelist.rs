//! 对齐 `lib/manage/whitelist.sh`。
//!
//! GUI 端不需要原 SH 中的 `paginated_multi_select` / `manage_whitelist_categories`
//! 等 TUI 交互函数,因此本模块只翻译数据 + 业务逻辑,不翻译 UI 层。

use std::io::Write;
use std::path::Path;

use crate::core::base::{FINDER_METADATA_SENTINEL, ensure_user_file, home_dir};

/// 对齐 clean.sh `perform_cleanup` 白名单读入结果：包含有效 pattern 及无效行的警告。
#[derive(Debug, Clone)]
pub struct WhitelistLoadResult {
    pub patterns: Vec<String>,
    pub warnings: Vec<String>,
}

/// 对齐 `lib/manage/whitelist.sh:13`
const WHITELIST_CONFIG_CLEAN: &str = "~/.config/molan/whitelist";
/// 对齐 `lib/manage/whitelist.sh:14`
const WHITELIST_CONFIG_OPTIMIZE: &str = "~/.config/molan/whitelist_optimize";
/// 对齐 `lib/manage/whitelist.sh:15`
const WHITELIST_CONFIG_OPTIMIZE_LEGACY: &str = "~/.config/molan/whitelist_checks";

/// 对齐 `lib/core/base.sh:88-112` 中的 `DEFAULT_WHITELIST_PATTERNS`。
pub fn default_whitelist_patterns() -> Vec<String> {
    let home = home_dir();
    vec![
        format!("{home}/Library/Caches/ms-playwright*"),
        format!("{home}/.cache/huggingface*"),
        format!("{home}/.m2/repository/*"),
        format!("{home}/.gradle/caches/*"),
        format!("{home}/.gradle/daemon/*"),
        format!("{home}/.ollama/models/*"),
        format!("{home}/Library/Caches/com.nssurge.surge-mac/*"),
        format!("{home}/Library/Application Support/com.nssurge.surge-mac/*"),
        format!("{home}/Library/Caches/org.R-project.R/R/renv/*"),
        format!("{home}/Library/Caches/pypoetry/virtualenvs*"),
        format!("{home}/Library/Caches/JetBrains*"),
        format!("{home}/Library/Caches/com.jetbrains.toolbox*"),
        format!("{home}/Library/Caches/tealdeer/tldr-pages"),
        format!("{home}/Library/Application Support/JetBrains*"),
        format!("{home}/Library/Caches/com.apple.finder"),
        format!("{home}/Library/Mobile Documents*"),
        format!("{home}/Library/Caches/com.apple.FontRegistry*"),
        format!("{home}/Library/Caches/com.apple.spotlight*"),
        format!("{home}/Library/Caches/com.apple.Spotlight*"),
        format!("{home}/Library/Caches/CloudKit*"),
        FINDER_METADATA_SENTINEL.to_string(),
    ]
}

/// 对齐 `lib/core/base.sh:114-118` 中的 `DEFAULT_OPTIMIZE_WHITELIST_PATTERNS`。
pub fn default_optimize_whitelist_patterns() -> Vec<String> {
    vec![
        "check_brew_health".to_string(),
        "check_touchid".to_string(),
        "check_git_config".to_string(),
    ]
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with('~') {
        path.replacen('~', &home_dir(), 1)
    } else {
        path.to_string()
    }
}

/// 对齐 `whitelist.sh:9-15`(配置文件路径解析)。
pub fn whitelist_config_path(mode: &str) -> String {
    let name = match mode {
        "optimize" => WHITELIST_CONFIG_OPTIMIZE,
        _ => WHITELIST_CONFIG_CLEAN,
    };
    expand_tilde(name)
}

fn whitelist_legacy_config_path(mode: &str) -> Option<String> {
    if mode == "optimize" {
        Some(expand_tilde(WHITELIST_CONFIG_OPTIMIZE_LEGACY))
    } else {
        None
    }
}

/// 对齐 `whitelist.sh:176-183`。仅做精确字符串匹配,不展开 glob,避免安全风险。
pub fn patterns_equivalent(a: &str, b: &str) -> bool {
    expand_tilde(a) == expand_tilde(b)
}

/// 对齐 `whitelist.sh:22-74` 中的 `save_whitelist_patterns`。
///
/// - clean 模式头部对齐 SH 第 44 行:含默认保护项说明。
/// - optimize 模式头部对齐 SH 第 41 行。
/// - 写入前按 `patterns_equivalent` 去重,对齐 SH 第 53-72 行。
pub fn save_whitelist_patterns(mode: &str, patterns: &[String]) {
    let config_file = whitelist_config_path(mode);
    if let Some(parent) = Path::new(&config_file).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    ensure_user_file(&config_file);

    let header = if mode == "optimize" {
        "# Mole Optimization Whitelist - These checks will be skipped during optimization\n"
            .to_string()
    } else {
        "# Mole Whitelist - Protected paths won't be deleted\n# Default protections: Playwright browsers, HuggingFace models, Maven repo, Ollama models, Surge Mac, R renv, Finder metadata\n# Add one pattern per line to keep items safe.\n"
            .to_string()
    };

    let mut file = match std::fs::File::create(&config_file) {
        Ok(f) => f,
        Err(_) => return,
    };
    let _ = file.write_all(header.as_bytes());

    if !patterns.is_empty() {
        let mut unique: Vec<String> = Vec::new();
        for p in patterns {
            if !unique.iter().any(|e| patterns_equivalent(e, p)) {
                unique.push(p.clone());
            }
        }
        if !unique.is_empty() {
            let _ = file.write_all(b"\n");
            for p in &unique {
                let _ = writeln!(file, "{p}");
            }
        }
    }
}

/// 对齐 clean.sh:70-96 的白名单 pattern 验证。
///
/// 对 `line` 展开 `~` / `$HOME` 后执行 5 项安全检查：
/// 1. 路径遍历（`..`）
/// 2. 控制字符
/// 3. 绝对路径（`FINDER_METADATA` sentinel 跳过此项与第 2 项）
/// 4. 连续斜杠
/// 5. 保护系统路径（`/`、`/System/`、`/bin/`……）
///
/// 返回 `None` 表示通过，`Some(warning)` 表示拒绝并给出原因。
fn validate_whitelist_pattern(line: &str, home: &str) -> Option<String> {
    let expanded = if line.starts_with('~') {
        line.replacen('~', home, 1)
    } else {
        line.to_string()
    };
    let expanded = expanded.replace("$HOME", home);
    let expanded = expanded.replace("${HOME}", home);

    if expanded.contains("..") {
        return Some(format!("Path traversal not allowed: {line}"));
    }

    if expanded == FINDER_METADATA_SENTINEL {
        return None;
    }

    if expanded.chars().any(|c| c.is_control()) {
        return Some(format!("Invalid path format: {line}"));
    }

    if !expanded.starts_with('/') {
        return Some(format!("Must be absolute path: {line}"));
    }

    if expanded.contains("//") {
        return Some(format!("Consecutive slashes: {line}"));
    }

    let protected: &[&str] = &[
        "/",
        "/System",
        "/bin",
        "/sbin",
        "/usr/bin",
        "/usr/sbin",
        "/etc",
        "/var/db",
    ];
    for &p in protected {
        if expanded == p || expanded.starts_with(&format!("{p}/")) {
            return Some(format!("Protected system path: {line}"));
        }
    }

    None
}

/// 对齐 `whitelist.sh:185-245` 中的 `load_whitelist`。
///
/// - 配置文件存在 → 读其内容，逐条验证（对齐 Shell 第 70–96 行）。
/// - 不存在但 legacy 文件存在(optimize 模式) → 读 legacy,加载完成后回写到新路径。
/// - 都不存在 → 回退到 `DEFAULT_*_PATTERNS`（默认值视为安全，不验证）。
/// - 最终结果按 `patterns_equivalent` 去重。
/// - 返回 `WhitelistLoadResult`，内含有效 patterns 及无效行的 warnings。
pub fn load_whitelist(mode: &str) -> WhitelistLoadResult {
    let config_file = whitelist_config_path(mode);
    let legacy_file = whitelist_legacy_config_path(mode);
    let home = home_dir();

    let mut using_legacy = false;
    let active_file = if Path::new(&config_file).is_file() {
        config_file.clone()
    } else if let Some(ref legacy) = legacy_file {
        if Path::new(legacy).is_file() {
            using_legacy = true;
            legacy.clone()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    if active_file.is_empty() {
        let patterns = match mode {
            "optimize" => default_optimize_whitelist_patterns(),
            _ => default_whitelist_patterns(),
        };
        return WhitelistLoadResult {
            patterns,
            warnings: Vec::new(),
        };
    }

    let raw_lines: Vec<String> = match std::fs::read_to_string(&active_file) {
        Ok(content) => content
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect(),
        Err(_) => Vec::new(),
    };

    let mut valid: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for line in &raw_lines {
        match validate_whitelist_pattern(line, &home) {
            Some(warning) => warnings.push(warning),
            None => {
                if !valid.iter().any(|e| patterns_equivalent(e, line)) {
                    valid.push(line.clone());
                }
            }
        }
    }

    if using_legacy && mode == "optimize" && !valid.is_empty() && active_file != config_file {
        save_whitelist_patterns(mode, &valid);
    }

    WhitelistLoadResult {
        patterns: valid,
        warnings,
    }
}

/// 对齐 `whitelist.sh:247-263`。仅精确字符串匹配,空白名单时永不命中。
pub fn is_whitelisted(pattern: &str, whitelist: &[String]) -> bool {
    if whitelist.is_empty() {
        return false;
    }
    let check = expand_tilde(pattern);
    whitelist.iter().any(|e| expand_tilde(e) == check)
}

/// 检查某 category id 是否命中白名单中的 `category:` 前缀条目。
///
/// 白名单配置文件中的 `category:dev_tools` 表示扫描展示时"Developer tools cache"模块默认不勾选。
/// 纯路径 pattern（如 `~/Library/Caches/*`）不在此函数检查范围内。
pub fn category_is_whitelisted(cat_id: &str, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let marker = format!("category:{cat_id}");
    patterns.iter().any(|p| *p == marker)
}

/// 对齐 `whitelist.sh:77-156` 中的 `get_all_cache_items`。
///
/// 返回三元组 `(display_name, pattern, category)`,顺序与 SH 一致。
/// `pattern` 已展开 `$HOME`;`FINDER_METADATA` 项使用 sentinel 字符串。
pub fn get_all_cache_items() -> Vec<(String, String, String)> {
    let home = home_dir();
    vec![
        (
            "Apple Mail cache".to_string(),
            format!("{home}/Library/Caches/com.apple.mail/*"),
            "system_cache".to_string(),
        ),
        (
            "Gradle build cache (Android Studio, Gradle projects)".to_string(),
            format!("{home}/.gradle/caches/*"),
            "ide_cache".to_string(),
        ),
        (
            "Gradle daemon processes cache".to_string(),
            format!("{home}/.gradle/daemon/*"),
            "ide_cache".to_string(),
        ),
        (
            "Xcode DerivedData (build outputs, indexes)".to_string(),
            format!("{home}/Library/Developer/Xcode/DerivedData/*"),
            "ide_cache".to_string(),
        ),
        (
            "Xcode archives (built app packages)".to_string(),
            format!("{home}/Library/Developer/Xcode/Archives/*"),
            "ide_cache".to_string(),
        ),
        (
            "Xcode internal cache files".to_string(),
            format!("{home}/Library/Caches/com.apple.dt.Xcode/*"),
            "ide_cache".to_string(),
        ),
        (
            "Xcode iOS device support symbols".to_string(),
            format!(
                "{home}/Library/Developer/Xcode/iOS DeviceSupport/*/Symbols/System/Library/Caches/*"
            ),
            "ide_cache".to_string(),
        ),
        (
            "Maven local repository (Java dependencies)".to_string(),
            format!("{home}/.m2/repository/*"),
            "ide_cache".to_string(),
        ),
        (
            "JetBrains IDEs data (IntelliJ, PyCharm, WebStorm, GoLand)".to_string(),
            format!("{home}/Library/Application Support/JetBrains/*"),
            "ide_cache".to_string(),
        ),
        (
            "JetBrains IDEs cache".to_string(),
            format!("{home}/Library/Caches/JetBrains/*"),
            "ide_cache".to_string(),
        ),
        (
            "Android Studio cache and indexes".to_string(),
            format!("{home}/Library/Caches/Google/AndroidStudio*/*"),
            "ide_cache".to_string(),
        ),
        (
            "Android build cache".to_string(),
            format!("{home}/.android/build-cache/*"),
            "ide_cache".to_string(),
        ),
        (
            "VS Code runtime cache".to_string(),
            format!("{home}/Library/Application Support/Code/Cache/*"),
            "ide_cache".to_string(),
        ),
        (
            "VS Code extension and update cache".to_string(),
            format!("{home}/Library/Application Support/Code/CachedData/*"),
            "ide_cache".to_string(),
        ),
        (
            "VS Code system cache (Cursor, VSCodium)".to_string(),
            format!("{home}/Library/Caches/com.microsoft.VSCode/*"),
            "ide_cache".to_string(),
        ),
        (
            "Cursor editor cache".to_string(),
            format!("{home}/Library/Caches/com.todesktop.230313mzl4w4u92/*"),
            "ide_cache".to_string(),
        ),
        (
            "Bazel build cache".to_string(),
            format!("{home}/.cache/bazel/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Go build cache".to_string(),
            format!("{home}/Library/Caches/go-build/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Go module cache".to_string(),
            format!("{home}/go/pkg/mod/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Rust Cargo registry cache".to_string(),
            format!("{home}/.cargo/registry/cache/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Rust documentation cache".to_string(),
            format!("{home}/.rustup/toolchains/*/share/doc/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Rustup toolchain downloads".to_string(),
            format!("{home}/.rustup/downloads/*"),
            "compiler_cache".to_string(),
        ),
        (
            "ccache compiler cache".to_string(),
            format!("{home}/.ccache/*"),
            "compiler_cache".to_string(),
        ),
        (
            "sccache distributed compiler cache".to_string(),
            format!("{home}/.cache/sccache/*"),
            "compiler_cache".to_string(),
        ),
        (
            "SBT Scala build cache".to_string(),
            format!("{home}/.sbt/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Ivy dependency cache".to_string(),
            format!("{home}/.ivy2/cache/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Turbo monorepo build cache".to_string(),
            format!("{home}/.turbo/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Next.js build cache".to_string(),
            format!("{home}/.next/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Vite build cache".to_string(),
            format!("{home}/.vite/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Parcel bundler cache".to_string(),
            format!("{home}/.parcel-cache/*"),
            "compiler_cache".to_string(),
        ),
        (
            "pre-commit hooks cache".to_string(),
            format!("{home}/.cache/pre-commit/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Ruff Python linter cache".to_string(),
            format!("{home}/.cache/ruff/*"),
            "compiler_cache".to_string(),
        ),
        (
            "MyPy type checker cache".to_string(),
            format!("{home}/.cache/mypy/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Pytest test cache".to_string(),
            format!("{home}/.pytest_cache/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Flutter SDK cache".to_string(),
            format!("{home}/.cache/flutter/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Swift Package Manager cache".to_string(),
            format!("{home}/.cache/swift-package-manager/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Zig compiler cache".to_string(),
            format!("{home}/.cache/zig/*"),
            "compiler_cache".to_string(),
        ),
        (
            "Deno cache".to_string(),
            format!("{home}/Library/Caches/deno/*"),
            "compiler_cache".to_string(),
        ),
        (
            "CocoaPods cache (iOS dependencies)".to_string(),
            format!("{home}/Library/Caches/CocoaPods/*"),
            "package_manager".to_string(),
        ),
        (
            "npm package cache".to_string(),
            format!("{home}/.npm/_cacache/*"),
            "package_manager".to_string(),
        ),
        (
            "pip Python package cache".to_string(),
            format!("{home}/.cache/pip/*"),
            "package_manager".to_string(),
        ),
        (
            "uv Python package cache".to_string(),
            format!("{home}/.cache/uv/*"),
            "package_manager".to_string(),
        ),
        (
            "R renv global cache (virtual environments)".to_string(),
            format!("{home}/Library/Caches/org.R-project.R/R/renv/*"),
            "package_manager".to_string(),
        ),
        (
            "tealdeer tldr pages cache".to_string(),
            format!("{home}/Library/Caches/tealdeer/tldr-pages"),
            "package_manager".to_string(),
        ),
        (
            "Homebrew downloaded packages".to_string(),
            format!("{home}/Library/Caches/Homebrew/*"),
            "package_manager".to_string(),
        ),
        (
            "Yarn package manager cache".to_string(),
            format!("{home}/.cache/yarn/*"),
            "package_manager".to_string(),
        ),
        (
            "pnpm package store".to_string(),
            format!("{home}/Library/pnpm/store/*"),
            "package_manager".to_string(),
        ),
        (
            "Composer PHP dependencies cache (legacy)".to_string(),
            format!("{home}/.composer/cache/*"),
            "package_manager".to_string(),
        ),
        (
            "Composer PHP dependencies cache".to_string(),
            format!("{home}/Library/Caches/composer/*"),
            "package_manager".to_string(),
        ),
        (
            "RubyGems cache".to_string(),
            format!("{home}/.gem/cache/*"),
            "package_manager".to_string(),
        ),
        (
            "Conda packages cache".to_string(),
            format!("{home}/.conda/pkgs/*"),
            "package_manager".to_string(),
        ),
        (
            "Anaconda packages cache".to_string(),
            format!("{home}/anaconda3/pkgs/*"),
            "package_manager".to_string(),
        ),
        (
            "PyTorch model cache".to_string(),
            format!("{home}/.cache/torch/*"),
            "ai_ml_cache".to_string(),
        ),
        (
            "TensorFlow model and dataset cache".to_string(),
            format!("{home}/.cache/tensorflow/*"),
            "ai_ml_cache".to_string(),
        ),
        (
            "HuggingFace models and datasets".to_string(),
            format!("{home}/.cache/huggingface/*"),
            "ai_ml_cache".to_string(),
        ),
        (
            "Playwright browser binaries".to_string(),
            format!("{home}/Library/Caches/ms-playwright*"),
            "ai_ml_cache".to_string(),
        ),
        (
            "Selenium WebDriver binaries".to_string(),
            format!("{home}/.cache/selenium/*"),
            "ai_ml_cache".to_string(),
        ),
        (
            "Ollama local AI models".to_string(),
            format!("{home}/.ollama/models/*"),
            "ai_ml_cache".to_string(),
        ),
        (
            "Weights & Biases ML experiments cache".to_string(),
            format!("{home}/.cache/wandb/*"),
            "ai_ml_cache".to_string(),
        ),
        (
            "Safari web browser cache".to_string(),
            format!("{home}/Library/Caches/com.apple.Safari/*"),
            "browser_cache".to_string(),
        ),
        (
            "Chrome browser cache".to_string(),
            format!("{home}/Library/Caches/Google/Chrome/*"),
            "browser_cache".to_string(),
        ),
        (
            "Firefox browser cache".to_string(),
            format!("{home}/Library/Caches/Firefox/*"),
            "browser_cache".to_string(),
        ),
        (
            "Brave browser cache".to_string(),
            format!("{home}/Library/Caches/BraveSoftware/Brave-Browser/*"),
            "browser_cache".to_string(),
        ),
        (
            "Surge proxy cache".to_string(),
            format!("{home}/Library/Caches/com.nssurge.surge-mac/*"),
            "network_tools".to_string(),
        ),
        (
            "Surge configuration and data".to_string(),
            format!("{home}/Library/Application Support/com.nssurge.surge-mac/*"),
            "network_tools".to_string(),
        ),
        (
            "Docker BuildX cache".to_string(),
            format!("{home}/.docker/buildx/cache/*"),
            "container_cache".to_string(),
        ),
        (
            "Podman container cache".to_string(),
            format!("{home}/.local/share/containers/cache/*"),
            "container_cache".to_string(),
        ),
        (
            "Font cache".to_string(),
            format!("{home}/Library/Caches/com.apple.FontRegistry/*"),
            "system_cache".to_string(),
        ),
        (
            "Spotlight metadata cache".to_string(),
            format!("{home}/Library/Caches/com.apple.spotlight/*"),
            "system_cache".to_string(),
        ),
        (
            "CloudKit cache".to_string(),
            format!("{home}/Library/Caches/CloudKit/*"),
            "system_cache".to_string(),
        ),
        (
            "Trash".to_string(),
            format!("{home}/.Trash"),
            "system_cache".to_string(),
        ),
        (
            "iOS/iPadOS device firmware (.ipsw) from iTunes/Finder".to_string(),
            format!("{home}/Library/iTunes/*Software Updates/*.ipsw"),
            "system_cache".to_string(),
        ),
        (
            "Apple Configurator 2 device firmware (.ipsw)".to_string(),
            format!("{home}/Library/Group Containers/*.group.com.apple.configurator/**/*.ipsw"),
            "system_cache".to_string(),
        ),
        (
            "Finder metadata, .DS_Store".to_string(),
            FINDER_METADATA_SENTINEL.to_string(),
            "system_cache".to_string(),
        ),
    ]
}

/// 对齐 `whitelist.sh:159-174` 中的 `get_optimize_whitelist_items`。
pub fn get_optimize_whitelist_items() -> Vec<(String, String, String)> {
    vec![
        (
            "macOS Firewall check".to_string(),
            "firewall".to_string(),
            "security_check".to_string(),
        ),
        (
            "Gatekeeper check".to_string(),
            "gatekeeper".to_string(),
            "security_check".to_string(),
        ),
        (
            "macOS system updates check".to_string(),
            "check_macos_updates".to_string(),
            "update_check".to_string(),
        ),
        (
            "Mole updates check".to_string(),
            "check_mole_update".to_string(),
            "update_check".to_string(),
        ),
        (
            "Homebrew health check (doctor)".to_string(),
            "check_brew_health".to_string(),
            "health_check".to_string(),
        ),
        (
            "SIP status check".to_string(),
            "check_sip".to_string(),
            "security_check".to_string(),
        ),
        (
            "FileVault status check".to_string(),
            "check_filevault".to_string(),
            "security_check".to_string(),
        ),
        (
            "TouchID sudo check".to_string(),
            "check_touchid".to_string(),
            "config_check".to_string(),
        ),
        (
            "Rosetta 2 check".to_string(),
            "check_rosetta".to_string(),
            "config_check".to_string(),
        ),
        (
            "Git configuration check".to_string(),
            "check_git_config".to_string(),
            "config_check".to_string(),
        ),
        (
            "Login items check".to_string(),
            "check_login_items".to_string(),
            "config_check".to_string(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_whitelist_not_empty() {
        let patterns = default_whitelist_patterns();
        assert!(!patterns.is_empty());
        assert!(patterns.len() >= 20);
    }

    #[test]
    fn validate_rejects_system_paths() {
        let home = home_dir();
        assert!(validate_whitelist_pattern("/System/Library", &home).is_some());
        assert!(validate_whitelist_pattern("/bin", &home).is_some());
        assert!(validate_whitelist_pattern("/usr/bin/bash", &home).is_some());
    }

    #[test]
    fn validate_rejects_relative_paths() {
        let home = home_dir();
        assert!(validate_whitelist_pattern("relative/path", &home).is_some());
    }

    #[test]
    fn validate_rejects_path_traversal() {
        let home = home_dir();
        assert!(validate_whitelist_pattern("~/../etc/passwd", &home).is_some());
    }

    #[test]
    fn validate_accepts_valid_pattern() {
        let home = home_dir();
        assert!(validate_whitelist_pattern("~/Library/Caches/something/*", &home).is_none());
    }

    #[test]
    fn is_whitelisted_exact_match() {
        let home = home_dir();
        let patterns = vec![format!("{home}/Library/Caches/foo")];
        assert!(is_whitelisted(
            &format!("{home}/Library/Caches/foo"),
            &patterns
        ));
        assert!(!is_whitelisted(
            &format!("{home}/Library/Caches/bar"),
            &patterns
        ));
    }

    #[test]
    fn is_whitelisted_empty_list_always_false() {
        let patterns: Vec<String> = vec![];
        assert!(!is_whitelisted("/any/path", &patterns));
    }

    #[test]
    fn category_is_whitelisted_matches() {
        let patterns = vec![
            "category:dev_tools".to_string(),
            "category:browser_cache".to_string(),
        ];
        assert!(category_is_whitelisted("dev_tools", &patterns));
        assert!(category_is_whitelisted("browser_cache", &patterns));
        assert!(!category_is_whitelisted("system_caches", &patterns));
    }

    #[test]
    fn category_is_whitelisted_no_category_prefix() {
        let patterns = vec!["~/Library/Caches/*".to_string()];
        assert!(!category_is_whitelisted("dev_tools", &patterns));
    }

    #[test]
    fn category_is_whitelisted_empty_patterns() {
        let patterns: Vec<String> = vec![];
        assert!(!category_is_whitelisted("dev_tools", &patterns));
    }

    #[test]
    fn patterns_equivalent_expands_tilde() {
        let home = home_dir();
        let a = "~/Library/Caches/test";
        let b = format!("{home}/Library/Caches/test");
        assert!(patterns_equivalent(a, &b));
    }

    #[test]
    fn patterns_equivalent_different_paths() {
        assert!(!patterns_equivalent(
            "~/Library/Caches/a",
            "~/Library/Caches/b"
        ));
    }
}
