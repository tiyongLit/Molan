use std::borrow::Cow;

/// 简单按语言码选择 .ftl 源（只支持 en / zh-CN / zh-TW 三种）
fn ftl_source(lang: &str) -> &'static str {
  match lang {
    "zh-CN" => include_str!("../localesV2/zh-Hans.ftl"),
    "zh-TW" => include_str!("../localesV2/zh-Hant.ftl"),
    _ => include_str!("../localesV2/en.ftl"),
  }
}

/// 解析 `.ftl` 中形如 `key = value` 或 `a.b.c = value` 的行，返回对应 value。
fn parse_ftl_value<'a>(source: &'a str, key: &str) -> Option<Cow<'a, str>> {
  for line in source.lines() {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }
    if let Some(eq) = trimmed.find('=') {
      let k = trimmed[..eq].trim();
      if k == key {
        let v = trimmed[eq + 1..].trim();
        return Some(Cow::Borrowed(v));
      }
    }
  }
  None
}

/// 与托盘一致：配置为 `system` 时按环境变量推断语言。
pub fn effective_lang(cfg_lang: &str) -> String {
  if cfg_lang != "system" {
    return cfg_lang.to_string();
  }
  if let Ok(lang) = std::env::var("LANG") {
    if lang.starts_with("zh_CN") {
      return "zh-CN".into();
    } else if lang.starts_with("zh_TW") {
      return "zh-TW".into();
    }
  }
  "en".into()
}

/// Rust 端简易 t()：从对应语言的 .ftl 里按 key 取值。
/// 仅用于托盘等少量文案，不支持复杂 Fluent 特性。
pub fn t(lang: &str, key: &str) -> String {
  let src = ftl_source(lang);
  parse_ftl_value(src, key)
    .unwrap_or_else(|| Cow::Owned(key.to_string()))
    .into_owned()
}

/// 命中则返回译文，否则 `None`（与 [`t`] 不同，不回落为 key）。
pub fn try_t(lang: &str, key: &str) -> Option<String> {
  let src = ftl_source(lang);
  parse_ftl_value(src, key).map(|c| c.into_owned())
}

/// 替换 `{name}` 占位符（与 Go `i18n.TParams` 一致）。
pub fn t_params(lang: &str, key: &str, params: &[(&str, &str)]) -> String {
  let mut s = t(lang, key);
  for (k, v) in params {
    s = s.replace(&format!("{{{}}}", k), v);
  }
  s
}

/// Core / 校验层返回的稳定码（或 `CODE|detail`），先查 `errors.validation`，再查 `errors.core`。
pub fn translate_flowshield_error(lang: &str, raw: &str) -> String {
  let (code, detail_opt) = raw
    .find('|')
    .map(|i| (&raw[..i], Some(&raw[i + 1..])))
    .unwrap_or((raw, None));

  let keys = [
    format!("errors.validation.{code}"),
    format!("errors.core.{code}"),
  ];
  for k in &keys {
    if let Some(mut s) = try_t(lang, k) {
      if let Some(d) = detail_opt {
        s = s.replace("{detail}", d);
      }
      return s;
    }
  }
  raw.to_string()
}
