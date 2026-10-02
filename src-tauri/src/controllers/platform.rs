use crate::platform::native_icon_registry::{self, NativeIconsResponse};
use crate::vendor::platform_info;

// ── commands ──

/// 获取平台安装形态信息（分发渠道、运行形态、沙盒状态等）。
#[tauri::command(rename_all = "snake_case")]
pub fn mole_get_platform_info() -> Result<platform_info::PlatformInfo, String> {
    Ok(platform_info::current().clone())
}

// ── NativeIconRegistry：内容寻址原生图标解析（替代 mole_get_icon_cached / mole_get_icons_batch）──

/// 批量解析原生图标：对每条路径做 mtime 校验，命中复用 content_id，未命中批量编码
/// （分批主线程调度 + 栅格指纹内容去重）并写入 contentStore。
///
/// 响应形状：
///   - entries: path → content_id（前端 pathIndex 层，约 100B/条）
///   - contents: content_id → SVG data URI（**覆盖 entries 全部引用**，批内去重；
///     跨会话重启时前端 contentStore 为空，必须随 entries 回传才能渲染）
///
/// 与已下线旧命令 `mole_get_icons_batch` 的核心区别：
///   - 返回的 contents 按**内容**去重（927 路径 → 108 种不同图标 → 108 条）；
///   - SVG 信封 viewBox = 128px，CSS 可任意缩放且像素对齐；
///   - 编码按栅格指纹（TIFF 字节）内容去重（583 个文件夹路径 → 1 次真实编码）。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_native_icons_resolve(
    paths: Vec<String>,
    app_handle: tauri::AppHandle,
) -> Result<NativeIconsResponse, String> {
    Ok(tauri::async_runtime::spawn_blocking(move || {
        native_icon_registry::resolve(&app_handle, &paths)
    })
    .await
    .map_err(|e| format!("native_icons_resolve task panicked: {e}"))?)
}

// ── Quick Look ──

/// macOS Quick Look 预览指定路径的文件/目录（自 `lib.rs` 迁入，命令契约不变）。
#[tauri::command]
pub fn mole_quick_look(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("qlmanage")
            .args(["-p", &path])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("启动快速查看失败: {}", e))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err("快速查看仅支持 macOS".to_string())
    }
}
