use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize)]
pub struct DiskStatus {
    pub mount: String,
    pub device: String,
    pub used: u64,
    pub total: u64,
    /// 可用空间（字节）——全应用磁盘可用值的单一事实来源（Single Source of Truth）。
    /// 根卷 "/" 使用 NSURLVolumeAvailableCapacityForImportantUsageKey（与 macOS「储存概述」完全一致）；
    /// 非根卷取 APFSContainerFree 或 df Available。
    /// 前端禁止再用 total - used 推导（APFS purgeable 会使两者不等）。
    pub free: u64,
    pub used_percent: f64,
    pub fstype: String,
    pub external: bool,
}

const SKIP_MOUNTS: &[&str] = &[
    "/System/Volumes/VM",
    "/System/Volumes/Preboot",
    "/System/Volumes/Update",
    "/System/Volumes/xarts",
    "/System/Volumes/Hardware",
    "/System/Volumes/Data",
    "/dev",
];

const SKIP_FSTYPES: &[&str] = &[
    "afpfs", "autofs", "cifs", "devfs", "fuse", "fuseblk", "fusefs", "macfuse", "nfs", "osxfuse",
    "procfs", "smbfs", "tmpfs", "webdav",
];

pub fn collect_disks() -> Vec<DiskStatus> {
    let started = Instant::now();
    let out = Command::new("df")
        .args(["-k", "-l"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();

    let mut result = Vec::new();
    let mut seen_device = HashSet::new();
    let mut seen_volume = HashSet::new();
    // 本次出现的静态字段缓存 key：循环结束后据此 prune，防 HashMap 无界增长
    let mut live_keys = HashSet::new();
    // 子进程计数（仅用于 debug 日志，稳态期望 plist_calls = 0）
    let mut plist_calls = 0usize;
    let mut cold_fetches = 0usize;
    let mut refreshes = 0usize;

    for line in out.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 9 {
            continue;
        }

        let device = fields[0].to_string();
        let mount = fields[8].to_string();

        if device.starts_with("map ") || device == "devfs" {
            continue;
        }
        if mount == "/dev" {
            continue;
        }
        if should_skip(&mount, &device, "") {
            continue;
        }

        let total_kb: u64 = fields[1].parse().unwrap_or(0);
        let used_kb: u64 = fields[2].parse().unwrap_or(0);
        let avail_kb: u64 = fields[3].parse().unwrap_or(0);
        if total_kb < 1_048_576 {
            continue;
        }
        let total = total_kb * 1024;
        let used = used_kb * 1024;
        let free = avail_kb * 1024;
        let used_percent = if total > 0 {
            used as f64 / total as f64 * 100.0
        } else {
            0.0
        };

        let base = base_device_name(&device);
        if seen_device.contains(&base) {
            continue;
        }

        let vol_key = format!("_vol_{}", total);
        if seen_volume.contains(&vol_key) {
            continue;
        }

        // fstype 走 statfs（零子进程）：f_fstypename 与 diskutil plist 的
        // FilesystemType 同源同值（apfs / msdos / exfat / nfs / smbfs ...）。
        // statfs 失败返回空串，与 diskutil 探测超时返回空串的行为一致（不跳过）。
        let fstype = statfs_fstype(&mount);
        if should_skip_fstype(&fstype) {
            continue;
        }

        // 静态字段（external / diskutil total）：TTL 缓存命中时零子进程。
        // 冷启动同步探测一次，保证 external 参与下面的 sort + truncate(3) 与旧实现一致。
        let meta_key = format!("{}#{}", base, mount);
        let fetch = disk_static_meta(&meta_key, &mount, &device);
        live_keys.insert(meta_key);
        if fetch.plist.is_some() {
            cold_fetches += 1;
            plist_calls += 1;
        }
        if fetch.refreshed {
            refreshes += 1;
        }

        // 根卷必为 APFS（macOS 10.13+）；statfs 失败时 fstype 为空串，
        // 故对 "/" 放宽判定，避免校正被跳过后可用值跌回 df 原始口径（少算 purgeable）。
        let (display_total, display_used, display_free, display_pct) =
            if fstype == "apfs" || mount == "/" {
                // ① 根卷优先 native（SSOT）：成功则无需任何 diskutil 子进程。
                //    total 校正也一并惰性化——native 成功时 diskutil total 会被覆盖，属纯浪费。
                let native = if mount == "/" {
                    native_root_correction()
                } else {
                    None
                };

                let c = match native {
                    Some(c) => c,
                    None => {
                        // ② 兜底：APFS 容器空闲校正。容器空闲是动态量，每 tick 现采、禁止进缓存；
                        //    冷启动时复用静态字段那次 plist 输出，避免同一卷 fork 两次。
                        let corrected_total =
                            correct_disk_total_bytes(total, fetch.meta.diskutil_total);
                        let container_free = fetch
                            .plist
                            .as_ref()
                            .and_then(|p| p.container_free)
                            .or_else(|| {
                                plist_calls += 1;
                                apfs_container_free(&mount)
                            });
                        apfs_container_correction(corrected_total, used, free, container_free)
                    }
                };

                (c.total, c.used, c.free, c.used_percent)
            } else {
                (
                    correct_disk_total_bytes(total, fetch.meta.diskutil_total),
                    used,
                    free,
                    used_percent,
                )
            };

        // external 探测失败时为 None，兜底 false（与旧 is_external_disk 失败返回 false 一致）
        let external = fetch.meta.external.unwrap_or(false);

        result.push(DiskStatus {
            mount: mount.clone(),
            device,
            used: display_used,
            total: display_total,
            free: display_free,
            used_percent: display_pct,
            fstype,
            external,
        });

        seen_device.insert(base);
        seen_volume.insert(vol_key);
    }

    // 拔掉的外接盘不会再出现在 df 里，其缓存条目在此清理
    prune_disk_meta(&live_keys);

    log::debug!(
        "[collect_disks] volumes={} plist_calls={} cold={} refresh={} elapsed={:.1}ms",
        result.len(),
        plist_calls,
        cold_fetches,
        refreshes,
        started.elapsed().as_secs_f64() * 1000.0
    );

    result.sort_by(|a, b| a.external.cmp(&b.external).then(b.total.cmp(&a.total)));
    result.truncate(3);
    result
}

/// F0 轻帧专用：只返回根卷，全程原生调用（statfs + NSURL 容量），无 df / diskutil 子进程。
///
/// GUI 三处消费点（Home / Analyze / 托盘）只展示 `pickPrimaryDisk('/')`，故单卷足够；
/// 完整 <=3 卷列表（含外接盘与 APFS 容器校正）由 F1/Full 帧经事件替换。
/// free/total 仍走 `volume_capacity_for_path`，即 SSOT：
/// `NSURLVolumeAvailableCapacityForImportantUsageKey`（与「储存概述」同源）。
#[cfg(target_os = "macos")]
pub fn collect_disks_instant() -> Vec<DiskStatus> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());

    let (available, capacity_total) = match volume_capacity_for_path(&home) {
        Ok((available, capacity_total)) if capacity_total > 0 && available <= capacity_total => {
            (available, capacity_total)
        }
        // native 不可用（虚拟机 / 沙箱 / API 失败）：退回 df 快路径（口径与 fast path 一致，
        // 后续由 enrichment 覆盖）。不用 statfs 的 f_bavail / f_blocks - f_bfree 推导：
        // 实测两者与 df 的 Available / Used 不等（df 对 APFS 做了 purgeable 校正：
        // 本机 df Available 比 f_bavail 多 ~505MB，Used 比 f_blocks - f_bfree 少 ~18GB），
        // 自造第四种口径会破坏 free 的单一事实来源。
        _ => return collect_disks_fast(),
    };

    // 元信息（device / mount / fstype）走 statfs：零子进程，且不涉及容量口径
    let Some(stat) = statfs_info("/") else {
        return collect_disks_fast();
    };

    let used = capacity_total - available;
    // 口径核对用：与 Full 帧的根卷三元组必须逐字节一致（已实测验证）
    log::debug!(
        "[disks:instant] total={} used={} free={} fstype={} device={}",
        capacity_total,
        used,
        available,
        stat.fstype,
        stat.device
    );
    vec![DiskStatus {
        mount: stat.mount,
        device: stat.device,
        used,
        total: capacity_total,
        free: available,
        used_percent: used as f64 / capacity_total as f64 * 100.0,
        fstype: stat.fstype,
        // 根卷恒为内置盘（外接盘不可能挂载在 "/"），无需 diskutil 探测
        external: false,
    }]
}

