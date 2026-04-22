//! 递归扫描（WalkDir，不跟随符号链接）+ Top-N 大文件 + 进度回调。
//! - 文件大小：对齐 Mole `getActualFileSize`（Unix `blocks * 512` 与 `len()` 组合）
//! - `full` + Unix：折叠目录用 `du -skP` 估算体积并剪枝（对齐 Mole `foldDirs`）
//! - `mas`：不调用外部 `du`，不剪枝，整树 WalkDir（沙箱友好）

#[cfg(all(feature = "full", unix))]
use log::debug;
use log::{info, warn};
use serde::Serialize;
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
#[cfg(all(feature = "full", unix))]
use walkdir::DirEntry;
use walkdir::WalkDir;

/// 与 Mole `cmd/analyze/constants.go` 中 `foldDirs` 对齐的核心集合（可按需扩充）
#[cfg(all(feature = "full", unix))]
const FOLD_DIR_NAMES: &[&str] = &[
    ".git", ".svn", ".hg", "node_modules", ".npm", "_npx", "_cacache", "_logs", "_locks", "_quick",
    "_libvips", "_prebuilds", "_update-notifier-last-checked", ".yarn", ".pnpm-store", ".next",
    ".nuxt", "bower_components", ".vite", ".turbo", ".parcel-cache", ".nx", ".rush", "tnpm",
    ".tnpm", ".bun", ".deno", "__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache", "venv",
    ".venv", "virtualenv", ".tox", "site-packages", ".eggs", ".pyenv", ".poetry", ".pip", ".pipx",
    "vendor", ".bundle", "gems", ".rbenv", "target", ".gradle", ".m2", ".ivy2", "out", "pkg",
    ".composer", ".cargo", "build", "dist", ".output", "coverage", ".coverage", ".idea", ".vscode",
    ".vs", ".fleet", ".cache", "__MACOSX", "Caches", ".Spotlight-V100", ".fseventsd",
    ".DocumentRevisions-V100", "$RECYCLE.BIN", ".temp", ".tmp", "_temp", "_tmp", ".Homebrew",
    ".rustup", ".sdkman", ".nvm", "Pods", "DerivedData", ".build", "xcuserdata", "Carthage",
    ".dart_tool", ".angular", ".svelte-kit", ".astro", ".docker", ".containerd",
];

/// 对齐 Mole `shouldFoldDirWithPath` 中的 `.npm` / `.tnpm` 路径规则
#[cfg(all(feature = "full", unix))]
fn should_fold_by_npm_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if !s.contains("/.npm/") && !s.contains("/.tnpm/") {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let Some(parent) = path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) else {
        return false;
    };
    parent == ".npm"
        || parent == ".tnpm"
        || parent.starts_with('_')
        || name.len() == 1
}

#[cfg(all(feature = "full", unix))]
fn should_fold_directory(path: &Path) -> bool {
    if should_fold_by_npm_path(path) {
        return true;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if FOLD_DIR_NAMES.iter().any(|&n| n == name) {
        return true;
    }
    name.ends_with(".egg-info")
}

/// 对齐 Mole `getActualFileSize`
fn actual_file_size(meta: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let len = meta.len();
        let blocks = meta.blocks();
        if blocks == 0 {
            return len;
        }
        let allocated = blocks.saturating_mul(512);
        if allocated < len {
            allocated
        } else {
            len
        }
    }
    #[cfg(not(unix))]
    {
        meta.len()
    }
}

/// `du -skP` 返回的字节数；与 Mole `getDirectorySizeFromDu` 一致（KB * 1024）
#[cfg(all(feature = "full", unix))]
fn directory_size_from_du(path: &Path) -> Result<u64, String> {
    use std::process::Command;
    let out = Command::new("/usr/bin/du")
        .args(["-skP", &path.to_string_lossy()])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "du 失败 status={:?} stderr={}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let kb: u64 = stdout
        .split_whitespace()
        .next()
        .ok_or_else(|| "du 输出为空".to_string())?
        .parse()
        .map_err(|e: std::num::ParseIntError| e.to_string())?;
    Ok(kb.saturating_mul(1024))
}

