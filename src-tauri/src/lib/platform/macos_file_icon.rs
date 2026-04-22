//! macOS 原生图标编码管线：`NSWorkspace.iconForFile(path)` → **128×128 像素精确重采样** →
//! PNG → SVG 信封（viewBox 与像素对齐）→ `data:image/svg+xml;base64,…` URI。
//!
//! # 设计原则
//!
//! - **显示尺寸对齐**：列表行 28px、treemap 格 30px，2x 视网膜 = 56–60 物理像素；
//!   编码到 128px（CSS 缩放到 64px 显示），相比旧实现的 1024px（`setSize(128)` +
//!   `TIFFRepresentation` 取最大表示）体积缩小 ~30×（870KB → ~30KB），同时**保留 macOS
//!   alias 角标**（Apple 在 64px 及以下表示不渲染角标，128px 表示才画）。
//! - **像素精确**：通过 `initWithBitmapDataPlanes(null, 128, 128, …)` 显式声明像素尺寸，
//!   不依赖当前显示器的 backing scale factor（旧 `setSize(128)` + `lockFocus` 在 2x
//!   下得到 256 像素、1x 下得到 128 像素，输出不稳定）。
//! - **SVG 信封**：PNG 包进 `<svg viewBox="0 0 128 128"><image href="data:png;base64,…"/></svg>`，
//!   CSS 缩放任意尺寸不会失真（浏览器双线性插值）；与纯 PNG data URI 用法完全一致，
//!   `<img src="…">` 直接可用。
//! - **原生观感零损失**：像素来源仍是 `NSWorkspace.iconForFile`（与 Finder/Lemon 同源），
//!   重采样只是让输出尺寸对齐显示尺寸，**不做任何视觉修改**。
//! - **内容级塌缩（栅格指纹备忘）**：`iconForFile` 对同一图标仍会**每次返回新 NSImage
//!   实例**（指针备忘实测 0 命中，共享实例假设已证伪），因此备忘 key 取「绘制产物
//!   （NSImage → TIFF 栅格字节）的指纹」——内容相同即命中，跳过 PNG 编码与 base64。
//!   指纹不同只会退化为重新编码，不会错图；跨会话由注册表磁盘快照兜底。
//!
//! # 调用约束
//!
//! - `encode_native_icon_inner` 必须在**主线程**调用（AppKit 约束）；`file_icons_batch`
//!   包装层处理跨线程调度。
//! - 路径不存在时返回 `Ok(None)`，避免前端拿到「假图标」（白色 txt）后无法回退到 emoji。
//! - **symlink 路径必须原样传入**，不可 `canonicalize`——`iconForFile` 对 symlink 返回
//!   「目标图标 + 左下角 alias 角标」，canonicalize 会抹掉角标，与 Finder/Lemon 不一致
//!   （回归测试 `probe_icon_for_file_keeps_symlink_badge` 守护此约束）。

#[cfg(target_os = "macos")]
fn encode_native_icon_inner(path: &str) -> Result<Option<String>, String> {
    use objc2::AnyThread;
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey, NSImage, NSWorkspace,
    };
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_foundation::{NSDictionary, NSFileManager, NSString};

    autoreleasepool(|_| {
        let ns_path = NSString::from_str(path);
        let fm = NSFileManager::defaultManager();
        if !fm.fileExistsAtPath(&ns_path) {
            return Ok(None);
        }

        let workspace = NSWorkspace::sharedWorkspace();
        let icon = workspace.iconForFile(&ns_path);

        // ── 关键：单表示 NSImage 重采样（保留系统角标合成路径） ──
        // 旧实现 setSize(128) + TIFFRepresentation 取最大表示 → 编码 1024×1024 → 870KB。
        // 新实现创建空 NSImage(128×128)，lockFocusFlipped 让 AppKit 分配单表示缓冲区，
        // icon.drawInRect 把源图缩放绘制进去（系统合成包括 alias 角标），
        // unlockFocus 后这张图只含一张 128×128 表示，TIFFRepresentation 导出它，
        // imageRepWithData 读取后编码 PNG。
        // 注：128px 而非 64px 是因为 macOS 在 64px 及以下表示不渲染 alias 角标，
        // symlink 的箭头标识会丢失（回归测试守护）；CSS 缩到 64px 显示仍是 2x 视网膜。
        const TARGET_PX: f64 = 128.0;
        let size = CGSize::new(TARGET_PX, TARGET_PX);
        let dst = NSImage::initWithSize(NSImage::alloc(), size);
        unsafe { dst.lockFocusFlipped(true) };
        unsafe {
            icon.drawInRect(CGRect {
                origin: CGPoint::new(0.0, 0.0),
                size,
            })
        };
        unsafe { dst.unlockFocus() };

        let Some(tiff) = dst.TIFFRepresentation() else {
            return Ok(None);
        };
        // 指纹 = TIFF 栅格字节（PNG 编码的直接输入，像素差异必然反映在字节里）；
        // 命中路径完全跳过 bitmap_rep 创建与 PNG 编码。
        let tiff_bytes = nsdata_to_vec(&tiff);
        if tiff_bytes.is_empty() {
            return Ok(None);
        }
        let fingerprint = tiff_fingerprint(&tiff_bytes);
        if let Some(cached) = encode_memo::get(fingerprint) {
            return Ok(Some(cached));
        }
        let Some(bitmap_rep) = NSBitmapImageRep::imageRepWithData(&tiff) else {
            return Ok(None);
        };

        let props =
            NSDictionary::<NSBitmapImageRepPropertyKey, objc2::runtime::AnyObject>::dictionary();
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

        // SVG 信封：viewBox 与像素对齐（128×128），CSS 可任意缩放；
        // data URI 整体 base64 一次（前端拿到 `data:image/svg+xml;base64,…` 直接用）。
        let svg = wrap_png_as_svg_data_uri(&png_bytes, TARGET_PX as u32);

        // 写入栅格指纹备忘（同内容图标后续直接命中）。
        encode_memo::insert(fingerprint, &svg);
        Ok(Some(svg))
    })
}

