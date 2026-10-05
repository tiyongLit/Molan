//! 操作引擎：规划 + 执行启动项的 start/stop/restart/enable/disable/delete。
//! 对齐 Launchdeck `actions.rs`：安全守卫 + 权限感知 + Brew 路由 + 命令预览。
//! 删除操作走废纸篓（对齐红线 4：不直接 rm）。

use std::collections::HashSet;

use crate::core::timeout::{run_with_timeout, run_with_timeout_capture_lossy};

use super::classic_login_items;
use super::model::*;

// ── 操作类型 ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Start,
    Stop,
    Restart,
    Enable,
    Disable,
    Delete,
}

impl ActionKind {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "start" => Some(Self::Start),
            "stop" => Some(Self::Stop),
            "restart" => Some(Self::Restart),
            "enable" => Some(Self::Enable),
            "disable" => Some(Self::Disable),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Enable => "enable",
            Self::Disable => "disable",
            Self::Delete => "delete",
        }
    }
}

// ── 操作计划 ──

#[derive(Debug, Clone)]
pub struct ActionPlan {
    pub kind: ActionKind,
    pub service_id: String,
    pub command: Vec<String>,
    /// 主命令成功后的第二步命令（对齐 Lemon 的组合语义：enable 配套 load、
    /// disable 配套 unload，让「启用/禁用」当场生效）。独立执行而非拼进 shell；
    /// 失败容忍——服务未加载时 unload、已加载时 load 都会报错，
    /// 但那正说明目标状态已达成，不作为整体失败。
    pub secondary: Option<Vec<String>>,
    pub warning: String,
    pub blocked_reason: Option<String>,
    pub needs_sudo: bool,
}

impl ActionPlan {
    pub fn is_blocked(&self) -> bool {
        self.blocked_reason.is_some()
    }

    pub fn command_display(&self) -> String {
        if self.command.is_empty() {
            return "-".to_string();
        }
        let mut parts = Vec::new();
        if self.needs_sudo {
            parts.push("sudo".to_string());
            parts.push("--".to_string());
        }
        parts.extend(self.command.iter().cloned());
        parts.join(" ")
    }
}

// ── 规划（不执行）──

pub fn plan(service: &Service, kind: ActionKind) -> ActionPlan {
    // 安全守卫：只读系统服务 → 阻止
    if matches!(service.safety_level, SafetyLevel::ReadonlySystem) {
        return blocked(
            service,
            kind,
            "系统服务（/System/Library）仅可查看，不可操作",
        );
    }

    // 安全守卫：受保护的 vendor/runtime 服务 → 阻止
    if matches!(service.safety_level, SafetyLevel::ProtectedVendor) {
        return blocked(service, kind, "此服务受保护，无法通过 launchctl 操作");
    }

    // 经典登录项（LSSharedFileList）→ 只支持删除操作
    if service
        .plist_path
        .as_deref()
        .is_some_and(|p| p.starts_with("classic:"))
    {
        return plan_classic_login_item(service, kind);
    }

    // Homebrew 服务 → 路由到 brew services
    if matches!(
        service.source,
        ServiceSource::Homebrew | ServiceSource::Both
    ) {
        return plan_brew(service, kind);
    }

    plan_launchd(service, kind)
}

fn blocked(service: &Service, kind: ActionKind, reason: &str) -> ActionPlan {
    ActionPlan {
        kind,
        service_id: service.id.clone(),
        command: Vec::new(),
        secondary: None,
        warning: reason.to_string(),
        blocked_reason: Some(reason.to_string()),
        needs_sudo: false,
    }
}

fn plan_brew(service: &Service, kind: ActionKind) -> ActionPlan {
    let Some(formula) = &service.brew_formula else {
        return blocked(service, kind, "Homebrew 服务缺少 formula 名称");
    };

    let subcommand = match kind {
        ActionKind::Start => "start",
        ActionKind::Stop => "stop",
        ActionKind::Restart => "restart",
        ActionKind::Enable | ActionKind::Disable => {
            return blocked(
                service,
                kind,
                "Homebrew 服务无独立 enable/disable，请使用 start/stop",
            );
        }
        ActionKind::Delete => {
            return blocked(
                service,
                kind,
                "Homebrew 服务请通过 `brew services stop` + `brew uninstall` 管理",
            );
        }
    };

    ActionPlan {
        kind,
        service_id: service.id.clone(),
        command: vec![
            "brew".into(),
            "services".into(),
            subcommand.into(),
            formula.clone(),
        ],
        secondary: None,
        warning: format!("Homebrew 将更新 {formula} 的服务注册"),
        blocked_reason: None,
        needs_sudo: false,
    }
}

