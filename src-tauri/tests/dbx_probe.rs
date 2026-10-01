//! 临时诊断探针：绕开 Tauri IPC / 前端，直接对 /Applications/DBX.app 跑
//! `collect_app_details`（mole_uninstall dry_run 的底层），把全链路 log::info! 打到 stderr。
//! 目的：定位「展开 DBX 只有 App Bundle、没有残留」的根因。
//! 跑法：cargo test --test dbx_probe -- --nocapture

struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        eprintln!("[probe] {}", record.args());
    }
    fn flush(&self) {}
}

static LOGGER: StderrLogger = StderrLogger;

/// 逐项复现 uninstall_live_bundle_has_other_install 的三个 indeterminate 来源。
#[test]
fn probe_indeterminate_sources() {
    // 1) pkg receipts 完整性
    let (paths, complete) =
        mole_lib::core::pkg_receipts::pkg_receipt_nonstandard_app_paths_complete();
    eprintln!("[probe] pkg complete={complete} paths={paths:?}");

    // 2) /Volumes 每个卷的 read_dir（find_volume_app_roots 同款逻辑）
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        for entry in rd.flatten() {
            let vol = entry.path();
            let apps = vol.join("Applications");
            eprintln!(
                "[probe] vol={} apps.is_dir={}",
                vol.display(),
                apps.is_dir()
            );
            match std::fs::read_dir(&vol) {
                Ok(_) => eprintln!("[probe]   vol readable: {}", vol.display()),
                Err(e) => eprintln!("[probe]   VOL UNREADABLE: {} err={e}", vol.display()),
            }
            // 卷根直挂的 *.app 也会被列为 root（find_volume_app_roots 同款）
            if let Ok(sub) = std::fs::read_dir(&vol) {
                for e in sub.flatten() {
                    let p = e.path();
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.ends_with(".app") {
                        eprintln!("[probe]   vol top-level app root: {}", p.display());
                        match std::fs::read_dir(&p) {
                            Ok(_) => eprintln!("[probe]     app root readable"),
                            Err(e2) => eprintln!("[probe]     APP ROOT UNREADABLE err={e2}"),
                        }
                    }
                }
            }
        }
    }

    // 3) live_app_roots 各 root 的可读性（find_app_bundles root 层）
    let home = std::env::var("HOME").unwrap_or_default();
    let roots = vec![
        "/Applications".to_string(),
        format!("{home}/Applications"),
        "/System/Applications".to_string(),
        "/Library/Input Methods".to_string(),
        format!("{home}/Library/Input Methods"),
        format!("{home}/Library/Application Support/Setapp/Applications"),
        "/opt/homebrew/Caskroom".to_string(),
        "/usr/local/Caskroom".to_string(),
        "/Volumes/Macintosh HD/Applications".to_string(),
    ];
    for root in roots {
        let p = std::path::Path::new(&root);
        if !p.exists() {
            eprintln!("[probe] root missing: {root}");
            continue;
        }
        if !p.is_dir() {
            eprintln!("[probe] root NOT DIR: {root}");
            continue;
        }
        match std::fs::read_dir(&root) {
            Ok(_) => eprintln!("[probe] root readable: {root}"),
            Err(e) => eprintln!("[probe] ROOT UNREADABLE: {root} err={e}"),
        }
    }
}

/// 冒烟证据：大输出命令在 run_with_timeout_capture 下是否死锁。
/// 预期（修复前）：~5s 后返回 None（管道未排空 → 子进程阻塞 → 超时被杀）。
#[test]
fn probe_pipe_deadlock() {
    let t0 = std::time::Instant::now();
    let r = mole_lib::core::timeout::run_with_timeout_capture(
        5.0,
        "pkgutil",
        &["--files", "org.golang.go"],
    );
    let dt = t0.elapsed();
    eprintln!(
        "[probe] pkgutil --files org.golang.go => is_some={} elapsed={:.1?} out_len={}",
        r.is_some(),
        dt,
        r.as_ref().map(|s| s.len()).unwrap_or(0)
    );

    let t1 = std::time::Instant::now();
    let r2 = mole_lib::core::timeout::run_with_timeout_capture(5.0, "pkgutil", &["--pkgs"]);
    eprintln!(
        "[probe] pkgutil --pkgs => is_some={} elapsed={:.1?}",
        r2.is_some(),
        t1.elapsed()
    );
}

#[test]
fn probe_dbx_collect_app_details() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Debug);

    let result =
        mole_lib::uninstall::batch::collect_app_details(&["/Applications/DBX.app".to_string()]);
    match result {
        Ok(apps) => {
            eprintln!("[probe] ===== details count = {} =====", apps.len());
            for d in &apps {
                eprintln!(
                    "[probe] app={} path={} bundle={} original_bundle={} guard={}",
                    d.app_name, d.app_path, d.bundle_id, d.original_bundle_id, d.sibling_guard
                );
                eprintln!("[probe] related_files:\n{}", d.related_files);
                eprintln!("[probe] review_system_files:\n{}", d.review_system_files);
            }
        }
        Err(e) => eprintln!("[probe] ===== collect_app_details Err: {e} ====="),
    }
}