/// 把 PNG 字节包成 SVG 信封的 data URI：
/// `<svg xmlns viewBox="0 0 N N"><image href="data:image/png;base64,…"/></svg>`。
///
/// - `size_px`：PNG 的像素边长，作为 SVG 的 viewBox 与宽高；
/// - 返回字符串可直接赋给 `<img src>`、存入前端 contentStore、作为注册表 value。
fn wrap_png_as_svg_data_uri(png_bytes: &[u8], size_px: u32) -> String {
    use base64::Engine;
    let png_b64 = base64::engine::general_purpose::STANDARD.encode(png_bytes);
    let svg_xml = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {s} {s}\" width=\"{s}\" height=\"{s}\">\
            <image width=\"{s}\" height=\"{s}\" href=\"data:image/png;base64,{b64}\"/>\
         </svg>",
        s = size_px,
        b64 = png_b64
    );
    let svg_b64 = base64::engine::general_purpose::STANDARD.encode(svg_xml.as_bytes());
    format!("data:image/svg+xml;base64,{svg_b64}")
}

/// NSData → Vec<u8>（AppKit 返回数据拷贝到 Rust 缓冲区）。
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

/// 绘制产物的栅格指纹：对 TIFF 字节（`NSImage.TIFFRepresentation`）做 SipHash。
///
/// - 为什么不用 `NSBitmapImageRep.bitmapData`：实测 `imageRepWithData` 的惰性解码下
///   该缓冲区**读不到真实像素差异**（带角标与不带角标的文件夹图指纹完全碰撞），
///   会把不同图标错误合并；
/// - TIFF 字节是 PNG 编码的直接输入，像素差异必然体现在字节里；
/// - 相同图标 → 相同绘制产物 → 相同指纹（不同 NSImage 实例也能命中）；
/// - 成本：64–260KB 的 SipHash，微秒级，远低于 PNG 编码。
#[cfg(target_os = "macos")]
fn tiff_fingerprint(tiff_bytes: &[u8]) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(tiff_bytes);
    hasher.finish()
}

// ── 编码内容备忘（进程级） ──
//
// key = 绘制产物的 TIFF 栅格指纹（内容寻址）。实测 `iconForFile` 对同一图标返回
// 不同 NSImage 实例（共享实例假设证伪），但「相同图标 ⇒ 相同栅格字节」（输出逐
// 字节一致，探针实测）——指纹相等即图标内容相等。命中即跳过 bitmap_rep 创建、
// PNG 编码与 base64；上限 CAP 条封顶内存，满时整体清空重填——备忘是纯优化层，
// 清空后同内容图标最多重编码一次，结果仍正确，且避免「永久拒收新条目」在长
// 会话（如连续扫描 /Applications，唯一图标数超过 CAP）后彻底失去去重能力。
#[cfg(target_os = "macos")]
mod encode_memo {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    /// 备忘上限（每条 = 1 份 SVG data URI，约 30KB，512 条约 15MB）
    const CAP: usize = 512;

    static MEMO: OnceLock<Mutex<HashMap<u64, String>>> = OnceLock::new();

