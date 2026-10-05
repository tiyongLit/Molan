//! 门 4 · 落地验证：SecStaticCode FFI（Apple 锚定 + 身份信息提取）。
//!
//! 对标 Burrow `BundleUpdateIdentity.read`：
//! - requirement `anchor apple generic`：证书链必须锚定 **Apple 签发的证书**。
//!   仅比对 Team ID 字符串不足以防伪造——自签名证书可随意声称任意 OU/Team 值，
//!   只有 Apple 签发的证书链能通过该 requirement（Burrow 源码同款论证）；
//! - `kSecCSCheckAllArchitectures`：对通用二进制**逐架构**校验（防篡改备用切片）；
//! - `SecCodeCopySigningInformation`：提取 signingIdentifier / teamIdentifier /
//!   CDHash / 被封印 Info.plist 的 bundle 元数据（bundle_id / version / build）。
//!
//! 红线豁免依据：Rust 直调 macOS 系统 API（Security.framework），不 spawn 外部
//! 二进制、不引入 ObjC/Swift 源码（与 `macos_mditem.rs` 同属项目既定 FFI 模式）。
//!
//! 已知边界：`SecStaticCodeCheckValidity` 依赖系统信任评估服务（trustd），
//! 服务异常时可能变慢——调用方在 `spawn_blocking` 中执行并自行加超时保护。

/// 代码签名身份快照（替换一致性与回滚判定的核心数据结构）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CodeSignatureInfo {
    /// CFBundleIdentifier（来自被封印的 Info.plist）
    pub bundle_id: Option<String>,
    /// CFBundleShortVersionString
    pub version: Option<String>,
    /// CFBundleVersion（build 号）
    pub build: Option<String>,
    /// 签名标识（kSecCodeInfoIdentifier）
    pub signing_identifier: Option<String>,
    /// 团队标识（kSecCodeInfoTeamIdentifier；Apple 平台签名可能缺失）
    pub team_identifier: Option<String>,
    /// 代码目录哈希（kSecCodeInfoUnique，CDHash 原始字节）
    pub code_directory_hash: Option<Vec<u8>>,
}