#[cfg(not(target_os = "macos"))]
pub fn collect_disks_instant() -> Vec<DiskStatus> {
    collect_disks_fast()
}

// ── statfs 原生取值（零子进程） ─────────────────────────

/// `statfs` 一次取齐卷的元信息（零子进程）。
/// 只用于 device / mount / fstype；**不用于容量口径**（见 `collect_disks_instant` 注释）。
#[cfg(target_os = "macos")]
struct StatfsInfo {
    /// f_mntfromname，如 "/dev/disk3s1s1"
    device: String,
    /// f_mntonname，如 "/"
    mount: String,
    /// f_fstypename（小写），与 diskutil plist 的 FilesystemType 同源同值
    fstype: String,
}

#[cfg(target_os = "macos")]
fn statfs_info(path: &str) -> Option<StatfsInfo> {
    use std::ffi::CString;

    let c_path = CString::new(path).ok()?;
    let mut s = unsafe { std::mem::zeroed::<libc::statfs>() };
    if unsafe { libc::statfs(c_path.as_ptr(), &mut s) } != 0 {
        return None;
    }
    Some(StatfsInfo {
        device: c_chars_to_string(&s.f_mntfromname),
        mount: c_chars_to_string(&s.f_mntonname),
        fstype: c_chars_to_string(&s.f_fstypename).to_lowercase(),
    })
}

