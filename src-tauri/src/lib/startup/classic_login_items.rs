//! 经典登录项扫描（LSSharedFileList API）。
//! 对齐柠檬 `LoginItemManager.getAllValidLoginItems`：读取系统偏好设置 → 用户与群组 → 登录项。
//! 这些条目存储在 macOS 的二进制数据库（SFL3）中，不是 plist 文件。
//!
//! 使用 macOS 原生 CoreServices.framework 的 LSSharedFileList C API 读取，
//! 不依赖任何外部进程。

use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use core_foundation::url::CFURL;
use std::ffi::c_void;
use std::ptr;

// ── FFI 绑定 ──

type LSSharedFileListRef = *mut c_void;
type LSSharedFileListItemRef = *mut c_void;
type CFErrorRef = *mut c_void;

// 标志位：防止远程服务器弹窗
const K_LS_SHARED_FILE_LIST_NO_USER_INTERACTION: u32 = 1;

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    static kLSSharedFileListSessionLoginItems: *const c_void;

    fn LSSharedFileListCreate(
        allocator: *const c_void,
        in_kind: *const c_void,
        in_options: *const c_void,
    ) -> LSSharedFileListRef;

    fn LSSharedFileListCopySnapshot(
        in_list: LSSharedFileListRef,
        out_seed: *mut u32,
    ) -> *const c_void; // CFArrayRef

    fn LSSharedFileListItemCopyDisplayName(
        in_item: LSSharedFileListItemRef,
    ) -> *const c_void; // CFStringRef

    fn LSSharedFileListItemCopyResolvedURL(
        in_item: LSSharedFileListItemRef,
        in_flags: u32,
        out_error: *mut CFErrorRef,
    ) -> *const c_void; // CFURLRef

    fn LSSharedFileListItemRemove(
        in_list: LSSharedFileListRef,
        in_item: LSSharedFileListItemRef,
    ) -> i32; // OSStatus (0 = noErr)

    fn CFRelease(cf: *const c_void);
}

// ── 数据结构 ──

/// 一个经典登录项（系统偏好设置 → 用户与群组 → 登录项）。
/// 对齐柠檬 `LMLoginItem`。
#[derive(Debug, Clone)]
pub struct ClassicLoginItem {
    /// 显示名称（`LSSharedFileListItemCopyDisplayName` 返回）。
    pub display_name: String,
    /// 完整路径（`LSSharedFileListItemCopyResolvedURL` 返回）。
    pub bundle_path: String,
}

// ── 扫描函数 ──

/// 扫描系统偏好设置中的经典登录项（LSSharedFileList）。
/// 对齐柠檬 `LoginItemManager.getAllValidLoginItems`。
///
/// 返回数组（不去重），保留重复条目以便用户清理。
/// 无需管理员权限（用户自己的登录项对用户可读）。
/// macOS 13+ 此 API 已标记 deprecated 但仍可用。
pub fn scan_classic_login_items() -> Vec<ClassicLoginItem> {
    let mut items = Vec::new();

    unsafe {
        // 创建 LSSharedFileList 引用
        let list = LSSharedFileListCreate(
            ptr::null(),
            kLSSharedFileListSessionLoginItems,
            ptr::null(),
        );
        if list.is_null() {
            return items;
        }

        // 获取快照（CFArray）
        let mut seed: u32 = 0;
        let snapshot = LSSharedFileListCopySnapshot(list, &mut seed);
        if snapshot.is_null() {
            CFRelease(list as *const c_void);
            return items;
        }

        // 遍历数组中的每个 item
        let array = core_foundation::array::CFArray::<core_foundation::base::CFType>::wrap_under_create_rule(
            snapshot as *const _,
        );

        for i in 0..array.len() {
            let Some(item) = array.get(i) else { continue };
            let item_ref = item.as_CFTypeRef() as LSSharedFileListItemRef;
            if item_ref.is_null() {
                continue;
            }

            // 1. 获取显示名称
            let display_name_ref = LSSharedFileListItemCopyDisplayName(item_ref);
            if display_name_ref.is_null() {
                continue;
            }
            let display_name = CFString::wrap_under_create_rule(display_name_ref as *const _)
                .to_string();

            // 2. 获取解析后的 URL（带 NoUserInteraction 标志，防止远程服务器弹窗）
            let mut error: CFErrorRef = ptr::null_mut();
            let url_ref = LSSharedFileListItemCopyResolvedURL(
                item_ref,
                K_LS_SHARED_FILE_LIST_NO_USER_INTERACTION,
                &mut error,
            );

            if url_ref.is_null() || !error.is_null() {
                if !error.is_null() {
                    CFRelease(error as *const c_void);
                }
                continue;
            }

            let url = CFURL::wrap_under_create_rule(url_ref as *const _);
            let Some(path_buf) = url.to_path() else {
                continue;
            };
            let bundle_path = path_buf.to_string_lossy().to_string();

            items.push(ClassicLoginItem { display_name, bundle_path });
        }

        CFRelease(list as *const c_void);
    }

    items
}

// ── 删除函数 ──

/// 删除指定名称的经典登录项（按 displayName 匹配，可删除所有同名条目）。
/// 对齐柠檬 `LoginItemManager.removeLoginItemsByName`。
///
/// 返回删除的数量。
pub fn remove_classic_login_item(display_name: &str) -> usize {
    let mut removed = 0;

    unsafe {
        let list = LSSharedFileListCreate(
            ptr::null(),
            kLSSharedFileListSessionLoginItems,
            ptr::null(),
        );
        if list.is_null() {
            return 0;
        }

        let mut seed: u32 = 0;
        let snapshot = LSSharedFileListCopySnapshot(list, &mut seed);
        if snapshot.is_null() {
            CFRelease(list as *const c_void);
            return 0;
        }

        let array = core_foundation::array::CFArray::<core_foundation::base::CFType>::wrap_under_create_rule(
            snapshot as *const _,
        );

        for i in 0..array.len() {
            let Some(item) = array.get(i) else { continue };
            let item_ref = item.as_CFTypeRef() as LSSharedFileListItemRef;
            if item_ref.is_null() {
                continue;
            }

            let name_ref = LSSharedFileListItemCopyDisplayName(item_ref);
            if name_ref.is_null() {
                continue;
            }
            let name = CFString::wrap_under_create_rule(name_ref as *const _).to_string();

            if name == display_name {
                let status = LSSharedFileListItemRemove(list, item_ref);
                if status == 0 {
                    removed += 1;
                }
            }
        }

        CFRelease(list as *const c_void);
    }

    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_returns_vec_without_panic() {
        // 仅验证 FFI 调用不会 panic，不依赖具体条目
        let items = scan_classic_login_items();
        // 在 CI/测试环境中可能为空，这是正常的
        assert!(items.len() < 1000);
    }
}
