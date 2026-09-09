//! 多源发现引擎：plist 目录扫描 + launchctl 运行时状态 + Homebrew 集成 + BTM 合并 + App 关联。
//! 对齐 Launchdeck `discovery.rs` 的 `load_inventory` 流程。
//! 所有外部命令通过 `core::timeout` 加超时保护，整个函数应在 spawn_blocking 中调用。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::core::base::home_dir;
use crate::core::timeout::run_with_timeout_capture_lossy;

use super::app_assoc::{self, AppIndex};
use super::classic_login_items;
use super::filter;
use super::login_items;
use super::model::*;

// ── 公共入口 ──

/// 完整发现流程。`include_login_items` 控制是否跑 sfltool（需授权）；
/// `show_system` 控制是否显示 Apple 系统服务；`app_index` 用于 App 关联。
pub fn load_inventory(
    include_login_items: bool,
    show_system: bool,
    app_index: &AppIndex,
    btm_dump: Option<&str>,
) -> StartupInventory {
    let uid = current_uid();
    let mut warnings = Vec::new();
    let mut services: BTreeMap<String, Service> = BTreeMap::new();

    // 步骤 1：plist 目录扫描（5 个目录）
    for (dir, scope) in service_dirs() {
        if let Err(err) = read_plist_dir(&dir, scope, uid, &mut services) {
            warnings.push(format!("{}: {err}", dir.display()));
        }
    }

    // 步骤 2：运行时状态（launchctl print）
    let runtime = read_runtime(uid, &mut warnings);
    apply_runtime(&mut services, runtime, uid);

    // 步骤 3：禁用状态（launchctl print-disabled）
    let disabled = read_disabled(uid, &mut warnings);
    apply_disabled(&mut services, disabled);

    // 步骤 4：Homebrew 集成
    apply_brew_services(uid, &mut services, &mut warnings);

    // 步骤 5：BTM 登录项合并
    if include_login_items {
        if let Some(dump) = btm_dump {
            let login = login_items::parse(dump);
            merge_btm_items(&mut services, login, uid);
        }
    }

    // 步骤 6：经典登录项扫描（LSSharedFileList）
    let classic_items = scan_classic_login_items(uid);

    // 步骤 7-9：App 关联 + Origin 分类 + Health 检查
    for service in services.values_mut() {
        service.app_info = app_assoc::associate(service, app_index);
        service.origin = classify_origin(service);
        finish_health(service);
    }

    // 步骤 10：过滤 + 分组输出
    build_inventory(services, classic_items, show_system, &mut warnings)
}

// ── 步骤 1：plist 目录扫描 ──

fn current_uid() -> u32 {
    unsafe { libc::getuid() }
}

fn service_dirs() -> Vec<(PathBuf, ServiceScope)> {
    let home = home_dir();
    vec![
        (PathBuf::from(format!("{home}/Library/LaunchAgents")), ServiceScope::UserAgent),
        (PathBuf::from("/Library/LaunchAgents"), ServiceScope::GlobalAgent),
        (PathBuf::from("/Library/LaunchDaemons"), ServiceScope::SystemDaemon),
        (PathBuf::from("/System/Library/LaunchAgents"), ServiceScope::GlobalAgent),
        (PathBuf::from("/System/Library/LaunchDaemons"), ServiceScope::SystemDaemon),
    ]
}

