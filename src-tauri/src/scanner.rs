//! 递归扫描（WalkDir，不跟随符号链接）+ Top-N 大文件 + 进度回调。
//! - **默认统计口径（Lemon / Finder）：** `metadata.len()` 逻辑大小
//! - **可选统计口径（Mole）：** Unix 下 `blocks * 512` 与 `len()` 组合（物理占用）
//! - Unix：命中折叠目录时纯 Rust 递归统计并剪枝，子树内 Top-N + 嵌套折叠节点

use log::{info, warn};
use serde::Serialize;
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use walkdir::WalkDir;

/// 单文件体积统计方式：默认对齐 Lemon/Finder；`Physical` 对齐 Mole。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SizeMetric {
    /// 逻辑大小：`metadata.len()`，与 Finder / Lemon 列表更接近
    #[default]
    Logical,
    /// Mole `getActualFileSize`：Unix 下 blocks×512 与 len 的组合
    Physical,
}

impl SizeMetric {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "logical" | "len" => Ok(Self::Logical),
            "physical" | "mole" | "allocated" => Ok(Self::Physical),
            _ => Err(format!(
                "未知 sizeMetric: {}（支持 logical | physical）",
                s
            )),
        }
    }

    pub fn as_api_str(self) -> &'static str {
        match self {
            Self::Logical => "logical",
            Self::Physical => "physical",
        }
    }
}

fn file_size(meta: &std::fs::Metadata, metric: SizeMetric) -> u64 {
    match metric {
        SizeMetric::Logical => meta.len(),
        SizeMetric::Physical => mole_physical_size(meta),
    }
}

/// 对齐 Mole `getActualFileSize`（仅用于 `SizeMetric::Physical`）
fn mole_physical_size(meta: &std::fs::Metadata) -> u64 {
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

/// 与 Mole `cmd/analyze/constants.go` 中 `foldDirs` 对齐的核心集合（可按需扩充）
#[cfg(unix)]
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
#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(not(unix))]
fn should_fold_directory(_path: &Path) -> bool {
    false
}

fn push_top_n_heap(heap: &mut BinaryHeap<Reverse<(u64, String)>>, top_n: usize, size: u64, key: String) {
    let k = top_n.max(1);
    if heap.len() < k {
        heap.push(Reverse((size, key)));
    } else if let Some(Reverse((min_sz, _))) = heap.peek().cloned() {
        if size > min_sz {
            heap.pop();
            heap.push(Reverse((size, key)));
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

/// 嵌套扫描节点：折叠目录带预计算的子 Top-N（含嵌套折叠子节点）；文件为叶子。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanNode {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub is_folded: bool,
    pub children: Vec<ScanNode>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub root: String,
    pub files_scanned: u64,
    pub dirs_seen: u64,
    pub bytes_total: u64,
    pub largest_files: Vec<LargeFile>,
    /// 根层展示项（折叠目录 + 非折叠区 Top 文件），按体积降序，供 Tree 使用
    pub items: Vec<ScanNode>,
    /// 因权限等原因跳过的 walk 错误数
    pub walk_errors: u64,
    /// 作为折叠根处理的目录数
    pub folded_dirs: u64,
    /// 折叠子树计入的总字节（与 `bytes_total` 中对应部分一致）
    pub folded_bytes: u64,
    /// 本次扫描使用的体积口径：`logical` | `physical`
    pub size_metric: String,
    /// Lemon 式分类骨架（编译期内嵌于 `embedded_rules`，不含外部 YAML）
    pub rule_categories: Vec<crate::embedded_rules::RuleCategoryBlueprint>,
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
    folded_nodes: Vec<ScanNode>,
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
            folded_nodes: Vec::new(),
        }
    }

    fn push_large(&mut self, size: u64, path_str: String) {
        push_top_n_heap(&mut self.heap, self.top_n, size, path_str);
    }
}

fn node_name(path: &Path) -> String {
    path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// 扫描单个折叠目录子树：总体积 + 子展示项 Top-N（文件与嵌套折叠节点按体积混排）
#[cfg(unix)]
fn scan_folded_subtree(
    path: &Path,
    top_n: usize,
    size_metric: SizeMetric,
    walk_errors: &mut u64,
    dirs_seen: &mut u64,
    files_scanned: &mut u64,
) -> ScanNode {
    let root_buf = path.to_path_buf();
    let mut total_bytes: u64 = 0;
    let mut files_heap: BinaryHeap<Reverse<(u64, String)>> = BinaryHeap::new();
    let mut nested_folds: Vec<ScanNode> = Vec::new();

    let mut it = WalkDir::new(&root_buf).follow_links(false).into_iter();
    while let Some(entry) = it.next() {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                *walk_errors += 1;
                warn!("walk 错误: {}", err);
                continue;
            }
        };

        let p = entry.path();
        if p == root_buf.as_path() {
            continue;
        }

        if entry.file_type().is_dir() {
            if should_fold_directory(p) {
                let nested = scan_folded_subtree(p, top_n, size_metric, walk_errors, dirs_seen, files_scanned);
                total_bytes = total_bytes.saturating_add(nested.size);
                nested_folds.push(nested);
                it.skip_current_dir();
                continue;
            }
            *dirs_seen += 1;
            continue;
        }

        if !entry.file_type().is_file() {
            continue;
        }

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                *walk_errors += 1;
                warn!("metadata 失败 {}: {}", p.display(), e);
                continue;
            }
        };

        let len = file_size(&meta, size_metric);
        *files_scanned += 1;
        total_bytes = total_bytes.saturating_add(len);
        let path_str = p.to_string_lossy().into_owned();
        push_top_n_heap(&mut files_heap, top_n, len, path_str);
    }

    let mut merged: Vec<ScanNode> = Vec::new();
    for n in nested_folds {
        merged.push(n);
    }
    for Reverse((size, path_str)) in files_heap.into_iter() {
        let pb = PathBuf::from(&path_str);
        merged.push(ScanNode {
            name: node_name(&pb),
            path: path_str,
            size,
            is_dir: false,
            is_folded: false,
            children: Vec::new(),
        });
    }
    merged.sort_by(|a, b| b.size.cmp(&a.size));
    merged.truncate(top_n.max(1));

    let path_str = root_buf.to_string_lossy().into_owned();
    ScanNode {
        name: node_name(&root_buf),
        path: path_str,
        size: total_bytes,
        is_dir: true,
        is_folded: true,
        children: merged,
    }
}

