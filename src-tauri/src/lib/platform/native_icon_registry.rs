//! NativeIconRegistry —— 内容寻址（content-addressed）原生图标注册表。
//!
//! # 核心思路
//!
//! 把「图标存储」和「路径索引」拆成两对 map：
//!
//! ```text
//! pathIndex:     path → { content_id, mtime }     // 路径级：mtime 校验
//! contentStore:  content_id → svg_data_uri        // 内容级：天然去重
//! ```
//!
//! `content_id` = SHA-256(SVG data URI 字节) 的前 32 位 hex。**同一像素内容 = 同一 ID**，
//! 927 个路径里只有 108 种不同图标时（实测数据），磁盘 / 内存 / IPC 全部按 108 计数，
//! 而不是 927 —— 普通文件夹图标 583 份副本被压到 1 份。
//!
//! # 持久化（跨会话命中）
//!
//! ```text
//! <app_cache>/native-icon-registry-v1/
//!   blobs/<content_id>.svg     # SVG data URI 文本（单份 ~15KB）
//!   index.jsonl                 # 追加写：每行 {"p":"/path","c":"<cid>","m":<mtime_ms>}
//! ```
//!
//! 启动时加载 `index.jsonl` 重建 `pathIndex`；新写入追加写盘，旧条目在下次命中时被新
//! 条目覆盖（同名键的最新 mtime / content_id 生效）。
//!
//! # 响应语义（与前端 contentStore 的契约）
//!
//! `resolve` 的响应里 `contents` **包含本响应所有 entries 引用到的内容**（不仅仅是
//! 本次新编码的项）：应用重启后前端 contentStore 为空，若命中项不回传内容，前端
//! 永远拿不到 SVG（emoji 兜底永不恢复）。批内按 content_id 去重，前端合并幂等，
//! 同一份内容在一次前端会话内最多传输一次。
//!
//! # 编码成本
//!
//! 未命中路径统一走 `file_icons_batch`：分批主线程调度（CHUNK=8，批间让出
//! runloop）+ 栅格指纹（TIFF 字节）内容级去重——普通文件夹只编码一次，583 个
//! 文件夹路径的真实编码次数从 583 塌缩为 1；分批保证图标渐进出现而非冻结主线程数秒。
//!
//! # 旧缓存迁移
//!
//! `ICON_CACHE_VERSION = 3`，目录名随之变化；`LEGACY_ICON_CACHE_DIRS` 含 `icon-cache`
//! 与 `icon-cache-v2`，首次启动时一次性删除（~794MB 孤儿文件自动清理）。
//!
//! # 线程安全
//!
//! `OnceLock<Mutex<RegistryState>>` 单例：Tauri 的 `spawn_blocking` 池内多 worker
//! 并发进入时互斥；mutex 中毒后取回内部值继续（与 `controllers/status.rs` 同款）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tauri::Manager;

// ── 全局 app_handle 单例 ──
//
// 进程级单例：状态采集链路（metrics_process::top_processes）在 rayon scoped thread 里
// 调用 app_icon_svg，无法从 MetricsCollector 传递 AppHandle。这里用 OnceLock 存一份，
// 第一次调用 `set_app_handle` 时写入，后续 `resolve_single` / `resolve` 内部直接读取。
//
// 设置入口：
//   - status 控制器启动 watch 时调用；
//   - 第一次 IPC `mole_native_icons_resolve` 也会调用（幂等）。
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

/// 设置进程级 AppHandle（幂等）。由 status 控制器或首次 IPC 调用。
pub fn set_app_handle(handle: tauri::AppHandle) {
    let _ = APP_HANDLE.set(handle);
}

// ── 缓存版本 ──

/// 原生图标注册表语义版本：底层编码规则变化时递增（目录名随之变化，旧目录自动清理）。
/// v1: 64px 像素精确 + SVG 信封 + 内容寻址（首次引入）。
pub const NATIVE_ICON_CACHE_VERSION: u32 = 1;

