//! 网络监控采集层（macOS）。
//!
//! 对齐策略：底层采集学腾讯柠檬（硬件类型白名单 + 兜底过滤），
//! 上层架构保持 Mole CLI 三层节奏。
//!
//! 速率差分器严格对齐 Mole CLI `metrics_network.go`：
//! - `minNetworkSampleInterval = 100 * time.Millisecond` → `MIN_SAMPLE_INTERVAL_SECS = 0.1`
//! - `counterDelta` 回退防御 → `saturating_sub`

use serde::Serialize;
use std::collections::HashMap;
use std::process::Command;
use std::time::Instant;

// ── macOS sysctl 绑定：NET_RT_IFLIST2 接口类型探测 ─────────────
//
// 对齐 Lemon Cleaner CmcGetNetPacketsInfo() 的底层数据来源。
// 一次 sysctl 调用同时获得接口名、硬件类型、64 位字节计数器，
// 避免 32 位 if_data 每 4 GiB 回绕的问题。
//
// 使用 if_msghdr2（64 位计数器），而非 if_msghdr（32 位）。
#[cfg(target_os = "macos")]
mod sysctl_net {
    pub const CTL_NET: libc::c_int = 4;
    pub const PF_ROUTE: libc::c_int = 17;
    pub const NET_RT_IFLIST2: libc::c_int = 6;
    pub const RTM_IFINFO2: u16 = 0x12;

    /// if_msghdr2 消息头大小（到 ifi_type 的偏移）。
    /// 公开头（usr/include/net/if.h）为 16 字节：msglen(2) + version(1) +
    /// type(1) + addrs(4) + flags(4) + index(2) + len(2)。
    /// macOS 12 实测 ifi_type 在绝对偏移 32，说明内核私有版本在公开头
    /// 与 if_data64 之间有 16 字节扩展字段。此值可能随 macOS 版本变化。
    pub const HDR_SIZE: usize = 32;

    extern "C" {
        pub fn sysctl(
            name: *mut libc::c_int,
            namelen: libc::c_uint,
            oldp: *mut libc::c_void,
            oldlenp: *mut usize,
            newp: *mut libc::c_void,
            newlen: usize,
        ) -> libc::c_int;
    }
}

// ── if_data64 公共头布局（仅 ifi_type 字段被代码读取）────────
//
// macOS XNU 64 位网络接口数据结构，嵌在 if_msghdr2 消息中。
// 公共头定义（usr/include/net/if.h）为 128 字节，不含 ifi_name。
// 但内核私有版本在 ifi_collisions 与 ifi_ibytes 之间有版本相关的
// 扩展字段（macOS 12 实测为 16 字节），导致 ifi_ibytes 偏移不稳定。
//
// 因此本模块仅读取 ifi_type（偏移 0，公共头起始），字节计数器
// 改用 sysinfo::Networks (getifaddrs) 获取，避免硬编码偏移量。
#[cfg(target_os = "macos")]
#[repr(C)]
#[allow(dead_code)]
struct IfData64 {
    ifi_type: u8,      // 0:  硬件接口类型（IFT_* 枚举值）
    ifi_typelen: u8,   // 1
    ifi_physical: u8,  // 2
    ifi_addrlen: u8,   // 3
    ifi_hdrlen: u8,    // 4
    ifi_recvquota: u8, // 5
    ifi_xmitquota: u8, // 6
    ifi_unused1: u8,   // 7:  padding → u32 对齐
    ifi_mtu: u32,      // 8
    ifi_metric: u32,   // 12
    ifi_baudrate: u64, // 16
    ifi_ipackets: u64, // 24
    ifi_ierrors: u64,  // 32
    ifi_opackets: u64, // 40
    ifi_oerrors: u64,  // 48
    ifi_collisions: u64, // 56
                       // ── 后续字段偏移随 macOS 版本变化，不在此定义 ──
}

// ── 公共类型 ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct NetworkStatus {
    pub name: String,
    pub rx_rate_mbs: f64,
    pub tx_rate_mbs: f64,
    pub ip: String,
}