#[cfg(target_os = "macos")]
fn c_chars_to_string(buf: &[libc::c_char]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let bytes: Vec<u8> = buf[..len].iter().map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// 卷的文件系统类型（取代旧的 `diskutil info -plist` 探测，每卷省一次 fork ≈110ms）。
#[cfg(target_os = "macos")]
fn statfs_fstype(mount: &str) -> String {
    statfs_info(mount).map(|s| s.fstype).unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
fn statfs_fstype(_mount: &str) -> String {
    String::new()
}

// ── diskutil plist：每卷一次子进程 ───────────────────────

/// `diskutil info -plist <target>` 的单次解析结果。
/// 实测单次输出即含 Internal / TotalSize / DiskSize / APFSContainerFree，
/// 故原先每键各 fork 一次（最多 4~5 次）收敛为 1 次。
#[derive(Clone, Default)]
struct VolumePlist {
    internal: Option<bool>,
    total_size: Option<u64>,
    disk_size: Option<u64>,
    container_free: Option<u64>,
}

fn diskutil_volume_plist(target: &str) -> Option<VolumePlist> {
    let out = Command::new("diskutil")
        .args(["info", "-plist", target])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    Some(VolumePlist {
        internal: plist_bool(&s, "Internal"),
        total_size: plist_uint(&s, "TotalSize"),
        disk_size: plist_uint(&s, "DiskSize"),
        container_free: plist_uint(&s, "APFSContainerFree"),
    })
}

/// 取 `<key>NAME</key>` 之后紧邻的 `<integer>N</integer>`。
fn plist_uint(plist: &str, key: &str) -> Option<u64> {
    let after_key = plist.split(&format!("<key>{}</key>", key)).nth(1)?;
    let after_int = after_key.split("<integer>").nth(1)?;
    after_int.split("</integer>").next()?.trim().parse().ok()
}

/// 取 `<key>NAME</key>` 之后紧邻的 `<true/>` / `<false/>`。
fn plist_bool(plist: &str, key: &str) -> Option<bool> {
    let after_key = plist.split(&format!("<key>{}</key>", key)).nth(1)?;
    let rest = after_key.trim_start();
    if rest.starts_with("<true/>") {
        Some(true)
    } else if rest.starts_with("<false/>") {
        Some(false)
    } else {
        None
    }
}

// ── 静态字段缓存（插拔/重分区才变） ─────────────────────

/// 命中 TTL：5 分钟。
const DISK_META_TTL: Duration = Duration::from_secs(300);
/// 探测失败 TTL：30s——避免一次 diskutil 失败把 external/total 冻结 5 分钟（新插盘尽快识别）。
const DISK_META_FAIL_TTL: Duration = Duration::from_secs(30);

/// 仅缓存「磁盘插拔/重分区才变」的字段。
/// 动态量（df 数值、APFSContainerFree、native 容量）永不进缓存，守住 free 的单一事实来源。
#[derive(Clone, Default)]
struct DiskStaticMeta {
    external: Option<bool>,
    diskutil_total: Option<u64>,
}

/// 一次静态字段查取的结果。
struct MetaFetch {
    meta: DiskStaticMeta,
    /// 本次同步探测到的 plist：供动态字段（APFSContainerFree）复用同一次子进程输出
    plist: Option<VolumePlist>,
    /// 本次是否派发了后台异步刷新
    refreshed: bool,
}

static DISK_META_CACHE: Mutex<Option<HashMap<String, (Instant, DiskStaticMeta)>>> =
    Mutex::new(None);

/// 后台刷新单飞标志：同一时刻最多一个刷新线程，避免 tick 叠加起线程。
static DISK_META_REFRESHING: AtomicBool = AtomicBool::new(false);

fn meta_lock() -> std::sync::MutexGuard<'static, Option<HashMap<String, (Instant, DiskStaticMeta)>>>
{
    // 与 controllers/status.rs 一致：毒化后取回内部值继续，而非永久失败
    DISK_META_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 取静态字段：
/// - 命中且未过期 → 零子进程；
/// - 冷未命中 → 同步探测一次（首帧 external 必须真实，它参与 sort + truncate(3)）；
/// - 命中但已过期 → 本次继续用旧值，另起后台线程刷新（stale-while-revalidate），
///   异常外接盘使 diskutil 卡住时只拖住 detached 线程，不阻塞采集 tick。
fn disk_static_meta(key: &str, mount: &str, device: &str) -> MetaFetch {
    enum Action {
        Cold,
        Hit(DiskStaticMeta),
        Stale(DiskStaticMeta),
    }

    let action = {
        let guard = meta_lock();
        match guard.as_ref().and_then(|m| m.get(key)) {
            Some((at, meta)) => {
                // 两字段皆 None 视为探测失败，用短 TTL 以便尽快重试
                let ttl = if meta.external.is_none() && meta.diskutil_total.is_none() {
                    DISK_META_FAIL_TTL
                } else {
                    DISK_META_TTL
                };
                if at.elapsed() < ttl {
                    Action::Hit(meta.clone())
                } else {
                    Action::Stale(meta.clone())
                }
            }
            None => Action::Cold,
        }
    };

    match action {
        Action::Hit(meta) => MetaFetch {
            meta,
            plist: None,
            refreshed: false,
        },
        Action::Cold => {
            let plist = diskutil_volume_plist(mount);
            let meta = meta_from_plist(plist.as_ref(), device);
            store_meta(key, meta.clone());
            MetaFetch {
                meta,
                plist,
                refreshed: false,
            }
        }
        Action::Stale(meta) => {
            let refreshed =
                schedule_meta_refresh(key.to_string(), mount.to_string(), device.to_string());
            MetaFetch {
                meta,
                plist: None,
                refreshed,
            }
        }
    }
}

fn meta_from_plist(plist: Option<&VolumePlist>, device: &str) -> DiskStaticMeta {
    let Some(p) = plist else {
        // diskutil 整体失败：两字段皆 None → 走失败 TTL（30s），下次尽快重试
        return DiskStaticMeta::default();
    };
    DiskStaticMeta {
        // Internal 键缺失时回退文本模式解析（既有实现，仅一次）
        external: Some(match p.internal {
            Some(internal) => !internal,
            None => is_external_disk(device),
        }),
        diskutil_total: p.total_size.or(p.disk_size),
    }
}

fn store_meta(key: &str, meta: DiskStaticMeta) {
    let mut guard = meta_lock();
    guard
        .get_or_insert_with(HashMap::new)
        .insert(key.to_string(), (Instant::now(), meta));
}

/// 派发后台刷新；返回是否真的起了线程（已有刷新在跑时为 false）。
fn schedule_meta_refresh(key: String, mount: String, device: String) -> bool {
    if DISK_META_REFRESHING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return false;
    }
    // 后台线程只碰 DISK_META_CACHE 与子进程，绝不获取 Collector 锁 → 无死锁
    std::thread::spawn(move || {
        let plist = diskutil_volume_plist(&mount);
        let meta = meta_from_plist(plist.as_ref(), &device);
        store_meta(&key, meta);
        DISK_META_REFRESHING.store(false, Ordering::SeqCst);
    });
    true
}

/// 按本次出现的卷集合清理陈旧条目（拔掉的外接盘不会再出现在 df 里）。
fn prune_disk_meta(live_keys: &HashSet<String>) {
    let mut guard = meta_lock();
    if let Some(map) = guard.as_mut() {
        map.retain(|k, _| live_keys.contains(k));
    }
}

fn should_skip(mount: &str, device: &str, _fstype: &str) -> bool {
    if device.starts_with("/dev/loop") {
        return true;
    }
    if SKIP_MOUNTS.contains(&mount) {
        return true;
    }
    if mount.starts_with("/System/Volumes/") {
        return true;
    }
    if mount.starts_with("/private/") {
        return true;
    }
    if !device.is_empty() && !device.starts_with("/dev/") {
        return true;
    }
    false
}

fn should_skip_fstype(fstype: &str) -> bool {
    if fstype.is_empty() {
        return false;
    }
    if SKIP_FSTYPES.contains(&fstype) {
        return true;
    }
    fstype.contains("fuse")
}

fn base_device_name(device: &str) -> String {
    let d = device.trim_start_matches("/dev/");
    if !d.starts_with("disk") {
        return d.to_string();
    }
    for (i, c) in d.char_indices().skip(4) {
        if c == 's' {
            return d[..i].to_string();
        }
    }
    d.to_string()
}

/// df 总量与 diskutil 总量偏差 >1GiB 时采用后者（对齐旧口径）。
/// `diskutil_total` 由调用方从静态字段缓存传入（惰性求值：根卷 native 成功时根本不需要）。
fn correct_disk_total_bytes(raw_total: u64, diskutil_total: Option<u64>) -> u64 {
    if raw_total == 0 {
        return raw_total;
    }
    let Some(diskutil_total) = diskutil_total.filter(|t| *t > 0) else {
        return raw_total;
    };
    let diff = if raw_total > diskutil_total {
        raw_total - diskutil_total
    } else {
        diskutil_total - raw_total
    };
    if diff > 1 << 30 {
        return diskutil_total;
    }
    raw_total
}

/// APFS 校正结果：total/used/free 三元组必须自洽（used + free ≈ total），
/// 杜绝「total 一个口径、used 另一个口径」导致前端 total - used 出现系统性偏差。
struct ApfsCorrection {
    total: u64,
    used: u64,
    free: u64,
    used_percent: f64,
}

/// 根卷 native 校正（SSOT）：`NSURLVolumeAvailableCapacityForImportantUsageKey`。
/// 成功返回自洽三元组（used + free == total）；失败返回 None，交由容器空闲兜底。
#[cfg(target_os = "macos")]
fn native_root_correction() -> Option<ApfsCorrection> {
    let (available, capacity_total) = get_volume_capacity().ok()?;
    if capacity_total == 0 || available > capacity_total {
        return None;
    }
    let used = capacity_total - available;
    Some(ApfsCorrection {
        total: capacity_total,
        used,
        free: available,
        used_percent: used as f64 / capacity_total as f64 * 100.0,
    })
}

#[cfg(not(target_os = "macos"))]
fn native_root_correction() -> Option<ApfsCorrection> {
    None
}

/// APFS 容器空闲校正（非根卷，以及根卷 native 失败的兜底）。
/// `container_free` 由调用方现采传入（动态量，不进缓存）。
fn apfs_container_correction(
    total: u64,
    raw_used: u64,
    raw_free: u64,
    container_free: Option<u64>,
) -> ApfsCorrection {
    if let Some(container_free) = container_free {
        if container_free <= total
            && raw_used > container_free
            && raw_used - container_free > 1 << 30
        {
            let corrected = total - container_free;
            let pct = corrected as f64 / total as f64 * 100.0;
            return ApfsCorrection {
                total,
                used: corrected,
                free: container_free,
                used_percent: pct,
            };
        }
    }

    // ── 兜底：df 原始值（fast path enrichment 覆盖前）──
    ApfsCorrection {
        total,
        used: raw_used,
        free: raw_free,
        used_percent: if total > 0 {
            raw_used as f64 / total as f64 * 100.0
        } else {
            0.0
        },
    }
}

/// 非根卷 APFS 容器空闲（动态量）：每 tick 现采一次 plist。
/// 调用方优先复用静态字段探测那次的 plist 输出，仅缓存命中时才走到这里。
fn apfs_container_free(mount: &str) -> Option<u64> {
    diskutil_volume_plist(mount)?.container_free
}

const VOLUME_CAPACITY_CACHE_TTL: Duration = Duration::from_secs(120);

/// 按 path 分 key 的容量缓存。原实现为单槽无 key：
/// `available_capacity_for_path`（Analyze 查任意卷）与 `get_volume_capacity`（查 HOME）
/// 共用一槽，120s 窗口内会拿到其他卷的值（跨卷串味）。
static VOLUME_CAPACITY_CACHE: Mutex<Option<HashMap<String, (Instant, u64, u64)>>> =
    Mutex::new(None);

/// 缓存条目上限：Analyze 可能对多个不同路径查询，超限直接清空重建（防无界增长）。
const VOLUME_CAPACITY_CACHE_MAX: usize = 64;

/// 获取任意路径所在卷的可用容量与总容量 —— 与腾讯柠檬 (Lemon Cleaner) 完全对齐。
///
/// 使用 `NSURLVolumeAvailableCapacityForImportantUsageKey`，这是 macOS 系统内部
/// 用于「储存概述」(About This Mac > Storage) 的精确数据源。该值包含：
/// - 卷上物理空闲块
/// - APFS 可清除空间（purgeable：本地 Time Machine 快照、系统缓存等）
/// - 系统在"重要操作"时可回收的其他空间
///
/// 参考：LemonSpaceAnalyse/LMSpaceResultViewController.m `getAllUsableBytes`
#[cfg(target_os = "macos")]
fn get_volume_capacity() -> Result<(u64, u64), ()> {
    // 使用用户 Home 目录（与柠檬的 NSHomeDirectory() 等价，同在根卷）
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    volume_capacity_for_path(&home)
}

/// `get_volume_capacity` 的按路径变体：供 Analyze 等模块获取目标路径所在卷的容量。
/// 返回值 `(可用, 总量)`；受 `VOLUME_CAPACITY_CACHE` 120s TTL 缓存保护。
#[cfg(target_os = "macos")]
pub(crate) fn available_capacity_for_path(path: &str) -> Option<u64> {
    volume_capacity_for_path(path)
        .ok()
        .map(|(free, _total)| free)
}

#[cfg(target_os = "macos")]
fn volume_capacity_for_path(path: &str) -> Result<(u64, u64), ()> {
    use objc2::msg_send;
    use objc2::rc::autoreleasepool;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSArray, NSString, NSURL};

    // 检查缓存（120s TTL，按 path 分 key）
    {
        let guard = VOLUME_CAPACITY_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(map) = guard.as_ref() {
            if let Some((cached_at, free, total)) = map.get(path) {
                if cached_at.elapsed() < VOLUME_CAPACITY_CACHE_TTL {
                    return Ok((*free, *total));
                }
            }
        }
    }

    let result = autoreleasepool(|_| unsafe {
        let ns_path = NSString::from_str(path);
        let url = NSURL::fileURLWithPath(&ns_path);

        // Resource value keys（常量字符串值与 Foundation 定义一致）
        let key_avail = NSString::from_str("NSURLVolumeAvailableCapacityForImportantUsageKey");
        let key_total = NSString::from_str("NSURLVolumeTotalCapacityKey");
        let keys = NSArray::from_retained_slice(&[key_avail.clone(), key_total.clone()]);

        // [url resourceValuesForKeys:keys error:nil]
        let nil_err: *mut *mut AnyObject = std::ptr::null_mut();
        let result: *mut AnyObject =
            msg_send![&*url, resourceValuesForKeys: &*keys, error: nil_err];
        if result.is_null() {
            return Err(());
        }

        // [result objectForKey:key] → NSNumber
        let avail_num: *mut AnyObject = msg_send![result, objectForKey: &*key_avail];
        let total_num: *mut AnyObject = msg_send![result, objectForKey: &*key_total];
        if avail_num.is_null() || total_num.is_null() {
            return Err(());
        }

        // [NSNumber longLongValue]
        let avail: i64 = msg_send![avail_num, longLongValue];
        let total: i64 = msg_send![total_num, longLongValue];
        if avail <= 0 || total <= 0 {
            return Err(());
        }

        Ok((avail as u64, total as u64))
    });

    // 写入缓存（按 path 分 key）
    if let Ok((free, total)) = result {
        let mut guard = VOLUME_CAPACITY_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        if map.len() >= VOLUME_CAPACITY_CACHE_MAX {
            map.clear();
        }
        map.insert(path.to_string(), (Instant::now(), free, total));
    }

    result
}

fn is_external_disk(device: &str) -> bool {
    let out = match Command::new("diskutil").args(["info", device]).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return false,
    };
    for line in out.lines() {
        let t = line.trim();
        if t.starts_with("Internal:") {
            return t.contains("No");
        }
        if t.starts_with("Device Location:") {
            return t.contains("External");
        }
    }
    false
}