/// 历史缓存目录名（含路径级缓存老版本 + 当前模块自身迭代）：升级版本后一次性清理，
/// 避免旧图标文件（单张数百 KB）成为孤儿。
///
/// - `icon-cache` / `icon-cache-v2`：早期路径级缓存；
/// - `icon-cache-v3`：旧控制器 `mole_get_icon_cached` 的缓存目录（PNG base64，
///   单张可达数百 KB），随旧命令下线一并清理。
pub const LEGACY_ICON_CACHE_DIRS: &[&str] = &["icon-cache", "icon-cache-v2", "icon-cache-v3"];

// ── 数据结构 ──

/// 注册表对外响应（与前端 `NativeIconsResponse` TS 类型对齐）。
///
/// - `entries`: path → content_id（前端索引层）
/// - `contents`: content_id → SVG data URI（前端内容层）
#[derive(Debug, Clone, Default, Serialize)]
pub struct NativeIconsResponse {
    pub entries: HashMap<String, String>,
    pub contents: HashMap<String, String>,
}

/// 索引条目（内存 + 磁盘 JSONL 共享结构）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexEntry {
    /// 路径
    p: String,
    /// content_id（128 位 hash hex）
    c: String,
    /// mtime（毫秒）
    m: u64,
}

/// 注册表内部状态（被 Mutex 包裹）。
struct RegistryState {
    /// content_id → SVG data URI
    contents: HashMap<String, String>,
    /// path → IndexEntry
    by_path: HashMap<String, IndexEntry>,
    /// 磁盘缓存根目录（`<app_cache>/native-icon-registry-v1/`）
    cache_dir: PathBuf,
}

static REGISTRY: OnceLock<Mutex<RegistryState>> = OnceLock::new();

fn registry_lock() -> std::sync::MutexGuard<'static, RegistryState> {
    REGISTRY
        .get_or_init(|| {
            Mutex::new(RegistryState {
                contents: HashMap::new(),
                by_path: HashMap::new(),
                cache_dir: PathBuf::new(),
            })
        })
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ── 公共 API ──

/// 初始化注册表：设置缓存目录、清理旧版本目录、从磁盘加载索引。
///
/// 在应用 `setup` 阶段或首次 `resolve` 前调用（`resolve` 也会按需触发）。
pub fn init(app_handle: &tauri::AppHandle) {
    // 同时把 app_handle 存到全局单例（供 resolve_single 在 rayon 线程里用）
    set_app_handle(app_handle.clone());
    let cache_dir = cache_dir(app_handle);
    let _ = std::fs::create_dir_all(&cache_dir);
    cleanup_legacy_caches(app_handle);

    let mut state = registry_lock();
    state.cache_dir = cache_dir.clone();
    // 加载磁盘索引到内存
    load_index_from_disk(&cache_dir, &mut state);
}

