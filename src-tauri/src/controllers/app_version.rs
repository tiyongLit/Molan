//! MoleStudio 应用版本检查（App Version）Tauri command 入口。
//!
//! 薄层：只做参数解析、调用 `lib/manage/app_version::*` 业务逻辑、发射事件。
//! 所有核心逻辑见 [`crate::manage::app_version`]。

use tauri::AppHandle;

use crate::manage::app_version;

/// 检查 MoleStudio 自身是否有新版本。
///
/// 官网版：使用 tauri-plugin-updater 查询配置好的 endpoints (Gitee → GitHub)。
/// MAS 版：返回 source="app_store"，前端引导 App Store。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_app_version_check(
    app: AppHandle,
) -> Result<app_version::AppVersionCheckResult, String> {
    app_version::check_for_update(&app).await
}

/// 执行更新下载安装（仅官网版）。
/// 下载进度通过 `app-version::progress` 事件推送。
/// 安装完成后请求重启。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_app_version_install(app: AppHandle) -> Result<(), String> {
    let app_for_progress = app.clone();
    let app_for_complete = app.clone();

    app_version::perform_update(
        &app,
        move |chunk_length, content_length| {
            let progress = match content_length {
                Some(total) if total > 0 => (chunk_length as f64 / total as f64) * 100.0,
                _ => 0.0,
            };
            crate::events::emit_app_version_progress(&app_for_progress, "downloading", progress);
        },
        move || {
            crate::events::emit_app_version_progress(&app_for_complete, "installing", 100.0);
        },
    )
    .await
}

/// MAS 版：引导用户到 App Store 更新页面。
#[tauri::command(rename_all = "snake_case")]
pub fn mole_app_version_open_appstore() -> Result<(), String> {
    app_version::open_appstore()
}
