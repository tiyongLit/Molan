//! 安全暂存目录：mkdtemp 原子创建 + 0700 + 归属感知清理。
//!
//! 对齐 Burrow `PrivateUpdateDirectory` 的核心纪律（P0 子集）：
//! - `mkdtemp` 原子创建随机目录（防预埋符号链接 / 路径竞态）；
//! - 创建后立即收紧 0700（仅属主可进）；
//! - 启动清理：仅删除「前缀匹配 + 属主为当前用户 + 超过 24h」的遗留目录。
//!
//! P0 不做 Burrow 的 fd-pinning（renameatx_np 相对操作）级防护——`0700` +
//! 随机名已挡住非属主干扰；如需对抗同用户恶意进程再评估加强。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 暂存目录名前缀（带尾随点，避免与其他程序前缀误匹配）。
pub const STAGING_PREFIX: &str = "MolanUpdateStaging.";

/// 超过该年龄的暂存目录视为遗留（进程内任务都是分钟级）。
const STALE_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// 创建安全暂存目录（原子创建、0700），返回目录路径。
pub fn create_staging_dir() -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStringExt;

    let template = std::env::temp_dir().join(format!("{STAGING_PREFIX}XXXXXX"));
    let mut buf = template.into_os_string().into_vec();
    buf.push(0); // NUL 结尾（C 字符串）
    let created = unsafe { libc::mkdtemp(buf.as_mut_ptr() as *mut libc::c_char) };
    if created.is_null() {
        return Err(format!(
            "创建暂存目录失败: {}",
            std::io::Error::last_os_error()
        ));
    }
    let path = PathBuf::from(unsafe { std::ffi::CStr::from_ptr(created) }.to_string_lossy().into_owned());

    // mkdtemp 以 0700 创建，但显式收紧一次（防御 umask / 平台差异）。
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("设置暂存目录权限失败: {e}"))?;
    Ok(path)
}

/// 取消 / 失败路径：递归删除暂存目录（尽力而为，不报错）。
pub fn discard_staging_dir(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

/// 进程启动时清理遗留暂存目录（三重判据：前缀 + 属主 + 年龄）。
pub fn cleanup_stale_staging_dirs() {
    let base = std::env::temp_dir();
    let Ok(entries) = std::fs::read_dir(&base) else {
        return;
    };
    let uid = unsafe { libc::getuid() };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(STAGING_PREFIX) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_dir() {
            continue;
        }
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != uid {
            continue; // 非本用户，不动
        }
        let Ok(mtime) = meta.modified() else { continue };
        let Ok(age) = now.duration_since(mtime) else { continue };
        if age < STALE_AGE {
            continue; // 新鲜目录可能属于其他在跑实例
        }
        let _ = std::fs::remove_dir_all(entry.path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_0700_dir_and_discards() {
        let dir = create_staging_dir().expect("创建暂存目录");
        assert!(dir.is_dir());
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with(STAGING_PREFIX), "名字应带前缀: {name}");

        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "权限应为 0700");

        discard_staging_dir(&dir);
        assert!(!dir.exists());
    }

    #[test]
    fn unique_dirs_per_call() {
        let a = create_staging_dir().unwrap();
        let b = create_staging_dir().unwrap();
        assert_ne!(a, b, "两次创建应为不同目录");
        discard_staging_dir(&a);
        discard_staging_dir(&b);
    }

    #[test]
    fn cleanup_keeps_fresh_and_unrelated_dirs() {
        // 新鲜的前缀目录（归属自己，< 24h）不应被清理
        let fresh = create_staging_dir().unwrap();
        // 非前缀目录不应被碰
        let unrelated = std::env::temp_dir().join("molan-unrelated-probe-9f3a");
        std::fs::create_dir_all(&unrelated).unwrap();

        cleanup_stale_staging_dirs();

        assert!(fresh.exists(), "新鲜暂存目录不应被清理");
        assert!(unrelated.exists(), "无关目录不应被碰");

        discard_staging_dir(&fresh);
        let _ = std::fs::remove_dir_all(&unrelated);
    }
}
