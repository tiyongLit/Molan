//! 与 `Mole/cmd/analyze/format.go` 对齐的纯格式化函数（无其它业务逻辑）。
//! Go `time.Time` 零值 → `format_unused_time(None)`；否则传 `Some(SystemTime)`。

use super::constants::{
    bar_width, color_blue, color_gray, color_green, color_red, color_reset, color_yellow,
};
use std::time::SystemTime;

pub fn display_path(path: &str) -> String {
    let Some(home) = dirs::home_dir() else {
        return path.to_string();
    };
    let home = home.to_string_lossy();
    if home.is_empty() {
        return path.to_string();
    }
    match path.strip_prefix(home.as_ref()) {
        Some(rest) => format!("~{rest}"),
        None => path.to_string(),
    }
}

/// Go `truncateMiddle`：`[]rune` → `chars()` 收集为 `Vec<char>` 做索引。
pub fn truncate_middle(s: &str, max_width: usize) -> String {
    let runes: Vec<char> = s.chars().collect();
    let current_width = display_width(s);

    if current_width <= max_width {
        return s.to_string();
    }

    if max_width < 10 {
        let mut width = 0usize;
        for (i, r) in runes.iter().enumerate() {
            width += rune_width(*r);
            if width > max_width {
                return runes[..i].iter().collect();
            }
        }
        return s.to_string();
    }

    let target_head_width = (max_width - 3) / 3;
    let target_tail_width = max_width - 3 - target_head_width;

    let mut head_width = 0usize;
    let mut head_idx = 0usize;
    for (i, r) in runes.iter().enumerate() {
        let w = rune_width(*r);
        if head_width + w > target_head_width {
            break;
        }
        head_width += w;
        head_idx = i + 1;
    }

    let mut tail_width = 0usize;
    let mut tail_idx = runes.len();
    let mut i = runes.len();
    while i > 0 {
        i -= 1;
        let w = rune_width(runes[i]);
        if tail_width + w > target_tail_width {
            break;
        }
        tail_width += w;
        tail_idx = i;
    }

    let head: String = runes[..head_idx].iter().collect();
    let tail: String = runes[tail_idx..].iter().collect();
    format!("{head}...{tail}")
}

pub fn format_number(n: i64) -> String {
    if n < 1000 {
        return format!("{n}");
    }
    if n < 1_000_000 {
        return format!("{:.1}k", n as f64 / 1000.0);
    }
    format!("{:.1}M", n as f64 / 1_000_000.0)
}

pub fn humanize_bytes(size: i64) -> String {
    if size < 0 {
        return "0 B".to_string();
    }
    const UNIT: i64 = 1000;
    if size < UNIT {
        return format!("{size} B");
    }
    let mut div = UNIT;
    let mut exp = 0usize;
    let mut n = size / UNIT;
    while n >= UNIT {
        div = div.saturating_mul(UNIT);
        exp += 1;
        n /= UNIT;
    }
    let value = size as f64 / div as f64;
    let ch = "kMGTPE"
        .chars()
        .nth(exp)
        .unwrap_or_else(|| "kMGTPE".chars().last().unwrap());
    format!("{value:.1} {ch}B")
}

pub fn colored_progress_bar(value: i64, max_value: i64, percent: f64) -> String {
    if max_value <= 0 {
        return format!("{}{}{}", color_gray, "░".repeat(bar_width), color_reset);
    }

    let filled = std::cmp::min(
        ((value.saturating_mul(bar_width as i64)) / max_value) as usize,
        bar_width,
    );

    let bar_color = if percent >= 50.0 {
        color_red
    } else if percent >= 20.0 {
        color_yellow
    } else if percent >= 5.0 {
        color_blue
    } else {
        color_green
    };

    let mut bar = String::new();
    bar.push_str(bar_color);
    for i in 0..bar_width {
        if i < filled {
            if i < filled.saturating_sub(1) {
                bar.push('█');
            } else {
                let remainder = (value.saturating_mul(bar_width as i64)) % max_value;
                if remainder > max_value / 2 {
                    bar.push('█');
                } else if remainder > max_value / 4 {
                    bar.push('▓');
                } else {
                    bar.push('▒');
                }
            }
        } else {
            bar.push_str(color_gray);
            bar.push('░');
            bar.push_str(bar_color);
        }
    }
    format!("{bar}{}", color_reset)
}

