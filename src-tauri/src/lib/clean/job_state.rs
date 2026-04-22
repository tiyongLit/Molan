//! Clean 任务状态机 — 扫描/执行任务的「后端唯一事实来源」。
//!
//! 背景：活动状态一度由前端 useState 私有持有，与后端 worker 的真实状态无对账；
//! 授权面板（阻塞式系统认证）等长耗时环节会让两端分叉（后端已在扫、前端仍 idle）。
//! 本模块把活动状态收归后端：每次转换 `seq++` 并广播全量快照（`clean::job-state`），
//! 前端只做投影（挂载先查询 → 订阅 → 按 seq 丢弃乱序），任何时刻都能自愈对齐。
//!
//! 不变式：
//! - 单写者：所有转换经 `CLEAN_JOB` 互斥锁，同刻至多一个活动任务。
//! - 终态必达：worker 持有 [`JobLease`]，Drop 兜底把状态置回 idle（含 panic 路径）。
//! - 结果先于 idle：结果入槽发生在置 idle 之前，前端看到 idle 时结果必然可取。

use serde::Serialize;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

/// 任务活动阶段。`idle` 为终态；`cancelling` 是「取消已受理、worker 尚未退出」的诚实瞬时态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanJobState {
    #[default]
    Idle,
    Authorizing,
    Scanning,
    /// 预留：执行（apply）任务迁移到本状态机后使用。
    Applying,
    Cancelling,
}

impl CleanJobState {
    fn is_active(self) -> bool {
        !matches!(self, CleanJobState::Idle)
    }

    fn is_cancellable(self) -> bool {
        matches!(
            self,
            CleanJobState::Authorizing | CleanJobState::Scanning | CleanJobState::Applying
        )
    }
}

/// 全量状态快照：转换时整体广播，前端按 `seq` 单调应用。
#[derive(Clone, Debug, Serialize)]
pub struct CleanJobSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_finished_job_id: Option<String>,
    /// 单调递增序号；前端只接受 `seq` 更大的快照，乱序到达自动丢弃。
    pub seq: u64,
    pub state: CleanJobState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// 任务类型："scan"（apply 迁移后扩展）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// 授权结果：pending / authorized / canceled / failed。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<String>,
    /// 任务受理时间（epoch 秒）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_metric: Option<String>,
}

#[derive(Default)]
struct Inner {
    seq: u64,
    state: CleanJobState,
    job_id: Option<String>,
    kind: Option<String>,
    auth: Option<String>,
    started_at: Option<u64>,
    size_metric: Option<String>,
    /// 最近一次完成的任务结果（job_id 归属校验），供 `clean_job_result` 读取。
    last_result: Option<(String, serde_json::Value)>,
}

static CLEAN_JOB: Mutex<Inner> = Mutex::new(Inner {
    seq: 0,
    state: CleanJobState::Idle,
    job_id: None,
    kind: None,
    auth: None,
    started_at: None,
    size_metric: None,
    last_result: None,
});

static JOB_ID_SEQ: AtomicU64 = AtomicU64::new(0);

/// 互斥锁中毒不丢弃状态：把既有数据继续交还调用方（状态机比锁完整性更重要）。
fn lock_job() -> MutexGuard<'static, Inner> {
    CLEAN_JOB.lock().unwrap_or_else(|e| e.into_inner())
}

fn snapshot_locked(g: &Inner) -> CleanJobSnapshot {
    CleanJobSnapshot {
        last_finished_job_id: g.last_result.as_ref().map(|(id, _)| id.clone()),
        seq: g.seq,
        state: g.state,
        job_id: g.job_id.clone(),
        kind: g.kind.clone(),
        auth: g.auth.clone(),
        started_at: g.started_at,
        size_metric: g.size_metric.clone(),
    }
}

fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn next_job_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let n = JOB_ID_SEQ.fetch_add(1, Ordering::SeqCst);
    format!("job-{millis}-{n}")
}

