//! 门 1 · 来源验证：Ed25519（EdDSA）验签。
//!
//! Sparkle 2 的 `sparkle:edSignature`（appcast enclosure 属性，base64 的 64 字节签名）
//! 是对**更新包原始字节**的签名；验证公钥是目标 App `Info.plist` 的 `SUPublicEDKey`
//! （base64 的 32 字节 Ed25519 公钥）。
//!
//! `verify_strict`：拒绝弱公钥/弱签名的严格模式（社区审计要求）。

use base64::Engine as _;

/// 校验 `signature_b64` 是否由 `public_key_b64` 对 `message`（更新包原始字节）签署。
///
/// 任一输入格式非法都会返回可读错误；校验失败返回 `EdDSA 签名校验失败`。
pub fn verify_ed25519(
    public_key_b64: &str,
    message: &[u8],
    signature_b64: &str,
) -> Result<(), String> {
    let engine = base64::engine::general_purpose::STANDARD;

    let key_bytes = engine
        .decode(public_key_b64.trim())
        .map_err(|e| format!("公钥 base64 解码失败: {e}"))?;
    let key_arr: [u8; 32] = key_bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("公钥长度异常: {} 字节（应为 32）", key_bytes.len()))?;
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&key_arr)
        .map_err(|e| format!("公钥无效: {e}"))?;

    let sig_bytes = engine
        .decode(signature_b64.trim())
        .map_err(|e| format!("签名 base64 解码失败: {e}"))?;
    let sig_arr: [u8; 64] = sig_bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("签名长度异常: {} 字节（应为 64）", sig_bytes.len()))?;
    let signature = ed25519_dalek::Signature::from_bytes(&sig_arr);

    verifying_key
        .verify_strict(message, &signature)
        .map_err(|_| "EdDSA 签名校验失败".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn valid_signature_passes() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let msg = b"iTerm2 update package bytes";
        let sig = sk.sign(msg);
        let pk = b64(sk.verifying_key().as_bytes());
        assert!(verify_ed25519(&pk, msg, &b64(&sig.to_bytes())).is_ok());
    }

    #[test]
    fn tampered_message_rejected() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let msg = b"iTerm2 update package bytes";
        let sig = sk.sign(msg);
        let pk = b64(sk.verifying_key().as_bytes());
        let mut tampered = msg.to_vec();
        tampered[0] ^= 0x01; // 翻转 1 字节必须被拒绝
        assert!(verify_ed25519(&pk, &tampered, &b64(&sig.to_bytes())).is_err());
    }

    #[test]
    fn malformed_inputs_rejected() {
        // base64 非法
        assert!(verify_ed25519("not-base64!!", b"m", "AAAA").is_err());
        // 公钥长度不对（16 字节）
        assert!(verify_ed25519(&b64(&[1u8; 16]), b"m", &b64(&[0u8; 64])).is_err());
        // 签名长度不对（32 字节）
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let pk = b64(sk.verifying_key().as_bytes());
        assert!(verify_ed25519(&pk, b"m", &b64(&[0u8; 32])).is_err());
    }

    #[test]
    fn wrong_key_rejected() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let other = SigningKey::from_bytes(&[8u8; 32]);
        let msg = b"payload";
        let sig = sk.sign(msg);
        let wrong_pk = b64(other.verifying_key().as_bytes());
        assert!(verify_ed25519(&wrong_pk, msg, &b64(&sig.to_bytes())).is_err());
    }
}