#[cfg(all(feature = "full", unix))]
fn try_fold_du(path: &Path) -> Option<u64> {
    match directory_size_from_du(path) {
        Ok(b) => Some(b),
        Err(e) => {
            warn!("du 失败，将展开目录: {} — {}", path.display(), e);
            None
        }
    }
}

/// 发给前端的扫描进度（节流由调用方控制）
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub files_scanned: u64,
    pub bytes_total: u64,
    pub dirs_seen: u64,
    pub last_path: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LargeFile {
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub root: String,
    pub files_scanned: u64,
    pub dirs_seen: u64,
    pub bytes_total: u64,
    pub largest_files: Vec<LargeFile>,
    /// 因权限等原因跳过的 walk 错误数
    pub walk_errors: u64,
    /// `full` 下通过 `du` 折叠的目录数（`mas` 恒为 0）
    pub folded_dirs: u64,
    /// 折叠目录计入的总字节（`du` 估算）
    pub folded_bytes: u64,
}

struct ScanInner {
    files_scanned: u64,
    dirs_seen: u64,
    bytes_total: u64,
    walk_errors: u64,
    folded_dirs: u64,
    folded_bytes: u64,
    last_path: Option<String>,
    heap: BinaryHeap<Reverse<(u64, String)>>,
    top_n: usize,
}

impl ScanInner {
    fn new(top_n: usize) -> Self {
        let k = top_n.max(1);
        Self {
            files_scanned: 0,
            dirs_seen: 0,
            bytes_total: 0,
            walk_errors: 0,
            folded_dirs: 0,
            folded_bytes: 0,
            last_path: None,
            heap: BinaryHeap::with_capacity(k + 1),
            top_n: k,
        }
    }

    fn push_large(&mut self, size: u64, path_str: String) {
        let k = self.top_n;
        if self.heap.len() < k {
            self.heap.push(Reverse((size, path_str)));
        } else if let Some(Reverse((min_sz, _))) = self.heap.peek().cloned() {
            if size > min_sz {
                self.heap.pop();
                self.heap.push(Reverse((size, path_str)));
            }
        }
    }
}