/// 当前快照（挂载/可见性对账）。
pub fn snapshot() -> CleanJobSnapshot {
    snapshot_locked(&lock_job())
}

/// 是否存在活动任务。
pub fn is_active() -> bool {
    lock_job().state.is_active()
}

/// 指定任务是否仍为当前活动任务（用于 worker 自检 / 兼容等待）。
pub fn is_job_running(job_id: &str) -> bool {
    lock_job().job_id.as_deref() == Some(job_id)
}

/// 受理一次扫描任务。已有活动任务时返回 `None`（幂等挂接，不重复启动）。
pub fn begin_scan_job(size_metric: &str) -> Option<CleanJobSnapshot> {
    let mut g = lock_job();
    if g.state.is_active() {
        return None;
    }
    Some(start_locked(&mut g, size_metric))
}

fn start_locked(g: &mut Inner, size_metric: &str) -> CleanJobSnapshot {
    g.seq += 1;
    g.state = CleanJobState::Authorizing;
    g.job_id = Some(next_job_id());
    g.kind = Some("scan".into());
    g.auth = Some("pending".into());
    g.started_at = Some(now_epoch_secs());
    g.size_metric = Some(size_metric.to_string());
    snapshot_locked(g)
}

/// 授权结束、进入扫描。仅当任务仍处于 `authorizing` 且 job_id 匹配时生效；
/// 若取消已先行（state=cancelling），保持取消态不回退。
pub fn mark_scanning(job_id: &str, auth: &str) -> CleanJobSnapshot {
    let mut g = lock_job();
    if g.job_id.as_deref() == Some(job_id) && g.state == CleanJobState::Authorizing {
        g.seq += 1;
        g.state = CleanJobState::Scanning;
        g.auth = Some(auth.to_string());
    }
    snapshot_locked(&g)
}

/// 受理取消：只对 job_id 匹配的活动任务生效（旧任务 id 的取消请求被忽略，避免误伤新任务）。
/// 返回（快照, 是否实际受理）。
pub fn request_cancel(job_id: Option<&str>) -> (CleanJobSnapshot, bool) {
    let mut g = lock_job();
    let matches = match (&g.job_id, job_id) {
        (Some(cur), Some(want)) => cur == want,
        (Some(_), None) => true,
        _ => false,
    };
    if matches && g.state.is_cancellable() {
        g.seq += 1;
        g.state = CleanJobState::Cancelling;
        (snapshot_locked(&g), true)
    } else {
        (snapshot_locked(&g), false)
    }
}

/// 指定任务是否已进入取消态（worker 在授权返回后据此决定是否跳过扫描）。
pub fn is_cancelling(job_id: &str) -> bool {
    let g = lock_job();
    g.job_id.as_deref() == Some(job_id) && g.state == CleanJobState::Cancelling
}

/// 结果入槽（须在置 idle 之前调用）。
pub fn store_result(job_id: &str, value: serde_json::Value) {
    let mut g = lock_job();
    g.last_result = Some((job_id.to_string(), value));
}

/// 按 job_id 归属取回结果。
pub fn result_for(job_id: &str) -> Option<serde_json::Value> {
    let g = lock_job();
    match &g.last_result {
        Some((id, value)) if id == job_id => Some(value.clone()),
        _ => None,
    }
}

/// 收尾：置回 idle。仅对 job_id 匹配的活动任务生效；幂等（重复调用返回 `None`）。
pub fn finish(job_id: &str) -> Option<CleanJobSnapshot> {
    let mut g = lock_job();
    if !g.state.is_active() || g.job_id.as_deref() != Some(job_id) {
        return None;
    }
    g.seq += 1;
    g.state = CleanJobState::Idle;
    g.job_id = None;
    g.kind = None;
    g.auth = None;
    g.started_at = None;
    g.size_metric = None;
    Some(snapshot_locked(&g))
}

