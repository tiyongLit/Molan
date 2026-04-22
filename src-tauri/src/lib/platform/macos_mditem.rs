//! Spotlight MDItem 元数据捷径 —— 柠檬 `MdlsToolsHelper`（mdls 子进程）的同义替代。
//!
//! 红线 1 禁止 spawn 外部二进制（MAS 公证硬约束），故不走柠檬的 `mdls` 命令，
//! 改为 CoreServices 框架 FFI 直调 `MDItemCreateWithURL` / `MDItemCopyAttribute`
//!（与 objc2/core-foundation 同属「Rust 调 macOS 系统 API」豁免类）。
//!
//! 用途（混合叶子捷径方案）：扫描期对 bundle 目录同步查询 Spotlight 索引，取
//! 预计算聚合物理大小与 bundle 元数据写入快照；钻取期供 UI 预填充（badge/副标题）。
//! MDItem 查询为同步 CF 调用，worker 线程可并发。
//!
//! 另供 Uninstall 卸载列表使用（原 `mdls` 子进程的原生替代）：
//! `logical_size`（kMDItemLogicalSize）/ `last_used_epoch`（kMDItemLastUsedDate）/
//! `spotlight_display_name`（kMDItemDisplayName）。

use serde::{Deserialize, Serialize};

/// Bundle 叶子捷径元数据（随快照持久化）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BundleShortcut {
    /// Spotlight 预计算聚合物理大小（字节）；<=0 视为未索引（调用侧回退递归）。
    pub physical_size: i64,
    pub display_name: Option<String>,
    pub bundle_id: Option<String>,
    /// `kMDItemContentTypeTree`，如 `["com.apple.application-bundle", "com.apple.bundle"]`。
    pub content_types: Vec<String>,
}

/// 查询路径的 bundle 捷径；非 macOS / 非索引对象 / 未索引（size<=0）返回 None。
#[cfg(target_os = "macos")]
pub fn bundle_shortcut(path: &str) -> Option<BundleShortcut> {
    use core_foundation::base::{CFRelease, CFTypeRef, TCFType, kCFAllocatorDefault};
    use core_foundation::url::CFURL;
    use ffi::{MDItemCreateWithURL, MDItemRef};

    let url = CFURL::from_path(path, true)?;
    let item: MDItemRef =
        unsafe { MDItemCreateWithURL(kCFAllocatorDefault, url.as_concrete_TypeRef()) };
    if item.is_null() {
        return None;
    }
    let shortcut = unsafe { read_shortcut(item) };
    unsafe { CFRelease(item as CFTypeRef) };
    shortcut
}

#[cfg(not(target_os = "macos"))]
pub fn bundle_shortcut(_path: &str) -> Option<BundleShortcut> {
    None
}

/// 查询路径的 Spotlight 逻辑大小（`kMDItemLogicalSize`，字节）；未索引返回 None。
/// 原 `mdls -raw -name kMDItemLogicalSize` 的同义替代。
#[cfg(target_os = "macos")]
pub fn logical_size(path: &str) -> Option<u64> {
    with_item(path, |item| unsafe {
        read_i64_attr(item, ffi::kMDItemLogicalSize)
    })?
    .filter(|v| *v > 0)
    .map(|v| v as u64)
}

#[cfg(not(target_os = "macos"))]
pub fn logical_size(_path: &str) -> Option<u64> {
    None
}

/// 查询路径的最后使用时间（`kMDItemLastUsedDate`，Unix 秒）；未索引返回 None。
/// 原 `mdls -raw -name kMDItemLastUsedDate` + `date -j -f` 解析的同义替代。
#[cfg(target_os = "macos")]
pub fn last_used_epoch(path: &str) -> Option<i64> {
    /// CFDate 绝对时间零点（2001-01-01T00:00:00Z）到 Unix epoch 的秒差。
    const CF_ABSOLUTE_TO_UNIX_SECS: f64 = 978_307_200.0;

    let secs = with_item(path, |item| unsafe {
        read_date_attr(item, ffi::kMDItemLastUsedDate)
    })?
    .map(|t| t + CF_ABSOLUTE_TO_UNIX_SECS)?;
    if secs <= 0.0 { None } else { Some(secs as i64) }
}

