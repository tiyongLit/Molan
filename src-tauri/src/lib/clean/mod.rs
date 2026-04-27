pub mod app_caches;
pub mod apps;
pub mod brew;
pub mod caches;
pub mod dev;
pub mod hints;
pub mod launch_services;
pub mod maven;
pub mod project;
pub mod purge_shared;
pub mod system;
pub mod user;

#[derive(Debug, Clone)]
pub struct SubItemResult {
    pub id: String,
    pub title: String,
    pub size_kb: u64,
    pub file_count: u64,
    /// 可选的文件系统路径，用于白名单匹配（二级/三级勾选）。
    /// 各模块在构建子项时可填入其清理的主要路径，clean.rs 据此判断 whitelist_matched。
    pub path: Option<String>,
}

impl SubItemResult {
    pub fn new(id: &str, title: &str, size_kb: u64, file_count: u64) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            size_kb,
            file_count,
            path: None,
        }
    }

    pub fn with_path(id: &str, title: &str, size_kb: u64, file_count: u64, path: &str) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            size_kb,
            file_count,
            path: Some(path.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModuleScanResult {
    pub items: Vec<SubItemResult>,
}

impl ModuleScanResult {
    pub fn single(id: &str, title: &str, size_kb: u64, file_count: u64) -> Self {
        Self {
            items: vec![SubItemResult::new(id, title, size_kb, file_count)],
        }
    }

    pub fn single_with_path(
        id: &str,
        title: &str,
        size_kb: u64,
        file_count: u64,
        path: &str,
    ) -> Self {
        Self {
            items: vec![SubItemResult::with_path(
                id, title, size_kb, file_count, path,
            )],
        }
    }

    pub fn empty() -> Self {
        Self { items: vec![] }
    }

    pub fn total_kb(&self) -> u64 {
        self.items.iter().map(|i| i.size_kb).sum()
    }

    pub fn total_count(&self) -> u64 {
        self.items.iter().map(|i| i.file_count).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_has_zero_totals() {
        let r = ModuleScanResult::empty();
        assert_eq!(r.total_kb(), 0);
        assert_eq!(r.total_count(), 0);
        assert!(r.items.is_empty());
    }

    #[test]
    fn single_stores_values() {
        let r = ModuleScanResult::single("a", "A", 100, 5);
        assert_eq!(r.total_kb(), 100);
        assert_eq!(r.total_count(), 5);
        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].id, "a");
        assert_eq!(r.items[0].title, "A");
    }

    #[test]
    fn single_zero_skipped_in_total() {
        let r = ModuleScanResult::single("x", "X", 0, 0);
        assert_eq!(r.total_kb(), 0);
        assert_eq!(r.total_count(), 0);
        assert_eq!(r.items.len(), 1);
    }

    #[test]
    fn multiple_items_sum_correctly() {
        let r = ModuleScanResult {
            items: vec![
                SubItemResult::new("a", "A", 10, 1),
                SubItemResult::new("b", "B", 20, 2),
                SubItemResult::new("c", "C", 0, 3),
            ],
        };
        assert_eq!(r.total_kb(), 30);
        assert_eq!(r.total_count(), 6);
        assert_eq!(r.items.len(), 3);
    }

    #[test]
    fn total_kb_u64_no_overflow_on_empty() {
        let r = ModuleScanResult::empty();
        assert_eq!(r.total_kb(), 0u64);
    }

    #[test]
    fn item_new_clones_str() {
        let s = String::from("hello");
        let item = SubItemResult::new(&s, "world", 5, 1);
        assert_eq!(item.id, "hello");
        assert_eq!(item.title, "world");
    }
}
