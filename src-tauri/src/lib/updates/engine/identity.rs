//! 门 5 · 身份一致性：装入的新 bundle 必须与旧 bundle 属于同一声明身份。
//!
//! 对齐 Burrow `BundleUpdateIdentity` 的比对规则（P0 子集）：
//! - `bundle_id` 必须双方存在且一致；
//! - `signing_identifier` 必须双方存在且一致；
//! - `team_identifier` 必须双方存在且一致——P0 仅面向第三方 Developer ID
//!   App（必然携带 team）；任一方缺失都视为「不可验证」并拒绝，不做宽免。
//!
//! inode / 文件身份（防"检查后被掉包"）由 installer 在替换瞬间另行校验，
//! 不属于本模块职责。

use super::verify::codesign::CodeSignatureInfo;

/// 校验新 bundle 与旧 bundle 身份一致。
/// 返回 `Err` 说明不具备替换资格——调用方按铁律丢弃，不得降级安装。
pub fn ensure_same_identity(
    old: &CodeSignatureInfo,
    new: &CodeSignatureInfo,
) -> Result<(), String> {
    let must_match = |name: &str, a: &Option<String>, b: &Option<String>| -> Result<(), String> {
        match (a, b) {
            (Some(x), Some(y)) if x == y => Ok(()),
            _ => Err(format!(
                "{name} 不一致或缺失（旧: {a:?}，新: {b:?}）——拒绝替换"
            )),
        }
    };
    must_match("bundle id", &old.bundle_id, &new.bundle_id)?;
    must_match("签名标识", &old.signing_identifier, &new.signing_identifier)?;
    must_match("团队标识", &old.team_identifier, &new.team_identifier)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(bundle: &str, sign: &str, team: &str) -> CodeSignatureInfo {
        CodeSignatureInfo {
            bundle_id: Some(bundle.to_string()),
            version: Some("1.0".to_string()),
            build: Some("1".to_string()),
            signing_identifier: Some(sign.to_string()),
            team_identifier: Some(team.to_string()),
            code_directory_hash: Some(vec![1, 2, 3]),
        }
    }

    #[test]
    fn identical_identity_passes() {
        let a = sample("com.example.app", "com.example.app", "TEAM123");
        let b = sample("com.example.app", "com.example.app", "TEAM123");
        assert!(ensure_same_identity(&a, &b).is_ok());
    }

    #[test]
    fn bundle_id_mismatch_fails() {
        let a = sample("com.example.app", "com.example.app", "TEAM123");
        let b = sample("com.evil.app", "com.example.app", "TEAM123");
        assert!(ensure_same_identity(&a, &b).is_err());
    }

    #[test]
    fn signing_identifier_mismatch_fails() {
        let a = sample("com.example.app", "com.example.app", "TEAM123");
        let b = sample("com.example.app", "com.other.app", "TEAM123");
        assert!(ensure_same_identity(&a, &b).is_err());
    }

    #[test]
    fn team_mismatch_or_missing_fails() {
        let a = sample("com.example.app", "com.example.app", "TEAM123");
        let b = sample("com.example.app", "com.example.app", "TEAM999");
        assert!(ensure_same_identity(&a, &b).is_err());

        let mut missing = sample("com.example.app", "com.example.app", "TEAM123");
        missing.team_identifier = None;
        assert!(ensure_same_identity(&a, &missing).is_err());
        assert!(ensure_same_identity(&missing, &a).is_err());
    }

    #[test]
    fn missing_bundle_id_fails() {
        let a = sample("com.example.app", "com.example.app", "TEAM123");
        let mut missing = sample("com.example.app", "com.example.app", "TEAM123");
        missing.bundle_id = None;
        assert!(ensure_same_identity(&a, &missing).is_err());
    }
}
