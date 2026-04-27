//! 自定义原生确认弹窗（替代 `tauri-plugin-dialog` 的 `ask()`）。
//!
//! 设计要点：
//! - 复用 `tauri::WebviewWindowBuilder` 开一个独立、透明、无边框、置顶、跳过任务栏的模态窗口，
//!   视觉由前端 React 自绘（毛玻璃 + 品牌色按钮 + 圆角），与 Lemon 自定义 NSWindow 同形态。
//! - 调用：`mole_system_confirm(args) -> bool`（async）；前端窗口读取 URL query 渲染，
//!   用户点击后 invoke `mole_system_confirm_reply({id, confirmed})` 回传，本端 resolve。
//! - 同一时刻只允许一个确认窗口：复用同名窗口时通过 `eval` 注入新参数并 show。
//! - 关闭窗口（Cmd+W / 红绿灯 / Esc）= 取消；窗口隐藏后保持已创建，下次复用，减少闪烁。
//! - 60s 超时自动判 false，避免前端崩溃导致永久挂起。
//!
//! 不引入新依赖：pending map 用 `std::sync::LazyLock`（Rust 1.80+ 稳定）。

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::{Mutex, oneshot};

/// 确认窗口的 label，复用同名窗口。
const WIN_LABEL: &str = "system-confirm";

/// 前端 query 字段名约定（与前端页面读取保持一致）。
const Q_ID: &str = "id";
const Q_TITLE: &str = "title";
const Q_MESSAGE: &str = "message";
const Q_OK_LABEL: &str = "okLabel";
const Q_CANCEL_LABEL: &str = "cancelLabel";
const Q_KIND: &str = "kind";

/// `kind` 取值（对齐 NSAlertStyle：informational / warning / critical）。
/// 前端按此值切换图标与主按钮色。
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfirmKind {
    Info,
    Warning,
    Critical,
}

impl ConfirmKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

impl Default for ConfirmKind {
    fn default() -> Self {
        Self::Info
    }
}

/// 调用参数：与 `@tauri-apps/plugin-dialog` 的 `ask()` 接近，便于前端平滑替换。
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemConfirmArgs {
    /// 调用方传入的唯一 id；若不传则自动生成（`sc-<unix_ms>-<n>`）。
    #[serde(default)]
    pub id: Option<String>,
    pub message: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub kind: Option<ConfirmKind>,
    #[serde(default)]
    pub ok_label: Option<String>,
    #[serde(default)]
    pub cancel_label: Option<String>,
}

/// 回传命令的参数。
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemConfirmReply {
    pub id: String,
    pub confirmed: bool,
}

/// pending oneshot 表：id → sender。用 tokio::Mutex 保护（async 上下文）。
type PendingMap = Mutex<HashMap<String, oneshot::Sender<bool>>>;

static PENDING: LazyLock<PendingMap> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// 把参数编码成前端 URL query（前端用 URLSearchParams 读取）。
fn build_query(args: &SystemConfirmArgs, id: &str) -> String {
    let mut q: Vec<(String, String)> = vec![
        (Q_ID.to_string(), id.to_string()),
        (Q_MESSAGE.to_string(), args.message.clone()),
        (Q_KIND.to_string(), args.kind.unwrap_or_default().as_str().to_string()),
    ];
    if let Some(t) = &args.title {
        q.push((Q_TITLE.to_string(), t.clone()));
    }
    if let Some(l) = &args.ok_label {
        q.push((Q_OK_LABEL.to_string(), l.clone()));
    }
    if let Some(l) = &args.cancel_label {
        q.push((Q_CANCEL_LABEL.to_string(), l.clone()));
    }
    q.iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// 极简 form-urlencoded 编码（仅处理必要字符，避免引入 `urlencoding` crate）。
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b' ' => out.push_str("%20"),
            b'&' | b'=' | b'#' | b'?' | b'+' | b'%' => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
            // 其余字节直接放行（含 UTF-8 多字节序列），前端 decodeURIComponent 接收
            _ => out.push(b as char),
        }
    }
    out
}

/// 生成唯一 id（不依赖 rand crate）。
fn gen_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("sc-{ms}-{n}")
}

/// 打开（或复用）自定义确认窗口，await 返回用户是否确认。
///
/// 调用：`let ok = mole_system_confirm(app, args).await?;`
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_system_confirm(app: AppHandle, args: SystemConfirmArgs) -> Result<bool, String> {
    let id = args.id.clone().unwrap_or_else(gen_id);
    log::info!(
        "[mole_system_confirm] id={id} title={:?} kind={:?} message_len={}",
        args.title,
        args.kind,
        args.message.len()
    );

    // ── 1. 注册 pending oneshot ──
    let (tx, rx) = oneshot::channel::<bool>();
    {
        let mut guard = PENDING.lock().await;
        (*guard).insert(id.clone(), tx);
    }

    // ── 2. 打开/复用窗口 ──
    if let Err(e) = show_or_create_window(&app, &args, &id) {
        // 失败时清理 pending，避免泄漏
        PENDING.lock().await.remove(&id);
        return Err(e);
    }

    // ── 3. await 结果（带 60s 超时） ──
    let result = match tokio::time::timeout(Duration::from_secs(60), rx).await {
        Ok(Ok(confirmed)) => Ok(confirmed),
        Ok(Err(_)) => Ok(false), // sender 被 drop（窗口关闭等），视为取消
        Err(_) => {
            log::warn!("[mole_system_confirm] timeout id={id}");
            let _ = app.get_webview_window(WIN_LABEL).map(|w| w.hide());
            Ok(false)
        }
    };

    // ── 4. 清理 pending（防漏） ──
    PENDING.lock().await.remove(&id);

    result
}

/// 前端窗口在用户点击按钮后调用此命令回传选择。
#[tauri::command(rename_all = "snake_case")]
pub async fn mole_system_confirm_reply(reply: SystemConfirmReply) -> Result<(), String> {
    log::info!(
        "[mole_system_confirm_reply] id={} confirmed={}",
        reply.id,
        reply.confirmed
    );
    let mut guard = PENDING.lock().await;
    if let Some(tx) = (*guard).remove(&reply.id) {
        let _ = tx.send(reply.confirmed);
    }
    Ok(())
}

/// 复用或创建确认窗口。
fn show_or_create_window(app: &AppHandle, args: &SystemConfirmArgs, id: &str) -> Result<(), String> {
    // 已存在则通过 eval 注入新参数并显示
    if let Some(window) = app.get_webview_window(WIN_LABEL) {
        let url = format!("/system-confirm?{}", build_query(args, id));
        // 触发前端 reload 到新 URL（最稳，避免 SPA 路由切换残留旧状态）
        let js = format!(
            r#"(function() {{ window.location.replace("{}"); }})();"#,
            url.replace('\\', "\\\\").replace('"', "\\\"")
        );
        window.eval(&js).map_err(|e| e.to_string())?;
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    // 否则新建模态窗口
    let url = format!("/system-confirm?{}", build_query(args, id));
    let window = WebviewWindowBuilder::new(app, WIN_LABEL, WebviewUrl::App(url.into()))
        .title("")
        .inner_size(420.0, 200.0)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;

    // 关闭按钮事件 = 取消：仅隐藏不销毁，下次复用
    let w = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = w.hide();
        }
    });

    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}