pub fn scan_directory<F>(
    root: &Path,
    top_n: usize,
    progress_every: u64,
    mut on_progress: F,
) -> Result<ScanResult, String>
where
    F: FnMut(ScanProgress),
{
    if !root.exists() {
        return Err(format!("路径不存在: {}", root.display()));
    }
    if !root.is_dir() {
        return Err(format!("不是目录: {}", root.display()));
    }

    let root_buf = root.to_path_buf();
    let inner = Rc::new(RefCell::new(ScanInner::new(top_n)));

    info!("扫描开始 root={}", root_buf.display());

    #[cfg(all(feature = "full", unix))]
    {
        for entry in WalkDir::new(&root_buf).follow_links(false).into_iter().filter_entry({
            let inner = inner.clone();
            move |e: &DirEntry| {
                if !e.file_type().is_dir() {
                    return true;
                }
                let path = e.path();
                if !should_fold_directory(path) {
                    return true;
                }
                if let Some(sz) = try_fold_du(path) {
                    let mut s = inner.borrow_mut();
                    s.bytes_total = s.bytes_total.saturating_add(sz);
                    s.folded_bytes = s.folded_bytes.saturating_add(sz);
                    s.folded_dirs += 1;
                    s.dirs_seen += 1;
                    debug!("折叠目录 du: {} -> {} bytes", path.display(), sz);
                    false
                } else {
                    true
                }
            }
        }) {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    inner.borrow_mut().walk_errors += 1;
                    warn!("walk 错误: {}", err);
                    continue;
                }
            };

            let path = entry.path();
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    inner.borrow_mut().walk_errors += 1;
                    warn!("metadata 失败 {}: {}", path.display(), e);
                    continue;
                }
            };

            if meta.is_dir() {
                inner.borrow_mut().dirs_seen += 1;
                continue;
            }

            if !meta.is_file() {
                continue;
            }

            let len = actual_file_size(&meta);
            {
                let mut s = inner.borrow_mut();
                s.files_scanned += 1;
                s.bytes_total = s.bytes_total.saturating_add(len);
                let path_str = path.to_string_lossy().into_owned();
                s.last_path = Some(path_str.clone());
                s.push_large(len, path_str);
            }

            let s = inner.borrow();
            if progress_every > 0 && s.files_scanned % progress_every == 0 {
                on_progress(ScanProgress {
                    files_scanned: s.files_scanned,
                    bytes_total: s.bytes_total,
                    dirs_seen: s.dirs_seen,
                    last_path: s.last_path.clone(),
                });
            }
        }
    }

    #[cfg(not(all(feature = "full", unix)))]
    {
        for entry in WalkDir::new(&root_buf).follow_links(false) {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    inner.borrow_mut().walk_errors += 1;
                    warn!("walk 错误: {}", err);
                    continue;
                }
            };

            let path = entry.path();
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    inner.borrow_mut().walk_errors += 1;
                    warn!("metadata 失败 {}: {}", path.display(), e);
                    continue;
                }
            };

            if meta.is_dir() {
                inner.borrow_mut().dirs_seen += 1;
                continue;
            }

            if !meta.is_file() {
                continue;
            }

            let len = actual_file_size(&meta);
            {
                let mut s = inner.borrow_mut();
                s.files_scanned += 1;
                s.bytes_total = s.bytes_total.saturating_add(len);
                let path_str = path.to_string_lossy().into_owned();
                s.last_path = Some(path_str.clone());
                s.push_large(len, path_str);
            }

            let s = inner.borrow();
            if progress_every > 0 && s.files_scanned % progress_every == 0 {
                on_progress(ScanProgress {
                    files_scanned: s.files_scanned,
                    bytes_total: s.bytes_total,
                    dirs_seen: s.dirs_seen,
                    last_path: s.last_path.clone(),
                });
            }
        }
    }

    let ScanInner {
        files_scanned,
        dirs_seen,
        bytes_total,
        walk_errors,
        folded_dirs,
        folded_bytes,
        last_path,
        heap,
        ..
    } = Rc::try_unwrap(inner)
        .map_err(|_| "扫描状态仍被引用（内部错误）".to_string())?
        .into_inner();

    on_progress(ScanProgress {
        files_scanned,
        bytes_total,
        dirs_seen,
        last_path: last_path.clone(),
    });

    let mut largest: Vec<LargeFile> = heap
        .into_iter()
        .map(|Reverse((size, path))| LargeFile { path, size })
        .collect();
    largest.sort_by(|a, b| b.size.cmp(&a.size));

    info!(
        "扫描结束 root={} files={} dirs_seen={} bytes_total={} folded_dirs={} folded_bytes={} walk_errors={}",
        root_buf.display(),
        files_scanned,
        dirs_seen,
        bytes_total,
        folded_dirs,
        folded_bytes,
        walk_errors
    );

    Ok(ScanResult {
        root: root_buf.to_string_lossy().into_owned(),
        files_scanned,
        dirs_seen,
        bytes_total,
        largest_files: largest,
        walk_errors,
        folded_dirs,
        folded_bytes,
    })
}

/// 校验 `path` 在 `root` 目录树内（基于 canonicalize，失败则保守拒绝）
pub fn path_is_under_root(root: &Path, path: &Path) -> bool {
    let Ok(root_canon) = std::fs::canonicalize(root) else {
        return false;
    };
    let Ok(path_canon) = std::fs::canonicalize(path) else {
        return false;
    };
    path_canon.starts_with(&root_canon)
}

pub fn trash_paths_under_root(root: &Path, paths: &[String]) -> Result<(), String> {
    for p in paths {
        let pb = PathBuf::from(p);
        if !path_is_under_root(root, &pb) {
            return Err(format!("拒绝：路径不在授权根目录下 — {}", p));
        }
        trash::delete(&pb).map_err(|e| format!("移到废纸篓失败 {}: {}", p, e))?;
    }
    Ok(())
}
