//! 全局忙碌状态计数器（RAII guard）。
//!
//! 长任务（analyze / clean / uninstall / optimize）进入 `spawn_blocking` 时，
//! 调用 `enter_busy()` 获取 `BusyGuard`，计数器 +1；
//! 任务结束 guard Drop，计数器 -1。
//!
//! 主要用于 Dock 退出拦截：`is_busy()` 为 true 时，拦截退出并弹确认框。
//! **仅标记分钟级任务**，不要给秒级操作加 busy 标记。

use std::sync::atomic::{AtomicUsize, Ordering};

/// 全局忙碌计数器。
static BUSY_COUNT: AtomicUsize = AtomicUsize::new(0);

/// 进入忙碌状态，返回 RAII guard。
///
/// guard 存活期间 `is_busy()` 返回 `true`。
/// 支持多个任务并发——计数器累加，所有任务完成后才恢复空闲。
pub fn enter_busy() -> BusyGuard {
    BUSY_COUNT.fetch_add(1, Ordering::SeqCst);
    log::info!(
        "[busy_state] enter, count={}",
        BUSY_COUNT.load(Ordering::SeqCst)
    );
    BusyGuard
}

/// 查询当前是否有长任务在运行。
pub fn is_busy() -> bool {
    BUSY_COUNT.load(Ordering::SeqCst) > 0
}

/// 获取当前忙碌计数（调试用）。
pub fn busy_count() -> usize {
    BUSY_COUNT.load(Ordering::SeqCst)
}

/// 忙碌状态 RAII guard，Drop 时自动 -1。
pub struct BusyGuard;

impl Drop for BusyGuard {
    fn drop(&mut self) {
        let prev = BUSY_COUNT.fetch_sub(1, Ordering::SeqCst);
        log::info!("[busy_state] leave, count={}", prev - 1);
    }
}
