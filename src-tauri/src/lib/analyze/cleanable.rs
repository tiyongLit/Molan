use std::io::Read;
use std::path::Path;

const CACHE_DIR_TAG_FILE: &str = "CACHEDIR.TAG";
const CACHE_DIR_TAG_SIGNATURE: &[u8; 43] = b"Signature: 8a477f597d28d172789f06886806bc55";

pub fn is_cleanable_dir(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }

    if is_handled_by_mo_clean(path) {
        return false;
    }

    // CACHEDIR.TAG marks the whole directory tree as regenerable cache.
    if has_valid_cache_dir_tag(path) {
        return true;
    }

    let base_name = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    PROJECT_DEPENDENCY_DIRS.contains(&base_name)
}

/// Go `hasValidCacheDirTag`：检查目录中是否有有效的 CACHEDIR.TAG 标记。
fn has_valid_cache_dir_tag(path: &str) -> bool {
    let tag_path = Path::new(path).join(CACHE_DIR_TAG_FILE);
    let meta = match std::fs::symlink_metadata(&tag_path) {
        Ok(m) if m.is_file() => m,
        _ => return false,
    };
    // 拒绝异常大小的标记文件
    if meta.len() < CACHE_DIR_TAG_SIGNATURE.len() as u64 {
        return false;
    }
    if meta.len() > 16384 {
        return false;
    }
    let mut file = match std::fs::File::open(&tag_path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = [0u8; 43];
    match file.read_exact(&mut buf) {
        Ok(()) => &buf == CACHE_DIR_TAG_SIGNATURE,
        Err(_) => false,
    }
}

fn is_handled_by_mo_clean(path: &str) -> bool {
    const CLEAN_PATHS: &[&str] = &[
        "/Library/Caches/",
        "/Library/Logs/",
        "/Library/Saved Application State/",
        "/.Trash/",
        "/Library/DiagnosticReports/",
    ];

    CLEAN_PATHS.iter().any(|p| path.contains(p))
}

const PROJECT_DEPENDENCY_DIRS: &[&str] = &[
    "node_modules",
    "bower_components",
    ".yarn",
    ".pnpm-store",
    "venv",
    ".venv",
    "virtualenv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".eggs",
    "htmlcov",
    ".ipynb_checkpoints",
    "vendor",
    ".bundle",
    ".gradle",
    "out",
    "build",
    "dist",
    "target",
    ".next",
    ".nuxt",
    ".output",
    ".parcel-cache",
    ".turbo",
    ".vite",
    ".nx",
    "coverage",
    ".coverage",
    ".nyc_output",
    ".angular",
    ".svelte-kit",
    ".astro",
    ".docusaurus",
    "DerivedData",
    "Pods",
    ".build",
    "Carthage",
    ".dart_tool",
    ".terraform",
];