fn read_plist_dir(
    dir: &Path,
    scope: ServiceScope,
    uid: u32,
    services: &mut BTreeMap<String, Service>,
) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("plist") {
            continue;
        }
        match service_from_plist(&path, scope, uid) {
            Ok(svc) => {
                services.insert(svc.id.clone(), svc);
            }
            Err(err) => {
                // 解析失败不消失，标记为 Unknown + health warning
                let label = path
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                let id = format!("{}:{}", scope.domain(uid), label);
                let domain = scope.domain(uid);
                let safety = safety_for_scope(&scope, &dir);
                let elevation = elevation_for(&domain, Some(&path), uid);
                services.insert(
                    id.clone(),
                    Service {
                        id,
                        label: label.clone(),
                        display_name: label,
                        source: ServiceSource::Launchd,
                        scope,
                        domain,
                        plist_path: Some(path.to_string_lossy().to_string()),
                        config: LaunchConfig::empty(),
                        pid: None,
                        exit_code: None,
                        status: ServiceStatus::Unknown,
                        enabled: None,
                        loaded: Some(false),
                        brew_formula: None,
                        brew_status: None,
                        safety_level: safety,
                        elevation,
                        origin: Origin::unknown(),
                        app_info: None,
                        health: vec![format!("plist 解析失败: {err}")],
                    },
                );
            }
        }
    }
    Ok(())
}

fn service_from_plist(path: &Path, scope: ServiceScope, uid: u32) -> Result<Service, String> {
    let value = plist::Value::from_file(path).map_err(|e| e.to_string())?;
    let dict = value.as_dictionary().ok_or("不是 plist 字典")?;

    let label = dict
        .get("Label")
        .and_then(|v| v.as_string())
        .map(String::from)
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        });

    let arguments: Vec<String> = dict
        .get("ProgramArguments")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_string().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let config = LaunchConfig {
        program: dict.get("Program").and_then(|v| v.as_string()).map(String::from),
        arguments,
        working_directory: dict.get("WorkingDirectory").and_then(|v| v.as_string()).map(String::from),
        stdout_path: dict.get("StandardOutPath").and_then(|v| v.as_string()).map(String::from),
        stderr_path: dict.get("StandardErrorPath").and_then(|v| v.as_string()).map(String::from),
        run_at_load: dict.get("RunAtLoad").and_then(|v| v.as_boolean()),
        keep_alive: dict.get("KeepAlive").map(describe_plist_value),
        start_interval: dict.get("StartInterval").and_then(|v| v.as_unsigned_integer()),
        start_calendar_intervals: parse_calendar_intervals(dict.get("StartCalendarInterval")),
    };

    let domain = scope.domain(uid);
    let safety = safety_for_scope(&scope, path.parent().unwrap_or(Path::new("/System/Library")));
    let elevation = elevation_for(&domain, Some(path), uid);
    let display_name = label
        .strip_prefix("homebrew.mxcl.")
        .unwrap_or(&label)
        .to_string();

    Ok(Service {
        id: format!("{domain}:{label}"),
        label,
        display_name,
        source: ServiceSource::Launchd,
        scope,
        domain,
        plist_path: Some(path.to_string_lossy().to_string()),
        config,
        pid: None,
        exit_code: None,
        status: ServiceStatus::Unloaded,
        enabled: None,
        loaded: Some(false),
        brew_formula: None,
        brew_status: None,
        safety_level: safety,
        elevation,
        origin: Origin::unknown(),
        app_info: None,
        health: Vec::new(),
    })
}

fn describe_plist_value(value: &plist::Value) -> String {
    match value {
        plist::Value::Boolean(b) => b.to_string(),
        plist::Value::Dictionary(_) => "conditional".to_string(),
        plist::Value::Array(_) => "array".to_string(),
        _ => "configured".to_string(),
    }
}

fn parse_calendar_intervals(value: Option<&plist::Value>) -> Vec<CalendarSchedule> {
    match value {
        Some(plist::Value::Dictionary(d)) => calendar_from_dict(d).into_iter().collect(),
        Some(plist::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_dictionary())
            .filter_map(calendar_from_dict)
            .collect(),
        _ => Vec::new(),
    }
}