/// 网络趋势历史。
///
/// `rx_latest` / `tx_latest` 为最近一次采样的速率值（MB/s），是前端
/// 消费网络数据的**主要字段**（对齐柠檬 KVO 单值推送语义）。
/// `rx_history` / `tx_history` 保留后端环形缓冲完整序列供 CLI 等消费者使用，
/// 不参与前端 JSON 序列化（`skip_serializing`，节省每秒 ~2 KB 传输开销）。
#[derive(Debug, Clone, Serialize)]
pub struct NetworkHistory {
    #[serde(skip_serializing)]
    pub rx_history: Vec<f64>,
    #[serde(skip_serializing)]
    pub tx_history: Vec<f64>,
    /// 最近一次采样下行速率（MB/s）
    pub rx_latest: f64,
    /// 最近一次采样上行速率（MB/s）
    pub tx_latest: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ProxyStatus {
    pub enabled: bool,
    #[serde(rename = "type")]
    pub proxy_type: String,
    pub host: String,
}

// ── 常量 ────────────────────────────────────────────────────────

/// 后端环形缓冲容量：每 2s 一帧，120 样本 ≈ 最近 240 秒（Mole CLI `NetworkHistorySize = 120`）。
pub const NETWORK_HISTORY_SIZE: usize = 120;

/// 对齐 Mole CLI `minNetworkSampleInterval = 100 * time.Millisecond`：
/// 速率差分的最小时间间隔，防止 dt → 0 导致速率爆炸。
/// （另有用 50ms 的实现，此处取 CLI 的 100ms，更保守。）
const MIN_SAMPLE_INTERVAL_SECS: f64 = 0.1;

/// 陈旧阈值：距上次采集超过此值时，视为首次采集。
/// 场景：watch 线程重启后，Collector 单例的 last_net_at 可能停留在数分钟前，
/// 此时差分计算会把多分钟的平均速率当作“当前速率”注入 history buffer，
/// 形成尖峰压扁后续真实瞬时值。重置后首帧返回 0，第二帧起恢复真实速率。
/// 取 5 × FAST_REFRESH_INTERVAL(2s) = 10s，避免正常 2s 节奏误触发。
const STALE_THRESHOLD_SECS: f64 = 10.0;

/// 噪声接口前缀：仅作为硬件类型过滤（`get_if_info_list`）的**兜底策略**——
/// sysctl 失败时回退到此名单，或物理白名单结果为空时排除虚拟接口。
/// 对齐 Mole CLI `isNoiseInterfaceName()`。
const NOISE_PREFIXES: &[&str] = &[
    "lo", "awdl", "utun", "llw", "bridge", "gif", "stf", "xhc", "anpi", "ap",
];

/// 对齐柠檬 `CmcGetNetPacketsInfo()` 硬件类型白名单：
/// 只统计真实物理网卡，排除虚拟/隧道/回环接口。
/// 柠檬：`ifType == IFT_ETHER || ifType == IFT_PPP || ifType == IFT_MODEM || ifType == IFT_CELLULAR`
#[cfg(target_os = "macos")]
const PHYSICAL_HW_TYPES: &[u8] = &[
    0x06, // IFT_ETHER    — Ethernet / Wi-Fi（macOS 将 Wi-Fi 也报为 IFT_ETHER）
    0x17, // IFT_PPP      — PPP 拨号
    0xFF, // IFT_CELLULAR — 蜂窝网络（libc 未暴露，硬编码）
];

// ── 主采集函数 ──────────────────────────────────────────────────

pub fn collect_network(
    prev_net: &mut HashMap<String, (u64, u64)>,
    last_net_at: &mut Option<Instant>,
    rx_history: &mut Vec<f64>,
    tx_history: &mut Vec<f64>,
    now: Instant,
) -> Result<Vec<NetworkStatus>, String> {
    let if_ips = get_interface_ips();

    // macOS: 通过 sysctl NET_RT_IFLIST2 获取硬件类型 + if_indextoname 获取接口名，
    // 字节计数器来自 sysinfo::Networks (getifaddrs)，与柠檬 CmcGetNetPacketsInfo 同源。
    // sysctl 失败时回退到纯 sysinfo 路径。
    #[cfg(target_os = "macos")]
    let if_types = get_if_info_list();

    // 陈旧检测：距上次采集超过 STALE_THRESHOLD_SECS 时，视为首次采集。
    // 场景：watch 线程因托盘关闭停止后重启，Collector 单例的 last_net_at
    // 可能停留在数/数十分钟前。此时 ibytes 与 prev_rx 的差值反映的是
    // 整段闲置期的累积流量，除以 elapsed 得到的是多分钟平均速率而非瞬时速率。
    // 该平均值作为 rx_latest 注入前端 ring buffer，会形成尖峰锚点，
    // 压扁后续所有真实瞬时值（Sparkline 自适应纵轴缺陷），视觉上表现为“冻结”。
    let is_stale = last_net_at.map_or(false, |t| {
        now.duration_since(t).as_secs_f64() > STALE_THRESHOLD_SECS
    });
    let is_first = last_net_at.is_none();

    // 诊断日志：首次采集（tick=0）或陈旧重启时打印接口清单和硬件类型
    if is_first || is_stale {
        #[cfg(target_os = "macos")]
        match &if_types {
            Some(_map) => {
                // 接口诊断日志已移除
            }
            None => log::warn!(
                "[net:diag] sysctl NET_RT_IFLIST2 调用失败(errno={}), 将回退到 sysinfo",
                std::io::Error::last_os_error()
            ),
        }
    }

    if is_first || is_stale {
        *last_net_at = Some(now);
        // 陈旧重启：清空 history buffer，避免陈旧的尖峰样本通过 rx_latest
        // 注入前端 ring buffer，形成视觉上“冻结”的尖峰锚点。
        if is_stale {
            rx_history.clear();
            tx_history.clear();
            prev_net.clear();
        }
        // 首帧/陈旧重启：初始化 prev 计数器，返回零速率条目（不返回空 Vec）
        // 前端 snap.network[0] 有值（0 KB/s），第二帧起有真实速率
        #[cfg(target_os = "macos")]
        if let Some(ref map) = if_types {
            for (name, &(_, ibytes, obytes)) in map {
                prev_net.insert(name.clone(), (ibytes, obytes));
            }
            let first: Vec<NetworkStatus> = map
                .iter()
                .filter(|(name, _)| is_physical_iface(&if_types, name))
                .map(|(name, _)| NetworkStatus {
                    name: name.clone(),
                    rx_rate_mbs: 0.0,
                    tx_rate_mbs: 0.0,
                    ip: if_ips.get(name).cloned().unwrap_or_default(),
                })
                .collect();
            return Ok(first);
        }
        // sysctl 不可用 → 走 sysinfo 兜底路径
        return collect_network_sysinfo_fallback(prev_net, last_net_at, now);
    }

    let elapsed = now
        .duration_since(last_net_at.unwrap())
        .as_secs_f64()
        .max(MIN_SAMPLE_INTERVAL_SECS);

    let mut result = Vec::new();

    // 柠檬两遍过滤策略（对齐 CmcGetNetPacketsInfo 兜底逻辑）
    let filter_passes: &[fn(&Option<HashMap<String, (u8, u64, u64)>>, &str) -> bool] =
        &[is_physical_iface, is_non_local_iface];

    // macOS: 从 get_if_info_list() 的字节计数器（sysinfo/getifaddrs）计算速率
    #[cfg(target_os = "macos")]
    if let Some(ref map) = if_types {
        for filter_fn in filter_passes.iter() {
            result.clear();
            for (name, &(_, ibytes, obytes)) in map {
                if !filter_fn(&if_types, name) {
                    continue;
                }
                let Some(&(prev_rx, prev_tx)) = prev_net.get(name) else {
                    continue;
                };
                let rx = ibytes.saturating_sub(prev_rx) as f64 / 1024.0 / 1024.0 / elapsed;
                let tx = obytes.saturating_sub(prev_tx) as f64 / 1024.0 / 1024.0 / elapsed;
                result.push(NetworkStatus {
                    name: name.clone(),
                    rx_rate_mbs: rx,
                    tx_rate_mbs: tx,
                    ip: if_ips.get(name).cloned().unwrap_or_default(),
                });
            }
            if !result.is_empty() {
                break;
            }
        }
        // 存储所有 sysctl 接口的当前计数器（含被过滤的），供下次差分用
        for (name, &(_, ibytes, obytes)) in map {
            prev_net.insert(name.clone(), (ibytes, obytes));
        }
    } else {
        // sysctl 失败 → 回退到 sysinfo
        let fallback = collect_network_sysinfo_fallback(prev_net, last_net_at, now);
        // sysinfo 兜底已在内部更新 prev_net / last_net_at
        return match fallback {
            Ok(r) => finish_network_results(r, rx_history, tx_history),
            Err(e) => Err(e),
        };
    }

    *last_net_at = Some(now);
    finish_network_results(result, rx_history, tx_history)
}

/// 网络采集 sysinfo 兜底路径（非 macOS 或 macOS sysctl 失败时使用）。
/// 从 sysinfo::Networks(getifaddrs) 获取字节计数器，沿用柠檬两遍过滤策略。
fn collect_network_sysinfo_fallback(
    prev_net: &mut HashMap<String, (u64, u64)>,
    last_net_at: &mut Option<Instant>,
    now: Instant,
) -> Result<Vec<NetworkStatus>, String> {
    use sysinfo::Networks;
    let nets = Networks::new_with_refreshed_list();
    let if_ips = get_interface_ips();
    let if_types: Option<HashMap<String, (u8, u64, u64)>> = None;

    if last_net_at.is_none() {
        *last_net_at = Some(now);
        for (name, data) in nets.iter() {
            prev_net.insert(
                name.clone(),
                (data.total_received(), data.total_transmitted()),
            );
        }
        return Ok(Vec::new());
    }

    let elapsed = now
        .duration_since(last_net_at.unwrap())
        .as_secs_f64()
        .max(MIN_SAMPLE_INTERVAL_SECS);
    let mut result = Vec::new();
    let filter_passes: &[fn(&Option<HashMap<String, (u8, u64, u64)>>, &str) -> bool] =
        &[is_physical_iface, is_non_local_iface];

    for filter_fn in filter_passes.iter() {
        result.clear();
        for (name, cur) in nets.iter() {
            if !filter_fn(&if_types, name) {
                continue;
            }
            let Some(&(prev_rx, prev_tx)) = prev_net.get(name) else {
                continue;
            };
            let rx =
                cur.total_received().saturating_sub(prev_rx) as f64 / 1024.0 / 1024.0 / elapsed;
            let tx =
                cur.total_transmitted().saturating_sub(prev_tx) as f64 / 1024.0 / 1024.0 / elapsed;
            result.push(NetworkStatus {
                name: name.to_string(),
                rx_rate_mbs: rx,
                tx_rate_mbs: tx,
                ip: if_ips.get(name).cloned().unwrap_or_default(),
            });
        }
        if !result.is_empty() {
            break;
        }
    }

    *last_net_at = Some(now);
    for (name, data) in nets.iter() {
        prev_net.insert(
            name.clone(),
            (data.total_received(), data.total_transmitted()),
        );
    }
    Ok(result)
}

/// 排序（Top 3）、聚合、更新 history buffer，返回最终结果。
fn finish_network_results(
    mut result: Vec<NetworkStatus>,
    rx_history: &mut Vec<f64>,
    tx_history: &mut Vec<f64>,
) -> Result<Vec<NetworkStatus>, String> {
    result.sort_by(|a, b| {
        (b.rx_rate_mbs + b.tx_rate_mbs)
            .partial_cmp(&(a.rx_rate_mbs + a.tx_rate_mbs))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    result.truncate(3);

    if result.is_empty() {
        log::warn!("[net:diag] 两遍过滤均无结果");
    }

    let total_rx: f64 = result.iter().map(|r| r.rx_rate_mbs).sum();
    let total_tx: f64 = result.iter().map(|r| r.tx_rate_mbs).sum();
    if rx_history.len() >= NETWORK_HISTORY_SIZE {
        rx_history.remove(0);
    }
    if tx_history.len() >= NETWORK_HISTORY_SIZE {
        tx_history.remove(0);
    }
    rx_history.push(total_rx);
    tx_history.push(total_tx);

    Ok(result)
}

// ── 接口类型探测（sysctl NET_RT_IFLIST2）──────────────────────
//
// 架构决策（2026-09 重构）：
// - sysctl NET_RT_IFLIST2 **仅用于硬件类型探测**（ifi_type，偏移 16）。
//   macOS 私有内核结构的 if_data64 在 collisions 与 ibytes 之间有版本相关
//   的扩展字段（macOS 12 实测为 16 字节），导致手工偏移量极难维护。
// - **接口名**通过 `if_indextoname` 获取（POSIX 标准，稳定可靠）。
// - **字节计数器**通过 `sysinfo::Networks`（底层 getifaddrs）获取，
//   与柠檬 `CmcGetNetPacketsInfo` 的数据同源（均来自内核计数器）。
//
// 历史 Bug 修复记录：
// 1. RTM_IFINFO2 类型字节在 if_msghdr2 的 offset 3（非 offset 4），
//    旧代码读成 ifm_addrs 低字节(=0x10)，永远匹配不到 0x12，
//    导致 HashMap 为空 → 所有接口全零。
// 2. ifi_name 不在 if_data64 内（旧代码从 offset HDR+24 读，
//    实际读到的是 ifi_ipackets 的低字节），接口名全为空串或乱码。
// 3. ifi_ibytes 偏移在 macOS 12 实测为 HDR+80（非 HDR+64），
//    中间 16 字节是内核私有扩展字段，旧代码读到的是 ifi_collisions(=0)。

extern "C" {
    fn if_indextoname(ifindex: u32, ifname: *mut u8) -> *mut u8;
}

/// 通过 sysctl NET_RT_IFLIST2 查询硬件类型 + if_indextoname 获取接口名，
/// 再用 sysinfo::Networks 获取字节计数器。
/// 返回 `HashMap<接口名, (硬件类型, 接收字节, 发送字节)>`。
#[cfg(target_os = "macos")]
fn get_if_info_list() -> Option<HashMap<String, (u8, u64, u64)>> {
    use sysctl_net::*;

    unsafe {
        let mut mib: [libc::c_int; 6] = [CTL_NET, PF_ROUTE, 0, 0, NET_RT_IFLIST2, 0];
        let mut needed: usize = 0;

        if sysctl(
            mib.as_mut_ptr(),
            6,
            std::ptr::null_mut(),
            &mut needed,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
        if needed == 0 {
            return Some(HashMap::new());
        }

        let mut buf = vec![0u8; needed];
        if sysctl(
            mib.as_mut_ptr(),
            6,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut needed,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }

        // 第一步：从 sysctl 提取 (index, ifi_type)
        // if_msghdr2 布局：
        //   [0-1] ifm_msglen (u16)
        //   [2]   ifm_version (u8)
        //   [3]   ifm_type (u8)    ← RTM_IFINFO2 = 0x12 在此处（非 offset 4!）
        //   [4-7] ifm_addrs (i32)
        //   [8-11] ifm_flags (i32)
        //   [12-13] ifm_index (u16)
        //   [14-15] ifm_len (u16)
        //   [16-31] 内核私有扩展字段（macOS 12 实测为 16 字节）
        //   [32]    ifi_type（硬件接口类型，IFT_ETHER=0x06 等）
        let mut hw_types: HashMap<u16, u8> = HashMap::new();
        let mut offset = 0usize;
        while offset + HDR_SIZE <= needed {
            let ptr = buf.as_ptr().add(offset);
            let msglen = *(ptr as *const u16) as usize;
            if msglen < HDR_SIZE || offset + msglen > needed {
                break;
            }
            // RTM_IFINFO2 在 byte 3（ifm_type），非 byte 4（ifm_addrs 低字节）
            let msg_type = *ptr.add(3);
            if msg_type == RTM_IFINFO2 as u8 {
                let index = u16::from_ne_bytes([*ptr.add(12), *ptr.add(13)]);
                let ifi_type = *ptr.add(HDR_SIZE);
                hw_types.insert(index, ifi_type);
            }
            offset += msglen;
        }

        // 第二步：通过 if_indextoname 获取接口名（POSIX 标准 API，稳定可靠）
        let mut result: HashMap<String, (u8, u64, u64)> = HashMap::new();
        for (&index, &ifi_type) in &hw_types {
            let mut name_buf = [0u8; 32];
            let name_ptr = if_indextoname(index as u32, name_buf.as_mut_ptr());
            if name_ptr.is_null() {
                continue;
            }
            let name = match std::ffi::CStr::from_ptr(name_ptr as *const libc::c_char).to_str() {
                Ok(s) => s.to_string(),
                Err(_) => continue,
            };
            // 字节计数器先填 0，随后由 sysinfo 覆盖
            result.insert(name, (ifi_type, 0, 0));
        }

        // 第三步：用 sysinfo::Networks (getifaddrs) 填充字节计数器
        // getifaddrs 与 sysctl 读取同一内核计数器，数值一致。
        use sysinfo::Networks;
        let nets = Networks::new_with_refreshed_list();
        for (name, data) in nets.iter() {
            if let Some(entry) = result.get_mut(name) {
                entry.1 = data.total_received();
                entry.2 = data.total_transmitted();
            }
        }

        Some(result)
    }
}

#[cfg(not(target_os = "macos"))]
fn get_if_info_list() -> Option<HashMap<String, (u8, u64, u64)>> {
    None
}

// ── 物理接口判定（柠檬白名单 + 兜底过滤）──────────────────────

/// 判断接口是否为物理网卡。
///
/// - sysctl 可用（`if_types.is_some()`）→ 硬件类型白名单（柠檬 `IFT_ETHER` / `IFT_PPP` / `IFT_CELLULAR`）
/// - sysctl 不可用 → 回退到前缀排除（Mole CLI 同款 `NOISE_PREFIXES`）
fn is_physical_iface(if_types: &Option<HashMap<String, (u8, u64, u64)>>, name: &str) -> bool {
    match if_types {
        Some(map) => match map.get(name) {
            Some(&(hw_type, _, _)) => {
                #[cfg(target_os = "macos")]
                {
                    PHYSICAL_HW_TYPES.contains(&hw_type)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    !is_noise_prefix(name)
                }
            }
            None => !is_noise_prefix(name),
        },
        None => !is_noise_prefix(name),
    }
}

fn is_noise_prefix(name: &str) -> bool {
    let lower = name.to_lowercase();
    NOISE_PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// 兜底过滤器：排除已知虚拟/本地接口，保留其余所有接口。
///
/// 对齐柠檬 `CmcGetNetPacketsInfo` 的兜底逻辑：当硬件类型白名单过滤结果为空
/// （如 Apple Silicon Wi-Fi 报了非 IFT_ETHER 类型），回退到此函数。
/// 柠檬的兜底：排除 IFT_LOOP / IFT_GIF / IFT_STF / IFT_PKTAP 后重新统计。
/// 我们的兜底：排除 NOISE_PREFIXES（lo/awdl/utun/llw/bridge/gif/stf/xhc/anpi/ap）。
///
/// 签名与 `is_physical_iface` 一致，可直接放入过滤函数数组。
fn is_non_local_iface(_if_types: &Option<HashMap<String, (u8, u64, u64)>>, name: &str) -> bool {
    !is_noise_prefix(name)
}

// ── 接口 IP 地址 ───────────────────────────────────────────────

fn get_interface_ips() -> HashMap<String, String> {
    let mut map = HashMap::new();
    use sysinfo::Networks;
    let nets = Networks::new_with_refreshed_list();
    for (name, data) in nets.iter() {
        for ip in data.ip_networks() {
            let addr = ip.addr.to_string();
            if addr.contains('.') && !addr.starts_with("127.") {
                map.insert(name.clone(), addr);
                break;
            }
        }
    }
    map
}

// ── 代理检测（全链路：env → scutil → TUN）─────────────────────

pub fn collect_proxy() -> ProxyStatus {
    if let Some(p) = proxy_from_env() {
        return p;
    }
    if let Some(p) = proxy_from_scutil() {
        return p;
    }
    proxy_from_tun()
}

fn proxy_from_tun() -> ProxyStatus {
    use sysinfo::Networks;
    let nets = Networks::new_with_refreshed_list();

    let mut active: Vec<String> = nets
        .iter()
        .filter_map(|(name, data)| {
            let lower = name.to_lowercase();
            let is_tun = lower.starts_with("utun") || lower.starts_with("tun");
            if !is_tun {
                return None;
            }
            if data.total_received() + data.total_transmitted() == 0 {
                return None;
            }
            Some(name.clone())
        })
        .collect();

    if active.is_empty() {
        return ProxyStatus::default();
    }
    active.sort();
    let host = if active.len() > 1 {
        format!("{}+", active[0])
    } else {
        active[0].clone()
    };
    ProxyStatus {
        enabled: true,
        proxy_type: "TUN".into(),
        host,
    }
}

fn proxy_from_env() -> Option<ProxyStatus> {
    let keys = [
        "https_proxy",
        "HTTPS_PROXY",
        "http_proxy",
        "HTTP_PROXY",
        "all_proxy",
        "ALL_PROXY",
    ];
    for key in &keys {
        if let Ok(val) = std::env::var(key) {
            let val = val.trim().to_string();
            if val.is_empty() {
                continue;
            }
            let host = parse_proxy_host(&val);
            let ptype = if val.to_lowercase().starts_with("socks") {
                "SOCKS"
            } else {
                "HTTP"
            };
            return Some(ProxyStatus {
                enabled: true,
                proxy_type: ptype.into(),
                host,
            });
        }
    }
    None
}

fn proxy_from_scutil() -> Option<ProxyStatus> {
    let out = Command::new("scutil").arg("--proxy").output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    if scutil_enabled(&s, "SOCKSEnable") {
        let host = join_host_port(scutil_val(&s, "SOCKSProxy"), scutil_val(&s, "SOCKSPort"));
        return Some(ProxyStatus {
            enabled: true,
            proxy_type: "SOCKS".into(),
            host: if host.is_empty() {
                "System Proxy".into()
            } else {
                host
            },
        });
    }
    if scutil_enabled(&s, "HTTPSEnable") {
        let host = join_host_port(scutil_val(&s, "HTTPSProxy"), scutil_val(&s, "HTTPSPort"));
        return Some(ProxyStatus {
            enabled: true,
            proxy_type: "HTTPS".into(),
            host: if host.is_empty() {
                "System Proxy".into()
            } else {
                host
            },
        });
    }
    if scutil_enabled(&s, "HTTPEnable") {
        let host = join_host_port(scutil_val(&s, "HTTPProxy"), scutil_val(&s, "HTTPPort"));
        return Some(ProxyStatus {
            enabled: true,
            proxy_type: "HTTP".into(),
            host: if host.is_empty() {
                "System Proxy".into()
            } else {
                host
            },
        });
    }
    if scutil_enabled(&s, "ProxyAutoConfigEnable") {
        let pac_url = scutil_val(&s, "ProxyAutoConfigURLString");
        let host = parse_proxy_host(pac_url);
        return Some(ProxyStatus {
            enabled: true,
            proxy_type: "PAC".into(),
            host: if host.is_empty() { "PAC".into() } else { host },
        });
    }
    if scutil_enabled(&s, "ProxyAutoDiscoveryEnable") {
        return Some(ProxyStatus {
            enabled: true,
            proxy_type: "WPAD".into(),
            host: "Auto Discovery".into(),
        });
    }
    None
}

fn scutil_enabled(out: &str, key: &str) -> bool {
    let needle = format!("{} : 1", key);
    out.lines().any(|l| l.trim() == needle)
}

fn scutil_val<'a>(out: &'a str, key: &str) -> &'a str {
    let prefix = format!("{} : ", key);
    out.lines()
        .find(|l| l.trim().starts_with(&prefix))
        .and_then(|l| l.trim().strip_prefix(&prefix))
        .unwrap_or("")
}

fn join_host_port(host: &str, port: &str) -> String {
    if host.is_empty() {
        return String::new();
    }
    if port.is_empty() {
        host.to_string()
    } else {
        format!("{}:{}", host, port)
    }
}

fn parse_proxy_host(val: &str) -> String {
    let s = val.trim();
    if let Some(stripped) = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))
        .or_else(|| s.strip_prefix("socks5://"))
    {
        stripped.split('/').next().unwrap_or(s).to_string()
    } else {
        s.to_string()
    }
}
