//! 进程域索引：运行中应用的**元信息**与**图标像素**分两级缓存。
//!
//! 与文件域 `macos_file_icon`（iconForFile(path)）职责分离：
//!   - 文件域：磁盘路径 → 图标（Uninstall / Analyze 等，路径权威，磁盘持久化 + mtime 校验）
//!   - 进程域：运行中应用 → pid / name / Regular 元信息 + 图标（Dashboard 内存列表）
//!
//! 两级缓存（两者变化频率完全不同，旧实现绑在一起是缺陷根源）：
//!   1. 元信息索引（`APPS_TTL` 8s）：pid / activationPolicy / localizedName /
//!      executableURL 末段 / bundleURL 路径。**只读属性、不编码图标**，
//!      数十个应用为毫秒级，故可用短 TTL 跟随应用启停。
//!   2. 图标像素（**无 TTL**，mtime 指纹校验）：bundle 路径 → PNG base64。
//!      失效条件是 bundle 内容变更（应用更新/重装），对标 `controllers/platform.rs`
//!      文件域 AppCache 的 `try_cache` 语义，实现「长期缓存、按需刷新」；
//!      且只对真正展示的少数进程（top_processes 截断后 ≤5 个）按需编码。
//!
//! 历史教训（勿回退）：旧实现把两者绑在同一个 30s TTL 索引里，每次过期都为**全部**
//! runningApplications（本机 85 个，含 daemon/helper）重新编码 128px PNG，实测 11.9s，
//! 且经 `DispatchQueue::main().exec_sync` 落在主线程 → 彩虹圈 + 托盘内存列表 12s 才出。
//! 另外旧实现每次命中都 `clone_index` 整表（含全部 base64），每 2s 一帧即数 MB 拷贝，
//! 现改为 `Arc` 共享，调用方只增引用计数。
//!
//! 进程匹配顺序见 metrics_process::top_processes：
//!   pid 命中 → ppid 命中（helper 的父进程即宿主主进程）→ name → command。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 元信息索引 TTL：跟随应用启停（用户开关应用的频率远高于应用图标变更）。
/// 取 8s：气泡每 2s 一帧，等于每 4 帧刷新一次列表，启停应用最多滞后 8s 进/出列表。
const APPS_TTL: Duration = Duration::from_secs(8);

/// 图标像素缓存条目上限（单张 128px PNG base64 可达数百 KB），超限清空重建。
const ICON_CACHE_MAX: usize = 256;

/// 运行中应用的元信息（不含图标像素）。
/// 像素存于 `ICON_CACHE` 并按 bundle 路径共享：同一应用的多个进程只存一份。
#[derive(Clone)]
pub struct RunningApp {
    pub pid: i32,
    /// NSRunningApplication.localizedName
    pub name: String,
    /// executableURL 末段（Burrow 同款双键之一）
    pub exec_name: String,
    /// bundleURL 路径：图标缓存键，同时下发给前端作为 iconService 的取图路径
    /// （与 Uninstall 的 `.app` 路径同源，可共享文件域磁盘缓存）。无 bundle 时为空串。
    pub bundle_path: String,
    /// activationPolicy == Regular（前台应用；内存列表只展示这类）
    pub regular: bool,
}

/// 进程域索引（纯元信息）。`Arc` 共享，调用方每 2s 取一次只增引用计数。
pub struct AppIconIndex {
    /// 名字键：localizedName + 可执行文件名（Burrow 同款双键）
    pub by_name: HashMap<String, Arc<RunningApp>>,
    /// pid 键：NSRunningApplication.processIdentifier（柠檬同款）
    pub by_pid: HashMap<i32, Arc<RunningApp>>,
    /// 前台应用 pid 集合（activationPolicy == Regular），供内存列表过滤
    pub regular_pids: HashSet<i32>,
}

impl AppIconIndex {
    fn empty() -> Self {
        Self {
            by_name: HashMap::new(),
            by_pid: HashMap::new(),
            regular_pids: HashSet::new(),
        }
    }
}

/// 元信息索引缓存（短 TTL）
static INDEX_CACHE: Mutex<Option<(Instant, Arc<AppIconIndex>)>> = Mutex::new(None);

/// 图标像素缓存（长期，无 TTL；失效靠 bundle mtime 指纹）：路径 → (指纹, base64)
static ICON_CACHE: Mutex<Option<HashMap<String, (u64, Arc<str>)>>> = Mutex::new(None);

fn index_lock() -> std::sync::MutexGuard<'static, Option<(Instant, Arc<AppIconIndex>)>> {
    // 与 controllers/status.rs 一致：毒化后取回内部值继续，而非永久失败
    INDEX_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn icon_lock() -> std::sync::MutexGuard<'static, Option<HashMap<String, (u64, Arc<str>)>>> {
    ICON_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 获取进程域索引（元信息，短 TTL 缓存，首次访问懒构建）。
/// 不含图标编码，故主线程占用为毫秒级（旧实现在这里是 11.9s）。
pub fn running_apps_icon_index() -> Arc<AppIconIndex> {
    {
        let guard = index_lock();
        if let Some((at, index)) = guard.as_ref() {
            if at.elapsed() < APPS_TTL {
                return Arc::clone(index);
            }
        }
    }

    let index = Arc::new(build_index());

    let mut guard = index_lock();
    *guard = Some((Instant::now(), Arc::clone(&index)));
    index
}