fn calendar_from_dict(dict: &plist::Dictionary) -> Option<CalendarSchedule> {
    let s = CalendarSchedule {
        minute: dict.get("Minute").and_then(|v| v.as_unsigned_integer()),
        hour: dict.get("Hour").and_then(|v| v.as_unsigned_integer()),
        day: dict.get("Day").and_then(|v| v.as_unsigned_integer()),
        weekday: dict.get("Weekday").and_then(|v| v.as_unsigned_integer()),
        month: dict.get("Month").and_then(|v| v.as_unsigned_integer()),
    };
    if s.minute.is_none() && s.hour.is_none() && s.day.is_none() && s.weekday.is_none() && s.month.is_none() {
        None
    } else {
        Some(s)
    }
}

// ── 步骤 2：运行时状态 ──

#[derive(Clone, Debug, Default)]
struct RuntimeState {
    pid: Option<u32>,
    exit_code: Option<i32>,
    loaded: bool,
}

fn read_runtime(uid: u32, warnings: &mut Vec<String>) -> HashMap<String, RuntimeState> {
    let mut runtime = HashMap::new();
    for domain in [format!("gui/{uid}"), "system".to_string()] {
        let out = run_with_timeout_capture_lossy(10.0, "/bin/launchctl", &["print", &domain]);
        match out {
            Some(text) => parse_launchctl_print(&domain, &text, &mut runtime),
            None => warnings.push(format!("launchctl print {domain}: 超时或失败")),
        }
    }
    runtime
}

fn parse_launchctl_print(domain: &str, text: &str, runtime: &mut HashMap<String, RuntimeState>) {
    let mut in_services = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "services = {" {
            in_services = true;
            continue;
        }
        if in_services && trimmed == "}" {
            in_services = false;
            continue;
        }
        if !in_services || trimmed.is_empty() {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        let label = parts[2..].join(" ");
        // 跳过 XPC / UIKit / application 运行时噪音
        if label.starts_with("com.apple.xpc.")
            || label.starts_with("application.")
            || label.starts_with("UIKitApplication:")
        {
            continue;
        }
        let pid = parts[0].parse::<u32>().ok().filter(|p| *p > 0);
        let exit_code = parts[1].parse::<i32>().ok();
        runtime.insert(
            format!("{domain}:{label}"),
            RuntimeState { pid, exit_code, loaded: true },
        );
    }
}

fn apply_runtime(services: &mut BTreeMap<String, Service>, runtime: HashMap<String, RuntimeState>, uid: u32) {
    let known_ids: HashSet<String> = services.keys().cloned().collect();

    for (id, state) in &runtime {
        if let Some(svc) = services.get_mut(id) {
            svc.pid = state.pid;
            svc.exit_code = state.exit_code;
            svc.loaded = Some(state.loaded);
            svc.status = status_from_state(state, &svc.config);
        }
    }

    // 运行时存在但磁盘无 plist 的条目 → runtime-only
    for (id, state) in runtime {
        if known_ids.contains(&id) {
            continue;
        }
        let Some((domain_str, label_str)) = id.split_once(':') else { continue };
        let domain_owned = domain_str.to_string();
        let label_owned = label_str.to_string();
        let scope = if domain_str == "system" {
            ServiceScope::SystemDaemon
        } else {
            ServiceScope::UserAgent
        };
        let elevation = elevation_for(&domain_owned, None, uid);
        let display_name = label_owned.strip_prefix("homebrew.mxcl.").unwrap_or(&label_owned).to_string();
        services.insert(
            id.clone(),
            Service {
                id,
                label: label_owned,
                display_name,
                source: ServiceSource::Launchd,
                scope,
                domain: domain_owned,
                plist_path: None,
                config: LaunchConfig::empty(),
                pid: state.pid,
                exit_code: state.exit_code,
                status: status_from_state(&state, &LaunchConfig::empty()),
                enabled: None,
                loaded: Some(true),
                brew_formula: None,
                brew_status: None,
                safety_level: SafetyLevel::ProtectedVendor,
                elevation,
                origin: Origin::unknown(),
                app_info: None,
                health: Vec::new(),
            },
        );
    }
}

