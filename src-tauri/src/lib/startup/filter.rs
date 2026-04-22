//! 启动项过滤规则：决定哪些服务默认对用户可见。
//! 对齐 Launchdeck `is_apple_service` + Lemon `needFilterFile` 的共识：
//! 隐藏 Apple 系统服务、XPC 运行时进程、MoleStudio 自身。

use super::model::Service;

/// 默认是否对用户可见。
/// `show_system` 为 true 时 bypass 规则 1-2（Apple/系统），用于"显示系统服务"开关。
pub fn is_user_visible(service: &Service, show_system: bool) -> bool {
    // 规则 1：com.apple.* → 隐藏（除非 show_system）
    if !show_system && service.label.starts_with("com.apple.") {
        return false;
    }

    // 规则 2：plist 在 /System/Library/ 下 → 隐藏（除非 show_system）
    if !show_system {
        if let Some(path) = &service.plist_path {
            if path.starts_with("/System/Library") {
                return false;
            }
        }
    }

    // 规则 3：XPC / UIKit / application 运行时进程 → 永远隐藏
    if service.label.starts_with("com.apple.xpc.")
        || service.label.starts_with("UIKitApplication:")
        || service.label.starts_with("application.")
    {
        return false;
    }

    // 规则 4：MoleStudio 自身 → 永远隐藏
    if service.label.contains("molestudio") || service.label.starts_with("com.mole.") {
        return false;
    }

    // 规则 5：无 plist 且无 PID 的幽灵条目 → 隐藏
    if service.plist_path.is_none() && service.pid.is_none() {
        return false;
    }

    true
}

/// 是否有健康问题（用于"问题"筛选）。
pub fn has_problems(service: &Service) -> bool {
    !service.health.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::startup::model::*;

    fn make_service(label: &str, plist: Option<&str>, pid: Option<u32>) -> Service {
        Service {
            id: format!("gui/501:{label}"),
            label: label.to_string(),
            display_name: label.to_string(),
            source: ServiceSource::Launchd,
            scope: ServiceScope::UserAgent,
            domain: "gui/501".to_string(),
            plist_path: plist.map(String::from),
            config: LaunchConfig::empty(),
            pid,
            exit_code: None,
            status: ServiceStatus::Unknown,
            enabled: None,
            loaded: None,
            brew_formula: None,
            brew_status: None,
            safety_level: SafetyLevel::UserWritable,
            elevation: ElevationNeeds::none(),
            origin: Origin::unknown(),
            app_info: None,
            health: Vec::new(),
        }
    }

    #[test]
    fn apple_services_hidden_by_default() {
        let svc = make_service(
            "com.apple.Spotlight",
            Some("/System/Library/LaunchAgents/com.apple.Spotlight.plist"),
            None,
        );
        assert!(!is_user_visible(&svc, false));
        assert!(is_user_visible(&svc, true));
    }

    #[test]
    fn xpc_always_hidden() {
        let svc = make_service("com.apple.xpc.launchd", None, Some(1));
        assert!(!is_user_visible(&svc, true));
    }

    #[test]
    fn molestudio_self_hidden() {
        let svc = make_service(
            "com.molestudio.helper",
            Some("/Users/x/Library/LaunchAgents/com.molestudio.helper.plist"),
            None,
        );
        assert!(!is_user_visible(&svc, false));
    }

    #[test]
    fn third_party_visible() {
        let svc = make_service(
            "com.docker.docker",
            Some("/Users/x/Library/LaunchAgents/com.docker.docker.plist"),
            Some(123),
        );
        assert!(is_user_visible(&svc, false));
    }

    #[test]
    fn ghost_entry_hidden() {
        let svc = make_service("com.ghost.service", None, None);
        assert!(!is_user_visible(&svc, false));
    }

    #[test]
    fn runtime_only_with_pid_visible() {
        let svc = make_service("com.vendor.agent", None, Some(456));
        assert!(is_user_visible(&svc, false));
    }
}