/// 端到端验证前端契约：mole_uninstall dry_run 必须把 4 条 DBX 残留全部返回，
/// 包括含空格的 Application Support 路径（du 解析修复）与 0 字节 Logs 目录
/// （size==0 过滤移除）。
#[test]
fn probe_dbx_dry_run_related_files() {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let value = rt.block_on(mole_lib::controllers::uninstall::mole_uninstall(
        "/Applications/DBX.app".to_string(),
        true,
    ));
    let value = match value {
        Ok(v) => v,
        Err(e) => panic!("[probe] mole_uninstall dry_run Err: {e}"),
    };

    let home = std::env::var("HOME").unwrap_or_default();
    let expect = [
        (
            format!("{home}/Library/Application Support/com.dbx.app"),
            true,
        ),
        (format!("{home}/Library/Caches/com.dbx.app"), true),
        (format!("{home}/Library/Logs/com.dbx.app"), false), // 允许 0 字节
        (format!("{home}/Library/WebKit/com.dbx.app"), true),
    ];

    let related = value["related_files"]
        .as_array()
        .expect("related_files array");
    let paths: Vec<&str> = related
        .iter()
        .map(|f| f["path"].as_str().unwrap_or(""))
        .collect();
    eprintln!("[probe] dry_run related_files={:?}", paths);

    for (p, want_size) in expect {
        let found = related
            .iter()
            .find(|f| f["path"].as_str() == Some(p.as_str()));
        let found = found.unwrap_or_else(|| panic!("[probe] missing related file: {p}"));
        let size = found["size"].as_u64().unwrap_or(0);
        if want_size {
            assert!(size > 0, "[probe] {p} size should be > 0, got {size}");
        }
        eprintln!("[probe]   ok: {p} size={size}");
    }
    assert_eq!(
        related.len(),
        4,
        "[probe] DBX 残留应恰为 4 条，实际 {:?}",
        paths
    );
}

/// 对真实 Visual Studio Code 验证新 VSCode 分支（SH 第 1473-1489 行）:
/// 稳定版应收集 ~/.vscode、Application Support/Code、Caches/com.microsoft.VSCode,
/// 且 CrashReporter 扫描不误收无关 plist（如 Electron_*.plist）。
#[test]
fn probe_vscode_find_app_files() {
    let app = "/Applications/Visual Studio Code.app";
    if !std::path::Path::new(app).is_dir() {
        eprintln!("[probe] VSCode 未安装,跳过");
        return;
    }
    let files = mole_lib::core::app_protection::find_app_files(
        "com.microsoft.VSCode",
        "Visual Studio Code",
        app,
    );
    let home = std::env::var("HOME").unwrap_or_default();
    let must_have = [
        format!("{home}/.vscode"),
        format!("{home}/Library/Application Support/Code"),
        format!("{home}/Library/Caches/com.microsoft.VSCode"),
    ];
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Debug);
    for p in &must_have {
        assert!(
            files.iter().any(|f| f == p),
            "[probe] VSCode 残留缺失: {p}\n实际: {files:#?}"
        );
    }
    // CrashReporter 扫描只收 "Visual Studio Code_*.plist" 前缀,不应混入 Electron_* 等
    for f in &files {
        assert!(
            !f.contains("/CrashReporter/") || f.contains("Visual Studio Code_"),
            "[probe] CrashReporter 误收: {f}"
        );
    }
    eprintln!("[probe] VSCode files={files:#?}");
}

/// 对齐 SH 实测(2026-08-16 本机跑过 _mole_privileged_path_has_mutable_ancestor):
/// /Applications 是 root:admin 775 → group-write 位 → 判 mutable。
/// 探针锁定 Rust 移植与 SH 行为一致,防止两版漂移。
#[test]
fn probe_mutable_ancestor_matches_sh() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Debug);

    // 真实 /Applications 下的 app:SH 判 MUTABLE(775 group-write)
    assert!(
        mole_lib::core::file_ops::_mole_privileged_path_has_mutable_ancestor(
            "/Applications/Safari.app"
        ),
        "[probe] /Applications 应与 SH 一致判 mutable"
    );
    // 不存在的路径:stat 失败 → fail-closed mutable
    assert!(
        mole_lib::core::file_ops::_mole_privileged_path_has_mutable_ancestor(
            "/Applications/Definitely-Not-Real.app"
        ),
        "[probe] 不存在的路径应 fail-closed 判 mutable"
    );
    // identity 格式:dev:ino:mode 三段,末段八进制
    let id = mole_lib::core::file_ops::stat_path_identity("/").expect("stat / 应成功");
    let parts: Vec<&str> = id.split(':').collect();
    assert_eq!(parts.len(), 3, "[probe] identity 应为 dev:ino:mode: {id}");
    assert!(
        parts[2].chars().all(|c| ('0'..='7').contains(&c)),
        "[probe] mode 段应为八进制: {id}"
    );
    eprintln!("[probe] root identity={id}");
}