// ── 快速采集 对齐 Go collectDisksFast() ─────────────────────────
// 跳过所有 diskutil/osascript/Finder 校正，只用 df -k -l 原始值。
// APFS purgeable / diskutil 矫正值由全量采集的 snapshotEnrichment 缓存覆盖。

pub fn collect_disks_fast() -> Vec<DiskStatus> {
    let out = Command::new("df")
        .args(["-k", "-l"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();

    let mut result = Vec::new();
    let mut seen_device = HashSet::new();
    let mut seen_volume = HashSet::new();

    for line in out.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 9 {
            continue;
        }

        let device = fields[0].to_string();
        let mount = fields[8].to_string();

        if device.starts_with("map ") || device == "devfs" {
            continue;
        }
        if mount == "/dev" {
            continue;
        }
        if should_skip(&mount, &device, "") {
            continue;
        }

        let total_kb: u64 = fields[1].parse().unwrap_or(0);
        let used_kb: u64 = fields[2].parse().unwrap_or(0);
        let avail_kb: u64 = fields[3].parse().unwrap_or(0);
        if total_kb < 1_048_576 {
            continue;
        }
        let total = total_kb * 1024;
        let used = used_kb * 1024;
        let free = avail_kb * 1024;
        let used_percent = if total > 0 {
            used as f64 / total as f64 * 100.0
        } else {
            0.0
        };

        let base = base_device_name(&device);
        if seen_device.contains(&base) {
            continue;
        }

        let vol_key = format!("_vol_{}", total);
        if seen_volume.contains(&vol_key) {
            continue;
        }

        // Fast path: 不做 diskutil fstype 查询，用空字符串占位。
        // free 取 df Available（不含保留块）；enrichment 注入时会用全量校准值整体覆盖。
        let fstype = String::new();
        let external = false;

        result.push(DiskStatus {
            mount: mount.clone(),
            device,
            used,
            total,
            free,
            used_percent,
            fstype,
            external,
        });

        seen_device.insert(base);
        seen_volume.insert(vol_key);
    }

    result.sort_by(|a, b| b.total.cmp(&a.total));
    result.truncate(3);
    result
}
