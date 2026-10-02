//! Clean 默认勾选策略 — 后端权威计算，前端只读。
//! 自 `controllers/clean.rs` 纯搬迁（行为不变）。

/// 判断分类是否为「谨慎清理」（影响 default_selected）。
pub fn is_cautious_category(cat_id: &str) -> bool {
    matches!(
        cat_id,
        "office_cache" | "virtualization" | "device_firmware" | "large_files"
    )
}

/// 计算 item 的默认勾选状态（后端权威，前端只读）。
/// 规则对齐前端原逻辑：status!=info && size>0 && !whitelist_matched && recommend && !cautious
pub fn compute_default_selected(
    item_status: &str,
    item_size: u64,
    item_whitelist_matched: bool,
    cat_recommend: bool,
    cat_id: &str,
    cat_requires_sudo: bool,
    has_sudo: bool,
) -> bool {
    if item_status != "cleanable" {
        return false;
    }
    if item_size == 0 {
        return false;
    }
    if item_whitelist_matched {
        return false;
    }
    if !cat_recommend {
        return false;
    }
    if is_cautious_category(cat_id) {
        return false;
    }
    if cat_requires_sudo && !has_sudo {
        return false;
    }
    true
}
