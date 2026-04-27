//! `sfltool dumpbtm` 输出解析：现代 Login/Background Items（BTM 层）。
//! 对齐 Burrow `LoginItemsReader.parse`——纯函数，sfltool spawn 在 controllers 层。

/// 一条 BTM 记录（内部结构；merge 后折叠为 `StartupItem`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginItem {
    pub name: String,
    pub identifier: String,
    pub developer: String,
    pub type_: String, // "developer" / "legacy daemon" / "agent" …（取 " (" 前部分）
    pub enabled: bool,
}

pub fn parse(dump: &str) -> Vec<LoginItem> {
    let mut items = Vec::new();
    for block in blocks(dump) {
        let f = fields(&block);
        let id = f.get("Identifier").cloned().unwrap_or_default();
        // Name 缺失或 "(null)" 都回落到 id（对齐 Burrow `name ?? id`）。
        let name_raw = f
            .get("Name")
            .map(|n| {
                if n == "(null)" {
                    String::new()
                } else {
                    n.clone()
                }
            })
            .unwrap_or_default();
        if id.is_empty() && name_raw.is_empty() {
            continue; // 跳过空占位记录
        }
        let disp = f
            .get("Disposition")
            .map(|d| d.to_lowercase())
            .unwrap_or_default();
        let enabled = !disp.contains("disabled") && disp.contains("enabled");
        items.push(LoginItem {
            name: if name_raw.is_empty() {
                id.clone()
            } else {
                name_raw
            },
            identifier: id,
            developer: f
                .get("Developer Name")
                .map(|n| {
                    if n == "(null)" {
                        String::new()
                    } else {
                        n.clone()
                    }
                })
                .unwrap_or_default(),
            type_: f
                .get("Type")
                .map(|t| t.split(" (").next().unwrap_or("").to_string())
                .unwrap_or_default(),
            enabled,
        });
    }
    items
}

/// 每条记录以 trim 后恰为 `#<n>:` 的行开始（对齐 Burrow `blocks`）。
fn blocks(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut in_item = false;
    for raw in s.lines() {
        let t = raw.trim();
        if is_block_header(t) {
            if in_item && !cur.is_empty() {
                out.push(cur.join("\n"));
            }
            cur = Vec::new();
            in_item = true;
        } else if in_item {
            cur.push(raw.to_string());
        }
    }
    if in_item && !cur.is_empty() {
        out.push(cur.join("\n"));
    }
    out
}

/// `^#\d+:$`（Swift 正则直译，无 regex 依赖）。
fn is_block_header(t: &str) -> bool {
    let body = t.strip_prefix('#').unwrap_or_default();
    let body = body.strip_suffix(':').unwrap_or_default();
    !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit())
}

/// `"        Key: Value"` → `[Key: Value]`，首个出现的 key 胜出（对齐 Burrow `fields`）。
fn fields(block: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for line in block.lines() {
        let Some(colon) = line.find(':') else {
            continue;
        };
        let key = line[..colon].trim();
        let val = line[colon + 1..].trim();
        if !key.is_empty() && !out.contains_key(key) {
            out.insert(key.to_string(), val.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_parse_btm_dump() {
        let dump = r#"Gathering login items now, please wait...
#16:
        URL: file:///Applications/Studio%20Route%20Guard.app
        Name: Studio Route Guard
        Identifier: 16.com.henry.studio-route-guard
        Type: developer (1)
        Disposition: [enabled]
        Developer Name: Henry

#17:
        Identifier: 17.com.unknown.app
        Type: legacy daemon (3)
        Disposition: [disabled]

#18:
        Identifier: unknown developer
        Type: developer (1)
        Disposition: [enabled]
"#;
        let items = parse(dump);
        assert_eq!(items.len(), 3);

        let a = &items[0];
        assert_eq!(a.name, "Studio Route Guard");
        assert_eq!(a.identifier, "16.com.henry.studio-route-guard");
        assert_eq!(a.developer, "Henry");
        assert_eq!(a.type_, "developer");
        assert!(a.enabled);

        // Name 缺失 → name 回落 id
        let b = &items[1];
        assert_eq!(b.name, "17.com.unknown.app");
        assert!(!b.enabled); // 含 disabled → enabled=false

        let c = &items[2];
        assert_eq!(c.identifier, "unknown developer");
    }

    #[test]
    fn probe_parse_null_name() {
        let dump = "#1:\n        Name: (null)\n        Identifier: 1.com.foo\n";
        let items = parse(dump);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "1.com.foo"); // "(null)" → 回落 id
    }

    #[test]
    fn probe_parse_skips_placeholder() {
        let dump = "#1:\n#2:\n        Name: Real App\n        Identifier: 2.com.real\n";
        let items = parse(dump);
        assert_eq!(items.len(), 1); // #1: 无 id 无 name → 跳过
        assert_eq!(items[0].name, "Real App");
    }
}
