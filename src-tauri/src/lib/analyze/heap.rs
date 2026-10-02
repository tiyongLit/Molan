//! 与 `Mole/cmd/analyze/heap.go` 对齐：最小堆按 `Size`，Rust 用 `BinaryHeap<Reverse<T>>`。
//! Go `dirEntry` / `fileEntry` → `DirEntry` / `FileEntry`；Go `entryHeap` / `largeFileHeap` → `EntryHeap` / `LargeFileHeap`。

use serde::{Deserialize, Serialize};
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::time::SystemTime;

/// `Option<SystemTime>` 序列化为 `Option<i64>`（Unix 秒）。
///
/// 用于 `cache.rs` 把 `DirEntry::last_access` 持久化到 JSON。秒级精度对缓存
/// freshness 判断足够（Go 版用 gob 直接编 time.Time，但 Rust 没必要绑死跨语言格式）。
pub mod opt_systime_secs {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub fn serialize<S: Serializer>(t: &Option<SystemTime>, s: S) -> Result<S::Ok, S::Error> {
        let opt: Option<i64> = t.as_ref().map(|t| {
            t.duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        });
        opt.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<SystemTime>, D::Error> {
        let opt = Option::<i64>::deserialize(d)?;
        Ok(opt.map(|secs| {
            if secs >= 0 {
                UNIX_EPOCH + Duration::from_secs(secs as u64)
            } else {
                UNIX_EPOCH
            }
        }))
    }
}

/// Go `dirEntry`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub size: i64,
    pub is_dir: bool,
    #[serde(with = "opt_systime_secs", default)]
    pub last_access: Option<SystemTime>,
    /// GUI 扩展（V2 前端副标题用，Go CLI 契约无此字段）：
    /// symlink 标记 — Go 版靠 name 的 `" →"` 后缀隐式表达（CLI 展示用），
    /// GUI 不需要箭头后缀，改用显式布尔标记；`serde(default)` 保证旧磁盘缓存兼容。
    #[serde(default)]
    pub is_symlink: bool,
    /// GUI 扩展：直接子项统计（文件数 / 目录数 / 符号链接数），
    /// 由组装点从子扫描结果的 entries 分类计数得出，随磁盘缓存持久化。
    #[serde(default)]
    pub child_files: i64,
    #[serde(default)]
    pub child_dirs: i64,
    #[serde(default)]
    pub child_links: i64,
    /// GUI 扩展：bundle 叶子捷径标记（首扫叶子化、钻取按需扫描），旧缓存 serde(default) 兼容。
    #[serde(default)]
    pub is_bundle_leaf: bool,
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub bundle_display_name: Option<String>,
}

impl PartialEq for DirEntry {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size && self.path == other.path && self.name == other.name
    }
}

impl Eq for DirEntry {}

impl PartialOrd for DirEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 与 Go `entryHeap.Less` 一致：按 `Size` 升序；`tie-break` 保证全序。
impl Ord for DirEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.size
            .cmp(&other.size)
            .then_with(|| self.path.cmp(&other.path))
            .then_with(|| self.name.cmp(&other.name))
    }
}

/// Go `fileEntry`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub size: i64,
}

impl PartialEq for FileEntry {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size && self.path == other.path && self.name == other.name
    }
}

impl Eq for FileEntry {}

impl PartialOrd for FileEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FileEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.size
            .cmp(&other.size)
            .then_with(|| self.path.cmp(&other.path))
            .then_with(|| self.name.cmp(&other.name))
    }
}

/// Go `entryHeap`：`Less` 为 `Size` 升序的最小堆；此处为 `BinaryHeap<Reverse<DirEntry>>`。
#[derive(Debug, Default)]
pub struct EntryHeap {
    inner: BinaryHeap<Reverse<DirEntry>>,
}

impl EntryHeap {
    /// Go `entryHeap.Len`
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Go `entryHeap.Push`（经 `heap.Push`）
    pub fn push(&mut self, x: DirEntry) {
        self.inner.push(Reverse(x));
    }

    /// Go `entryHeap.Pop`（经 `heap.Pop`）：弹出当前最小 `Size` 元素。
    pub fn pop(&mut self) -> Option<DirEntry> {
        self.inner.pop().map(|r| r.0)
    }

    pub fn peek(&self) -> Option<&DirEntry> {
        self.inner.peek().map(|r| &r.0)
    }
}

// ── helpers for tests ──

#[cfg(test)]
impl DirEntry {
    fn new(name: &str, path: &str, size: i64) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
            size,
            is_dir: false,
            last_access: None,
            is_symlink: false,
            child_files: 0,
            child_dirs: 0,
            child_links: 0,
            is_bundle_leaf: false,
            bundle_id: None,
            bundle_display_name: None,
        }
    }
}

#[cfg(test)]
impl FileEntry {
    fn new(name: &str, path: &str, size: i64) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
            size,
        }
    }
}

/// Go `largeFileHeap`
#[derive(Debug, Default)]
pub struct LargeFileHeap {
    inner: BinaryHeap<Reverse<FileEntry>>,
}