fn status_from_state(state: &RuntimeState, config: &LaunchConfig) -> ServiceStatus {
    if state.pid.is_some() {
        ServiceStatus::Running
    } else if state.exit_code.is_some_and(|c| c != 0) {
        ServiceStatus::Failed
    } else if config.has_schedule() {
        ServiceStatus::Scheduled
    } else {
        ServiceStatus::Stopped
    }
}

// ── 步骤 3：禁用状态 ──

fn read_disabled(uid: u32, warnings: &mut Vec<String>) -> HashSet<String> {
    let mut disabled = HashSet::new();
    for domain in [format!("gui/{uid}"), format!("user/{uid}"), "system".to_string()] {
        let out = run_with_timeout_capture_lossy(8.0, "/bin/launchctl", &["print-disabled", &domain]);
        match out {
            Some(text) => parse_disabled(&domain, &text, &mut disabled),
            None => warnings.push(format!("launchctl print-disabled {domain}: 超时或失败")),
        }
    }
    disabled
}

fn parse_disabled(domain: &str, text: &str, disabled: &mut HashSet<String>) {
    for line in text.lines() {
        let trimmed = line.trim();
        let Some((label_part, state)) = trimmed.split_once("=>") else { continue };
        let state = state.trim();
        // macOS >= 13: "=> disabled" / "=> enabled"；macOS <= 12: "=> true" / "=> false"
        if !matches!(state, "true" | "disabled") {
            continue;
        }
        let label = label_part.trim().trim_matches('"');
        disabled.insert(format!("{domain}:{label}"));
    }
}

fn apply_disabled(services: &mut BTreeMap<String, Service>, disabled: HashSet<String>) {
    for svc in services.values_mut() {
        if disabled.contains(&svc.id) {
            svc.enabled = Some(false);
            svc.status = ServiceStatus::Disabled;
        } else {
            svc.enabled = Some(true);
        }
    }
}

// ── 步骤 4：Homebrew 集成 ──

#[derive(serde::Deserialize)]
struct BrewService {
    name: String,
    status: String,
    user: Option<String>,
    file: Option<String>,
    exit_code: Option<i32>,
}

fn apply_brew_services(uid: u32, services: &mut BTreeMap<String, Service>, warnings: &mut Vec<String>) {
    let out = run_with_timeout_capture_lossy(12.0, "brew", &["services", "list", "--json"]);
    let Some(json) = out else {
        // brew 未安装或超时 → 静默跳过，不是错误
        return;
    };
    let brew_list: Vec<BrewService> = match serde_json::from_str(&json) {
        Ok(v) => v,
        Err(e) => {
            warnings.push(format!("brew services JSON 解析失败: {e}"));
            return;
        }
    };

    for brew in brew_list {
        let label = format!("homebrew.mxcl.{}", brew.name);
        let domain = if brew.user.as_deref() == Some("root") {
            "system".to_string()
        } else {
            format!("gui/{uid}")
        };
        let id = format!("{domain}:{label}");
        let file = brew.file.clone();

        if let Some(svc) = services.get_mut(&id) {
            // 已有 plist → 标记 Both + 回填 brew 元数据
            svc.source = ServiceSource::Both;
            svc.display_name = brew.name.clone();
            svc.brew_formula = Some(brew.name.clone());
            svc.brew_status = Some(brew.status.clone());
            if svc.exit_code.is_none() {
                svc.exit_code = brew.exit_code;
            }
            if svc.status == ServiceStatus::Unloaded {
                svc.status = status_from_brew(&brew.status, brew.exit_code);
            }
            continue;
        }

        // 没有对应 plist → 新增条目
        let scope = if domain == "system" {
            ServiceScope::SystemDaemon
        } else {
            ServiceScope::UserAgent
        };
        let elevation = elevation_for(&domain, file.as_deref().map(Path::new), uid);
        services.insert(
            id.clone(),
            Service {
                id,
                label: label.clone(),
                display_name: brew.name.clone(),
                source: ServiceSource::Homebrew,
                scope,
                domain,
                plist_path: file,
                config: LaunchConfig::empty(),
                pid: None,
                exit_code: brew.exit_code,
                status: status_from_brew(&brew.status, brew.exit_code),
                enabled: None,
                loaded: None,
                brew_formula: Some(brew.name),
                brew_status: Some(brew.status),
                safety_level: SafetyLevel::UserWritable,
                elevation,
                origin: Origin::unknown(),
                app_info: None,
                health: Vec::new(),
            },
        );
    }
}

