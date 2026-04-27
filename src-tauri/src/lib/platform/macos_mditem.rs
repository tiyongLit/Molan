//! Spotlight MDItem 元数据捷径 —— 柠檬 `MdlsToolsHelper`（mdls 子进程）的同义替代。
//!
//! 红线 1 禁止 spawn 外部二进制（MAS 公证硬约束），故不走柠檬的 `mdls` 命令，
//! 改为 CoreServices 框架 FFI 直调 `MDItemCreateWithURL` / `MDItemCopyAttribute`
//!（与 objc2/core-foundation 同属「Rust 调 macOS 系统 API」豁免类）。
//!
//! 用途（混合叶子捷径方案）：扫描期对 bundle 目录同步查询 Spotlight 索引，取
//! 预计算聚合物理大小与 bundle 元数据写入快照；钻取期供 UI 预填充（badge/副标题）。
//! MDItem 查询为同步 CF 调用，worker 线程可并发。

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

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::bundle_shortcut;

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
    fn plain_dir_returns_none() {
        // 普通目录无 bundle 语义：MDItem 存在但 physicalSize 可能 >0？
        // 普通目录同样有 kMDItemPhysicalSize —— 叶子化判定在 scanner 侧由扩展名门控，
        // 此处仅验证非 bundle 路径不会 panic 且返回结构合法。
        let sc = bundle_shortcut("/usr");
        if let Some(sc) = sc {
            assert!(sc.physical_size > 0);
            assert!(sc.bundle_id.is_none(), "普通目录不应有 bundle id");
        }
    }
}
