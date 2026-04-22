//! 会话级内存常驻树 —— 对齐 Lemon Cleaner 的「一次扫描、内存常驻、导航即读」模型。
//!
//! 设计原则:
//! - 扫描完成后整棵 DirNode 树常驻进程内存（RwLock 保护），直到用户离开 Analyze 或显式重扫;
//! - 目录导航 = 一次 read lock + entries_for_dir，微秒级返回，零磁盘 I/O;
//! - 删除操作后就地修补内存树（remove node + adjust ancestor sizes），无需重扫;
//! - 不做跨 session 持久化（无快照文件、无 mtime 新鲜度校验）—— 磁盘分析是时间点快照，
//!   用户期望每次打开都看到最新数据，陈旧缓存无价值。

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use super::heap::{DirEntry, FileEntry};
use super::scanner::{DirNode, ScanOutcome, entries_for_dir};

// ── 全局会话树 ──────────────────────────────────────────────────────────────

/// 会话树：一次扫描的完整产物，常驻内存供后续导航读取。
pub struct SessionTree {
    /// 扫描根路径（如 "/" 或 "/Users/xxx"）
    pub root: String,
    /// 全量目录节点（key = 绝对路径）
    pub nodes: HashMap<String, DirNode>,
    /// Top-N 大文件
    pub large_files: Vec<FileEntry>,
    /// 根节点子树总大小
    pub total_size: i64,
    /// 根节点子树文件总数
    pub total_files: i64,
}

static ACTIVE_SESSION: OnceLock<RwLock<Option<SessionTree>>> = OnceLock::new();

fn session() -> &'static RwLock<Option<SessionTree>> {
    ACTIVE_SESSION.get_or_init(|| RwLock::new(None))
}

// ── 公共 API ────────────────────────────────────────────────────────────────

/// 扫描完成后存入会话树（替换上一次的树）。
pub fn store(root: &str, outcome: ScanOutcome) {
    let nodes_count = outcome.nodes.len();
    let total_size = outcome.result.total_size;
    let tree = SessionTree {
        root: root.to_string(),
        total_size: outcome.result.total_size,
        total_files: outcome.result.total_files,
        large_files: outcome.result.large_files,
        nodes: outcome.nodes,
    };
    let mut guard = session().write().unwrap_or_else(|e| e.into_inner());
    *guard = Some(tree);
    log::info!("[session] stored tree: root={root}, nodes={nodes_count}, total_size={total_size}");
}

/// 导航结果：当前目录的直接子项 + 聚合统计。
///
/// 不含 large_files —— Top20 大文件是全树固定数据，前端 scanRoot 时已拿到并缓存，
/// 每次导航重复 clone + 序列化纯属浪费（navigate 返回体中 large_files 为空数组，
/// 前端保留上一轮值）。
pub struct NavigateResult {
    pub entries: Vec<DirEntry>,
    pub total_size: i64,
    pub total_files: i64,
}

/// 从内存树中读取指定路径的直接子项（微秒级，零 I/O）。
///
/// 返回 `None` 表示：会话树不存在，或该路径不在树中（如 bundle 叶子钻取）。
pub fn navigate(path: &str) -> Option<NavigateResult> {
    let guard = session().read().unwrap_or_else(|e| e.into_inner());
    let tree = guard.as_ref()?;

    // bundle 叶子节点：内部未递归，需要按需扫描（返回 None 让调用方 fallback）
    if let Some(node) = tree.nodes.get(path) {
        if node.bundle_leaf
            && node.children.is_empty()
            && node.files.is_empty()
            && tree.root != path
        {
            return None;
        }
    }

    let entries = entries_for_dir(&tree.nodes, path);
    // 路径不在树中
    if entries.is_empty() && !tree.nodes.contains_key(path) {
        return None;
    }

    let node = tree.nodes.get(path)?;
    Some(NavigateResult {
        entries,
        total_size: node.size,
        total_files: node.total_files,
    })
}

/// 获取当前会话树的根路径（用于前端判断是否需要重扫）。
pub fn active_root() -> Option<String> {
    let guard = session().read().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().map(|t| t.root.clone())
}