/// 广播快照：所有转换（含 lease Drop 兜底）统一经此 emit，日志自证时序。
pub fn emit(app: &tauri::AppHandle, snap: &CleanJobSnapshot) {
    log::info!(
        "[clean-job] seq={} state={:?} job={:?} auth={:?}",
        snap.seq,
        snap.state,
        snap.job_id,
        snap.auth
    );
    use tauri::Emitter;
    if let Err(e) = app.emit(crate::events::EVT_CLEAN_JOB_STATE, snap) {
        log::warn!("[clean-job] emit job-state failed: {e}");
    }
}

/// worker 租约：Drop 必达收尾（含 panic/unwind 路径），保证状态永不卡在活动态。
pub struct JobLease {
    app: tauri::AppHandle,
    job_id: String,
}

impl JobLease {
    pub fn new(app: tauri::AppHandle, job_id: impl Into<String>) -> Self {
        Self {
            app,
            job_id: job_id.into(),
        }
    }
}

impl Drop for JobLease {
    fn drop(&mut self) {
        if let Some(snap) = finish(&self.job_id) {
            log::info!("[clean-job] job {} finished, release to idle", self.job_id);
            emit(&self.app, &snap);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 静态状态为进程级共享，测试串行执行避免相互干扰
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn reset() -> MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *lock_job() = Inner::default();
        guard
    }

    #[test]
    fn scan_lifecycle_transitions() {
        let _g = reset();
        let snap = begin_scan_job("logical").expect("should begin");
        assert_eq!(snap.state, CleanJobState::Authorizing);
        let job_id = snap.job_id.clone().unwrap();

        let snap = mark_scanning(&job_id, "authorized");
        assert_eq!(snap.state, CleanJobState::Scanning);
        assert_eq!(snap.auth.as_deref(), Some("authorized"));

        let snap = finish(&job_id).expect("should finish");
        assert_eq!(snap.state, CleanJobState::Idle);
        assert!(snap.job_id.is_none());
        assert!(finish(&job_id).is_none(), "finish must be idempotent");
    }

    #[test]
    fn begin_is_idempotent_while_active() {
        let _g = reset();
        let first = begin_scan_job("logical").expect("should begin");
        assert!(
            begin_scan_job("logical").is_none(),
            "second begin must attach"
        );
        let job_id = first.job_id.clone().unwrap();
        finish(&job_id);
        assert!(begin_scan_job("logical").is_some(), "begin after idle");
    }

    #[test]
    fn cancel_requires_matching_job_id() {
        let _g = reset();
        let snap = begin_scan_job("logical").expect("should begin");
        let job_id = snap.job_id.unwrap();

        let (_, applied) = request_cancel(Some("stale-job"));
        assert!(!applied, "stale cancel must be ignored");
        assert!(!is_cancelling(&job_id));

        let (snap, applied) = request_cancel(Some(&job_id));
        assert!(applied);
        assert_eq!(snap.state, CleanJobState::Cancelling);
        assert!(is_cancelling(&job_id));

        // 重复取消不产生新转换
        let (_, again) = request_cancel(Some(&job_id));
        assert!(!again);
    }

    #[test]
    fn cancel_during_authorizing_blocks_scanning() {
        let _g = reset();
        let snap = begin_scan_job("logical").expect("should begin");
        let job_id = snap.job_id.unwrap();

        request_cancel(Some(&job_id));
        // 授权返回后 worker 校验：仍为 cancelling，不得回退到 scanning
        assert!(is_cancelling(&job_id));
        let snap = mark_scanning(&job_id, "authorized");
        assert_eq!(snap.state, CleanJobState::Cancelling);

        finish(&job_id);
        assert!(!is_active());
    }

    #[test]
    fn result_is_owned_by_job_id() {
        let _g = reset();
        let snap = begin_scan_job("logical").expect("should begin");
        let job_id = snap.job_id.unwrap();
        store_result(&job_id, serde_json::json!({ "cancelled": false }));
        assert!(result_for(&job_id).is_some());
        assert!(result_for("other-job").is_none());
    }
}
