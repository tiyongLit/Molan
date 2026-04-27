//! macOS：`NSWorkspace.icon(forFile:)` → TIFF → `NSBitmapImageRep` → PNG(NSData) → Base64，
//! 全程用 AppKit/Foundation，不再依赖 `image` crate。
//! 非 Apple 平台返回 `Ok(None)`。

#[cfg(target_os = "macos")]
fn nsdata_to_vec(data: &objc2_foundation::NSData) -> Vec<u8> {
    use core::ptr::NonNull;
    let len = data.length() as usize;
    if len == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u8; len];
    unsafe {
        data.getBytes_length(
            NonNull::new(buf.as_mut_ptr().cast()).expect("NSData copy buffer"),
            data.length(),
        );
    }
    buf
}

#[cfg(target_os = "macos")]
fn file_icon_png_base64_inner(path: &str) -> Result<Option<String>, String> {
    use base64::Engine;
    use objc2::rc::autoreleasepool;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey, NSWorkspace,
    };
    use objc2_core_foundation::CGSize;
    use objc2_foundation::{NSDictionary, NSFileManager, NSString};

    autoreleasepool(|_| {
        // 关键 1：NSWorkspace.iconForFile 对不存在的路径也会返回通用文件图标（白色 txt），
        // 导致前端拿到「假图标」而非 null，无法回退到首字母色块。
        // 先校验路径存在性，不存在直接返回 None。
        let ns_path_orig = NSString::from_str(path);
        let fm = NSFileManager::defaultManager();
        if !fm.fileExistsAtPath(&ns_path_orig) {
            return Ok(None);
        }

        // 关键 2：符号链接路径**必须原样传入**，不能 canonicalize。
        // iconForFile 对 symlink 返回的是「目标图标 + 左下角 alias 角标」（系统已合成），
        // canonicalize 后拿到的是纯目标图标 —— 角标丢失，与 Finder / lemon-cleaner 展示不一致。
        // （实测：iconForFile("/etc") 底图与 iconForFile("/private/etc") 相同，仅多出角标。）
        let ns_path = NSString::from_str(path);

        let workspace = NSWorkspace::sharedWorkspace();
        let icon = workspace.iconForFile(&ns_path);
        icon.setSize(CGSize::new(128.0, 128.0));

        let Some(tiff) = icon.TIFFRepresentation() else {
            return Ok(None);
        };

        let Some(bitmap_rep) = NSBitmapImageRep::imageRepWithData(&tiff) else {
            return Ok(None);
        };

        let props = NSDictionary::<NSBitmapImageRepPropertyKey, AnyObject>::dictionary();
        let png_data = unsafe {
            bitmap_rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props)
        };
        let Some(png_data) = png_data else {
            return Ok(None);
        };

        let png_bytes = nsdata_to_vec(&png_data);
        if png_bytes.is_empty() {
            return Ok(None);
        }

        Ok(Some(
            base64::engine::general_purpose::STANDARD.encode(png_bytes),
        ))
    })
}

/// 与 Finder / Pearcleaner 使用的卷标图标一致（对根路径一般为内置硬盘图标）。
#[cfg(target_os = "macos")]
pub fn file_icon_png_base64(path: &str) -> Result<Option<String>, String> {
    use dispatch2::DispatchQueue;
    use objc2::MainThreadMarker;

    if MainThreadMarker::new().is_some() {
        return file_icon_png_base64_inner(path);
    }

    let path = path.to_string();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    DispatchQueue::main().exec_sync(move || {
        let _ = tx.send(file_icon_png_base64_inner(&path));
    });
    rx.recv()
        .map_err(|_| "file icon: main queue result channel closed".to_string())?
}

#[cfg(not(target_os = "macos"))]
pub fn file_icon_png_base64(_path: &str) -> Result<Option<String>, String> {
    Ok(None)
}

// 回归测试：iconForFile 对 symlink 返回「目标图标 + alias 角标」，路径必须原样传入。
// 断言依据（实测 PNG 字节）：/etc 与 /tmp 同为指向目录的 symlink → 同一张带角标图标；
// /usr 与 /private/etc 为真实目录 → 纯文件夹图标，与带角标版本不同。
#[cfg(all(test, target_os = "macos"))]
mod probe_tests {
    use super::file_icon_png_base64_inner;

    #[test]
    fn probe_icon_for_file_keeps_symlink_badge() {
        let etc = file_icon_png_base64_inner("/etc").unwrap();
        let tmp = file_icon_png_base64_inner("/tmp").unwrap();
        let usr = file_icon_png_base64_inner("/usr").unwrap();
        // symlink 指向的目录本身（canonicalize 会退化成这张无角标图）
        let private_etc = file_icon_png_base64_inner("/private/etc").unwrap();
        let plain_file =
            file_icon_png_base64_inner("/System/Library/CoreServices/SystemVersion.plist").unwrap();
        // 两个 symlink 图标一致（都是「文件夹 + 角标」）
        assert_eq!(etc, tmp, "symlink-to-dir icons must be identical");
        // symlink 图标必须区别于纯目录图标（差在那枚 alias 角标）
        assert_ne!(etc, usr, "symlink icon must carry the alias badge");
        // 关键回归点：解析目标后的纯文件夹图标不能等于 symlink 图标，
        // 一旦实现里重新引入 canonicalize，本断言会失败。
        assert_eq!(
            usr, private_etc,
            "real dir and its symlink target must share one icon"
        );
        assert_ne!(
            etc, private_etc,
            "must not resolve symlink before iconForFile"
        );
        assert_ne!(
            plain_file, usr,
            "plain file icon must differ from folder icon"
        );
    }
}
