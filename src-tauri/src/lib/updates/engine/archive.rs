//! zip 解压（带 Zip Slip 防护）。
//!
//! 防护口径（设计 §4 门 2/3 之间的解压步）：
//! - 仅接受 `enclosed_name()` 为 `Some` 的条目（zip crate 内建的路径穿越防护：
//!   拒绝绝对路径与 `..` 组件）；
//! - **再加一层组件级复核**（双保险：只放行 Normal/CurDir 组件）；
//! - 逐条目解压到调用方提供的暂存目录（0700，由 staging 保证）；
//! - 保留 zip 记录的 Unix 权限位（解压出的 .app 需要可执行位）。

use std::path::{Component, Path, PathBuf};

/// 解压 `zip_path` 到 `dest_dir`，返回其中第一个 `.app` 目录（按路径排序取
/// 稳定第一个；正常更新包只有一个）。
pub fn extract_zip_and_find_app(zip_path: &Path, dest_dir: &Path) -> Result<PathBuf, String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("打开更新包失败: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("读取更新包失败: {e}"))?;
    std::fs::create_dir_all(dest_dir).map_err(|e| format!("创建解压目录失败: {e}"))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("读取条目 #{i} 失败: {e}"))?;

        // 双保险①：crate 内建防护（绝对路径 / `..` → None）
        let Some(rel) = entry.enclosed_name() else {
            return Err(format!("更新包含非法路径（Zip Slip）: {}", entry.name()));
        };
        // 双保险②：组件级复核
        if rel
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return Err(format!("更新包含非法路径组件: {}", entry.name()));
        }

        let out_path = dest_dir.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| format!("创建目录失败: {e}"))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建父目录失败: {e}"))?;
        }

        // 符号链接：zip 以 `S_IFLNK` 位 + 内容为链接目标表示（.app 内的
        // `Frameworks/*.framework/Versions/Current` 等）。若按普通文件写出，
        // bundle 的框架链会被破坏 → 解压产物无法通过 Apple 锚定代码签名校验
        //（实测报 OSStatus -67062，iTerm2 包内这类链接有 33 处）。
        if let Some(mode) = entry.unix_mode() {
            if mode & 0o170000 == 0o120000 {
                use std::io::Read as _;
                let mut target = String::new();
                entry
                    .read_to_string(&mut target)
                    .map_err(|e| format!("读取符号链接目标失败: {e}"))?;
                let target = target.trim();
                if target.is_empty() {
                    return Err(format!("更新包含空目标符号链接: {}", entry.name()));
                }
                // 深化防御：目标须词法上不逃逸解压根（EdDSA 前置已从源头挡住
                // 恶意包——能走到解压的包已通过官方签名；这里再堵
                //「symlink + 后续条目穿越」的理论攻击面）。
                let link_parent = out_path.parent().unwrap_or(dest_dir);
                if !lexically_within(dest_dir, &link_parent.join(target)) {
                    return Err(format!(
                        "更新包含逃逸符号链接（已拒绝）: {} -> {}",
                        entry.name(),
                        target
                    ));
                }
                let _ = std::fs::remove_file(&out_path);
                std::os::unix::fs::symlink(target, &out_path)
                    .map_err(|e| format!("创建符号链接失败: {e}"))?;
                continue;
            }
        }

        let mut out_file =
            std::fs::File::create(&out_path).map_err(|e| format!("写入文件失败: {e}"))?;
        std::io::copy(&mut entry, &mut out_file).map_err(|e| format!("解压写入失败: {e}"))?;

        // 保留 zip 记录的权限位（.app 内的可执行文件必需）。
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode));
        }
    }

    let mut apps = Vec::new();
    collect_apps(dest_dir, 0, &mut apps);
    apps.sort();
    apps.into_iter()
        .next()
        .ok_or_else(|| "更新包中未找到 .app".to_string())
}