fn status_from_brew(status: &str, exit_code: Option<i32>) -> ServiceStatus {
    match status {
        "started" => ServiceStatus::Running,
        "stopped" => ServiceStatus::Stopped,
        "error" => ServiceStatus::Failed,
        "none" => ServiceStatus::Unloaded,
        _ if exit_code.is_some_and(|c| c != 0) => ServiceStatus::Failed,
        _ => ServiceStatus::Unknown,
    }
}

// ── 步骤 5：BTM 登录项合并 ──

fn merge_btm_items(services: &mut BTreeMap<String, Service>, login: Vec<login_items::LoginItem>, uid: u32) {
    let known_labels: HashSet<String> = services.values().map(|s| s.label.to_lowercase()).collect();
    let domain = format!("gui/{uid}");

    for li in login {
        let norm = normalize_btm_id(&li.identifier);
        if norm.is_empty() || li.identifier.to_lowercase() == "unknown developer" {
            continue;
        }
        if known_labels.contains(&norm) || known_labels.contains(&li.identifier.to_lowercase()) {
            continue;
        }
        let label = if li.name.is_empty() { li.identifier.clone() } else { li.name.clone() };
        let id = format!("btm:{}", li.identifier);
        services.insert(
            id.clone(),
            Service {
                id,
                label: label.clone(),
                display_name: label,
                source: ServiceSource::Launchd,
                scope: ServiceScope::UserAgent,
                domain: domain.clone(),
                plist_path: Some(format!("btm:{}", li.identifier)),
                config: LaunchConfig::empty(),
                pid: None,
                exit_code: None,
                status: if li.enabled { ServiceStatus::Running } else { ServiceStatus::Disabled },
                enabled: Some(li.enabled),
                loaded: None,
                brew_formula: None,
                brew_status: None,
                safety_level: SafetyLevel::ProtectedVendor, // BTM 项不可 launchctl 操作
                elevation: ElevationNeeds::none(),
                origin: Origin::unknown(),
                app_info: None,
                health: Vec::new(),
            },
        );
    }
}

fn normalize_btm_id(id: &str) -> String {
    let stripped = match id.split_once('.') {
        Some((head, rest)) if !head.is_empty() && head.bytes().all(|b| b.is_ascii_digit()) => rest,
        _ => id,
    };
    stripped.to_lowercase()
}

// ── 步骤 7：Origin 分类 ──