/// 经典登录项（LSSharedFileList）只支持删除操作，其他操作引导到系统设置。
fn plan_classic_login_item(service: &Service, kind: ActionKind) -> ActionPlan {
    match kind {
        ActionKind::Delete => {
            // 从 id 中提取索引和名称：classic:{idx}:{display_name}
            let parts: Vec<&str> = service.id.splitn(3, ':').collect();
            if parts.len() < 3 {
                return blocked(service, kind, "经典登录项 ID 格式错误");
            }
            let display_name = parts[2];
            ActionPlan {
                kind,
                service_id: service.id.clone(),
                command: vec!["classic_delete".into(), display_name.to_string()],
                secondary: None,
                warning: format!("将从登录项中删除「{display_name}」"),
                blocked_reason: None,
                needs_sudo: false,
            }
        }
        _ => blocked(
            service,
            kind,
            "经典登录项仅支持删除操作，其他管理请在「系统设置 → 通用 → 登录项」中操作",
        ),
    }
}

fn plan_launchd(service: &Service, kind: ActionKind) -> ActionPlan {
    let target = format!("{}/{}", service.domain, service.label);
    let needs_sudo = match kind {
        ActionKind::Start
        | ActionKind::Stop
        | ActionKind::Restart
        | ActionKind::Enable
        | ActionKind::Disable => service.elevation.runtime,
        ActionKind::Delete => service.elevation.plist_remove || service.elevation.runtime,
    };

    // 第二步命令（对齐 Lemon 的组合语义，仅 enable/disable 使用）
    let mut secondary: Option<Vec<String>> = None;

    let command = match kind {
        ActionKind::Start => {
            if service.loaded == Some(false) {
                let Some(path) = &service.plist_path else {
                    return blocked(service, kind, "未加载的服务无 plist 可 bootstrap");
                };
                vec![
                    "/bin/launchctl".into(),
                    "bootstrap".into(),
                    service.domain.clone(),
                    path.clone(),
                ]
            } else {
                vec!["/bin/launchctl".into(), "kickstart".into(), target]
            }
        }
        ActionKind::Stop => {
            vec!["/bin/launchctl".into(), "bootout".into(), target]
        }
        ActionKind::Restart => {
            vec!["/bin/launchctl".into(), "kickstart".into(), "-k".into(), target]
        }
        ActionKind::Enable => {
            // 对齐 Lemon：`launchctl enable <target> && launchctl load <plist>`
            // ——清掉禁用印记后立即加载，让「启用」当场生效。
            if let Some(path) = &service.plist_path {
                secondary = Some(vec!["/bin/launchctl".into(), "load".into(), path.clone()]);
            }
            vec!["/bin/launchctl".into(), "enable".into(), target]
        }
        ActionKind::Disable => {
            // 对齐 Lemon：`launchctl disable <target> && launchctl unload <plist>`
            // ——写入禁用印记后立即卸载停止，服务不能"禁用了还在跑"。
            if let Some(path) = &service.plist_path {
                secondary = Some(vec!["/bin/launchctl".into(), "unload".into(), path.clone()]);
            }
            vec!["/bin/launchctl".into(), "disable".into(), target]
        }
        ActionKind::Delete => {
            let Some(path) = &service.plist_path else {
                return blocked(service, kind, "无 plist 文件可删除");
            };
            if path.starts_with("btm:") {
                return blocked(service, kind, "BTM 登录项请在系统设置中管理");
            }
            // 先 bootout 再移入废纸篓（红线 4：不直接 rm）
            vec![
                "/bin/launchctl".into(),
                "bootout".into(),
                target,
                "&&".into(),
                "trash".into(),
                path.clone(),
            ]
        }
    };

    let warning = match kind {
        ActionKind::Start => "将请求 launchd 启动此服务".to_string(),
        ActionKind::Stop => "将从 launchd 域中卸载此服务（下次开机不再自动启动）".to_string(),
        ActionKind::Restart => "将终止并立即重启此服务".to_string(),
        ActionKind::Enable => "将在 launchd 域中启用此服务并立即加载".to_string(),
        ActionKind::Disable => "将写入禁用印记并立即卸载停止此服务（重启后保持禁用）".to_string(),
        ActionKind::Delete => "将卸载服务并把 plist 移入废纸篓".to_string(),
    };

    ActionPlan {
        kind,
        service_id: service.id.clone(),
        command,
        secondary,
        warning,
        blocked_reason: None,
        needs_sudo,
    }
}

// ── 执行 ──

