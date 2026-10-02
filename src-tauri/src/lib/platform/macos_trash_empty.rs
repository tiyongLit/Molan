//! 清空当前用户废纸篓（~/.Trash）—— 独立于 Clean 缓存清理链路的专用能力。
//!
//! 安全边界：
//! - 路径服务端硬编码为 `base::home_dir_opt()/.Trash`，不接受任何外部路径参数（零信任）；
//! - 用原生 `std::fs` 删除（`remove_file` / `remove_dir_all`），不调用外部 `rm` 二进制（守红线1）；
//! - 不跟随符号链接：symlink 用 `remove_file` 断链，绝不递归进其目标；
//! - 清空废纸篓 = 永久删除（文件已在废纸篓内，无法再次移入），属红线4记录在案的例外：
//!   仅作用于用户自己已丢弃的内容，调用方（提醒浮窗）强制二次确认。

use serde::Serialize;
use std::path::Path;

/// TCC / 权限类错误判定：EPERM 为 TCC 拦截的典型值，EACCES 作 POSIX 权限兜底。
/// 与 runtime::trash_watch 的同名判定口径一致（权限失败给出可引导的错误码）。
fn is_permission_denied(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::EPERM) | Some(libc::EACCES))
}

/// 清空结果统计。`reclaimedBytes` 为成功删除条目的逻辑大小累加。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmptyTrashResult {
    /// 成功删除的顶层条目数。
    pub deleted: usize,
    /// 删除失败的顶层条目数。
    pub failed: usize,
    /// 回收的逻辑字节数（成功删除条目的大小累加）。
    pub reclaimed_bytes: u64,
}

/// 清空 `~/.Trash`。目录不存在/不可读返回稳定错误码；单条失败计入 `failed` 不中断整体。
pub fn empty_trash() -> Result<EmptyTrashResult, String> {
    let home = crate::core::base::home_dir_opt().ok_or("TRASH_HOME_UNAVAILABLE")?;
    let trash = home.join(".Trash");
    // 二次校验：必须是当前用户 ~/.Trash 的真实目录（非符号链接），杜绝被诱导删到别处。
    let meta = std::fs::symlink_metadata(&trash).map_err(|e| {
        if is_permission_denied(&e) {
            "TRASH_PERMISSION_DENIED"
        } else {
            "TRASH_UNAVAILABLE"
        }
    })?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("TRASH_UNAVAILABLE".into());
    }
    let entries = std::fs::read_dir(&trash).map_err(|e| {
        if is_permission_denied(&e) {
            "TRASH_PERMISSION_DENIED"
        } else {
            "TRASH_UNREADABLE"
        }
    })?;

    let mut deleted = 0usize;
    let mut failed = 0usize;
    let mut reclaimed_bytes = 0u64;
    for entry in entries {
        let Ok(entry) = entry else {
            failed += 1;
            continue;
        };
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            failed += 1;
            continue;
        };
        let size = logical_size(&path, &meta);
        // 真实目录 → remove_dir_all（不跟随内部 symlink）；文件/符号链接 → remove_file（断链）。
        let result = if meta.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match result {
            Ok(()) => {
                deleted += 1;
                reclaimed_bytes = reclaimed_bytes.saturating_add(size);
            }
            Err(e) => {
                failed += 1;
                log::warn!(
                    "[trash-empty] remove {:?} failed: {e}",
                    path.file_name().unwrap_or_default()
                );
            }
        }
    }
    log::info!("[trash-empty] deleted={deleted} failed={failed} reclaimed={reclaimed_bytes}B");
    Ok(EmptyTrashResult {
        deleted,
        failed,
        reclaimed_bytes,
    })
}

/// 条目逻辑大小：文件/符号链接取自身元数据 len；真实目录迭代累加（不跟随符号链接）。
fn logical_size(path: &Path, meta: &std::fs::Metadata) -> u64 {
    if !meta.is_dir() {
        return meta.len();
    }
    let mut total = 0u64;
    let mut stack = vec![path.to_owned()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            // symlink_metadata 对「指向目录的符号链接」返回 is_dir()=false，走 else 只累加链接自身大小。
            let Ok(m) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if m.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(m.len());
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_size_sums_files_without_following_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), vec![0; 100]).unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("b"), vec![0; 50]).unwrap();
        let meta = std::fs::symlink_metadata(dir.path()).unwrap();
        assert_eq!(logical_size(dir.path(), &meta), 150);

        // 外部大文件经符号链接不可被计入（不跟随）。
        let external = tempfile::tempdir().unwrap();
        std::fs::write(external.path().join("big"), vec![0; 1024 * 1024]).unwrap();
        std::os::unix::fs::symlink(external.path(), dir.path().join("link")).unwrap();
        let meta = std::fs::symlink_metadata(dir.path()).unwrap();
        // 仅新增链接自身元数据大小，远小于 1MiB 目标。
        assert!(logical_size(dir.path(), &meta) < 1024 * 1024);
    }

    #[test]
    fn logical_size_file_is_own_len() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f");
        std::fs::write(&f, vec![0; 42]).unwrap();
        let meta = std::fs::symlink_metadata(&f).unwrap();
        assert_eq!(logical_size(&f, &meta), 42);
    }
}