fn merge_root_items(mut folded: Vec<ScanNode>, largest: Vec<LargeFile>) -> Vec<ScanNode> {
    let mut items: Vec<ScanNode> = Vec::new();
    for n in folded.drain(..) {
        items.push(n);
    }
    for f in largest {
        let pb = PathBuf::from(&f.path);
        items.push(ScanNode {
            name: node_name(&pb),
            path: f.path,
            size: f.size,
            is_dir: false,
            is_folded: false,
            children: Vec::new(),
        });
    }
    items.sort_by(|a, b| b.size.cmp(&a.size));
    items
}

pub fn scan_directory<F>(
    root: &Path,
    top_n: usize,
    progress_every: u64,
    size_metric: SizeMetric,
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
    let metric_label = size_metric.as_api_str().to_string();

    info!(
        "扫描开始 root={} size_metric={}",
        root_buf.display(),
        metric_label
    );

    #[cfg(unix)]
    {
        let mut it = WalkDir::new(&root_buf).follow_links(false).into_iter();
        while let Some(entry) = it.next() {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    inner.borrow_mut().walk_errors += 1;
                    warn!("walk 错误: {}", err);
                    continue;
                }
            };

            let path = entry.path();
            if path == root_buf.as_path() {
                continue;
            }

            if entry.file_type().is_dir() {
                if should_fold_directory(path) {
                    let mut w = 0u64;
                    let mut d = 0u64;
                    let mut f = 0u64;
                    let node = scan_folded_subtree(path, top_n, size_metric, &mut w, &mut d, &mut f);
                    {
                        let mut s = inner.borrow_mut();
                        s.walk_errors += w;
                        s.dirs_seen += d.saturating_add(1);
                        s.files_scanned += f;
                        s.bytes_total = s.bytes_total.saturating_add(node.size);
                        s.folded_bytes = s.folded_bytes.saturating_add(node.size);
                        s.folded_dirs += 1;
                        s.last_path = Some(node.path.clone());
                        s.folded_nodes.push(node);
                    }
                    it.skip_current_dir();
                    continue;
                }
                inner.borrow_mut().dirs_seen += 1;
                continue;
            }

            if !entry.file_type().is_file() {
                continue;
            }

            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    inner.borrow_mut().walk_errors += 1;
                    warn!("metadata 失败 {}: {}", path.display(), e);
                    continue;
                }
            };

            let len = file_size(&meta, size_metric);
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

    #[cfg(not(unix))]
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

            let len = file_size(&meta, size_metric);
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
        top_n: _,
        folded_nodes,
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

    let items = merge_root_items(folded_nodes, largest.clone());

    info!(
        "扫描结束 root={} files={} dirs_seen={} bytes_total={} folded_dirs={} folded_bytes={} walk_errors={} size_metric={}",
        root_buf.display(),
        files_scanned,
        dirs_seen,
        bytes_total,
        folded_dirs,
        folded_bytes,
        walk_errors,
        metric_label
    );

    Ok(ScanResult {
        root: root_buf.to_string_lossy().into_owned(),
        files_scanned,
        dirs_seen,
        bytes_total,
        largest_files: largest,
        items,
        walk_errors,
        folded_dirs,
        folded_bytes,
        size_metric: metric_label,
        rule_categories: crate::embedded_rules::build_rule_categories(&root_buf),
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
