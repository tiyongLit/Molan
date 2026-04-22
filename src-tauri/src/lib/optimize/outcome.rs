//! 对齐 `lib/optimize/outcomes.sh`（#1293）：任务级六态显式结局协议。
//!
//! 每个优化任务在派发执行期间必须报告且仅报告一个结局：
//! - `applied`：变更已完成（dry-run 模式下为"将完成"）；
//! - `unchanged`：检查完成，无需变更；
//! - `skipped`：策略或运行上下文主动阻止执行；
//! - `unavailable`：主机不提供所需能力；
//! - `attention`：检查完成，发现需要用户处理的问题；
//! - `failed`：本可执行的操作未能完成。

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizeOutcome {
    Applied,
    Unchanged,
    Skipped,
    Unavailable,
    Attention,
    Failed,
}

impl OptimizeOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            OptimizeOutcome::Applied => "applied",
            OptimizeOutcome::Unchanged => "unchanged",
            OptimizeOutcome::Skipped => "skipped",
            OptimizeOutcome::Unavailable => "unavailable",
            OptimizeOutcome::Attention => "attention",
            OptimizeOutcome::Failed => "failed",
        }
    }

    /// 对齐 outcomes.sh `optimize_task_result_from_counts`：任何 failed > 0 → failed；
    /// applied > 0 → applied；skipped > 0 → skipped；否则 unchanged。
    pub fn from_counts(applied: u32, failed: u32, skipped: u32) -> OptimizeOutcome {
        if failed > 0 {
            OptimizeOutcome::Failed
        } else if applied > 0 {
            OptimizeOutcome::Applied
        } else if skipped > 0 {
            OptimizeOutcome::Skipped
        } else {
            OptimizeOutcome::Unchanged
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_resolve_like_shell() {
        assert_eq!(
            OptimizeOutcome::from_counts(1, 0, 0),
            OptimizeOutcome::Applied
        );
        assert_eq!(
            OptimizeOutcome::from_counts(0, 0, 0),
            OptimizeOutcome::Unchanged
        );
        assert_eq!(
            OptimizeOutcome::from_counts(0, 0, 1),
            OptimizeOutcome::Skipped
        );
        // 任何 failed > 0 都判 failed，即使有 applied
        assert_eq!(
            OptimizeOutcome::from_counts(2, 1, 0),
            OptimizeOutcome::Failed
        );
    }

    #[test]
    fn serde_names_match_outcomes_sh() {
        assert_eq!(OptimizeOutcome::Applied.as_str(), "applied");
        assert_eq!(OptimizeOutcome::Unchanged.as_str(), "unchanged");
        assert_eq!(OptimizeOutcome::Skipped.as_str(), "skipped");
        assert_eq!(OptimizeOutcome::Unavailable.as_str(), "unavailable");
        assert_eq!(OptimizeOutcome::Attention.as_str(), "attention");
        assert_eq!(OptimizeOutcome::Failed.as_str(), "failed");
    }
}
