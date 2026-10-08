//! 原生 NSAlert 确认/提示通道 —— 命令层（实现见 `lib/platform/macos_dialog`）。
//!
//! 前端契约：
//! - `invoke("mole_dialog", { args })` → `Result<bool, String>`；
//!   `true` = 主按钮（确认）；`false` = 取消 / 关闭 / Esc。
//! - `invoke("mole_dialog_choice", { args })` → `Result<DialogChoice, String>`；
//!   三按钮模式（"ok" / "alt" / "cancel"），用于「仅本次 / 全局生效」类三选场景。
//! `cancelLabel: null` = 单按钮提示模式。

use serde::Deserialize;

/// 弹窗语义（对齐 NSAlertStyle）。
#[derive(Clone, Copy, Debug, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DialogKind {
    #[default]
    Info,
    Warning,
    Critical,
}

/// 三按钮模式的用户选择（serde 小写透传前端）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DialogChoice {
    Ok,
    Alt,
    Cancel,
}

/// 调用参数。
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogArgs {
    /// 主文案（自动折行、粗体居中）。
    pub message: String,
    /// 副文案（常规字重居中，置于主文案下方）；缺省不显示。
    #[serde(default)]
    pub informative_text: Option<String>,
    /// 主按钮文案；缺省「确认」。
    #[serde(default = "default_ok_label")]
    pub ok_label: String,
    /// 取消按钮文案；缺省「取消」；显式传 `null` = 单按钮提示模式。
    #[serde(default = "default_cancel_label")]
    pub cancel_label: Option<String>,
    /// 备选按钮文案（三按钮模式）；缺省无双按钮形态。
    #[serde(default)]
    pub alt_label: Option<String>,
    /// 弹窗语义类型。
    #[serde(default)]
    pub kind: DialogKind,
}

fn default_ok_label() -> String {
    "确认".to_string()
}

fn default_cancel_label() -> Option<String> {
    Some("取消".to_string())
}

/// 组装平台层 DialogRequest（两个命令共用）。
fn build_request(args: DialogArgs) -> crate::platform::macos_dialog::DialogRequest {
    use crate::platform::macos_dialog::DialogKind as ImplKind;

    crate::platform::macos_dialog::DialogRequest {
        message: args.message,
        informative_text: args.informative_text,
        ok_label: args.ok_label,
        cancel_label: args.cancel_label,
        alt_label: args.alt_label,
        kind: match args.kind {
            DialogKind::Info => ImplKind::Info,
            DialogKind::Warning => ImplKind::Warning,
            DialogKind::Critical => ImplKind::Critical,
        },
    }
}

/// 呈现原生确认/提示弹窗：sheet 挂调用方窗口（不可见回退 app-modal），
/// 垂直三行流（app 图标 / 文案 / 原生按钮），详见 `lib/platform/macos_dialog`。
#[tauri::command]
pub async fn mole_dialog(window: tauri::WebviewWindow, args: DialogArgs) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        use crate::platform::macos_dialog::{self, DialogResponse, ParentWindow};

        // 调用方窗口作为 sheet 宿主（指针由 Tauri 持有，命令执行期内有效）。
        let parent = ParentWindow::new(window.ns_window().ok());
        let response = macos_dialog::present(build_request(args), parent).await?;
        Ok(matches!(response, DialogResponse::Primary))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, args);
        Err("原生对话框仅在 macOS 可用".to_string())
    }
}

/// 三按钮形态的原生弹窗：主按钮 → "ok"，备选按钮 → "alt"，取消/关闭/Esc → "cancel"。
#[tauri::command]
pub async fn mole_dialog_choice(
    window: tauri::WebviewWindow,
    args: DialogArgs,
) -> Result<DialogChoice, String> {
    #[cfg(target_os = "macos")]
    {
        use crate::platform::macos_dialog::{self, DialogResponse, ParentWindow};

        let parent = ParentWindow::new(window.ns_window().ok());
        let response = macos_dialog::present(build_request(args), parent).await?;
        Ok(match response {
            DialogResponse::Primary => DialogChoice::Ok,
            DialogResponse::Alt => DialogChoice::Alt,
            DialogResponse::Cancel => DialogChoice::Cancel,
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, args);
        Err("原生对话框仅在 macOS 可用".to_string())
    }
}