fn classify_origin(service: &Service) -> Origin {
    let plist = service.plist_path.as_deref().unwrap_or("");
    let command = service
        .config
        .program
        .as_deref()
        .or_else(|| service.config.arguments.first().map(String::as_str))
        .unwrap_or("");

    let origin = |kind: Provenance, confidence: Confidence, evidence: &str| Origin {
        kind,
        confidence,
        evidence: vec![evidence.to_string()],
    };

    // Homebrew（最高优先级）
    if service.brew_formula.is_some() {
        return origin(Provenance::Homebrew, Confidence::High, "brew services 已注册此 formula");
    }
    if service.label.starts_with("homebrew.mxcl.") {
        return origin(Provenance::Homebrew, Confidence::High, "label 使用 homebrew.mxcl. 前缀");
    }
    if plist.contains("/Cellar/") || command.contains("/Cellar/") {
        return origin(Provenance::Homebrew, Confidence::Medium, "路径指向 Homebrew Cellar");
    }

    // Nix / Mise
    if command.contains("/nix/store/") {
        return origin(Provenance::Unknown, Confidence::High, "command 在 /nix/store 中");
    }

    // Apple 系统
    if plist.starts_with("/System/Library") {
        return origin(Provenance::System, Confidence::High, "plist 位于 /System/Library");
    }
    if service.label.starts_with("com.apple.") {
        return origin(Provenance::System, Confidence::High, "label 使用 com.apple. 前缀");
    }

    // 无 plist 的运行时条目
    if service.plist_path.is_none() {
        return origin(Provenance::RuntimeOnly, Confidence::High, "已加载但磁盘上无 plist");
    }

    // Vendor App（命令在 .app 包内）
    if looks_like_vendor_command(command) {
        return origin(Provenance::VendorApp, Confidence::Medium, "命令位于 app bundle 内");
    }

    // 用户自建
    let home = home_dir();
    if plist.starts_with(&format!("{home}/Library/LaunchAgents")) {
        return origin(Provenance::UserPlist, Confidence::High, "plist 位于 ~/Library/LaunchAgents");
    }

    // 机器级 plist，无安装器签名
    if plist.starts_with("/Library/Launch") {
        return origin(Provenance::VendorApp, Confidence::Guess, "机器级 plist，无安装器签名");
    }

    Origin::unknown()
}

fn looks_like_vendor_command(command: &str) -> bool {
    command.contains(".app/Contents/")
        || command.contains("/Library/Application Support/")
        || command.contains("/Library/PrivilegedHelperTools/")
}

// ── 步骤 8：Health 检查 ──

fn finish_health(service: &mut Service) {
    if service.plist_path.is_none() {
        push_health(service, "已加载的服务无磁盘 plist 文件");
        return;
    }
    if service.config.program.is_none() && service.config.arguments.is_empty() {
        push_health(service, "plist 中无 Program 或 ProgramArguments");
    }
    if let Some(path) = &service.plist_path {
        if !path.starts_with("btm:") && !Path::new(path).exists() {
            push_health(service, &format!("plist 文件不存在: {path}"));
        }
    }
    // 可执行文件缺失检查（含外接盘防误判）
    if let Some(program) = service.config.arguments.first().or(service.config.program.as_ref()) {
        if program.starts_with('/') && !Path::new(program).exists() {
            if !is_on_unplugged_volume(program) {
                push_health(service, &format!("可执行文件不存在: {program}"));
            }
        }
    }
}

fn push_health(service: &mut Service, msg: &str) {
    let s = msg.to_string();
    if !service.health.contains(&s) {
        service.health.push(s);
    }
}

/// 外接盘未挂载不算 broken（对齐 RemovableVolumeGuard）。
fn is_on_unplugged_volume(path: &str) -> bool {
    if !path.starts_with("/Volumes/") {
        return false;
    }
    let comps: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    if comps.len() < 2 {
        return false;
    }
    let vol_root = format!("/Volumes/{}", comps[1]);
    !Path::new(&vol_root).exists()
}

// ── 步骤 6：经典登录项扫描 ──

/// 扫描经典登录项（LSSharedFileList）并转换为 Service。
/// 返回 Vec 以保留重复条目（对齐柠檬的数组行为）。
fn scan_classic_login_items(uid: u32) -> Vec<Service> {
    let items = classic_login_items::scan_classic_login_items();
    let domain = format!("gui/{uid}");

    items
        .into_iter()
        .enumerate()
        .map(|(idx, item)| {
            // 用索引区分同名重复条目
            let id = format!("classic:{idx}:{}", item.display_name);
            Service {
                id,
                label: item.display_name.clone(),
                display_name: item.display_name.clone(),
                source: ServiceSource::Launchd,
                scope: ServiceScope::UserAgent,
                domain: domain.clone(),
                plist_path: Some(format!("classic:{}", item.bundle_path)),
                config: LaunchConfig {
                    program: Some(item.bundle_path.clone()),
                    arguments: Vec::new(),
                    working_directory: None,
                    stdout_path: None,
                    stderr_path: None,
                    run_at_load: None,
                    keep_alive: None,
                    start_interval: None,
                    start_calendar_intervals: Vec::new(),
                },
                pid: None,
                exit_code: None,
                status: ServiceStatus::Unknown,
                enabled: Some(true),
                loaded: None,
                brew_formula: None,
                brew_status: None,
                safety_level: SafetyLevel::UserWritable,
                elevation: ElevationNeeds::none(),
                origin: Origin::unknown(),
                app_info: None,
                health: Vec::new(),
            }
        })
        .collect()
}