pub fn rune_width(r: char) -> usize {
    let u = r as u32;
    if (u >= 0x4E00 && u <= 0x9FFF)
        || (u >= 0x3400 && u <= 0x4DBF)
        || (u >= 0x20000 && u <= 0x2A6DF)
        || (u >= 0x2A700 && u <= 0x2B73F)
        || (u >= 0x2B740 && u <= 0x2B81F)
        || (u >= 0x2B820 && u <= 0x2CEAF)
        || (u >= 0x3040 && u <= 0x30FF)
        || (u >= 0x31F0 && u <= 0x31FF)
        || (u >= 0xAC00 && u <= 0xD7AF)
        || (u >= 0xFF00 && u <= 0xFFEF)
        || (u >= 0x1F300 && u <= 0x1F6FF)
        || (u >= 0x1F900 && u <= 0x1F9FF)
        || (u >= 0x2600 && u <= 0x26FF)
        || (u >= 0x2700 && u <= 0x27BF)
        || (u >= 0xFE10 && u <= 0xFE1F)
        || (u >= 0x1F000 && u <= 0x1F02F)
    {
        2
    } else {
        1
    }
}

pub fn display_width(s: &str) -> usize {
    let mut width = 0usize;
    for r in s.chars() {
        width += rune_width(r);
    }
    width
}

/// Go `calculateNameWidth`
pub fn calculate_name_width(term_width: usize) -> usize {
    const FIXED_WIDTH: usize = 61;
    let available = term_width.saturating_sub(FIXED_WIDTH);

    if available < 24 {
        return 24;
    }
    if available > 60 {
        return 60;
    }
    available
}

pub fn trim_name_with_width(name: &str, max_width: usize) -> String {
    const ELLIPSIS: &str = "...";
    const ELLIPSIS_WIDTH: usize = 3;

    let runes: Vec<char> = name.chars().collect();
    let mut widths = vec![0usize; runes.len()];
    for (i, r) in runes.iter().enumerate() {
        widths[i] = rune_width(*r);
    }

    let mut current_width = 0usize;
    for (i, w) in widths.iter().enumerate() {
        if current_width + *w > max_width {
            let mut sub_width = current_width;
            let mut j = i;
            while j > 0 && sub_width + ELLIPSIS_WIDTH > max_width {
                j -= 1;
                sub_width -= widths[j];
            }
            if j == 0 {
                return ELLIPSIS.to_string();
            }
            let prefix: String = runes[..j].iter().collect();
            return format!("{prefix}{ELLIPSIS}");
        }
        current_width += *w;
    }

    name.to_string()
}

pub fn pad_name(name: &str, target_width: usize) -> String {
    let current_width = display_width(name);
    if current_width >= target_width {
        return name.to_string();
    }
    format!(
        "{}{}",
        name,
        " ".repeat(target_width.saturating_sub(current_width))
    )
}

/// Go `formatUnusedTime`：`time.Since(last)` 等价于 `now.duration_since(last)`。
/// `None` 表示 Go 的零 `time.Time`（`IsZero`）。
pub fn format_unused_time(last_access: Option<SystemTime>) -> String {
    let Some(last_access) = last_access else {
        return String::new();
    };

    let duration = match SystemTime::now().duration_since(last_access) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };

    // 与 Go `int(duration.Hours() / 24)` 一致：按整日截断
    let days = (duration.as_secs() / 86400) as i32;

    if days < 90 {
        return String::new();
    }

    let months = days / 30;
    let years = days / 365;

    if years >= 2 {
        return format!(">{years}yr");
    } else if years >= 1 {
        return ">1yr".to_string();
    } else if months >= 3 {
        return format!(">{months}mo");
    }

    String::new()
}
