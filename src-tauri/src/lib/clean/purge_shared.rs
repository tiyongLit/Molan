use std::path::Path;

use crate::core::base::home_dir;

/// Canonical purge targets — 严格对齐 purge_shared.sh:MOLE_PURGE_TARGETS。
/// 任何新增/删除项必须先改 SH 端,再同步过来,以保持两端语义一致。
pub const MOLE_PURGE_TARGETS: [&str; 33] = [
    "node_modules",
    "target",        // Rust, Maven
    "build",         // Gradle, various
    "dist",          // JS builds
    "venv",          // Python
    ".venv",         // Python
    ".pytest_cache", // Python (pytest)
    ".mypy_cache",   // Python (mypy)
    ".tox",          // Python (tox virtualenvs)
    ".nox",          // Python (nox virtualenvs)
    ".ruff_cache",   // Python (ruff)
    ".gradle",       // Gradle local
    "__pycache__",   // Python
    ".next",         // Next.js
    ".nuxt",         // Nuxt.js
    ".output",       // Nuxt.js
    "vendor",        // PHP Composer
    "bin",           // .NET build output (受 is_protected_purge_artifact 守护)
    "obj",           // C# / Unity
    ".turbo",        // Turborepo cache
    ".parcel-cache", // Parcel bundler
    ".dart_tool",    // Flutter/Dart build cache
    ".zig-cache",    // Zig
    "zig-out",       // Zig
    ".angular",      // Angular
    ".svelte-kit",   // SvelteKit
    ".astro",        // Astro
    "coverage",      // Code coverage reports
    "DerivedData",   // Xcode
    "Pods",          // CocoaPods
    ".cxx",          // React Native Android NDK build cache
    ".expo",         // Expo
    ".build",        // Swift Package Manager
];

pub const MOLE_PURGE_DEFAULT_SEARCH_PATHS: [&str; 8] = [
    "~/www",
    "~/dev",
    "~/Projects",
    "~/GitHub",
    "~/Code",
    "~/Workspace",
    "~/Repos",
    "~/Development",
];

pub const MOLE_PURGE_MONOREPO_INDICATORS: [&str; 4] =
    ["lerna.json", "pnpm-workspace.yaml", "nx.json", "rush.json"];

/// 项目根识别器:严格对齐 purge_shared.sh:MOLE_PURGE_PROJECT_INDICATORS。
/// 注意:SH 端没有 `.hg`,Rust 之前自行加进来会让一些 Mercurial 旧目录被误判为项目。
pub const MOLE_PURGE_PROJECT_INDICATORS: [&str; 15] = [
    "package.json",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "requirements.txt",
    "pom.xml",
    "build.gradle",
    "Gemfile",
    "composer.json",
    "pubspec.yaml",
    "Package.swift", // Swift Package Manager
    "Makefile",
    "build.zig",
    "build.zig.zon",
    ".git",
];

pub const MOLE_PURGE_QUICK_HINT_EXCLUDED_TARGETS: [&str; 2] = ["bin", "vendor"];

pub fn mole_purge_is_project_root(dir: &str) -> bool {
    let p = Path::new(dir);
    if !p.is_dir() {
        return false;
    }
    for indicator in MOLE_PURGE_MONOREPO_INDICATORS.iter() {
        if p.join(indicator).exists() {
            return true;
        }
    }
    for indicator in MOLE_PURGE_PROJECT_INDICATORS.iter() {
        if p.join(indicator).exists() {
            return true;
        }
    }
    false
}

pub fn mole_purge_quick_hint_target_names() -> Vec<String> {
    MOLE_PURGE_TARGETS
        .iter()
        .filter(|t| !MOLE_PURGE_QUICK_HINT_EXCLUDED_TARGETS.contains(t))
        .map(|s| s.to_string())
        .collect()
}

pub fn mole_purge_resolve_path_case(path: &str) -> String {
    let expanded = path.replacen('~', &home_dir(), 1);
    if Path::new(&expanded).is_dir() {
        std::fs::canonicalize(&expanded)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(expanded)
    } else {
        expanded
    }
}

pub fn mole_purge_read_paths_config(config_file: &str) -> Vec<String> {
    let content = match std::fs::read_to_string(config_file) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut paths = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let expanded = trimmed.replacen('~', &home_dir(), 1);
        paths.push(mole_purge_resolve_path_case(&expanded));
    }
    paths
}
