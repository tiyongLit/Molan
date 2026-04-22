//! Lemon 式 plist → App 关联算法。
//! 移植自 `QMLoginItemManager.getAppInfoWithFileName` + `getLaunchServiceItems`：
//! 将散落的 LaunchAgent/Daemon plist 反向关联回用户可识别的应用程序。

use super::model::{AppAssociation, Service};

/// 已安装 App 索引条目（从 mole_list_apps 结果提取）。
#[derive(Debug, Clone)]
pub struct AppIndexEntry {
    pub bundle_id: String,
    pub app_name: String,
    pub app_path: String,
}

/// 已安装 App 索引。
pub struct AppIndex {
    entries: Vec<AppIndexEntry>,
}

impl AppIndex {
    /// 从 (bundle_id, app_name, app_path) 三元组构建索引。
    /// 跳过 bundle_id 为空的条目。
    pub fn new(items: Vec<(String, String, String)>) -> Self {
        let entries = items
            .into_iter()
            .filter(|(bid, _, _)| !bid.is_empty())
            .map(|(bundle_id, app_name, app_path)| AppIndexEntry {
                bundle_id,
                app_name,
                app_path,
            })
            .collect();
        Self { entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 对一个 Service 尝试关联 App。
/// 返回 None 表示无法关联（归入 standalone_services）。
///
/// 关联优先级（对齐 Lemon）：
/// 1. plist 文件名（去 .plist 扩展名）lowercased contains app.bundle_id lowercased
/// 2. 特殊处理：bundle_id 长度 < 5 时用 starts_with(bundle_id + ".") 严格匹配
/// 3. 回退：config.arguments[0] 或 config.program 包含 ".app/" → 提取宿主 App 路径 → 在 index 中查找
/// 4. 都没匹配 → None
pub fn associate(service: &Service, index: &AppIndex) -> Option<AppAssociation> {
    // 策略 1+2：plist 文件名匹配 bundle_id
    if let Some(plist) = &service.plist_path {
        let file_name = plist
            .rsplit('/')
            .next()
            .unwrap_or("")
            .strip_suffix(".plist")
            .unwrap_or("")
            .to_lowercase();

        if !file_name.is_empty() {
            let mut best_match: Option<&AppIndexEntry> = None;
            let mut best_score: usize = 0;

            for entry in &index.entries {
                let bid_lower = entry.bundle_id.to_lowercase();
                if bid_lower.is_empty() {
                    continue;
                }

                // 短 bundle_id（如 "st"）用严格前缀匹配，避免误关联
                if bid_lower.len() < 5 {
                    let strict_prefix = format!("{bid_lower}.");
                    if file_name.starts_with(&strict_prefix) {
                        return Some(AppAssociation {
                            app_name: entry.app_name.clone(),
                            app_path: entry.app_path.clone(),
                            bundle_id: entry.bundle_id.clone(),
                        });
                    }
                    continue;
                }

                // 策略 A：filename contains bundle_id（标准匹配）
                if file_name.contains(&bid_lower) {
                    return Some(AppAssociation {
                        app_name: entry.app_name.clone(),
                        app_path: entry.app_path.clone(),
                        bundle_id: entry.bundle_id.clone(),
                    });
                }

                // 策略 B：反向匹配——bundle_id contains filename 的主域段
                // 例: bundle_id="net.yanue.V2rayU" 匹配 filename="yanue.v2rayu.v2ray-core"
                let file_segments: Vec<&str> = file_name.split('.').collect();
                let bid_segments: Vec<&str> = bid_lower.split('.').collect();

                // 尝试 filename 包含 bundle_id 的后 N 段（N >= 2）
                if bid_segments.len() >= 2 && file_segments.len() >= 2 {
                    for n in (2..=bid_segments.len().min(file_segments.len())).rev() {
                        let bid_suffix = bid_segments[bid_segments.len() - n..].join(".");
                        if file_name.contains(&bid_suffix) {
                            let score = bid_suffix.len();
                            if score > best_score {
                                best_score = score;
                                best_match = Some(entry);
                            }
                            break;
                        }
                    }
                }

                // 策略 C：反向——filename 的主域段 包含在 bundle_id 中
                // 例: filename="yanue.v2rayu.v2ray-core", bundle_id="net.yanue.V2rayU"
                //      → "yanue.v2rayu" 在 bundle_id 中出现
                if file_segments.len() >= 2 {
                    let file_prefix_2 = file_segments[..2].join(".");
                    if file_prefix_2.len() >= 5 && bid_lower.contains(&file_prefix_2) {
                        let score = file_prefix_2.len();
                        if score > best_score {
                            best_score = score;
                            best_match = Some(entry);
                        }
                    }
                }
            }

            if let Some(entry) = best_match {
                return Some(AppAssociation {
                    app_name: entry.app_name.clone(),
                    app_path: entry.app_path.clone(),
                    bundle_id: entry.bundle_id.clone(),
                });
            }
        }
    }

    // 策略 3：WorkingDirectory 包含 .app/ → 提取宿主 App
    if let Some(wd) = &service.config.working_directory {
        if let Some(app_path) = extract_app_path(wd) {
            for entry in &index.entries {
                if entry.app_path == app_path
                    || app_path.starts_with(&format!("{}/", entry.app_path))
                {
                    return Some(AppAssociation {
                        app_name: entry.app_name.clone(),
                        app_path: entry.app_path.clone(),
                        bundle_id: entry.bundle_id.clone(),
                    });
                }
            }
        }
    }

    // 策略 4：可执行路径包含 .app/ → 提取宿主 App
    let command = service
        .config
        .program
        .as_deref()
        .or_else(|| service.config.arguments.first().map(String::as_str))
        .unwrap_or("");

    if let Some(app_path) = extract_app_path(command) {
        // 在索引中查找匹配的 App（路径前缀匹配）
        for entry in &index.entries {
            if entry.app_path == app_path || app_path.starts_with(&format!("{}/", entry.app_path)) {
                return Some(AppAssociation {
                    app_name: entry.app_name.clone(),
                    app_path: entry.app_path.clone(),
                    bundle_id: entry.bundle_id.clone(),
                });
            }
        }
        // 索引中没找到，但路径明确是 .app → 用路径推断名称
        let app_name = app_path
            .rsplit('/')
            .next()
            .unwrap_or("Unknown")
            .strip_suffix(".app")
            .unwrap_or("Unknown")
            .to_string();
        return Some(AppAssociation {
            app_name,
            app_path,
            bundle_id: String::new(),
        });
    }

    None
}

/// 从可执行路径中提取 .app 路径。
/// 例："/Applications/Docker.app/Contents/MacOS/helper" → "/Applications/Docker.app"
fn extract_app_path(command: &str) -> Option<String> {
    let idx = command.find(".app/")?;
    let app_path = &command[..idx + 4]; // 包含 ".app"
    if app_path.starts_with('/') {
        Some(app_path.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::startup::model::*;

    fn make_index() -> AppIndex {
        AppIndex::new(vec![
            (
                "com.docker.docker".into(),
                "Docker".into(),
                "/Applications/Docker.app".into(),
            ),
            (
                "com.google.Chrome".into(),
                "Google Chrome".into(),
                "/Applications/Google Chrome.app".into(),
            ),
            (
                "st".into(),
                "FinalShell".into(),
                "/Applications/FinalShell.app".into(),
            ),
            (
                "com.adobe.AccrobatPro".into(),
                "Adobe Acrobat".into(),
                "/Applications/Adobe Acrobat Pro.app".into(),
            ),
        ])
    }

    fn make_service(label: &str, plist: Option<&str>, program: Option<&str>) -> Service {
        let mut config = LaunchConfig::empty();
        config.program = program.map(String::from);
        Service {
            id: format!("gui/501:{label}"),
            label: label.to_string(),
            display_name: label.to_string(),
            source: ServiceSource::Launchd,
            scope: ServiceScope::UserAgent,
            domain: "gui/501".to_string(),
            plist_path: plist.map(String::from),
            config,
            pid: None,
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
    fn match_by_plist_filename_contains_bundle_id() {
        let idx = make_index();
        let svc = make_service(
            "com.docker.docker",
            Some("/Users/x/Library/LaunchAgents/com.docker.docker.plist"),
            None,
        );
        let assoc = associate(&svc, &idx).unwrap();
        assert_eq!(assoc.app_name, "Docker");
        assert_eq!(assoc.bundle_id, "com.docker.docker");
    }

    #[test]
    fn short_bundle_id_uses_strict_prefix() {
        let idx = make_index();
        // "st.plist" 不应匹配 bundle_id "st"（需要 "st." 前缀）
        let svc = make_service("st", Some("/Users/x/Library/LaunchAgents/st.plist"), None);
        assert!(associate(&svc, &idx).is_none());

        // "st.helper.plist" 应匹配
        let svc2 = make_service(
            "st.helper",
            Some("/Users/x/Library/LaunchAgents/st.helper.plist"),
            None,
        );
        let assoc = associate(&svc2, &idx).unwrap();
        assert_eq!(assoc.app_name, "FinalShell");
    }

    #[test]
    fn match_by_program_path_app_bundle() {
        let idx = make_index();
        let svc = make_service(
            "com.unknown.helper",
            Some("/Library/LaunchAgents/com.unknown.helper.plist"),
            Some("/Applications/Google Chrome.app/Contents/Frameworks/helper"),
        );
        let assoc = associate(&svc, &idx).unwrap();
        assert_eq!(assoc.app_name, "Google Chrome");
    }

    #[test]
    fn no_match_returns_none() {
        let idx = make_index();
        let svc = make_service(
            "org.random.thing",
            Some("/Users/x/Library/LaunchAgents/org.random.thing.plist"),
            Some("/usr/local/bin/random"),
        );
        assert!(associate(&svc, &idx).is_none());
    }

    #[test]
    fn v2rayu_reverse_segment_match() {
        // V2rayU: bundle_id="net.yanue.V2rayU" but plist="yanue.v2rayu.v2ray-core.plist"
        // 文件名缺 "net." 前缀，需要用反向匹配（策略 C）
        let idx = AppIndex::new(vec![(
            "net.yanue.V2rayU".into(),
            "V2rayU".into(),
            "/Applications/V2rayU.app".into(),
        )]);
        let svc = make_service(
            "yanue.v2rayu.v2ray-core",
            Some("/Users/x/Library/LaunchAgents/yanue.v2rayu.v2ray-core.plist"),
            None,
        );
        let assoc = associate(&svc, &idx).unwrap();
        assert_eq!(assoc.app_name, "V2rayU");
        assert_eq!(assoc.bundle_id, "net.yanue.V2rayU");
    }

    #[test]
    fn virtualbox_suffix_segment_match() {
        // VirtualBox: bundle_id="org.virtualbox.app" but plist="org.virtualbox.startup.plist"
        let idx = AppIndex::new(vec![(
            "org.virtualbox.app".into(),
            "VirtualBox".into(),
            "/Applications/VirtualBox.app".into(),
        )]);
        let svc = make_service(
            "org.virtualbox.startup",
            Some("/Library/LaunchDaemons/org.virtualbox.startup.plist"),
            None,
        );
        let assoc = associate(&svc, &idx).unwrap();
        assert_eq!(assoc.app_name, "VirtualBox");
    }

    #[test]
    fn working_directory_association() {
        // WorkingDirectory 指向 .app 内部
        let idx = make_index();
        let mut svc = make_service(
            "com.unknown.helper",
            Some("/Users/x/Library/LaunchAgents/com.unknown.helper.plist"),
            None,
        );
        svc.config.working_directory = Some("/Applications/Docker.app/Contents/MacOS".into());
        let assoc = associate(&svc, &idx).unwrap();
        assert_eq!(assoc.app_name, "Docker");
    }

    #[test]
    fn extract_app_path_from_nested() {
        assert_eq!(
            extract_app_path("/Applications/Docker.app/Contents/MacOS/helper"),
            Some("/Applications/Docker.app".to_string())
        );
        assert_eq!(extract_app_path("/usr/local/bin/mysql"), None);
        assert_eq!(extract_app_path("relative/App.app/bin"), None);
    }
}
