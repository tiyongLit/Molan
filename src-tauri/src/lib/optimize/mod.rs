pub mod diagnostics;
pub mod maintenance;
pub mod outcome;
pub mod tasks;

use std::sync::Mutex;

/// 最近一次任务失败的详情（任务粒度单槽）。
///
/// 六态结局协议里 `OptimizeOutcome::Failed` 不携带原因，任务 handler 签名
/// 统一为 `fn() -> OptimizeOutcome`。失败原因由 handler 内部经 [`record_failure`]
/// 旁路写入；控制器在任务返回后 [`take_failure`] 取出，随 `task_done` 事件与
/// `results[].error` 交给前端展示。任务串行执行，单槽即可覆盖。
static LAST_FAILURE: Mutex<Option<String>> = Mutex::new(None);

/// 记录失败原因；同一任务内多条失败信息以 "; " 追加合并。
pub fn record_failure(message: &str) {
    if let Ok(mut slot) = LAST_FAILURE.lock() {
        match slot.as_mut() {
            Some(existing) => {
                existing.push_str("; ");
                existing.push_str(message);
            }
            None => *slot = Some(message.to_string()),
        }
    }
}

/// 取出并清空失败原因（无记录时返回 None）。
pub fn take_failure() -> Option<String> {
    LAST_FAILURE.lock().ok().and_then(|mut slot| slot.take())
}

/// 清空失败原因（每个任务执行前调用，避免上一任务残留串台）。
pub fn clear_failure() {
    if let Ok(mut slot) = LAST_FAILURE.lock() {
        *slot = None;
    }
}
