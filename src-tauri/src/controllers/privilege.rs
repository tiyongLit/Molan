//! 权限能力查询与管理员会话（对齐 `clean.sh` SYSTEM_CLEAN / sudo + 未来 Helper 双路径）。

use serde_json::{Value, json};

use crate::core::sudo;
use crate::platform::privileged_route::{
    PrivilegedHelperInstallRoute, macos_semantic_version, recommended_helper_install_route,
};

#[tauri::command(rename_all = "snake_case")]
pub fn mole_privilege_capabilities() -> Result<Value, String> {
    let sudo_session_active = sudo::is_admin_authorized();
    let route = recommended_helper_install_route();
    let version = macos_semantic_version()
        .map(|(a, b, c)| format!("{a}.{b}.{c}"))
        .unwrap_or_else(|| "unknown".into());

    let route_str = match route {
        Some(PrivilegedHelperInstallRoute::SmaAppService) => "sma_app_service",
        Some(PrivilegedHelperInstallRoute::SmJobBless) => "sm_job_bless",
        None => "unavailable",
    };

    Ok(json!({
        "platform": if cfg!(target_os = "macos") { "macos" } else { "non_macos" },
        "macos_product_version": version,
        "system_clean": {
            "gate_aligned_with_clean_sh": true,
            "sudo_session_active": sudo_session_active,
            "system_sections_enabled_when_active": sudo_session_active,
        },
        "privileged_helper": {
            "recommended_install_route": route_str,
            "dual_path_policy": "macOS 13+: SMAppService; macOS 12 and below: SMJobBless; one shared XPC protocol (future).",
            "installed": false,
            "native_bridge": "pending",
        },
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_request_admin_session(prompt: Option<String>) -> Result<Value, String> {
    let authorized = sudo::is_admin_authorized();
    log::info!("[diagnose] mole_request_admin_session called, is_admin_authorized={authorized}");
    let _ = prompt;
    // 三态返回（对齐 Burrow AuthCancel）：前端可按 status 展示不同文案。
    // authorized 字段保留布尔语义，旧调用方 `if (!res.authorized)` 仍可用。
    let result = sudo::ensure_admin_session_detailed();
    let status = match result {
        sudo::AdminAuthResult::Authorized => "authorized",
        sudo::AdminAuthResult::UserCanceled => "canceled",
        sudo::AdminAuthResult::Failed => "failed",
    };
    log::info!("[diagnose] mole_request_admin_session result={status}");
    Ok(json!({
        "authorized": result == sudo::AdminAuthResult::Authorized,
        "status": status,
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn mole_revoke_admin_session() -> Result<bool, String> {
    Ok(sudo::revoke_admin_session())
}