pub fn execute(plan: &ActionPlan) -> ActionResult {
    if let Some(reason) = &plan.blocked_reason {
        return ActionResult {
            success: false,
            message: reason.clone(),
        };
    }

    // 经典登录项删除：直接调用 LSSharedFileListItemRemove
    if plan.command.first().is_some_and(|c| c == "classic_delete") {
        return execute_classic_delete(plan);
    }

    // Delete 操作特殊处理：bootout + trash（不走 shell 拼接）
    if plan.kind == ActionKind::Delete {
        return execute_delete(plan);
    }

    let Some(program) = plan.command.first() else {
        return ActionResult {
            success: false,
            message: "操作无命令".to_string(),
        };
    };

    let args: Vec<&str> = plan.command[1..].iter().map(String::as_str).collect();
    let timeout = 15.0;

    let success_message = if plan.needs_sudo {
        // 需要 root：把完整 argv（绝对路径程序名 + 参数）交给 sudo_output。
        // sudo_output 接受完整参数列表，exec_root 会按 TRUSTED_ROOT_BINARIES
        // 校验 args[0]——program 必须是白名单内的绝对路径（/bin/launchctl）；
        // 禁止再拼接参数字符串，否则 launchctl 会收到单一错误参数。
        let argv: Vec<&str> = plan.command.iter().map(String::as_str).collect();
        let output = crate::core::sudo::sudo_output(&argv);
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return ActionResult {
                success: false,
                message: format!("操作失败: {}", err.trim()),
            };
        }
        "操作成功（root）".to_string()
    } else {
        let rc = run_with_timeout(timeout, program, &args);
        if rc != 0 {
            return ActionResult {
                success: false,
                message: format!("launchctl 退出码: {rc}"),
            };
        }
        "操作成功".to_string()
    };

    // 第二步（对齐 Lemon 的组合语义）：enable 后立即 load、disable 后立即 unload，
    // 让「启用/禁用」当场生效——否则只写印记，服务照旧运行（用户观感「没反应」）。
    // 失败容忍：服务本就未加载时 unload、已加载时 load 都会报错，
    // 但那正说明目标状态已达成；主命令（印记）的成败已在上方判定。
    if let Some(secondary) = &plan.secondary {
        if secondary.len() > 1 {
            let sec_program = secondary[0].as_str();
            let sec_args: Vec<&str> = secondary[1..].iter().map(String::as_str).collect();
            if plan.needs_sudo {
                let sec_argv: Vec<&str> = secondary.iter().map(String::as_str).collect();
                let _ = crate::core::sudo::sudo_output(&sec_argv);
            } else {
                let _ = run_with_timeout(timeout, sec_program, &sec_args);
            }
        }
    }

    ActionResult {
        success: true,
        message: success_message,
    }
}

fn execute_delete(plan: &ActionPlan) -> ActionResult {
    // 从 command 中提取 target 和 plist path
    // command 格式: ["/bin/launchctl", "bootout", "<target>", "&&", "trash", "<path>"]
    if plan.command.len() < 6 {
        return ActionResult {
            success: false,
            message: "删除命令格式错误".to_string(),
        };
    }
    let target = &plan.command[2];
    let plist_path = &plan.command[5];

    // 步骤 1：bootout（可能失败，服务未加载时正常）
    if plan.needs_sudo {
        let _ = crate::core::sudo::sudo_output(&["/bin/launchctl", "bootout", target]);
    } else {
        let _ = run_with_timeout(10.0, "/bin/launchctl", &["bootout", target]);
    }

    // 步骤 2：移入废纸篓（红线 4：不直接 rm）
    let trash_result = trash::delete(plist_path);
    match trash_result {
        Ok(_) => ActionResult {
            success: true,
            message: "已卸载并移入废纸篓".to_string(),
        },
        Err(e) => ActionResult {
            success: false,
            message: format!("已卸载但移入废纸篓失败: {e}"),
        },
    }
}

/// 经典登录项删除：调用 LSSharedFileListItemRemove。
/// command 格式: ["classic_delete", "<display_name>"]
fn execute_classic_delete(plan: &ActionPlan) -> ActionResult {
    if plan.command.len() < 2 {
        return ActionResult {
            success: false,
            message: "经典登录项删除命令格式错误".to_string(),
        };
    }
    let display_name = &plan.command[1];
    let removed = classic_login_items::remove_classic_login_item(display_name);
    if removed > 0 {
        ActionResult {
            success: true,
            message: format!("已删除 {removed} 个「{display_name}」登录项"),
        }
    } else {
        ActionResult {
            success: false,
            message: format!("未找到名为「{display_name}」的登录项"),
        }
    }
}

// ── 执行后验证（re-read disabled 状态）──

/// 执行 enable/disable 后重新读取禁用集合，确认实际生效状态。
/// 对齐现有 control.rs 的验证模式：launchctl 退出码不可靠。
pub fn verify_disabled_state(uid: u32) -> HashSet<String> {
    let domain = format!("gui/{uid}");
    let out = run_with_timeout_capture_lossy(8.0, "/bin/launchctl", &["print-disabled", &domain]);
    let mut disabled = HashSet::new();
    if let Some(text) = out {
        for line in text.lines() {
            let trimmed = line.trim();
            let Some((label_part, state)) = trimmed.split_once("=>") else {
                continue;
            };
            let state = state.trim();
            if !matches!(state, "true" | "disabled") {
                continue;
            }
            let label = label_part.trim().trim_matches('"');
            disabled.insert(label.to_string());
        }
    }
    disabled
}