#[cfg(not(target_os = "macos"))]
pub fn last_used_epoch(_path: &str) -> Option<i64> {
    None
}

/// 查询路径的 Spotlight 显示名（`kMDItemDisplayName`）；未索引返回 None。
/// 原 `mdls -raw -name kMDItemDisplayName` 的同义替代。
#[cfg(target_os = "macos")]
pub fn spotlight_display_name(path: &str) -> Option<String> {
    with_item(path, |item| unsafe {
        copy_string(item, ffi::kMDItemDisplayName)
    })
    .flatten()
}

#[cfg(not(target_os = "macos"))]
pub fn spotlight_display_name(_path: &str) -> Option<String> {
    None
}

/// 打开路径的 MDItem 并执行读取，结束后释放（CF "Create" 规则配对释放）。
#[cfg(target_os = "macos")]
fn with_item<T>(path: &str, f: impl FnOnce(ffi::MDItemRef) -> T) -> Option<T> {
    use core_foundation::base::{CFRelease, CFTypeRef, TCFType, kCFAllocatorDefault};
    use core_foundation::url::CFURL;
    use ffi::{MDItemCreateWithURL, MDItemRef};

    let url = CFURL::from_path(path, true)?;
    let item: MDItemRef =
        unsafe { MDItemCreateWithURL(kCFAllocatorDefault, url.as_concrete_TypeRef()) };
    if item.is_null() {
        return None;
    }
    let out = f(item);
    unsafe { CFRelease(item as CFTypeRef) };
    Some(out)
}

#[cfg(target_os = "macos")]
mod ffi {
    use core_foundation::base::{CFAllocatorRef, CFTypeRef};
    use core_foundation::string::CFStringRef;
    use core_foundation::url::CFURLRef;

    /// CoreServices 不透明类型（MDItemRef = CFTypeRef 子类）。
    pub type MDItemRef = *mut std::os::raw::c_void;

    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        pub static kMDItemPhysicalSize: CFStringRef;
        pub static kMDItemDisplayName: CFStringRef;
        pub static kMDItemCFBundleIdentifier: CFStringRef;
        pub static kMDItemContentTypeTree: CFStringRef;
        pub static kMDItemLogicalSize: CFStringRef;
        pub static kMDItemLastUsedDate: CFStringRef;

        pub fn MDItemCreateWithURL(allocator: CFAllocatorRef, url: CFURLRef) -> MDItemRef;
        pub fn MDItemCopyAttribute(item: MDItemRef, name: CFStringRef) -> CFTypeRef;
    }
}

#[cfg(target_os = "macos")]
unsafe fn read_shortcut(item: ffi::MDItemRef) -> Option<BundleShortcut> {
    use core_foundation::base::TCFType;
    use core_foundation::number::CFNumber;

    // 聚合物理大小：Spotlight 未索引时属性缺失/为 0 → None（调用侧回退递归，对齐柠檬）
    let size_attr = unsafe { ffi::MDItemCopyAttribute(item, ffi::kMDItemPhysicalSize) };
    let physical_size = if size_attr.is_null() {
        0
    } else {
        let num = unsafe { CFNumber::wrap_under_create_rule(size_attr as _) };
        num.to_i64().unwrap_or(0)
    };
    if physical_size <= 0 {
        return None;
    }

    Some(BundleShortcut {
        physical_size,
        display_name: copy_string(item, ffi::kMDItemDisplayName),
        bundle_id: copy_string(item, ffi::kMDItemCFBundleIdentifier),
        content_types: copy_string_array(item, ffi::kMDItemContentTypeTree),
    })
}

#[cfg(target_os = "macos")]
unsafe fn copy_string(
    item: ffi::MDItemRef,
    key: core_foundation::string::CFStringRef,
) -> Option<String> {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;

    let attr = unsafe { ffi::MDItemCopyAttribute(item, key) };
    if attr.is_null() {
        return None;
    }
    let s = unsafe { CFString::wrap_under_create_rule(attr as _) };
    Some(s.to_string())
}