/// 删除文件/目录后就地修补内存树：移除节点 + 逐级调整祖先 size。
///
/// 对齐 Lemon 的「删除后树即刻反映变化」行为，无需重扫。
pub fn remove_paths(deleted_paths: &[String]) {
    let mut guard = session().write().unwrap_or_else(|e| e.into_inner());
    let Some(tree) = guard.as_mut() else { return };

    for path in deleted_paths {
        remove_single_path(tree, path);
    }

    // 重算根节点 total_size / total_files
    if let Some(root_node) = tree.nodes.get(&tree.root) {
        tree.total_size = root_node.size;
        tree.total_files = root_node.total_files;
    }

    // 从 large_files 中移除已删除的
    tree.large_files
        .retain(|f| !deleted_paths.iter().any(|p| f.path.starts_with(p.as_str())));
}

/// 释放会话树（用户离开 Analyze 页面时调用）。
pub fn clear() {
    let mut guard = session().write().unwrap_or_else(|e| e.into_inner());
    if guard.is_some() {
        log::info!("[session] cleared");
    }
    *guard = None;
}

/// 克隆当前会话树的 nodes（仅供快照落盘兼容过渡期使用，后续删除）。
pub fn snapshot_nodes_clone() -> HashMap<String, DirNode> {
    let guard = session().read().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        Some(tree) => tree.nodes.clone(),
        None => HashMap::new(),
    }
}

// ── 内部实现 ────────────────────────────────────────────────────────────────

/// 从树中移除单个路径（文件或目录），并向上调整祖先的 size/total_files。
fn remove_single_path(tree: &mut SessionTree, path: &str) {
    // 确定被删除项的大小和类型
    let (removed_size, removed_files, is_dir) = if let Some(node) = tree.nodes.get(path) {
        // 是目录节点
        (node.size, node.total_files, true)
    } else {
        // 是文件：从父节点的 files 列表中查找
        let parent = parent_path(path);
        let name = file_name(path);
        if let Some(parent_node) = tree.nodes.get(parent) {
            let file_entry = parent_node.files.iter().find(|f| f.name == name);
            match file_entry {
                Some(f) => (f.size, 0, false),
                None => return, // 找不到，跳过
            }
        } else {
            return;
        }
    };

    if is_dir {
        // 递归删除目录及其所有子节点
        remove_subtree(tree, path);
    } else {
        // 从父节点的 files 列表中移除
        let parent = parent_path(path);
        let name = file_name(path);
        if let Some(parent_node) = tree.nodes.get_mut(parent) {
            parent_node.files.retain(|f| f.name != name);
            parent_node.own_size = parent_node.own_size.saturating_sub(removed_size);
            parent_node.child_files = (parent_node.child_files - 1).max(0);
        }
    }

    // 向上逐级调整祖先的 size 和 total_files
    let mut current = parent_path(path).to_string();
    loop {
        let Some(node) = tree.nodes.get_mut(&current) else {
            break;
        };
        node.size = node.size.saturating_sub(removed_size);
        node.total_files = (node.total_files - removed_files).max(0);
        if current == tree.root || current.is_empty() {
            break;
        }
        let next = parent_path(&current).to_string();
        if next == current {
            break;
        }
        current = next;
    }
}

/// 递归移除子树中所有节点。
fn remove_subtree(tree: &mut SessionTree, path: &str) {
    // 收集要删除的所有子路径
    let mut to_remove = Vec::new();
    collect_subtree_paths(&tree.nodes, path, &mut to_remove);
    to_remove.push(path.to_string());

    // 从父节点的 children 中移除
    let parent = parent_path(path);
    if let Some(parent_node) = tree.nodes.get_mut(parent) {
        parent_node.children.retain(|c| c != path);
        parent_node.child_dirs = (parent_node.child_dirs - 1).max(0);
    }

    // 删除所有节点
    for p in &to_remove {
        tree.nodes.remove(p);
    }
}

