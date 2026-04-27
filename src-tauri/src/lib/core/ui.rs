// 对应 ui.sh（非终端交互部分）
// 排除: clear_screen/hide_cursor/show_cursor/read_key/menu/spinner (GUI 前端负责)

use std::path::Path;
use std::sync::OnceLock;

static FDA_CACHE: OnceLock<Option<bool>> = OnceLock::new();

/// 计算字符串的"终端显示宽度":CJK 字符算 2,ASCII 算 1,
/// 零宽连接符与 emoji variation selector 算 0。
///
/// 与 SH 第 21-80 行的实现等价但更精确:Rust 直接用 Unicode 范围,不用 byte/char 估算。
/// 这是 truncate_by_display_width 等函数的基础;前端在按宽度截断 App 名时也会用到。
pub fn get_display_width(s: &str) -> usize {
    if s.is_empty() {
        return 0;
    }
    s.chars().map(char_width).sum()
}

/// 单字符宽度。覆盖 CJK Unified Ideographs / Hangul / Halfwidth-Fullwidth / Misc Symbols
/// / Emoji presentation 等常见的"宽字符"区段。零宽连接符与 VS16 返回 0。
pub fn char_width(c: char) -> usize {
    let code = c as u32;
    // 零宽
    if c == '\u{200D}' || c == '\u{FE0F}' || c == '\u{FE0E}' || c == '\u{200B}' {
        return 0;
    }
    // 控制字符:这里按 0,与终端 echo 行为一致
    if code < 0x20 || (0x7F..=0x9F).contains(&code) {
        return 0;
    }
    // 常见的 East Asian Wide / Fullwidth 区段
    let wide = matches!(
        code,
        0x1100..=0x115F     // Hangul Jamo
        | 0x2329 | 0x232A
        | 0x2E80..=0x303E    // CJK Radicals / Kangxi
        | 0x3041..=0x33FF    // Hiragana / Katakana / Bopomofo / CJK Symbols
        | 0x3400..=0x4DBF    // CJK Extension A
        | 0x4E00..=0x9FFF    // CJK Unified Ideographs
        | 0xA000..=0xA4CF    // Yi
        | 0xAC00..=0xD7A3    // Hangul Syllables
        | 0xF900..=0xFAFF    // CJK Compatibility Ideographs
        | 0xFE30..=0xFE4F    // CJK Compatibility Forms
        | 0xFF00..=0xFF60    // Fullwidth ASCII
        | 0xFFE0..=0xFFE6    // Fullwidth signs
        | 0x1F300..=0x1F64F  // Misc Symbols & Pictographs / Emoticons
        | 0x1F680..=0x1F6FF  // Transport
        | 0x1F900..=0x1F9FF  // Supplemental Symbols & Pictographs
        | 0x20000..=0x2FFFD  // CJK Extension B-F
        | 0x30000..=0x3FFFD
    );
    if wide { 2 } else { 1 }
}

/// 按显示宽度截断字符串。当超长时尾部追加 "…"(本身宽度 1)。对齐 truncate_by_display_width()。
pub fn truncate_by_display_width(s: &str, max_width: usize) -> String {
    let total = get_display_width(s);
    if total <= max_width {
        return s.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    let budget = max_width.saturating_sub(1);
    let mut out = String::new();
    let mut acc = 0usize;
    for c in s.chars() {
        let w = char_width(c);
        if acc + w > budget {
            break;
        }
        acc += w;
        out.push(c);
    }
    out.push('\u{2026}');
    out
}

pub fn has_full_disk_access() -> Option<bool> {
    let cached = FDA_CACHE.get();
    if let Some(val) = cached {
        return *val;
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let protected_dirs: [String; 3] = [
        format!("{home}/Library/Safari/LocalStorage"),
        format!("{home}/Library/Mail/V10"),
        format!("{home}/Library/Messages/chat.db"),
    ];

    let mut tested_count = 0usize;
    let mut accessible_count = 0usize;

    for test_path in &protected_dirs {
        if Path::new(test_path).exists() {
            tested_count += 1;
            if std::fs::metadata(test_path).is_ok() {
                accessible_count += 1;
            }
        }
    }

    let result = if tested_count == 0 {
        None
    } else if accessible_count > 0 {
        Some(true)
    } else {
        Some(false)
    };

    let _ = FDA_CACHE.set(result);
    result
}

pub fn format_last_used_summary(value: &str) -> String {
    match value {
        "" | "Unknown" => "Unknown".to_string(),
        "Never" | "Recent" | "Today" | "Yesterday" | "This year" | "Old" => value.to_string(),
        other => {
            // 优先匹配最长后缀,避免 "month(s) ago" 被 "months ago" 误吃头
            if let Some(n) = extract_number_before(other, "month(s) ago", "") {
                return format!("{n}m ago");
            }
            if let Some(n) = extract_number_before(other, "days ago", "day ago") {
                return format!("{n}d ago");
            }
            if let Some(n) = extract_number_before(other, "weeks ago", "week ago") {
                return format!("{n}w ago");
            }
            if let Some(n) = extract_number_before(other, "months ago", "month ago") {
                return format!("{n}m ago");
            }
            if let Some(n) = extract_number_before(other, "years ago", "year ago") {
                return format!("{n}y ago");
            }
            other.to_string()
        }
    }
}

fn extract_number_before<'a>(
    text: &'a str,
    suffix_plural: &str,
    suffix_singular: &str,
) -> Option<&'a str> {
    for suffix in [suffix_plural, suffix_singular] {
        if suffix.is_empty() {
            continue;
        }
        if let Some(pos) = text.find(suffix) {
            if pos > 0 {
                let prefix = &text[..pos];
                let num = prefix.trim();
                if num.chars().all(|c| c.is_ascii_digit()) {
                    return Some(num);
                }
            }
        }
    }
    None
}
