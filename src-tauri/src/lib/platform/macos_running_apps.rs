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

/// 运行中应用的元信息（不含图标像素）。图标通过 `native_icon_registry` 按
/// bundle_path 按需解析（内容寻址 + 128px + SVG 信封，与文件域同源共享 contentStore）。
#[derive(Clone)]
pub struct RunningApp {
    pub pid: i32,
    /// NSRunningApplication.localizedName
    pub name: String,
    /// executableURL 末段（Burrow 同款双键之一）
    pub exec_name: String,
    /// executableURL 完整路径：图标 fallback 键（对齐柠檬 `iconForFile:pExecutePath`）。
    /// 当 bundleURL 为 nil（裸二进制 / dev 模式）时，用此路径走 NSWorkspace.iconForFile 取图。
    pub exec_path: String,
    /// bundleURL 路径：图标取图键（传给 `native_icon_registry`），同时下发给前端。
    /// 与 Uninstall 的 `.app` 路径同源，可共享内容存储。无 bundle 时为空串。
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

fn index_lock() -> std::sync::MutexGuard<'static, Option<(Instant, Arc<AppIconIndex>)>> {
    // 与 controllers/status.rs 一致：毒化后取回内部值继续，而非永久失败
    INDEX_CACHE
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
    // 健壮性：本次构建为空（主线程瞬时失败/panic）但缓存里有上一份非空索引时，
    // 保留旧值且不刷新时间戳（旧戳已过期 → 下次访问立即重试构建），
    // 避免一次瞬时失败把内存列表清空。
    if index.regular_pids.is_empty() {
        if let Some((_, prev)) = guard.as_ref() {
            if !prev.regular_pids.is_empty() {
                log::warn!("[running_apps_icon_index] build returned empty, 保留上一份索引");
                return Arc::clone(prev);
            }
        }
    }
    *guard = Some((Instant::now(), Arc::clone(&index)));
    index
}

/// 按需取进程图标（走 `native_icon_registry`，内容寻址 + 128px + SVG 信封）。
///
/// 只为**真正要展示**的进程调用（top_processes 截断后 ≤5 个）：
/// - 命中 contentStore → 零原生调用、零主线程占用；
/// - 未命中或 mtime 变化 → 编码一次，写入 contentStore（与文件域共享），长期保留。
///
/// 返回 SVG data URI（`data:image/svg+xml;base64,…`），调用方可直接赋给 `<img src>`。
/// 与旧 `app_icon_base64` 签名保持兼容（Option<String> 替代 Option<Arc<str>>），
/// 调用方不需要改签名，只是返回的内容从 PNG base64 变为 SVG data URI。
pub fn app_icon_svg(bundle_path: &str) -> Option<String> {
    if bundle_path.is_empty() {
        return None;
    }
    super::native_icon_registry::resolve_single(bundle_path)
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
            let exec_path = app
                .executableURL()
                .and_then(|url| url.path().map(|p| p.to_string()))
                .unwrap_or_default();
            let bundle_path = app
                .bundleURL()
                .and_then(|url| url.path().map(|p| p.to_string()))
                .unwrap_or_default();

            let entry = Arc::new(RunningApp {
                pid,
                name,
                exec_name,
                exec_path,
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