#[cfg(target_os = "macos")]
unsafe fn copy_string_array(
    item: ffi::MDItemRef,
    key: core_foundation::string::CFStringRef,
) -> Vec<String> {
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;

    let attr = unsafe { ffi::MDItemCopyAttribute(item, key) };
    if attr.is_null() {
        return Vec::new();
    }
    let arr = unsafe { CFArray::<CFString>::wrap_under_create_rule(attr as CFArrayRef) };
    arr.iter().map(|s| s.to_string()).collect()
}

/// 读 CFNumber 属性为 i64；属性缺失/类型不符返回 None。
#[cfg(target_os = "macos")]
unsafe fn read_i64_attr(
    item: ffi::MDItemRef,
    key: core_foundation::string::CFStringRef,
) -> Option<i64> {
    use core_foundation::base::TCFType;
    use core_foundation::number::CFNumber;

    let attr = unsafe { ffi::MDItemCopyAttribute(item, key) };
    if attr.is_null() {
        return None;
    }
    let num = unsafe { CFNumber::wrap_under_create_rule(attr as _) };
    num.to_i64()
}

/// 读 CFDate 属性为 CF 绝对时间（秒，2001 纪元）；属性缺失/类型不符返回 None。
#[cfg(target_os = "macos")]
unsafe fn read_date_attr(
    item: ffi::MDItemRef,
    key: core_foundation::string::CFStringRef,
) -> Option<f64> {
    use core_foundation::base::TCFType;
    use core_foundation::date::CFDate;

    let attr = unsafe { ffi::MDItemCopyAttribute(item, key) };
    if attr.is_null() {
        return None;
    }
    let date = unsafe { CFDate::wrap_under_create_rule(attr as _) };
    Some(date.abs_time())
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::{bundle_shortcut, last_used_epoch, logical_size, spotlight_display_name};

    #[test]
    fn real_app_returns_spotlight_shortcut() {
        let path = "/System/Applications/Utilities/Terminal.app";
        if !std::path::Path::new(path).exists() {
            return; // 非标准系统布局环境跳过
        }
        let sc = bundle_shortcut(path).expect("Terminal.app 应被 Spotlight 索引");
        assert!(sc.physical_size > 0, "聚合物理大小应 > 0");
        assert_eq!(sc.bundle_id.as_deref(), Some("com.apple.Terminal"));
        assert!(
            sc.content_types
                .iter()
                .any(|t| t.contains("application-bundle") || t.contains("bundle")),
            "contentTypeTree 应含 bundle 类型: {:?}",
            sc.content_types
        );
    }

    #[test]
    fn real_app_has_logical_size_and_display_name() {
        let path = "/System/Applications/Utilities/Terminal.app";
        if !std::path::Path::new(path).exists() {
            return;
        }
        assert!(logical_size(path).unwrap_or(0) > 0, "逻辑大小应 > 0");
        // 显示名为本地化名（中文环境即「终端」），只断言非空且不是路径
        let name = spotlight_display_name(path).expect("应有 Spotlight 显示名");
        assert!(!name.is_empty(), "显示名不应为空");
        assert!(!name.starts_with('/'), "显示名不应是路径: {name}");
    }

    #[test]
    fn last_used_epoch_is_plausible_or_absent() {
        let path = "/System/Applications/Utilities/Terminal.app";
        if !std::path::Path::new(path).exists() {
            return;
        }
        if let Some(epoch) = last_used_epoch(path) {
            // 2001-01-01（978307200）之后且不在未来太多都算合理
            assert!(epoch > 978_307_200, "epoch 应晚于 2001: {epoch}");
        }
    }

    #[test]
    fn nonexistent_path_returns_none() {
        // 不存在的路径：MDItemCreateWithURL 无有效项，三个读取口都应安全返回 None。
        let bogus = "/nonexistent-mole-probe-9f3a/Terminal.app";
        assert!(logical_size(bogus).is_none());
        assert!(last_used_epoch(bogus).is_none());
        assert!(spotlight_display_name(bogus).is_none());
    }
}
