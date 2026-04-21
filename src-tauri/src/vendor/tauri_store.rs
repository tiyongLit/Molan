// 配置存储：与 Electron ElectronStore 对齐的默认值与单 key 存储

use serde::{Deserialize, Serialize};
use std::io;
use tauri::AppHandle;
use tauri_plugin_store::{Store, StoreExt};

pub const CONFIG_STORE_PATH: &str = "config.json";
const CONFIG_KEY: &str = "config";

/// 应用配置，字段与 ElectronStore defaults 对齐（JSON 用 camelCase）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub prompt_tone: bool,
    pub proxy: String,
    pub use_proxy: bool,
    pub delete_segments: bool,
    pub open_in_new_window: bool,
    pub block_ads: bool,
    pub theme: String,
    pub use_extension: bool,
    pub is_mobile: bool,
    pub max_runner: u32,
    pub language: String,
    pub notify_placement: String,
    pub show_terminal: bool,
    pub privacy: bool,
    pub machine_id: String,
}

impl AppConfig {
    /// 与 ElectronStore 一致的默认值
    pub fn default_values() -> Self {
        Self {
            prompt_tone: true,
            proxy: String::new(),
            use_proxy: false,
            delete_segments: true,
            open_in_new_window: false,
            block_ads: true,
            theme: "system".to_string(),
            use_extension: false,
            is_mobile: false,
            max_runner: 2,
            language: "system".to_string(),
            notify_placement: "topRight".to_string(),
            show_terminal: false,
            privacy: false,
            machine_id: String::new(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self::default_values()
    }
}

fn store_not_loaded_error() -> tauri_plugin_store::Error {
    tauri_plugin_store::Error::Io(io::Error::new(
        io::ErrorKind::NotFound,
        "config store not loaded",
    ))
}

/// 从已加载的 store 中读取配置；若不存在或反序列化失败则返回默认并写入
pub fn load_or_init(app: &AppHandle) -> tauri_plugin_store::Result<AppConfig> {
    let store = app.get_store(CONFIG_STORE_PATH).ok_or_else(store_not_loaded_error)?;
    let cfg = get_config_inner(store.as_ref()).unwrap_or_else(AppConfig::default);
    if store.is_empty() {
        let _ = set_config_inner(store.as_ref(), &cfg);
        let _ = store.save();
    }
    Ok(cfg)
}

/// 只读获取配置；若 store 未加载或 key 不存在则返回默认值
pub fn get_config(app: &AppHandle) -> tauri_plugin_store::Result<AppConfig> {
    let store = app.get_store(CONFIG_STORE_PATH).ok_or_else(store_not_loaded_error)?;
    Ok(get_config_inner(store.as_ref()).unwrap_or_default())
}

/// 写入并保存整个配置
pub fn set_config(app: &AppHandle, cfg: &AppConfig) -> tauri_plugin_store::Result<()> {
    let store = app.get_store(CONFIG_STORE_PATH).ok_or_else(store_not_loaded_error)?;
    set_config_inner(store.as_ref(), cfg)?;
    store.save()
}

fn get_config_inner(store: &Store<tauri::Wry>) -> Option<AppConfig> {
    let val = store.get(CONFIG_KEY)?;
    serde_json::from_value(val.clone()).ok()
}

fn set_config_inner(store: &Store<tauri::Wry>, cfg: &AppConfig) -> tauri_plugin_store::Result<()> {
    let val = serde_json::to_value(cfg).map_err(|e| tauri_plugin_store::Error::Serialize(Box::new(e)))?;
    store.set(CONFIG_KEY.to_string(), val);
    Ok(())
}
