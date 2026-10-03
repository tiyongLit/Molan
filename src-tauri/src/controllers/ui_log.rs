//! 前端 UI 时序埋点转发。
//!
//! WebView 侧的 `uiTrace()`（`src/utils/uiTrace.ts`）把关键时间轴——页面挂载、
//! 任务快照迁移（authorizing → scanning → idle）、扫描事件计数、渲染帧率、
//! 路由提交耗时——通过 `mole_ui_log` 转发到后端日志（molan.log 的 `[ui:*]` 行），
//! 与 `[clean-job]` / `[section]` 等后端日志落在同一份文件、同一时间轴，
//! 用于定位「清理界面卡顿 / 路由点击不响应」这类跨前后端问题。
//!
//! 纯信息记录：无业务副作用，写日志失败不影响前端流程（前端调用侧 catch 静默）。

/// 前端 UI 埋点日志：tag 形如 `clean.tick`，message 为纯文本（含相对时间戳）。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_ui_log(tag: String, message: String) {
    log::debug!("[ui:{tag}] {message}");
}
