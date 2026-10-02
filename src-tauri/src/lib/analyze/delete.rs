use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use wait_timeout::ChildExt;

use super::cache;
use super::protected::is_protected_entry_path;

pub fn trash_path_with_progress(
    root: &str,
    counter: Option<&std::sync::atomic::AtomicI64>,
) -> Result<i64, String> {
    let p = Path::new(root);
    // 存在性检查（对齐 Go os.Lstat，兼容 broken symlink）
    fs::symlink_metadata(p).map_err(|e| e.to_string())?;

    // Go `trashPathWithProgress`（9cb63949）：废纸篓每次移动的是一个整体路径，
    // 先递归 WalkDir 数文件会让大目录删除在移动开始前显得假死。计数恒为 1。
    let count: i64 = 1;
    if let Some(cnt) = counter {
        cnt.store(count, std::sync::atomic::Ordering::SeqCst);
    }

    move_to_trash(root)?;

    // 失效被删除路径的父目录缓存，确保下次扫描不会返回过期数据。
    if let Some(parent) = Path::new(root).parent() {
        cache::invalidate_cache(&parent.to_string_lossy());
    }

    Ok(count)
}

pub fn delete_multiple_paths(
    paths: &[String],
    counter: Option<&std::sync::atomic::AtomicI64>,
) -> (i64, Option<MultiDeleteError>) {
    let mut sorted: Vec<String> = paths.to_vec();
    sorted.sort_by(|a, b| {
        let da = a.matches(std::path::MAIN_SEPARATOR).count();
        let db = b.matches(std::path::MAIN_SEPARATOR).count();
        db.cmp(&da)
    });

    let mut total: i64 = 0;
    let mut errors: Vec<String> = Vec::new();

    for path in &sorted {
        match trash_path_with_progress(path, counter) {
            Ok(c) => total += c,
            Err(e) => {
                if Path::new(path).symlink_metadata().is_err() {
                    continue;
                }
                errors.push(e);
            }
        }
    }

    let err = if errors.is_empty() {
        None
    } else {
        Some(MultiDeleteError { errors })
    };

    (total, err)
}

#[derive(Debug)]
pub struct MultiDeleteError {
    errors: Vec<String>,
}

impl MultiDeleteError {
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn len(&self) -> usize {
        self.errors.len()
    }
}

impl fmt::Display for MultiDeleteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.errors.len() == 1 {
            return write!(f, "{}", self.errors[0]);
        }
        let limit = self.errors.len().min(3);
        write!(f, "{}", self.errors[..limit].join("; "))
    }
}

impl std::error::Error for MultiDeleteError {}

fn move_to_trash(path: &str) -> Result<(), String> {
    validate_trash_target(path)?;
    // std::path::absolute：只做绝对化，不解析符号链接（canonicalize 会把 symlink
    // 解析为目标路径，导致误删目标文件而非链接本身）。参考 Zed fs.rs trash 实现。
    let abs = std::path::absolute(Path::new(path))
        .map_err(|e| format!("failed to resolve path: {}", e))?;
    let abs_str = abs.to_string_lossy();
    validate_trash_target(&abs_str)?;

    // 双重验证：底层兜底检查受保护路径，防止通过符号链接绕过
    if is_protected_entry_path(&abs_str) {
        let name = abs.file_name().and_then(|n| n.to_str()).unwrap_or(&abs_str);
        log::warn!("[trash] blocked protected path (resolved): {abs_str}");
        return Err(format!("受系统保护的路径，不可删除: {name}"));
    }

    // trash(8) 优先（SSH 场景可靠），trash crate（Finder AppleScript）兜底。
    // Go moveToTrash 链为 Binary → Filesystem → Finder；现代 macOS 都自带 trash(8)，
    // Rust 侧 crate 兜底即 Finder 路线，Filesystem 中间层无需移植。
    if move_to_trash_via_binary(&abs).is_ok() {
        return Ok(());
    }

    trash::delete(&abs).map_err(|e| format!("failed to move to Trash: {}", e))
}

/// Go `trashBinary`：Apple 自带 trash(8)。不经 Finder 直接把路径移入用户废纸篓，
/// 这正是 SSH 场景下删除可用的原因：Finder AppleScript 路线会在实体机上弹出对话框，
/// 远程用户无法应答，删除只会一直超时（issue #474）。
///
/// 用绝对路径调用而非 PATH 查找，避免 PATH 中更靠前的同名可执行文件劫持删除请求。
/// 参数恒为绝对路径，无需 "--" 分隔符；传了反而会让 trash(8) 把 "--" 当作缺失文件报错
/// 并以非零退出——即使目标其实已删除，也会触发 Finder 兜底重复删除。
const TRASH_BINARY: &str = "/usr/bin/trash";