/// 词法包含判断（不访问文件系统）：折叠 `..`/`.` 后判断 candidate 是否落在
/// base 之内。用于拒绝逃逸符号链接目标。
fn lexically_within(base: &Path, candidate: &Path) -> bool {
    fn normalize(p: &Path) -> PathBuf {
        let mut out = PathBuf::new();
        for c in p.components() {
            match c {
                Component::ParentDir => {
                    out.pop();
                }
                Component::CurDir => {}
                other => out.push(other.as_os_str()),
            }
        }
        out
    }
    normalize(candidate).starts_with(normalize(base))
}

/// 收集深度 ≤3 的 `.app` 目录（更新包通常根层即单个 .app，少数包一层目录）。
fn collect_apps(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) == Some("app") {
            out.push(path);
        } else {
            collect_apps(&path, depth + 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn build_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        for (name, content) in entries {
            writer.start_file(*name, opts).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap();
    }

    /// 含符号链接的最小 zip fixture（系统 `zip -y` 生成：
    /// `Test.app/Contents/Versions/Current -> A`，另含 `A/file` 普通文件）。
    ///
    /// 注：zip crate writer 的 `unix_permissions()` 仅保留低 9 位权限、**丢弃
    /// 文件类型位**（其文档明确无法表示 symlink），因此这里的 symlink 用例
    /// 必须用真实工具生成的字节做 fixture。
    const SYMLINK_ZIP_B64: &str = "UEsDBAoAAAAAADtlRV0AAAAAAAAAAAAAAAAJABwAVGVzdC5hcHAvVVQJAAOSKsNqkirDanV4CwABBPUBAAAEAAAAAFBLAwQKAAAAAAA7ZUVdAAAAAAAAAAAAAAAAEgAcAFRlc3QuYXBwL0NvbnRlbnRzL1VUCQADkirDapIqw2p1eAsAAQT1AQAABAAAAABQSwMECgAAAAAAO2VFXQAAAAAAAAAAAAAAABsAHABUZXN0LmFwcC9Db250ZW50cy9WZXJzaW9ucy9VVAkAA5Iqw2qSKsNqdXgLAAEE9QEAAAQAAAAAUEsDBAoAAAAAADtlRV0AAAAAAAAAAAAAAAAdABwAVGVzdC5hcHAvQ29udGVudHMvVmVyc2lvbnMvQS9VVAkAA5Iqw2qSKsNqdXgLAAEE9QEAAAQAAAAAUEsDBAoAAAAAADtlRV1j8/OtBAAAAAQAAAAhABwAVGVzdC5hcHAvQ29udGVudHMvVmVyc2lvbnMvQS9maWxlVVQJAAOSKsNqkirDanV4CwABBPUBAAAEAAAAAGRhdGFQSwMECgAAAAAAO2VFXYue2dMBAAAAAQAAACIAHABUZXN0LmFwcC9Db250ZW50cy9WZXJzaW9ucy9DdXJyZW50VVQJAAOSKsNqkirDanV4CwABBPUBAAAEAAAAAEFQSwECHgMKAAAAAAA7ZUVdAAAAAAAAAAAAAAAACQAYAAAAAAAAABAA7UEAAAAAVGVzdC5hcHAvVVQFAAOSKsNqdXgLAAEE9QEAAAQAAAAAUEsBAh4DCgAAAAAAO2VFXQAAAAAAAAAAAAAAABIAGAAAAAAAAAAQAO1BQwAAAFRlc3QuYXBwL0NvbnRlbnRzL1VUBQADkirDanV4CwABBPUBAAAEAAAAAFBLAQIeAwoAAAAAADtlRV0AAAAAAAAAAAAAAAAbABgAAAAAAAAAEADtQY8AAABUZXN0LmFwcC9Db250ZW50cy9WZXJzaW9ucy9VVAUAA5Iqw2p1eAsAAQT1AQAABAAAAABQSwECHgMKAAAAAAA7ZUVdAAAAAAAAAAAAAAAAHQAYAAAAAAAAABAA7UHkAAAAVGVzdC5hcHAvQ29udGVudHMvVmVyc2lvbnMvQS9VVAUAA5Iqw2p1eAsAAQT1AQAABAAAAABQSwECHgMKAAAAAAA7ZUVdY/PzrQQAAAAEAAAAIQAYAAAAAAABAAAApIE7AQAAVGVzdC5hcHAvQ29udGVudHMvVmVyc2lvbnMvQS9maWxlVVQFAAOSKsNqdXgLAAEE9QEAAAQAAAAAUEsBAh4DCgAAAAAAO2VFXYue2dMBAAAAAQAAACIAGAAAAAAAAAAAAO2hmgEAAFRlc3QuYXBwL0NvbnRlbnRzL1ZlcnNpb25zL0N1cnJlbnRVVAUAA5Iqw2p1eAsAAQT1AQAABAAAAABQSwUGAAAAAAYABgA6AgAA9wEAAAAA";

    /// 含逃逸符号链接的 fixture（`Test.app/escape -> ../../../../etc`）。
    const ESCAPE_SYMLINK_ZIP_B64: &str = "UEsDBAoAAAAAADtlRV0AAAAAAAAAAAAAAAAJABwAVGVzdC5hcHAvVVQJAAOSKsNqkirDanV4CwABBPUBAAAEAAAAAFBLAwQKAAAAAAA7ZUVdTs3gTg8AAAAPAAAADwAcAFRlc3QuYXBwL2VzY2FwZVVUCQADkirDapIqw2p1eAsAAQT1AQAABAAAAAAuLi8uLi8uLi8uLi9ldGNQSwMECgAAAAAAO2VFXQAAAAAAAAAAAAAAABIAHABUZXN0LmFwcC9Db250ZW50cy9VVAkAA5Iqw2qSKsNqdXgLAAEE9QEAAAQAAAAAUEsDBAoAAAAAADtlRV1VeTNJCAAAAAgAAAAcABwAVGVzdC5hcHAvQ29udGVudHMvSW5mby5wbGlzdFVUCQADkirDapIqw2p1eAsAAQT1AQAABAAAAAA8cGxpc3QvPlBLAQIeAwoAAAAAADtlRV0AAAAAAAAAAAAAAAAJABgAAAAAAAAAEADtQQAAAABUZXN0LmFwcC9VVAUAA5Iqw2p1eAsAAQT1AQAABAAAAABQSwECHgMKAAAAAAA7ZUVdTs3gTg8AAAAPAAAADwAYAAAAAAAAAAAA7aFDAAAAVGVzdC5hcHAvZXNjYXBlVVQFAAOSKsNqdXgLAAEE9QEAAAQAAAAAUEsBAh4DCgAAAAAAO2VFXQAAAAAAAAAAAAAAABIAGAAAAAAAAAAQAO1BmwAAAFRlc3QuYXBwL0NvbnRlbnRzL1VUBQADkirDanV4CwABBPUBAAAEAAAAAFBLAQIeAwoAAAAAADtlRV1VeTNJCAAAAAgAAAAcABgAAAAAAAEAAACkgecAAABUZXN0LmFwcC9Db250ZW50cy9JbmZvLnBsaXN0VVQFAAOSKsNqdXgLAAEE9QEAAAQAAAAAUEsFBgAAAAAEAAQAXgEAAEUBAAAAAA==";

    fn write_fixture_b64(dir: &Path, name: &str, b64: &str) -> PathBuf {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("fixture base64 解码");
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn extracts_normal_zip_and_finds_app() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("update.zip");
        build_zip(
            &zip_path,
            &[
                ("Test.app/Contents/Info.plist", b"<plist/>"),
                ("Test.app/Contents/MacOS/Test", b"binary"),
            ],
        );
        let dest = tmp.path().join("extracted");
        let app = extract_zip_and_find_app(&zip_path, &dest).expect("应解出 Test.app");
        assert_eq!(app.file_name().unwrap(), "Test.app");
        assert!(app.join("Contents/Info.plist").is_file());
    }

    #[test]
    fn symlink_entries_are_recreated_as_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = write_fixture_b64(tmp.path(), "sym.zip", SYMLINK_ZIP_B64);
        let dest = tmp.path().join("extracted");
        let app = extract_zip_and_find_app(&zip_path, &dest).expect("应解出 Test.app");
        let link = app.join("Contents/Versions/Current");
        let meta = std::fs::symlink_metadata(&link).unwrap();
        assert!(
            meta.file_type().is_symlink(),
            "Current 应为符号链接（普通文件 = 解压器未重建链接，会破坏 .app 签名）"
        );
        assert_eq!(std::fs::read_link(&link).unwrap(), PathBuf::from("A"));
        // 链接可用性：经链接读到真实文件内容
        assert_eq!(std::fs::read_to_string(link.join("file")).unwrap(), "data");
    }

    #[test]
    fn escaping_symlink_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = write_fixture_b64(tmp.path(), "evil-sym.zip", ESCAPE_SYMLINK_ZIP_B64);
        let dest = tmp.path().join("extracted");
        let r = extract_zip_and_find_app(&zip_path, &dest);
        assert!(r.is_err(), "逃逸符号链接必须被拒绝: {r:?}");
    }

    #[test]
    fn zip_slip_entry_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("evil.zip");
        build_zip(
            &zip_path,
            &[
                ("Test.app/Contents/Info.plist", b"<plist/>"),
                ("../evil.txt", b"pwned"),
            ],
        );
        let dest = tmp.path().join("extracted");
        let r = extract_zip_and_find_app(&zip_path, &dest);
        assert!(r.is_err(), "含 ../ 条目的包必须被拒绝: {r:?}");
        assert!(!tmp.path().join("evil.txt").exists(), "不得写出目标目录之外");
    }

    #[test]
    fn zip_without_app_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("noapp.zip");
        build_zip(&zip_path, &[("readme.txt", b"nothing")]);
        let dest = tmp.path().join("extracted");
        assert!(extract_zip_and_find_app(&zip_path, &dest).is_err());
    }

    /// 真实包离线演练（本机手动跑，回归 -67062 事故）：
    /// `cargo test --lib real_iterm2_zip -- --ignored --nocapture`
    ///
    /// 用真实 iTerm2 更新包（含 33 个符号链接）验证「解压 → Apple 锚定代码
    /// 签名校验」全链路——若解压破坏 bundle 结构（如 symlink 未重建），
    /// `inspect` 会以 -67062 失败，本测试即回归防线。
    #[test]
    #[ignore = "需 /tmp/iterm2_probe.zip（真实包离线演练）；手动运行"]
    fn real_iterm2_zip_extract_passes_apple_anchor() {
        let zip_path = std::path::Path::new("/tmp/iterm2_probe.zip");
        assert!(
            zip_path.exists(),
            "缺少 /tmp/iterm2_probe.zip（真实 iTerm2 更新包）"
        );
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("extracted");
        let app = extract_zip_and_find_app(zip_path, &dest).expect("解压 iTerm2 包");

        // 符号链接关键路径抽查（-67062 事故的根因点）
        let link = app.join("Contents/Frameworks/Sparkle.framework/Versions/Current");
        let meta = std::fs::symlink_metadata(&link).expect("framework 链接应存在");
        assert!(meta.file_type().is_symlink(), "framework 链接必须被重建为符号链接");

        // 门 4：Apple 锚定代码签名校验（-67062 的判定环节）
        let info = crate::updates::engine::verify::codesign::inspect(app.to_str().unwrap())
            .expect("iTerm2 包必须通过 Apple 锚定校验");
        assert_eq!(info.bundle_id.as_deref(), Some("com.googlecode.iterm2"));
        eprintln!(
            "OK: {} v{:?} team={:?}",
            info.bundle_id.as_deref().unwrap_or("?"),
            info.version,
            info.team_identifier
        );
    }
}