/// 递归收集子树下所有节点路径。
fn collect_subtree_paths(nodes: &HashMap<String, DirNode>, path: &str, out: &mut Vec<String>) {
    if let Some(node) = nodes.get(path) {
        for child in &node.children {
            out.push(child.clone());
            collect_subtree_paths(nodes, child, out);
        }
    }
}

/// 取路径的父目录：`/a/b` → `/a`；`/a` → `/`。
fn parent_path(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) => "/",
        Some(i) => &path[..i],
        None => "",
    }
}

/// 取路径的文件名部分。
fn file_name(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::analyze::scanner::DirNode;

    fn make_test_tree() -> SessionTree {
        let mut nodes = HashMap::new();
        nodes.insert(
            "/".to_string(),
            DirNode {
                name: "/".into(),
                size: 1000,
                own_size: 100,
                total_files: 10,
                child_files: 2,
                child_dirs: 2,
                children: vec!["/Users".into(), "/Apps".into()],
                files: vec![super::super::scanner::FileRec {
                    name: "root_file".into(),
                    size: 100,
                    is_dir: false,
                    is_symlink: false,
                }],
                ..DirNode::default()
            },
        );
        nodes.insert(
            "/Users".to_string(),
            DirNode {
                name: "Users".into(),
                size: 600,
                own_size: 200,
                total_files: 6,
                depth: 1,
                child_files: 2,
                child_dirs: 0,
                children: vec![],
                files: vec![super::super::scanner::FileRec {
                    name: "file_a".into(),
                    size: 200,
                    is_dir: false,
                    is_symlink: false,
                }],
                ..DirNode::default()
            },
        );
        nodes.insert(
            "/Apps".to_string(),
            DirNode {
                name: "Apps".into(),
                size: 300,
                own_size: 300,
                total_files: 3,
                depth: 1,
                child_files: 3,
                child_dirs: 0,
                children: vec![],
                files: vec![],
                ..DirNode::default()
            },
        );

        SessionTree {
            root: "/".into(),
            nodes,
            large_files: vec![FileEntry {
                name: "big.mkv".into(),
                path: "/Users/big.mkv".into(),
                size: 500,
            }],
            total_size: 1000,
            total_files: 10,
        }
    }

    #[test]
    fn navigate_returns_entries_for_known_path() {
        let tree = make_test_tree();
        let entries = entries_for_dir(&tree.nodes, "/");
        // root has: 1 file (root_file, 100) + 2 dirs (Users=600, Apps=300)
        assert_eq!(entries.len(), 3);
        // sorted by size desc: Users(600) > Apps(300) > root_file(100)
        assert_eq!(entries[0].name, "Users");
        assert_eq!(entries[1].name, "Apps");
        assert_eq!(entries[2].name, "root_file");
    }

    #[test]
    fn remove_file_adjusts_ancestor_sizes() {
        let mut tree = make_test_tree();
        // 删除 /Users/file_a (size=200)
        remove_single_path(&mut tree, "/Users/file_a");

        let users = tree.nodes.get("/Users").unwrap();
        assert_eq!(users.own_size, 0); // 200 - 200
        assert_eq!(users.size, 400); // 600 - 200
        assert_eq!(users.child_files, 1); // 2 - 1

        let root = tree.nodes.get("/").unwrap();
        assert_eq!(root.size, 800); // 1000 - 200
    }

    #[test]
    fn remove_dir_removes_subtree() {
        let mut tree = make_test_tree();
        // 删除 /Users 目录 (size=600)
        remove_single_path(&mut tree, "/Users");

        assert!(!tree.nodes.contains_key("/Users"));
        let root = tree.nodes.get("/").unwrap();
        assert_eq!(root.size, 400); // 1000 - 600
        assert!(!root.children.contains(&"/Users".to_string()));
        assert_eq!(root.child_dirs, 1); // 2 - 1
    }

    #[test]
    fn parent_path_works() {
        assert_eq!(parent_path("/Users/foo"), "/Users");
        assert_eq!(parent_path("/Users"), "/");
        assert_eq!(parent_path("/"), "/");
    }
}
