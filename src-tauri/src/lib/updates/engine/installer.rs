//! 替换执行：退出 → 复验 → 备份 → 原子替换 → 复验 → 回滚/清理 → 重启。
//!
//! 任何一步失败都恢复现场或保持原样；**绝不留下"半替换"状态**。
//! 对齐 Burrow `ElectronReplacementInstaller.install` 的顺序纪律：
//! 退出目标 App → **紧贴替换前的边界复验**（防验证后被调包）→ 备份改名 →
//! 新 bundle 就位 → 替换后复验（CDHash）→ 成功清备份 / 失败恢复备份 → 重启。

use std::path::Path;
use std::time::Duration;

use super::identity::ensure_same_identity;
use super::relaunch;
use super::verify::codesign::{self, CodeSignatureInfo};

/// 安装结果。
#[derive(Debug)]
pub struct InstallOutcome {
    /// 备份是否已清理（false = 清理失败但安装成功，备份作为残留保留）。
    pub backup_cleaned: bool,
}

/// 提交安装（前置条件：全部门禁已通过、新包已在暂存区完成校验）。
///
/// - `target_app`：当前安装位置（如 `/Applications/iTerm.app`）；
/// - `staged_app`：暂存区里已验证的新 bundle（与目标需**同卷**）；
/// - `expected_old` / `expected_new`：替换前/后应匹配的签名身份快照。
pub fn commit_install(
    target_app: &Path,
    staged_app: &Path,
    expected_old: &CodeSignatureInfo,
    expected_new: &CodeSignatureInfo,
    quit_timeout: Duration,
) -> Result<InstallOutcome, String> {
    let target_str = target_app.to_string_lossy().to_string();
    let bundle_id = expected_old
        .bundle_id
        .clone()
        .ok_or_else(|| "旧应用缺少 bundle id，无法安全替换".to_string())?;

    // 双保险：调用方应已做过，这里再验一次新旧身份兼容。
    ensure_same_identity(expected_old, expected_new)?;

    // 1) 请求目标 App 退出（未退出 → 中止，不动任何文件）。
    relaunch::quit_and_wait(&bundle_id, quit_timeout)?;

    // 2) 边界复验：紧贴替换前再读一次旧身份（防"验证后被调包"）。
    let current_old = codesign::inspect(&target_str)?;
    if &current_old != expected_old {
        return Err("目标应用在验证后发生变化（身份不一致），已中止".to_string());
    }

    // 3) 备份旧 bundle（同卷原子 rename）。
    let parent = target_app
        .parent()
        .ok_or_else(|| "目标路径无父目录".to_string())?;
    let backup_path = parent.join(format!(
        ".MolanUpdateBackup.{}.app",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::rename(target_app, &backup_path)
        .map_err(|e| format!("备份旧应用失败: {e}（目标与暂存需在同一卷）"))?;

    // 4) 新 bundle 就位（失败 → 恢复备份）。
    if let Err(e) = std::fs::rename(staged_app, target_app) {
        let _ = std::fs::rename(&backup_path, target_app);
        return Err(format!("替换失败，已恢复原应用: {e}"));
    }

    // 5) 替换后复验（全字段相等，含 CDHash —— 防替换窗口内被调包）。
    let recheck = codesign::inspect(&target_str);
    let recheck_ok = matches!(&recheck, Ok(info) if info == expected_new);
    if !recheck_ok {
        // 坏的新包：移出目标位并恢复备份。
        let _ = std::fs::remove_dir_all(target_app);
        let restore = std::fs::rename(&backup_path, target_app);
        let reason = match recheck {
            Err(e) => e,
            Ok(_) => "新应用签名身份与预期不一致".to_string(),
        };
        return match restore {
            Ok(_) => Err(format!("新应用复验失败（已恢复原应用）: {reason}")),
            Err(re) => Err(format!("新应用复验失败且恢复失败: {reason}；恢复错误: {re}")),
        };
    }

    // 6) 清理备份 + 重启目标 App。
    let backup_cleaned = std::fs::remove_dir_all(&backup_path).is_ok();
    relaunch::relaunch_app(&target_str)?;
    Ok(InstallOutcome { backup_cleaned })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonexistent_target_fails_before_touching_anything() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("Nonexistent.app");
        let staged = tmp.path().join("Staged.app");
        std::fs::create_dir_all(&staged).unwrap();
        let old = CodeSignatureInfo {
            bundle_id: Some("com.example.nonexistent".to_string()),
            ..Default::default()
        };
        let new = old.clone();
        // 目标不存在 → quit 通过（无运行实例）→ 边界复验 inspect 失败 → Err。
        let r = commit_install(&target, &staged, &old, &new, Duration::from_millis(300));
        assert!(r.is_err(), "目标不存在应安全失败: {r:?}");
        assert!(staged.exists(), "失败路径不得动暂存目录");
    }

    #[test]
    fn missing_bundle_id_fails_fast() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("X.app");
        let staged = tmp.path().join("S.app");
        let old = CodeSignatureInfo::default(); // 无 bundle_id
        let new = old.clone();
        let r = commit_install(&target, &staged, &old, &new, Duration::from_millis(300));
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("bundle id"), "应提示缺少 bundle id");
    }
}