/// Go `trashTimeout`：trash(8) 单次删除超时。
const TRASH_TIMEOUT: Duration = Duration::from_secs(30);

/// Go `moveToTrashViaBinary`：用 trash(8) 移入废纸篓。二进制不存在时返回错误，由调用方回退 Finder。
fn move_to_trash_via_binary(abs_path: &Path) -> Result<(), String> {
    if !Path::new(TRASH_BINARY).exists() {
        return Err("trash binary not found".into());
    }

    let mut child = std::process::Command::new(TRASH_BINARY)
        .arg(abs_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn trash: {}", e))?;

    let status = match child.wait_timeout(TRASH_TIMEOUT) {
        Ok(Some(s)) => s,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("timeout moving to Trash".into());
        }
        Err(e) => return Err(format!("failed to wait for trash: {}", e)),
    };

    if status.success() {
        return Ok(());
    }

    let mut stderr = String::new();
    if let Some(mut s) = child.stderr.take() {
        use std::io::Read;
        let _ = s.read_to_string(&mut stderr);
    }
    Err(format!("failed to move to Trash: {}", stderr.trim()))
}

/// Go `validateTrashTarget`：基础路径校验 + OrbStack 专项保护。
fn validate_trash_target(path: &str) -> Result<(), String> {
    validate_path(path)?;
    if is_protected_analyze_delete_path(path) {
        return Err(format!("protected path cannot be deleted: {path}"));
    }
    Ok(())
}

/// Go `isProtectedAnalyzeDeletePath`：保护 OrbStack VM 数据与 EDR 缓存免于误删。
///
/// 保护范围：
/// 1. EDR / Darwin 缓存（最先检查，不依赖 HOME）
/// 2. `~/.orbstack` 及其所有子目录/文件
/// 3. `~/Library/Group Containers/<...dev.orbstack...>/**` 下的所有内容
fn is_protected_analyze_delete_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }

    // EDR / Darwin 缓存保护基于绝对路径，不依赖 HOME，最先检查：
    // 即使 HOME 未设置（如 `env -u HOME mo analyze`），Falcon 缓存也不得漏过。
    if is_endpoint_security_cache_path(path) {
        return true;
    }

    let home = match crate::core::base::home_dir_opt() {
        Some(h) => h,
        None => return false,
    };

    let clean = match Path::new(path).canonicalize() {
        Ok(c) => c,
        Err(_) => PathBuf::from(path),
    };
    let clean_s = clean.to_string_lossy();

    // 规则 1：~/.orbstack 及其子内容
    let orbstack_state = home.join(".orbstack");
    let orbstack_s = orbstack_state.to_string_lossy();
    if clean_s == orbstack_s || clean_s.starts_with(&format!("{orbstack_s}/")) {
        return true;
    }

    // 规则 2：~/Library/Group Containers 下以 dev.orbstack 结尾的容器
    let gc = home.join("Library").join("Group Containers");
    let gc_s = gc.to_string_lossy();
    let rel = match clean.strip_prefix(&*gc_s) {
        Ok(r) => r,
        Err(_) => return false,
    };
    let rel_s = rel.to_string_lossy();
    if rel_s.is_empty() || rel_s == "." || rel_s == ".." || rel_s.starts_with("..") {
        return false;
    }

    let container_name = match rel_s.find('/') {
        Some(idx) => &rel_s[..idx],
        None => &rel_s,
    };
    container_name.ends_with("dev.orbstack")
}

/// 镜像 Go `endpointSecurityBundlePrefixes`（lib/core/app_protection_data.sh）。
/// 删除这些 EDR/MDM 代理的 per-user Darwin 缓存会触发 sensor 篡改检测
/// （如 CrowdStrike MacFalconSensorTamper, MITRE T1562.001），analyze 绝不可将其移入废纸篓。
static ENDPOINT_SECURITY_BUNDLE_PREFIXES: &[&str] = &[
    "com.crowdstrike.",
    "com.sentinelone.",
    "com.sentinel-labs.",
    "com.eset.",
    "com.jamf.",
    "com.jamfsoftware.",
    "com.paloaltonetworks.",
    "com.cisco.anyconnect",
    "com.cisco.secureclient",
];

