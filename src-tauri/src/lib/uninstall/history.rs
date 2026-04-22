//! 卸载历史记录管理
//!
//! 存储位置：~/.config/mole/uninstall_history.json
//! 保留策略：最近 20 条
//!
//! 对齐 Pearcleaner 的 UndoHistoryManager，但简化实现：
//! - 不打包到废纸篓文件夹（依赖系统废纸篓的原生恢复能力）
//! - 只记录删除路径清单（用于追溯）
//! - 提供"打开废纸篓"按钮（让用户手动恢复）

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// 卸载历史记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UninstallHistoryRecord {
    /// 唯一 ID（使用时间戳）
    pub id: String,
    /// ISO 8601 时间戳
    pub timestamp: String,
    /// 应用名称
    pub app_name: String,
    /// 应用路径
    pub app_path: String,
    /// 是否只清数据（Clear Data 模式）
    pub data_only: bool,
    /// 删除的路径列表
    pub deleted_paths: Vec<String>,
    /// 进废纸篓的数量（可恢复）
    pub trashed_count: usize,
    /// 永久删除的数量（不可恢复）
    pub sudo_removed_count: usize,
    /// 总大小（字节）
    pub total_size_bytes: u64,
    /// 文件总数
    pub file_count: usize,
}

/// 历史记录存储结构
#[derive(Debug, Default, Serialize, Deserialize)]
struct HistoryStore {
    records: Vec<UninstallHistoryRecord>,
}

/// 获取历史记录文件路径
fn history_file_path() -> PathBuf {
    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("mole");

    // 确保目录存在
    fs::create_dir_all(&config_dir).ok();

    config_dir.join("uninstall_history.json")
}

/// 加载历史记录
pub fn load_history() -> Vec<UninstallHistoryRecord> {
    let path = history_file_path();

    if !path.exists() {
        return Vec::new();
    }

    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str::<HistoryStore>(&content)
            .map(|store| store.records)
            .unwrap_or_default(),
        Err(e) => {
            log::error!("[uninstall.history] 加载历史记录失败: {}", e);
            Vec::new()
        }
    }
}

/// 保存历史记录
fn save_history(records: Vec<UninstallHistoryRecord>) -> Result<(), String> {
    let path = history_file_path();
    let store = HistoryStore { records };

    serde_json::to_string_pretty(&store)
        .map_err(|e| format!("序列化失败: {}", e))
        .and_then(|json| fs::write(&path, json).map_err(|e| format!("写入文件失败: {}", e)))
}

/// 添加历史记录
pub fn add_record(record: UninstallHistoryRecord) -> Result<(), String> {
    let mut records = load_history();

    // 插入到开头（最新的在前）
    records.insert(0, record);

    // 自动清理：保留最近 20 条
    if records.len() > 20 {
        records.truncate(20);
    }

    save_history(records)
}

/// 清空历史记录
pub fn clear_history() -> Result<(), String> {
    save_history(Vec::new())
}

/// 获取历史记录
pub fn get_history() -> Vec<UninstallHistoryRecord> {
    load_history()
}

/// 生成唯一 ID（使用时间戳）
fn generate_id() -> String {
    chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or(0)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_history_file_path() {
        let path = history_file_path();
        assert!(path.to_string_lossy().contains("mole"));
        assert!(path.to_string_lossy().ends_with("uninstall_history.json"));
    }

    #[test]
    fn test_generate_id() {
        let id1 = generate_id();
        let id2 = generate_id();
        // ID 应该不同（纳秒级精度）
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_serialization() {
        let record = UninstallHistoryRecord {
            id: "test-id".to_string(),
            timestamp: "2026-09-03T14:30:25Z".to_string(),
            app_name: "TestApp".to_string(),
            app_path: "/Applications/TestApp.app".to_string(),
            data_only: false,
            deleted_paths: vec!["/path1".to_string(), "/path2".to_string()],
            trashed_count: 2,
            sudo_removed_count: 0,
            total_size_bytes: 1024,
            file_count: 2,
        };

        let json = serde_json::to_string(&record).unwrap();
        let deserialized: UninstallHistoryRecord = serde_json::from_str(&json).unwrap();

        assert_eq!(record.id, deserialized.id);
        assert_eq!(record.app_name, deserialized.app_name);
        assert_eq!(record.deleted_paths.len(), deserialized.deleted_paths.len());
    }
}