/// 按需取图标像素（长期缓存 + mtime 指纹校验）。
///
/// 只为**真正要展示**的进程调用（top_processes 截断后 ≤5 个）：
/// - 命中且指纹未变 → 零原生调用、零主线程占用；
/// - 未命中或指纹变化 → 编码一次（走文件域 `file_icon_png_base64`，与 Uninstall
///   同一管线、同一 128px 口径，故前端两条取图路径产出的 data URI 逐字节相同，
///   即使发生源切换也不会有视觉跳变），随后长期保留。
pub fn app_icon_base64(bundle_path: &str) -> Option<Arc<str>> {
    if bundle_path.is_empty() {
        return None;
    }
    let fingerprint = bundle_fingerprint(bundle_path);

    {
        let guard = icon_lock();
        if let Some(map) = guard.as_ref() {
            if let Some((cached, b64)) = map.get(bundle_path) {
                // 指纹为 0 表示 bundle 无法 stat（已卸载/路径异常），不吃缓存
                if fingerprint != 0 && *cached == fingerprint {
                    return Some(Arc::clone(b64));
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    let encoded = super::macos_file_icon::file_icon_png_base64(bundle_path)
        .ok()
        .flatten();
    #[cfg(not(target_os = "macos"))]
    let encoded: Option<String> = None;

    let b64: Arc<str> = Arc::from(encoded?.as_str());

    let mut guard = icon_lock();
    let map = guard.get_or_insert_with(HashMap::new);
    if map.len() >= ICON_CACHE_MAX {
        map.clear();
    }
    map.insert(bundle_path.to_string(), (fingerprint, Arc::clone(&b64)));
    Some(b64)
}

/// bundle 新鲜度指纹：`.app` 目录与其 `Contents/Info.plist` 的 mtime 取大者。
///
/// 只用目录 mtime 不够——应用更新通常改写 `Contents/` 内部文件，`.app` 目录自身的
/// mtime 可能不变；`Info.plist` 每次版本更新必然重写，是可靠信号。两次 `stat`
/// 成本可忽略（只对展示的少数进程调用）。
fn bundle_fingerprint(bundle_path: &str) -> u64 {
    let dir = path_mtime(bundle_path);
    let plist = path_mtime(&format!(
        "{}/Contents/Info.plist",
        bundle_path.trim_end_matches('/')
    ));
    dir.max(plist)
}

fn path_mtime(path: &str) -> u64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── 元信息构建（主线程，仅读属性） ─────────────────────────────

#[cfg(target_os = "macos")]
fn build_index() -> AppIconIndex {
    use dispatch2::DispatchQueue;
    use objc2::MainThreadMarker;

    // AppKit 查询统一走主线程（对齐 macos_file_icon 的做法）；
    // 调用方在 status 采集线程（spawn_blocking 内），不会自锁。
    // 本函数只读 pid/policy/name/bundleURL，不编码图标，主线程占用为毫秒级。
    if MainThreadMarker::new().is_some() {
        return build_index_inner();
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    DispatchQueue::main().exec_sync(move || {
        let _ = tx.send(build_index_inner());
    });
    rx.recv().unwrap_or_else(|_| AppIconIndex::empty())
}

#[cfg(target_os = "macos")]
fn build_index_inner() -> AppIconIndex {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};

    autoreleasepool(|_| {
        let mut index = AppIconIndex::empty();
        let workspace = NSWorkspace::sharedWorkspace();
        let apps = workspace.runningApplications().to_vec();
        for app in &apps {
            let pid = app.processIdentifier();
            let regular = app.activationPolicy() == NSApplicationActivationPolicy::Regular;
            let name = app
                .localizedName()
                .map(|n| n.to_string())
                .unwrap_or_default();
            let exec_name = app
                .executableURL()
                .and_then(|url| url.lastPathComponent().map(|s| s.to_string()))
                .unwrap_or_default();
            let bundle_path = app
                .bundleURL()
                .and_then(|url| url.path().map(|p| p.to_string()))
                .unwrap_or_default();

            let entry = Arc::new(RunningApp {
                pid,
                name,
                exec_name,
                bundle_path,
                regular,
            });

            index
                .by_pid
                .entry(pid)
                .or_insert_with(|| Arc::clone(&entry));
            // 双键索引：本地化显示名 + 可执行文件名（Burrow 同款）
            if !entry.name.is_empty() {
                index
                    .by_name
                    .entry(entry.name.clone())
                    .or_insert_with(|| Arc::clone(&entry));
            }
            if !entry.exec_name.is_empty() {
                index
                    .by_name
                    .entry(entry.exec_name.clone())
                    .or_insert(Arc::clone(&entry));
            }
            // Regular 集合与图标无关，即使无 bundle 也要记录（供内存列表过滤）
            if regular {
                index.regular_pids.insert(pid);
            }
        }
        index
    })
}

#[cfg(not(target_os = "macos"))]
fn build_index() -> AppIconIndex {
    AppIconIndex::empty()
}