/// Go `isEndpointSecurityCachePath`：判断路径是否位于 per-user Darwin 文件夹
/// （/private/var/folders 或其 /var/folders 符号链接形式）下的端点安全/EDR 代理文件。
fn is_endpoint_security_cache_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    if !lower.starts_with("/private/var/folders/") && !lower.starts_with("/var/folders/") {
        return false;
    }
    ENDPOINT_SECURITY_BUNDLE_PREFIXES
        .iter()
        .any(|p| lower.contains(p))
}

fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("path is empty".into());
    }
    if !Path::new(path).is_absolute() {
        return Err(format!("path must be absolute: {}", path));
    }
    if path.contains('\0') {
        return Err("path contains null bytes".into());
    }
    if path.split(std::path::MAIN_SEPARATOR).any(|c| c == "..") {
        return Err(format!("path contains traversal components: {}", path));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicI64;

    // ── validate_path tests ──────────────────────────────────────────

    #[test]
    fn test_validate_path_table() {
        let cases: &[(&str, bool)] = &[
            // 基本合法路径
            ("/Users/test/file.txt", false),
            ("/Users/test/My Documents/file.txt", false),
            ("/", false),
            // 中文路径
            ("/Users/test/中文文件夹/文件.txt", false),
            ("/Users/test/Downloads/报告2024.pdf", false),
            // Emoji 路径
            ("/Users/test/📁文件夹/📝笔记.txt", false),
            ("/Users/test/🎉/🎊.txt", false),
            // 特殊字符路径
            ("/Users/test/$HOME/workspace", false),
            ("/Users/test/project;v2", false),
            ("/Users/test/project:2024", false),
            ("/Users/test/R&D/project", false),
            ("/Users/test/user@domain", false),
            ("/Users/test/project#123", false),
            ("/Users/test/100% complete", false),
            ("/Users/test/important!.txt", false),
            ("/Users/test/user's files", false),
            ("/Users/test/key=value", false),
            ("/Users/test/file+v2", false),
            ("/Users/test/[2024] report", false),
            ("/Users/test/project (copy)", false),
            ("/Users/test/file, backup", false),
            // 非法路径
            ("", true),
            ("relative/path", true),
            ("./file.txt", true),
            ("/Users/test\x00/file", true),
            ("/Users/test/../../../etc", true),
        ];

        for &(path, want_err) in cases {
            let result = validate_path(path);
            assert_eq!(
                result.is_err(),
                want_err,
                "validate_path({path:?}) err={result:?}, want_err={want_err}"
            );
        }
    }

    #[test]
    fn test_validate_path_with_chinese_and_special_chars() {
        let parent = tempfile::TempDir::new().expect("tempdir");
        let subdirs = [
            "中文文件夹",
            "📁 文档",
            "报告-2024_v2 (终稿) [已审核]",
            "Project$2024; Q1: R&D",
            "用户@公司 100% 完成!",
        ];

        for name in subdirs {
            let full = parent.path().join(name);
            fs::create_dir_all(&full).expect("mkdir");
            let full_str = full.to_string_lossy();
            validate_path(&full_str)
                .unwrap_or_else(|e| panic!("validate_path rejected valid path {full_str:?}: {e}"));
        }
    }

    // ── EDR endpoint-security cache protection ───────────────────────

    #[test]
    fn test_is_endpoint_security_cache_path() {
        // 命中：EDR 代理的 per-user Darwin 缓存（含 /var/folders 符号链接形式）
        for p in &[
            "/private/var/folders/zz/aa/C/com.crowdstrike.falcon.App/com.apple.metalfe",
            "/var/folders/zz/aa/C/com.jamf.management/cache",
            "/private/var/folders/zz/aa/X/com.sentinelone.agent.code_sign_clone",
        ] {
            assert!(
                is_endpoint_security_cache_path(p),
                "expected EDR cache: {p}"
            );
        }
        // 大小写不敏感（对齐 Go TestEndpointSecurityCachePathIsCaseInsensitive）
        assert!(is_endpoint_security_cache_path(
            "/PRIVATE/VAR/FOLDERS/9D/ABC/C/COM.CROWDSTRIKE.FALCON.APP/cache"
        ));
        // 未命中：不在 Darwin folders 下，或 bundle 前缀不匹配
        for p in &[
            "/private/var/folders/zz/aa/C/com.apple.metalfe",
            "/Users/foo/Library/Caches/com.crowdstrike.falcon",
            "/var/folders/zz/aa/C/com.example.app/cache",
        ] {
            assert!(
                !is_endpoint_security_cache_path(p),
                "expected NOT EDR cache: {p}"
            );
        }
    }

    #[test]
    fn test_validate_trash_target_rejects_endpoint_security_caches() {
        // EDR 检查不依赖 HOME：is_protected_analyze_delete_path 在读取 HOME 之前即返回，
        // 覆盖 Go TestValidateTrashTargetRejectsEndpointSecurityCachesWithoutHOME 场景。
        for p in &[
            "/private/var/folders/zz/aa/C/com.crowdstrike.falcon.App/com.apple.metalfe",
            "/private/var/folders/zz/aa/X/com.sentinelone.agent.code_sign_clone",
            "/var/folders/zz/aa/C/com.jamf.management/cache",
        ] {
            let err = validate_trash_target(p).expect_err("expected protected path error");
            assert!(err.contains("protected path"), "unexpected error: {err}");
        }
    }

    // ── move_to_trash tests ──────────────────────────────────────────

    #[test]
    fn test_move_to_trash_non_existent() {
        let err = move_to_trash("/nonexistent/path/that/does/not/exist");
        assert!(err.is_err(), "expected error for non-existent path");
    }

    #[test]
    fn test_move_to_trash_rejects_traversal() {
        let err = move_to_trash("/tmp/fakedir/../../../etc/passwd");
        assert!(
            err.is_err(),
            "expected error for path with traversal components"
        );
        let msg = err.unwrap_err();
        assert!(
            msg.contains("traversal"),
            "expected traversal error, got: {msg}"
        );
    }

    // ── trash tests (may require macOS permissions) ──────────────────

    fn skip_if_trash_unavailable() -> bool {
        if std::env::var("CI").is_ok() || std::env::var("MOLE_SKIP_FINDER_TESTS").is_ok() {
            return true;
        }
        false
    }

    #[test]
    fn test_move_to_trash_via_binary() {
        if skip_if_trash_unavailable() {
            return;
        }

        let parent = tempfile::TempDir::new().expect("tempdir");
        let target = parent.path().join("victim.txt");
        fs::write(&target, b"content").expect("write file");

        move_to_trash_via_binary(&target).expect("trash(8) should succeed");
        assert!(
            target.symlink_metadata().is_err(),
            "expected target to be moved to Trash"
        );
    }

    #[test]
    fn test_trash_path_with_progress() {
        if skip_if_trash_unavailable() {
            return;
        }

        let parent = tempfile::TempDir::new().expect("tempdir");
        let target = parent.path().join("target");
        fs::create_dir_all(&target).expect("create target");

        let files = [target.join("one.txt"), target.join("two.txt")];
        for f in &files {
            fs::write(f, b"content").expect("write file");
        }

        let counter = AtomicI64::new(0);
        let target_str = target.to_string_lossy().to_string();
        let count = trash_path_with_progress(&target_str, Some(&counter))
            .expect("trash_path_with_progress should succeed");
        // Go 9cb63949：计数恒为 1（整体移动，不递归数文件）
        assert_eq!(count, 1, "expected count of 1 for a single path move");
        // 路径应已被移入废纸篓
        assert!(
            target.symlink_metadata().is_err(),
            "expected target to be moved to Trash"
        );
    }

    #[test]
    fn test_delete_multiple_paths_handles_parent_child() {
        if skip_if_trash_unavailable() {
            return;
        }

        let base = tempfile::TempDir::new().expect("tempdir");
        let parent = base.path().join("parent");
        let child = parent.join("child");

        // Structure: parent/fileA, parent/child/fileC.
        fs::create_dir_all(&child).expect("mkdir child");
        fs::write(parent.join("fileA"), b"a").expect("write fileA");
        fs::write(child.join("fileC"), b"c").expect("write fileC");

        let counter = AtomicI64::new(0);
        let paths: Vec<String> = vec![
            parent.to_string_lossy().to_string(),
            child.to_string_lossy().to_string(),
        ];
        let (count, err) = delete_multiple_paths(&paths, Some(&counter));
        assert!(err.is_none(), "unexpected error: {:?}", err);
        assert_eq!(count, 2, "expected 2 paths trashed, got {count}");
        assert!(
            parent.symlink_metadata().is_err(),
            "expected parent to be moved to Trash"
        );
    }
}