/// 批量解析：对每条路径做 mtime 校验，命中则复用 content_id，未命中则批量编码。
///
/// 响应语义：
/// - `entries`: path → content_id，**所有**成功解析的路径都会出现（含命中项）；
/// - `contents`: content_id → SVG data URI，**覆盖本响应 entries 引用到的全部内容**
///   （跨会话重启时前端 contentStore 为空，必须随 entries 一起回传才能渲染），
///   批内按 content_id 去重；前端合并幂等。
///
/// 未命中路径统一走 `macos_file_icon::file_icons_batch`：分批主线程调度
/// （CHUNK=8，批间让出 runloop）+ 栅格指纹（TIFF 字节）内容级去重。
pub fn resolve(app_handle: &tauri::AppHandle, paths: &[String]) -> NativeIconsResponse {
    // 确保注册表已初始化（首次调用时懒加载）
    {
        let state = registry_lock();
        if state.cache_dir.as_os_str().is_empty() {
            drop(state);
            init(app_handle);
        }
    }

    let state = registry_lock();
    let mut response = NativeIconsResponse::default();
    // (path, mtime)：pathIndex 未命中或内容缺失、待批量编码的路径
    let mut misses: Vec<(String, u64)> = Vec::new();

    for path in paths {
        if path.is_empty() {
            continue;
        }

        let current_mtime = read_mtime(path);
        if current_mtime == 0 {
            // 路径不存在或无法 stat：跳过（前端回退到 emoji）
            continue;
        }

        // ── pathIndex 命中、mtime 一致、且内容仍在内存：复用 content_id ──
        // 内容缺失（blob 被外部清理）时视为未命中，走重编码自愈。
        if let Some(entry) = state.by_path.get(path) {
            if entry.m == current_mtime && state.contents.contains_key(&entry.c) {
                response.entries.insert(path.clone(), entry.c.clone());
                if !response.contents.contains_key(&entry.c) {
                    response
                        .contents
                        .insert(entry.c.clone(), state.contents[&entry.c].clone());
                }
                continue;
            }
        }
        misses.push((path.clone(), current_mtime));
    }

    if misses.is_empty() {
        return response;
    }

    // ── 未命中路径批量编码：分批主线程调度 + 栅格指纹去重 ──
    // 先释放注册表锁再编码：编码可能持续数百毫秒到数秒（分批 + 批间让出），
    // 期间状态采集（rayon 线程里的 resolve_single 快速路径）不应被整段阻塞；
    // 同时消除「持锁等主队列 × 主线程等锁」的死锁窗口。编码产物内容寻址
    // （SHA-256 前缀）、写回幂等，与并发 resolve 竞争无副作用。
    drop(state);

    let miss_paths: Vec<String> = misses.iter().map(|(p, _)| p.clone()).collect();
    let encoded = encode_native_icons_batch(&miss_paths);

    let mut state = registry_lock();
    for ((path, current_mtime), maybe_uri) in misses.into_iter().zip(encoded) {
        let Some(svg_uri) = maybe_uri else {
            continue;
        };
        let cid = sha256_prefix(&svg_uri);

        // contentStore 去重：同一内容只存一份
        if !state.contents.contains_key(&cid) {
            state.contents.insert(cid.clone(), svg_uri.clone());
            // 同时写盘（blob）
            persist_blob(&state.cache_dir, &cid, &svg_uri);
        }
        // 追加写索引（同一路径的新条目会覆盖旧条目语义）
        persist_index_entry(&state.cache_dir, &path, &cid, current_mtime);

        // 更新内存索引
        state.by_path.insert(
            path.clone(),
            IndexEntry {
                p: path.clone(),
                c: cid.clone(),
                m: current_mtime,
            },
        );

        response.entries.insert(path.clone(), cid.clone());
        if !response.contents.contains_key(&cid) {
            response.contents.insert(cid.clone(), svg_uri);
        }
    }

    response
}

/// 单路径解析（进程域场景：running_apps / metrics_process）。
///
/// 返回 SVG data URI 或 None。与批量走同一管线，命中 contentStore 时零原生调用。
///
/// 无需传 app_handle：第一次 IPC `mole_native_icons_resolve` 或 status 控制器启动时
/// 已调用 `set_app_handle`，本函数从进程级单例读取。
///
/// 调用方需在 status 采集启动前确保 `set_app_handle` 已调用（status 控制器已处理）。
pub fn resolve_single(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let handle = APP_HANDLE.get()?;

    // 快路径：内存命中直接返回（不构造整份响应；进程域每帧调用走这里）
    {
        let state = registry_lock();
        let current_mtime = read_mtime(path);
        if current_mtime != 0 {
            if let Some(entry) = state.by_path.get(path) {
                if entry.m == current_mtime && state.contents.contains_key(&entry.c) {
                    return state.contents.get(&entry.c).cloned();
                }
            }
        }
    }

    // miss：走完整 resolve（编码 + 落盘 + 索引）
    let response = resolve(handle, &[path.to_string()]);
    let cid = response.entries.get(path)?;
    response.contents.get(cid).cloned()
}

// ── 内部工具 ──

