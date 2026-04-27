//! macOS：在 Finder 中定位（高亮）指定文件 / 文件夹。
//!
//! 对齐 Lemon 的 `ResultCellView`「Show in Finder」按钮：
//! 通过 `NSWorkspace.activateFileViewerSelectingURLs:` 激活 Finder 并选中目标路径。
//! 非 Apple 平台返回 `Err`。

#[cfg(target_os = "macos")]
fn reveal_in_finder_inner(path: &str) -> Result<(), String> {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSArray, NSString, NSURL};

    // 目标路径必须真实存在，否则 Finder 无法定位，直接给出可读错误。
    if !std::path::Path::new(path).exists() {
        return Err(format!("Path does not exist: {path}"));
    }

    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let ns_path = NSString::from_str(path);
        let url = NSURL::fileURLWithPath(&ns_path);
        // 单元素数组，激活 Finder 并高亮选中该路径。
        let urls = NSArray::from_retained_slice(&[url]);
        workspace.activateFileViewerSelectingURLs(&urls);
        Ok(())
    })
}

/// 在 Finder 中定位路径（macOS 10.14+）。
/// 非主线程时切到主队列执行，保证 NSWorkspace 的 UI 调用安全。
#[cfg(target_os = "macos")]
pub fn reveal_in_finder(path: &str) -> Result<(), String> {
    use dispatch2::DispatchQueue;
    use objc2::MainThreadMarker;

    if MainThreadMarker::new().is_some() {
        return reveal_in_finder_inner(path);
    }

    let path = path.to_string();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    DispatchQueue::main().exec_sync(move || {
        let _ = tx.send(reveal_in_finder_inner(&path));
    });
    rx.recv()
        .map_err(|_| "reveal in finder: main queue result channel closed".to_string())?
}

#[cfg(not(target_os = "macos"))]
pub fn reveal_in_finder(_path: &str) -> Result<(), String> {
    Err("reveal in Finder is only available on macOS".into())
}
