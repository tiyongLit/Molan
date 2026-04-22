// 前后端共享常量（单一事实来源）。
//
// 本文件中的常量会由 `src-tauri/build.rs` 读取并生成 `src/constants/shared.ts`：
// - 后端通过 `crate::constants::*` 引用
// - 前端通过 `@/constants/shared` 引用
// 修改本文件后需重新构建（cargo build / tauri dev），`shared.ts` 会随之重新生成。

/// 字节换算基数：统一使用二进制 1000 进制（KB/MB/GB/TB），前后端一致。
pub const SIZE_BASE: u64 = 1000;

// ── Analyze EntryRow 副标题：文件类型描述表 ──────────────────────────────
//
// 对标 lemon-cleaner 的 UTI 本地化描述方案（NSWorkspace.localizedDescriptionForType），
// 改为自建静态表保证前后端一致性与确定性（系统 UTI 对 rar/mkv 等返回动态 UTI → 空白，
// 且 7z/flac 等 UTI 依赖本机第三方软件注册，行为不一致）。
//
// 约束（build.rs 直接拼接生成 TS，不做转义）：
// - key 必须全小写、唯一，不含引号/反斜杠
// - value 为中文文案，不含单引号/反斜杠

/// 文件扩展名（小写、不含点）→ 类型描述。
/// 查找 miss 时前端走 fallback（空白，对齐柠檬"未知类型不显示"）。
pub const FILE_TYPE_DESCS: &[(&str, &str)] = &[
    // ── 归档 / 压缩 / 镜像 / 安装包 ──
    ("zip", "ZIP 归档"),
    ("rar", "RAR 压缩包"),
    ("7z", "7-Zip 归档"),
    ("tar", "TAR 归档"),
    ("gz", "GZip 归档"),
    ("bz2", "BZip2 归档"),
    ("xz", "XZ 归档"),
    ("iso", "ISO 磁盘映像"),
    ("dmg", "磁盘映像"),
    ("pkg", "安装器软件包"),
    ("ipa", "iOS 应用包"),
    ("apk", "Android 应用包"),
    // ── 图片 ──
    ("png", "PNG 图像"),
    ("jpg", "JPEG 图像"),
    ("jpeg", "JPEG 图像"),
    ("heic", "HEIF 图像"),
    ("gif", "GIF 图像"),
    ("svg", "SVG 图像"),
    ("tiff", "TIFF 图像"),
    ("webp", "WebP 图像"),
    ("psd", "Photoshop 文档"),
    ("icns", "图标文件"),
    // ── 视频 ──
    ("mp4", "MPEG-4 影片"),
    ("mov", "QuickTime 影片"),
    ("mkv", "Matroska 视频"),
    ("avi", "AVI 影片"),
    ("wmv", "Windows Media 视频"),
    ("m4v", "iTunes 视频"),
    ("webm", "WebM 视频"),
    // ── 音频 ──
    ("mp3", "MP3 音频"),
    ("wav", "波形音频"),
    ("flac", "FLAC 无损音频"),
    ("aac", "AAC 音频"),
    ("m4a", "MPEG-4 音频"),
    ("aiff", "AIFF 音频"),
    // ── 文档 ──
    ("pdf", "PDF 文档"),
    ("doc", "Word 文档"),
    ("docx", "Word 文档"),
    ("xls", "Excel 电子表格"),
    ("xlsx", "Excel 电子表格"),
    ("ppt", "PowerPoint 演示文稿"),
    ("pptx", "PowerPoint 演示文稿"),
    ("txt", "纯文本"),
    ("md", "Markdown 文档"),
    ("epub", "EPUB 电子书"),
    // ── 开发 / 数据 / 字体 ──
    ("dylib", "Mach-O 动态库"),
    ("jar", "Java 归档"),
    ("sqlite", "SQLite 数据库"),
    ("db", "数据库文件"),
    ("ttf", "TrueType 字体"),
    ("otf", "OpenType 字体"),
    ("exe", "Windows 可执行文件"),
];

/// 目录名后缀（小写、含点）→ 类型描述。
/// .app/.bundle 等本质是目录（保留钻入能力），副标题按后缀查此表，
/// 命中时替代普通目录的"X 项"统计文案（对标柠檬 specialFileExtensions 的展示效果）。
pub const BUNDLE_TYPE_DESCS: &[(&str, &str)] = &[
    (".app", "应用程序"),
    (".bundle", "资源束"),
    (".framework", "框架"),
    (".simruntime", "模拟器运行时"),
    (".dsym", "调试符号"),
    (".xcodeproj", "Xcode 工程"),
    (".xcworkspace", "Xcode 工作区"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// key 约束校验：全小写、非空、不含引号与反斜杠（build.rs 拼接 TS 不做转义的前提）。
    #[test]
    fn type_desc_keys_are_safe_for_ts_generation() {
        for (table, name) in [
            (FILE_TYPE_DESCS, "FILE_TYPE_DESCS"),
            (BUNDLE_TYPE_DESCS, "BUNDLE_TYPE_DESCS"),
        ] {
            for (k, v) in table {
                assert!(!k.is_empty(), "{name} 存在空 key");
                assert!(
                    k.chars().all(|c| c.is_ascii_lowercase()
                        || c.is_ascii_digit()
                        || c == '.'
                        || c == '-'),
                    "{name} key 必须全小写（允许数字/./-）: {k}"
                );
                assert!(
                    !v.contains('\'') && !v.contains('\\') && !v.contains('"'),
                    "{name} value 不得含引号/反斜杠: {v}"
                );
            }
        }
    }

    /// key 唯一性校验：重复 key 会让查表结果不确定（TS Record 后者覆盖前者）。
    #[test]
    fn type_desc_keys_are_unique() {
        for (table, name) in [
            (FILE_TYPE_DESCS, "FILE_TYPE_DESCS"),
            (BUNDLE_TYPE_DESCS, "BUNDLE_TYPE_DESCS"),
        ] {
            let mut seen = std::collections::HashSet::new();
            for (k, _) in table {
                assert!(seen.insert(*k), "{name} 存在重复 key: {k}");
            }
        }
    }

    /// 两张表语义隔离：文件表不含点前缀，目录表必须以点开头。
    #[test]
    fn file_table_and_bundle_table_are_disjoint_domains() {
        for (k, _) in FILE_TYPE_DESCS {
            assert!(!k.starts_with('.'), "FILE_TYPE_DESCS key 不应以点开头: {k}");
        }
        for (k, _) in BUNDLE_TYPE_DESCS {
            assert!(
                k.starts_with('.'),
                "BUNDLE_TYPE_DESCS key 必须以点开头: {k}"
            );
        }
    }
}