fn cache_dir(app_handle: &tauri::AppHandle) -> PathBuf {
    let root = app_handle.path().app_cache_dir().unwrap_or_default();
    root.join(format!("native-icon-registry-v{NATIVE_ICON_CACHE_VERSION}"))
}

fn cleanup_legacy_caches(app_handle: &tauri::AppHandle) {
    let root = app_handle.path().app_cache_dir().unwrap_or_default();
    for legacy in LEGACY_ICON_CACHE_DIRS {
        let old = root.join(legacy);
        if old.is_dir() {
            let _ = std::fs::remove_dir_all(&old);
        }
    }
}

fn read_mtime(path: &str) -> u64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn sha256_prefix(input: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    // 用两个独立种子取 128 位指纹（两个 DefaultHasher 拼接），
    // 足以杜绝内容冲突（2^64 ≈ 1.8e19，远低于图标总数），且无新依赖。
    let mut h1 = DefaultHasher::new();
    input.hash(&mut h1);
    let mut h2 = DefaultHasher::new();
    0xDEADBEEF_u64.hash(&mut h2);
    input.hash(&mut h2);
    format!("{:016x}{:016x}", h1.finish(), h2.finish())
}

/// 调用 macos_file_icon 批量编码原生图标（分批主线程调度 + 栅格指纹去重）。
/// 非 macOS 平台返回与输入等长的 None 列表。
fn encode_native_icons_batch(paths: &[String]) -> Vec<Option<String>> {
    #[cfg(target_os = "macos")]
    {
        crate::platform::macos_file_icon::file_icons_batch(paths)
            .unwrap_or_else(|_| vec![None; paths.len()])
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = paths;
        Vec::new()
    }
}

// ── 持久化 ──

fn persist_blob(cache_dir: &PathBuf, cid: &str, svg_uri: &str) {
    let blob_path = cache_dir.join("blobs").join(format!("{}.svg", cid));
    if blob_path.exists() {
        return;
    }
    let _ = std::fs::create_dir_all(cache_dir.join("blobs"));
    if let Err(e) = std::fs::write(&blob_path, svg_uri) {
        log::warn!(
            "[native_icon_registry] blob write failed for {}: {}",
            blob_path.display(),
            e
        );
    }
}

fn persist_index_entry(cache_dir: &PathBuf, path: &str, cid: &str, mtime: u64) {
    let entry = IndexEntry {
        p: path.to_string(),
        c: cid.to_string(),
        m: mtime,
    };
    let Ok(line) = serde_json::to_string(&entry) else {
        return;
    };
    // 追加写（open + write 一次完成，避免持有文件句柄跨多次调用）
    let mut file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(cache_dir.join("index.jsonl"))
    {
        Ok(f) => f,
        Err(e) => {
            log::warn!("[native_icon_registry] index.jsonl open failed: {}", e);
            return;
        }
    };
    let _ = writeln!(file, "{}", line);
}

fn load_index_from_disk(cache_dir: &PathBuf, state: &mut RegistryState) {
    // 1. 加载所有 blob 到 contentStore
    let blobs_dir = cache_dir.join("blobs");
    if blobs_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&blobs_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("svg") {
                    continue;
                }
                let Some(cid) = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
                else {
                    continue;
                };
                if let Ok(svg_uri) = std::fs::read_to_string(&path) {
                    state.contents.insert(cid, svg_uri);
                }
            }
        }
    }

    // 2. 加载 index.jsonl 重建 pathIndex（同名路径取最新条目）
    let index_path = cache_dir.join("index.jsonl");
    let Ok(file) = std::fs::File::open(&index_path) else {
        return;
    };
    let reader = std::io::BufReader::new(file);
    for line in reader.lines().flatten() {
        let Ok(entry) = serde_json::from_str::<IndexEntry>(&line) else {
            continue;
        };
        state.by_path.insert(entry.p.clone(), entry);
    }
    log::debug!(
        "[native_icon_registry] loaded {} blobs + {} index entries from {}",
        state.contents.len(),
        state.by_path.len(),
        cache_dir.display()
    );
}