// ── 步骤 10：过滤 + 分组 ──

fn build_inventory(
    services: BTreeMap<String, Service>,
    classic_items: Vec<Service>,
    show_system: bool,
    warnings: &mut Vec<String>,
) -> StartupInventory {
    let visible: Vec<Service> = services
        .into_values()
        .filter(|s| filter::is_user_visible(s, show_system))
        .collect();

    // 按 App 分组
    let mut groups: BTreeMap<String, Vec<Service>> = BTreeMap::new();
    let mut standalone: Vec<Service> = Vec::new();

    for svc in visible {
        if let Some(app) = &svc.app_info {
            let key = if app.bundle_id.is_empty() {
                app.app_path.clone()
            } else {
                app.bundle_id.clone()
            };
            groups.entry(key).or_default().push(svc);
        } else {
            standalone.push(svc);
        }
    }

    let app_groups: Vec<AppGroup> = groups
        .into_iter()
        .map(|(_, mut svcs)| {
            svcs.sort_by(|a, b| a.label.cmp(&b.label));
            let app = svcs[0].app_info.clone().unwrap();
            let enable_status = compute_enable_status(&svcs);
            AppGroup {
                app_name: app.app_name,
                app_path: app.app_path,
                bundle_id: app.bundle_id,
                services: svcs,
                enable_status,
            }
        })
        .collect();

    // standalone 按 display_name 排序
    standalone.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));

    StartupInventory {
        app_groups,
        standalone_services: standalone,
        classic_login_items: classic_items,
        warnings: warnings.clone(),
    }
}

fn compute_enable_status(services: &[Service]) -> EnableStatus {
    let enabled_count = services.iter().filter(|s| s.enabled != Some(false)).count();
    if enabled_count == 0 {
        EnableStatus::AllDisabled
    } else if enabled_count == services.len() {
        EnableStatus::AllEnabled
    } else {
        EnableStatus::SomeEnabled
    }
}

// ── 安全 / 权限辅助 ──

fn safety_for_scope(scope: &ServiceScope, dir: &Path) -> SafetyLevel {
    if dir.starts_with("/System/Library") {
        SafetyLevel::ReadonlySystem
    } else {
        match scope {
            ServiceScope::UserAgent => SafetyLevel::UserWritable,
            ServiceScope::GlobalAgent | ServiceScope::SystemDaemon => SafetyLevel::AdminRequired,
        }
    }
}

fn elevation_for(domain: &str, plist_path: Option<&Path>, uid: u32) -> ElevationNeeds {
    ElevationNeeds {
        runtime: domain == "system" && uid != 0,
        plist_write: !plist_path.is_some_and(|p| is_writable(p, uid)),
        plist_remove: !plist_path
            .and_then(Path::parent)
            .is_some_and(|dir| is_writable(dir, uid)),
    }
}

fn is_writable(path: &Path, uid: u32) -> bool {
    if uid == 0 {
        return true;
    }
    let Ok(meta) = path.metadata() else { return false };
    use std::os::unix::fs::MetadataExt;
    let mode = meta.mode();
    if meta.uid() == uid {
        mode & 0o200 != 0
    } else {
        mode & 0o002 != 0
    }
}