    fn lock() -> std::sync::MutexGuard<'static, HashMap<u64, String>> {
        MEMO.get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 查询栅格指纹对应的 SVG data URI。
    pub fn get(fingerprint: u64) -> Option<String> {
        lock().get(&fingerprint).cloned()
    }

    /// 写入备忘（满 CAP 时整体清空重填，长会话不失去去重能力）。
    pub fn insert(fingerprint: u64, svg: &str) {
        let mut map = lock();
        if map.contains_key(&fingerprint) {
            return;
        }
        if map.len() >= CAP {
            // 备忘是纯优化层：清空不产生任何错误图，后续同内容图标最多重编码一次；
            // 相比「永久拒收新条目」，长会话（>CAP 种唯一图标）仍能持续重建去重覆盖。
            map.clear();
        }
        map.insert(fingerprint, svg.to_string());
    }
}
/// 批量图标编码（分批主线程调度）—— 磁盘分析的统一取图入口。
///
/// - 已在主线程：直接逐路径调用内部实现；
/// - 非主线程：按 `CHUNK` 分批 `DispatchQueue::main().exec_sync`，批间短暂让出——
///   每批结束主线程都回到 runloop 处理 UI 事件，图标**渐进出现**而不是冻结数秒
///   （实测单张绘制+指纹 ~10ms，583 张若一次跑完会冻主线程 ~6s）；
/// - 批内每张图标先查栅格指纹备忘，同内容（如普通文件夹）只在首次真实编码。
///
/// 返回与输入等长的 Vec：None = 路径不存在 / 编码失败（调用方回退 emoji）。
#[cfg(target_os = "macos")]
pub fn file_icons_batch(paths: &[String]) -> Result<Vec<Option<String>>, String> {
    use dispatch2::DispatchQueue;
    use objc2::MainThreadMarker;

    /// 单个主线程批的路径数：越小 UI 越顺滑、调度开销越多；
    /// 8 张 × ~10ms ≈ 80ms 的理论占用，配合批间 1ms 主线程让出。
    const CHUNK: usize = 8;

    if MainThreadMarker::new().is_some() {
        return Ok(paths
            .iter()
            .map(|p| encode_native_icon_inner(p).ok().flatten())
            .collect());
    }

    let mut out: Vec<Option<String>> = Vec::with_capacity(paths.len());
    for chunk in paths.chunks(CHUNK) {
        // 路径所有权移入主线程闭包（借用不能跨越 exec_sync 边界）
        let owned: Vec<String> = chunk.to_vec();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        DispatchQueue::main().exec_sync(move || {
            let res = owned
                .iter()
                .map(|p| encode_native_icon_inner(p).ok().flatten())
                .collect::<Vec<_>>();
            let _ = tx.send(res);
        });
        let res = rx
            .recv()
            .map_err(|_| "file icons batch: main queue result channel closed".to_string())?;
        out.extend(res);
        // 让出窗口：下一次提交前主队列为空，主线程 runloop 得以处理 UI 事件
        if out.len() < paths.len() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    Ok(out)
}

#[cfg(not(target_os = "macos"))]
pub fn file_icons_batch(paths: &[String]) -> Result<Vec<Option<String>>, String> {
    Ok(vec![None; paths.len()])
}

// 回归测试：iconForFile 对 symlink 返回「目标图标 + alias 角标」，路径必须原样传入。
// 断言依据（实测 PNG 字节）：/etc 与 /tmp 同为指向目录的 symlink → 同一张带角标图标；
// /usr 与 /private/etc 为真实目录 → 纯文件夹图标，与带角标版本不同。
//
// 同时守护「内容去重（栅格指纹）」的正确性：带角标与不带角标的图标必须产出不同
// SVG——指纹一旦碰撞（曾因 `bitmapData` 惰性解码读不到像素差异引发），后编码的
// 图标会错误复用先编码者的图（`assert_ne!(etc, usr)` 即失败信号）。
#[cfg(all(test, target_os = "macos"))]
mod probe_tests {
    use super::encode_native_icon_inner;

    #[test]
    fn probe_icon_for_file_keeps_symlink_badge() {
        let etc = encode_native_icon_inner("/etc").unwrap();
        let tmp = encode_native_icon_inner("/tmp").unwrap();
        let usr = encode_native_icon_inner("/usr").unwrap();
        // symlink 指向的目录本身（canonicalize 会退化成这张无角标图）
        let private_etc = encode_native_icon_inner("/private/etc").unwrap();
        let plain_file =
            encode_native_icon_inner("/System/Library/CoreServices/SystemVersion.plist").unwrap();
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