/// CDHash 的小写 hex 表示（与 `codesign -dv` 输出格式一致，用于对照测试/日志）。
pub fn cdhash_hex(hash: &[u8]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// 「证书链锚定 Apple」要求文本（与 Burrow `appleAnchoredRequirement` 同口径）。
#[cfg(target_os = "macos")]
const APPLE_ANCHOR_REQUIREMENT: &str = "anchor apple generic";

/// 对 bundle 执行「签名有效 + 证书链锚定 Apple」校验，并提取身份信息。
///
/// 任何一步失败（创建对象 / 锚定校验 / 读取签名信息）都返回 `Err`——
/// 调用方按铁律丢弃暂存，**不得降级**为"跳过验证继续安装"。
#[cfg(target_os = "macos")]
pub fn inspect(app_path: &str) -> Result<CodeSignatureInfo, String> {
    use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
    use core_foundation::url::CFURL;

    let url = CFURL::from_path(app_path, true).ok_or_else(|| format!("路径无效: {app_path}"))?;
    let mut code: ffi::SecStaticCodeRef = std::ptr::null_mut();
    let status = unsafe {
        ffi::SecStaticCodeCreateWithPath(
            url.as_concrete_TypeRef(),
            ffi::K_SEC_CS_DEFAULT_FLAGS,
            &mut code,
        )
    };
    if status != ffi::ERR_SEC_SUCCESS || code.is_null() {
        return Err(format!("创建代码对象失败（OSStatus {status}）: {app_path}"));
    }

    // 借用期内做全部校验与提取；出口统一释放（CF "Create" 规则配对）。
    let result = unsafe { inspect_borrowed(code) };
    unsafe { CFRelease(code as CFTypeRef) };
    result
}

#[cfg(not(target_os = "macos"))]
pub fn inspect(_app_path: &str) -> Result<CodeSignatureInfo, String> {
    Err("代码签名校验仅支持 macOS".to_string())
}

/// 校验 + 提取（借用已创建的 static code，不负责释放）。
#[cfg(target_os = "macos")]
unsafe fn inspect_borrowed(code: ffi::SecStaticCodeRef) -> Result<CodeSignatureInfo, String> {
    use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
    use core_foundation::string::CFString;

    // ① 创建「锚定 Apple」要求并做全架构校验。
    let requirement_text = CFString::new(APPLE_ANCHOR_REQUIREMENT);
    let mut requirement: ffi::SecRequirementRef = std::ptr::null_mut();
    let status = unsafe {
        ffi::SecRequirementCreateWithString(
            requirement_text.as_concrete_TypeRef(),
            ffi::K_SEC_CS_DEFAULT_FLAGS,
            &mut requirement,
        )
    };
    if status != ffi::ERR_SEC_SUCCESS || requirement.is_null() {
        return Err(format!("创建验证要求失败（OSStatus {status}）"));
    }
    let status = unsafe {
        ffi::SecStaticCodeCheckValidity(
            code,
            ffi::K_SEC_CS_CHECK_ALL_ARCHITECTURES,
            requirement,
        )
    };
    unsafe { CFRelease(requirement as CFTypeRef) };
    if status != ffi::ERR_SEC_SUCCESS {
        return Err(format!("签名未通过 Apple 锚定校验（OSStatus {status}）"));
    }

    // ② 提取签名身份信息。
    let mut info: ffi::CFDictionaryRef = std::ptr::null();
    let status = unsafe {
        ffi::SecCodeCopySigningInformation(code, ffi::K_SEC_CS_SIGNING_INFORMATION, &mut info)
    };
    if status != ffi::ERR_SEC_SUCCESS || info.is_null() {
        return Err(format!("读取签名信息失败（OSStatus {status}）"));
    }
    let extracted = unsafe { extract_signing_info(info) };
    unsafe { CFRelease(info as CFTypeRef) };
    Ok(extracted)
}

/// 从签名信息字典提取字段（借用字典，不释放）。
#[cfg(target_os = "macos")]
unsafe fn extract_signing_info(dict: ffi::CFDictionaryRef) -> CodeSignatureInfo {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;

    let signing_identifier = unsafe { dict_string(dict, ffi::kSecCodeInfoIdentifier) };
    let team_identifier = unsafe { dict_string(dict, ffi::kSecCodeInfoTeamIdentifier) };
    let code_directory_hash = unsafe { dict_data(dict, ffi::kSecCodeInfoUnique) };

    // 被封印的 Info.plist（kSecCodeInfoPList 值为 CFDictionary）。
    let mut bundle_id = None;
    let mut version = None;
    let mut build = None;
    let plist_value =
        unsafe { ffi::CFDictionaryGetValue(dict, ffi::kSecCodeInfoPList as *const _) };
    if !plist_value.is_null() {
        let plist = plist_value as ffi::CFDictionaryRef;
        let key_id = CFString::new("CFBundleIdentifier");
        let key_version = CFString::new("CFBundleShortVersionString");
        let key_build = CFString::new("CFBundleVersion");
        bundle_id = unsafe { dict_string(plist, key_id.as_concrete_TypeRef()) };
        version = unsafe { dict_string(plist, key_version.as_concrete_TypeRef()) };
        build = unsafe { dict_string(plist, key_build.as_concrete_TypeRef()) };
    }

    CodeSignatureInfo {
        bundle_id,
        version,
        build,
        signing_identifier,
        team_identifier,
        code_directory_hash,
    }
}

/// 字典取 CFString 值（Get 规则：`wrap_under_get_rule` 配对 drop 释放）。
#[cfg(target_os = "macos")]
unsafe fn dict_string(
    dict: ffi::CFDictionaryRef,
    key: core_foundation::string::CFStringRef,
) -> Option<String> {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;

    let value = unsafe { ffi::CFDictionaryGetValue(dict, key as *const _) };
    if value.is_null() {
        return None;
    }
    Some(unsafe { CFString::wrap_under_get_rule(value as _) }.to_string())
}

/// 字典取 CFData 值（CDHash 等）。
#[cfg(target_os = "macos")]
unsafe fn dict_data(
    dict: ffi::CFDictionaryRef,
    key: core_foundation::string::CFStringRef,
) -> Option<Vec<u8>> {
    use core_foundation::base::TCFType;
    use core_foundation::data::CFData;

    let value = unsafe { ffi::CFDictionaryGetValue(dict, key as *const _) };
    if value.is_null() {
        return None;
    }
    let data = unsafe { CFData::wrap_under_get_rule(value as _) };
    Some(data.bytes().to_vec())
}

#[cfg(target_os = "macos")]
mod ffi {
    pub use core_foundation::dictionary::CFDictionaryRef;
    use core_foundation::string::CFStringRef;
    use core_foundation::url::CFURLRef;

    /// Security.framework 不透明类型（SecStaticCodeRef / SecRequirementRef = CFTypeRef 子类）。
    pub type SecStaticCodeRef = *mut std::os::raw::c_void;
    pub type SecRequirementRef = *mut std::os::raw::c_void;
    pub type OSStatus = i32;

    pub const ERR_SEC_SUCCESS: OSStatus = 0;
    pub const K_SEC_CS_DEFAULT_FLAGS: u32 = 0;
    /// `kSecCSCheckAllArchitectures`（SecStaticCode.h: `1 << 0`）
    pub const K_SEC_CS_CHECK_ALL_ARCHITECTURES: u32 = 1 << 0;
    /// `kSecCSSigningInformation`（SecCode.h: `1 << 1`）
    pub const K_SEC_CS_SIGNING_INFORMATION: u32 = 1 << 1;

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        /// kSecCodeInfoPList / Identifier / TeamIdentifier / Unique（CFStringRef 键）
        pub static kSecCodeInfoPList: CFStringRef;
        pub static kSecCodeInfoIdentifier: CFStringRef;
        pub static kSecCodeInfoTeamIdentifier: CFStringRef;
        pub static kSecCodeInfoUnique: CFStringRef;

        pub fn SecStaticCodeCreateWithPath(
            path: CFURLRef,
            flags: u32,
            static_code: *mut SecStaticCodeRef,
        ) -> OSStatus;

        pub fn SecRequirementCreateWithString(
            text: CFStringRef,
            flags: u32,
            requirement: *mut SecRequirementRef,
        ) -> OSStatus;

        pub fn SecStaticCodeCheckValidity(
            static_code: SecStaticCodeRef,
            flags: u32,
            requirement: SecRequirementRef,
        ) -> OSStatus;

        pub fn SecCodeCopySigningInformation(
            code: SecStaticCodeRef,
            flags: u32,
            information: *mut CFDictionaryRef,
        ) -> OSStatus;

        pub fn CFDictionaryGetValue(
            dict: CFDictionaryRef,
            key: *const std::os::raw::c_void,
        ) -> *const std::os::raw::c_void;
    }
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    const TERMINAL: &str = "/System/Applications/Utilities/Terminal.app";

    #[test]
    fn validates_apple_signed_system_app_and_matches_codesign_cli() {
        if !std::path::Path::new(TERMINAL).exists() {
            return; // 非标准系统布局环境跳过
        }
        let info = inspect(TERMINAL).expect("系统 App 应通过 Apple 锚定校验");
        assert_eq!(info.bundle_id.as_deref(), Some("com.apple.Terminal"));
        assert_eq!(info.signing_identifier.as_deref(), Some("com.apple.Terminal"));
        let hash = info.code_directory_hash.expect("应有 CDHash");
        assert!(!hash.is_empty(), "CDHash 不应为空");

        // 与 codesign CLI 对照：Identifier 与 CDHash 必须一致
        let out = std::process::Command::new("/usr/bin/codesign")
            .args(["-dv", "--verbose=4", TERMINAL])
            .output()
            .expect("codesign 应可执行");
        let text = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(
            text.contains("Identifier=com.apple.Terminal"),
            "codesign 输出应含 Identifier: {text}"
        );
        let cdhash_hex_str = cdhash_hex(&hash);
        assert!(
            text.to_lowercase().contains(&cdhash_hex_str),
            "codesign 输出应含同一 CDHash {cdhash_hex_str}: {text}"
        );
    }

    #[test]
    fn adhoc_signed_app_fails_apple_anchor() {
        // 本项目产物（ad-hoc 签名）不得通过「锚定 Apple」校验——反例验证。
        let candidate = "/Applications/Molan.app";
        if !std::path::Path::new(candidate).exists() {
            return; // 未安装则跳过
        }
        let r = inspect(candidate);
        assert!(r.is_err(), "ad-hoc 签名不应通过 Apple 锚定校验: {r:?}");
    }

    #[test]
    fn nonexistent_path_fails() {
        let r = inspect("/nonexistent-molan/probe-9f3a.app");
        assert!(r.is_err());
    }
}