impl LargeFileHeap {
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn push(&mut self, x: FileEntry) {
        self.inner.push(Reverse(x));
    }

    pub fn pop(&mut self) -> Option<FileEntry> {
        self.inner.pop().map(|r| r.0)
    }

    pub fn peek(&self) -> Option<&FileEntry> {
        self.inner.peek().map(|r| &r.0)
    }
}

// ── tests (mirrors Mole/cmd/analyze/heap_test.go) ──

#[cfg(test)]
mod tests {
    use super::*;

    // ── EntryHeap ──

    #[test]
    fn entry_heap_basic_operations() {
        let mut h = EntryHeap::default();

        h.push(DirEntry::new("medium", "/b", 500));
        h.push(DirEntry::new("small", "/a", 100));
        h.push(DirEntry::new("large", "/c", 1000));

        assert_eq!(h.len(), 3, "Len after 3 pushes");

        // Min-heap: smallest Size comes out first.
        let first = h.pop().unwrap();
        assert_eq!(first.name, "small");
        assert_eq!(first.size, 100);

        let second = h.pop().unwrap();
        assert_eq!(second.name, "medium");
        assert_eq!(second.size, 500);

        let third = h.pop().unwrap();
        assert_eq!(third.name, "large");
        assert_eq!(third.size, 1000);

        assert_eq!(h.len(), 0, "Len after all pops");
    }

    #[test]
    fn entry_heap_empty() {
        let h = EntryHeap::default();
        assert_eq!(h.len(), 0);
        assert!(h.is_empty());
    }

    #[test]
    fn entry_heap_single_element() {
        let mut h = EntryHeap::default();
        h.push(DirEntry::new("only", "/x", 42));
        let popped = h.pop().unwrap();
        assert_eq!(popped.name, "only");
        assert_eq!(popped.size, 42);
        assert!(h.is_empty());
    }

    #[test]
    fn entry_heap_equal_sizes() {
        let mut h = EntryHeap::default();

        h.push(DirEntry::new("a", "/1", 100));
        h.push(DirEntry::new("b", "/2", 100));
        h.push(DirEntry::new("c", "/3", 100));

        // Rust tie-breaks by path+name when sizes equal; Go has no guarantee.
        // Both are correct min-heaps — all popped entries have the same size.
        for _ in 0..3 {
            let popped = h.pop().unwrap();
            assert_eq!(popped.size, 100);
        }
        assert!(h.is_empty());
    }

    #[test]
    fn entry_heap_peek() {
        let mut h = EntryHeap::default();
        h.push(DirEntry::new("big", "/a", 999));
        h.push(DirEntry::new("tiny", "/b", 1));

        let top = h.peek().unwrap();
        assert_eq!(top.name, "tiny");
        assert_eq!(top.size, 1);
        assert_eq!(h.len(), 2); // peek doesn't remove
    }

    // ── LargeFileHeap ──

    #[test]
    fn large_file_heap_basic_operations() {
        let mut h = LargeFileHeap::default();

        h.push(FileEntry::new("medium.bin", "/b", 500));
        h.push(FileEntry::new("small.txt", "/a", 100));
        h.push(FileEntry::new("large.iso", "/c", 1000));

        assert_eq!(h.len(), 3);

        // Min-heap: smallest comes out first.
        let first = h.pop().unwrap();
        assert_eq!(first.name, "small.txt");
        assert_eq!(first.size, 100);

        let second = h.pop().unwrap();
        assert_eq!(second.name, "medium.bin");
        assert_eq!(second.size, 500);

        let third = h.pop().unwrap();
        assert_eq!(third.name, "large.iso");
        assert_eq!(third.size, 1000);

        assert_eq!(h.len(), 0);
    }

    #[test]
    fn large_file_heap_top_n_largest() {
        // The real usage pattern: keep only the top N largest entries.
        let mut h = LargeFileHeap::default();
        let max_size = 3;

        let files = [
            FileEntry::new("a", "/a", 50),
            FileEntry::new("b", "/b", 200),
            FileEntry::new("c", "/c", 30),
            FileEntry::new("d", "/d", 150),
            FileEntry::new("e", "/e", 300),
        ];

        for f in files {
            h.push(f);
            if h.len() > max_size {
                h.pop(); // Remove smallest, keep only top N.
            }
        }

        assert_eq!(h.len(), max_size);

        // Extract remaining — should be the 3 largest: 150, 200, 300.
        // Min-heap pops in ascending Size order.
        let sizes: Vec<i64> = std::iter::from_fn(|| h.pop()).map(|e| e.size).collect();
        assert_eq!(sizes, vec![150, 200, 300]);
    }

    #[test]
    fn large_file_heap_empty_and_peek() {
        let mut h = LargeFileHeap::default();
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
        assert!(h.pop().is_none());
        assert!(h.peek().is_none());

        h.push(FileEntry::new("only", "/x", 7));
        assert_eq!(h.peek().unwrap().size, 7);
        assert_eq!(h.pop().unwrap().size, 7);
        assert!(h.is_empty());
    }
}
