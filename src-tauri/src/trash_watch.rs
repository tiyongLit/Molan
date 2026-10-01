//! 废纸篓容量提醒专用状态机。快照是事实源，事件仅通知；目录统计无删除副作用。

use chrono::{DateTime, Local, TimeZone};
use crossbeam_channel::{Receiver, Sender, bounded};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_store::StoreExt;

const HEALTH: Duration = Duration::from_secs(120);
const CACHE_TTL: Duration = Duration::from_secs(600);
const FIRST_CHECK: Duration = Duration::from_secs(5);
static STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub enabled: bool,
    pub threshold: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold: 4096,
        }
    }
}
impl Config {
    fn validate(&self) -> Result<(), String> {
        if [512, 1024, 2048, 4096, 10240, 20480].contains(&self.threshold) {
            Ok(())
        } else {
            Err("TRASH_CONFIG_INVALID".into())
        }
    }
    /// 旧版小容量阈值（1/10/50 MB）已从产品侧下架（现档位 512MB–20GB，默认 4GB）；
    /// 读路径把存量旧值归一化为默认 4096，避免升级后被误判为损坏配置；
    /// 不写回磁盘，待用户下次在设置页保存时自然修复。
    fn migrate_legacy_threshold(&mut self) {
        if [1, 10, 50].contains(&self.threshold) {
            self.threshold = 4096;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Hidden,
    Candidate,
    Visible,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub reminder_id: Option<String>,
    pub revision: u64,
    pub state: Phase,
    pub enabled: bool,
    #[serde(rename = "thresholdMB")]
    pub threshold_mb: u64,
    pub size_metric: &'static str,
    pub snoozed_until: i64,
    pub error_code: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Show,
    Shown,
    Hide,
    Snooze,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionArgs {
    pub action: Action,
    pub reminder_id: Option<String>,
    pub revision: u64,
}

#[derive(Default)]
struct Debounce {
    first: Option<Instant>,
    last: Option<Instant>,
}
impl Debounce {
    fn mark(&mut self, now: Instant) {
        self.first.get_or_insert(now);
        self.last = Some(now);
    }
    fn due(&self) -> Option<Instant> {
        Some((self.last? + Duration::from_secs(1)).min(self.first? + Duration::from_secs(5)))
    }
}

struct Inner {
    snapshot: Snapshot,
    config: Option<Config>,
    ready: Option<Instant>,
    dirty: Debounce,
    calendar: String,
}
impl Default for Inner {
    fn default() -> Self {
        Self {
            snapshot: Snapshot {
                reminder_id: None,
                revision: 1,
                state: Phase::Hidden,
                enabled: false,
                threshold_mb: 4096,
                size_metric: "logical",
                snoozed_until: 0,
                error_code: None,
            },
            config: None,
            ready: None,
            dirty: Debounce::default(),
            calendar: String::new(),
        }
    }
}

pub struct Service {
    inner: Mutex<Inner>,
    settings_lock: Mutex<()>,
    epoch: AtomicU64,
    stopped: AtomicBool,
    wake: Sender<()>,
    native_stop: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}
impl Service {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn snapshot(&self, ready: bool) -> Snapshot {
        let mut g = self.lock();
        if ready && g.ready.is_none() {
            g.ready = Some(Instant::now());
            g.dirty.mark(Instant::now());
            let _ = self.wake.try_send(());
        }
        g.snapshot.clone()
    }
    pub fn dirty(&self) {
        self.lock().dirty.mark(Instant::now());
        let _ = self.wake.try_send(());
    }
    fn invalidate(&self, g: &mut Inner) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        g.snapshot.state = Phase::Hidden;
        g.snapshot.reminder_id = None;
        g.snapshot.error_code = None;
        g.dirty.mark(Instant::now());
        g.snapshot.revision += 1;
    }
    fn publish(&self, app: &AppHandle) {
        let snap = self.snapshot(false);
        if let Err(e) = app.emit_to(
            "trash-reminder",
            crate::events::EVT_TRASH_REMINDER_STATE,
            &snap,
        ) {
            log::warn!(
                "[trash-reminder] delivery revision={} failed: {e}",
                snap.revision
            );
        }
    }
    pub(crate) fn refresh(&self, app: &AppHandle) {
        let _write = self.settings_lock.lock().unwrap_or_else(|e| e.into_inner());
        let now = Local::now();
        let calendar = now.format("%Y-%m-%d/%z").to_string();
        let mut g = self.lock();
        match read_settings(app) {
            Ok((config, until)) => {
                let until = effective_snooze(&now, until);
                if g.config.as_ref() != Some(&config)
                    || g.calendar != calendar
                    || g.snapshot.snoozed_until != until
                {
                    self.invalidate(&mut g);
                    g.snapshot.enabled = config.enabled;
                    g.snapshot.threshold_mb = config.threshold;
                    g.snapshot.snoozed_until = until;
                    g.config = Some(config);
                    g.calendar = calendar;
                }
            }
            Err(e) => {
                if g.config.is_some() || g.snapshot.error_code.as_deref() != Some(&e) {
                    self.invalidate(&mut g);
                    g.config = None;
                    g.snapshot.enabled = false;
                    g.snapshot.error_code = Some(e);
                }
            }
        }
    }
    pub fn update_settings(&self, app: &AppHandle, config: Config) -> Result<Snapshot, String> {
        config.validate()?;
        let _write = self.settings_lock.lock().map_err(|_| "TRASH_SAVE_FAILED")?;
        persist(
            app,
            "trashReminder",
            serde_json::json!({ "enabled": config.enabled, "threshold": config.threshold }),
        )?;
        let mut g = self.lock();
        self.invalidate(&mut g);
        g.snapshot.enabled = config.enabled;
        g.snapshot.threshold_mb = config.threshold;
        g.config = Some(config);
        drop(g);
        self.publish(app);
        let _ = self.wake.try_send(());
        Ok(self.snapshot(false))
    }
    fn save_snooze(&self, app: &AppHandle, reminder: &str) -> Result<Snapshot, String> {
        let result = self.save_snooze_with(reminder, |until| {
            persist(app, "trashReminderSnoozedUntil", serde_json::json!(until))
        });
        self.publish(app);
        result
    }
    fn save_snooze_with(
        &self,
        reminder: &str,
        save: impl FnOnce(i64) -> Result<(), String>,
    ) -> Result<Snapshot, String> {
        let _write = self.settings_lock.lock().map_err(|_| "TRASH_SAVE_FAILED")?;
        let mut g = self.lock();
        require_reminder(&g, reminder)?;
        if g.snapshot.state == Phase::Hidden {
            return Ok(g.snapshot.clone());
        }
        let until = next_midnight(&Local::now())?;
        if let Err(error) = save(until) {
            g.snapshot.state = Phase::Error;
            g.snapshot.error_code = Some("TRASH_SAVE_FAILED".into());
            g.snapshot.revision += 1;
            log::warn!("[trash-reminder] snooze {reminder}: {error}");
            return Err("TRASH_SAVE_FAILED".into());
        }
        self.epoch.fetch_add(1, Ordering::SeqCst);
        g.snapshot.snoozed_until = until;
        g.snapshot.state = Phase::Hidden;
        g.snapshot.error_code = None;
        g.snapshot.revision += 1;
        Ok(g.snapshot.clone())
    }
    pub fn action(&self, app: &AppHandle, args: ActionArgs) -> Result<Snapshot, String> {
        let id = args.reminder_id.as_deref().ok_or("TRASH_STALE_REMINDER")?;
        if args.action == Action::Snooze {
            {
                let g = self.lock();
                require_reminder(&g, id)?;
            }
            return self.save_snooze(app, id);
        }
        let mut g = self.lock();
        require_reminder(&g, id)?;
        match args.action {
            Action::Shown => {
                if args.revision == g.snapshot.revision && g.snapshot.state == Phase::Candidate {
                    g.snapshot.state = Phase::Visible;
                    g.snapshot.revision += 1;
                }
            }
            _ => return Err("TRASH_ACTION_INVALID".into()),
        }
        drop(g);
        self.publish(app);
        Ok(self.snapshot(false))
    }
    /// 清空废纸篓成功后立即让当前提醒失效隐藏：废纸篓已空必低于阈值，
    /// 无需等下一轮目录事件；invalidate 会 dirty 触发 watcher 重新基线化。
    pub fn on_trash_emptied(&self, app: &AppHandle) {
        {
            let mut g = self.lock();
            self.invalidate(&mut g);
        }
        self.publish(app);
        let _ = self.wake.try_send(());
    }
    pub fn install_native_stop(&self, stop: Box<dyn Fn() + Send + Sync>) {
        let mut slot = self.native_stop.lock().unwrap_or_else(|e| e.into_inner());
        if self.is_stopped() {
            stop();
        } else {
            *slot = Some(stop);
        }
    }
    fn commit_measurement(&self, epoch: u64, result: Measurement, blocked: bool) {
        let mut g = self.lock();
        if self.epoch.load(Ordering::SeqCst) != epoch || blocked {
            g.dirty.mark(Instant::now());
            return;
        }
        if !g.snapshot.enabled || g.snapshot.snoozed_until > Local::now().timestamp_millis() {
            return;
        }
        match result {
            Measurement::Over => {
                if g.snapshot.state == Phase::Hidden {
                    g.snapshot.reminder_id = Some(format!(
                        "trash-{}-{}",
                        Local::now().timestamp_millis(),
                        g.snapshot.revision
                    ));
                    g.snapshot.state = Phase::Candidate;
                    g.snapshot.error_code = None;
                    g.snapshot.revision += 1;
                }
            }
            Measurement::Below | Measurement::Unknown => {
                let error = (result == Measurement::Unknown).then(|| "TRASH_UNREADABLE".into());
                if error != g.snapshot.error_code {
                    log::info!("[trash-reminder] measurement={result:?}");
                }
                g.snapshot.state = Phase::Hidden;
                g.snapshot.reminder_id = None;
                g.snapshot.error_code = error;
                g.snapshot.revision += 1;
            }
            Measurement::Cancelled => {}
        }
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        let _ = self.wake.try_send(());
        if let Some(stop) = self
            .native_stop
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            stop();
        }
    }
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }
}

fn require_reminder(g: &Inner, id: &str) -> Result<(), String> {
    if g.snapshot.reminder_id.as_deref() == Some(id) {
        Ok(())
    } else {
        Err("TRASH_STALE_REMINDER".into())
    }
}

pub fn service(app: &AppHandle) -> Arc<Service> {
    app.state::<Arc<Service>>().inner().clone()
}

/// 所有原生窗口变更在主线程按最新版本检查，旧动画回调不能隐藏新提醒。
pub async fn window_action(app: AppHandle, args: ActionArgs) -> Result<Snapshot, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| {
            let svc = service(&handle);
            let g = svc.lock();
            if args.revision != g.snapshot.revision || args.reminder_id != g.snapshot.reminder_id {
                return Ok(g.snapshot.clone());
            }
            let win = handle
                .get_webview_window("trash-reminder")
                .ok_or("TRASH_WINDOW_UNAVAILABLE")?;
            match args.action {
                Action::Hide if g.snapshot.state == Phase::Hidden => {
                    win.hide().map_err(|e| e.to_string())?;
                }
                Action::Show if g.snapshot.state != Phase::Hidden => {
                    if g.snapshot.state == Phase::Candidate && busy() {
                        return Err("TRASH_BUSY".into());
                    }
                    #[cfg(target_os = "macos")]
                    crate::platform::macos_reminder_window::show(&win)?;
                    #[cfg(not(target_os = "macos"))]
                    win.show().map_err(|e| e.to_string())?;
                }
                _ => {}
            }
            Ok(g.snapshot.clone())
        })();
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;
    rx.await
        .map_err(|_| "TRASH_WINDOW_UNAVAILABLE".to_string())?
}

fn parse_settings(value: &serde_json::Value) -> Result<(Config, i64), String> {
    let object = value.as_object().ok_or("TRASH_CONFIG_INVALID")?;
    let config = match object.get("trashReminder") {
        Some(value) => {
            let mut config = serde_json::from_value::<Config>(value.clone())
                .map_err(|_| "TRASH_CONFIG_INVALID")?;
            config.migrate_legacy_threshold();
            config
        }
        None => Config::default(),
    };
    config.validate()?;
    let until = match object.get("trashReminderSnoozedUntil") {
        None => 0,
        Some(value) => value
            .as_i64()
            .filter(|v| *v >= 0)
            .ok_or("TRASH_CONFIG_INVALID")?,
    };
    Ok((config, until))
}
fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|_| "TRASH_CONFIG_UNAVAILABLE")?
        .join("settings.json"))
}
fn read_settings_document(app: &AppHandle) -> Result<serde_json::Value, String> {
    match std::fs::read(settings_path(app)?) {
        Ok(data) => serde_json::from_slice(&data).map_err(|_| "TRASH_CONFIG_INVALID".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(_) => Err("TRASH_CONFIG_UNAVAILABLE".into()),
    }
}
fn read_settings(app: &AppHandle) -> Result<(Config, i64), String> {
    parse_settings(&read_settings_document(app)?)
}
fn persist(app: &AppHandle, key: &str, value: serde_json::Value) -> Result<(), String> {
    // plugin-store 对损坏文件可能退回空 store；先验证磁盘，避免覆盖其他设置。
    let mut candidate = read_settings_document(app)?;
    candidate
        .as_object_mut()
        .ok_or("TRASH_CONFIG_INVALID")?
        .insert(key.into(), value.clone());
    // 允许用户显式修复本域非法值；无效 JSON 或其他损坏字段不覆盖。
    parse_settings(&candidate)?;
    let store = app
        .store("settings.json")
        .map_err(|_| "TRASH_SAVE_FAILED")?;
    let previous = store.get(key);
    store.set(key, value);
    if let Err(e) = store.save() {
        if let Some(value) = previous {
            store.set(key, value);
        } else {
            store.delete(key);
        }
        log::warn!("[trash-reminder] store save: {e}");
        return Err("TRASH_SAVE_FAILED".into());
    }
    Ok(())
}

fn next_midnight<T: TimeZone>(now: &DateTime<T>) -> Result<i64, String> {
    let tomorrow = now.date_naive().succ_opt().ok_or("TRASH_CLOCK_INVALID")?;
    // 少数时区在零点跳时：取次日首个合法分钟；歧义时选较早边界。
    for minute in 0..1440 {
        let local = tomorrow
            .and_hms_opt(minute / 60, minute % 60, 0)
            .ok_or("TRASH_CLOCK_INVALID")?;
        if let Some(date) = now.timezone().from_local_datetime(&local).earliest() {
            return Ok(date.timestamp_millis());
        }
    }
    Err("TRASH_CLOCK_INVALID".into())
}
fn effective_snooze<T: TimeZone>(now: &DateTime<T>, until: i64) -> i64 {
    if until > now.timestamp_millis() && next_midnight(now).is_ok_and(|end| until <= end) {
        until
    } else {
        0
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Measurement {
    Over,
    Below,
    Unknown,
    Cancelled,
}
fn measure(path: &Path, threshold: u64, cancelled: impl Fn() -> bool) -> Measurement {
    if threshold == 0 {
        return Measurement::Unknown;
    }
    let mut stack = vec![path.to_owned()];
    let mut total = 0u64;
    let mut incomplete = false;
    while let Some(path) = stack.pop() {
        if cancelled() {
            return Measurement::Cancelled;
        }
        // 根/中途目录被换为 symlink 时同样不跟随。
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => {}
            _ => {
                incomplete = true;
                continue;
            }
        }
        let entries = match std::fs::read_dir(&path) {
            Ok(v) => v,
            Err(_) => {
                incomplete = true;
                continue;
            }
        };
        for entry in entries {
            if cancelled() {
                return Measurement::Cancelled;
            }
            let meta = entry.and_then(|e| e.metadata().map(|m| (e.path(), m)));
            match meta {
                Ok((path, meta)) if meta.is_dir() => stack.push(path),
                Ok((_, meta)) => {
                    total = total.saturating_add(meta.len());
                    if total >= threshold {
                        return Measurement::Over;
                    }
                }
                Err(_) => incomplete = true,
            }
        }
    }
    if incomplete {
        Measurement::Unknown
    } else {
        Measurement::Below
    }
}
fn busy() -> bool {
    crate::core::busy_state::is_busy()
        || crate::clean::job_state::is_active()
        || crate::controllers::clean::legacy_clean_busy()
}
fn gate(path: &Path) -> std::io::Result<(std::time::SystemTime, usize)> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_dir() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    let count = std::fs::read_dir(path)?
        .collect::<Result<Vec<_>, _>>()?
        .len();
    Ok((meta.modified()?, count))
}

pub fn start_trash_watch(app: AppHandle) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let (tx, rx) = bounded(1);
    let svc = Arc::new(Service {
        inner: Mutex::new(Inner::default()),
        settings_lock: Mutex::new(()),
        epoch: AtomicU64::new(0),
        stopped: AtomicBool::new(false),
        wake: tx,
        native_stop: Mutex::new(None),
    });
    app.manage(svc.clone());
    if let Some(window) = app.get_webview_window("trash-reminder") {
        let handle = app.clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if crate::macos_dock_quit::is_tray_exit_confirmed() {
                    return;
                }
                api.prevent_close();
                let handle = handle.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let svc = service(&handle);
                    let snap = svc.snapshot(false);
                    if snap.state != Phase::Hidden {
                        let result = svc.action(
                            &handle,
                            ActionArgs {
                                action: Action::Snooze,
                                reminder_id: snap.reminder_id,
                                revision: snap.revision,
                            },
                        );
                        if let Err(e) = result {
                            log::warn!("[trash-reminder] close deferred: {e}");
                        }
                    }
                });
            }
        });
    }
    let handle = app.clone();
    let worker = svc.clone();
    std::thread::Builder::new()
        .name("trash-reminder".into())
        .spawn(move || schedule(handle, worker, rx))
        .expect("trash reminder worker");
    #[cfg(target_os = "macos")]
    crate::platform::macos_trash_watch::start(svc);
}

fn schedule(app: AppHandle, svc: Arc<Service>, rx: Receiver<()>) {
    let Some(path) = crate::core::base::home_dir_opt().map(|p| p.join(".Trash")) else {
        return;
    };
    let mut heartbeat = Instant::now();
    let mut last_scan: Option<Instant> = None;
    let mut last_gate = None;
    let mut last_epoch = 0;
    loop {
        if svc.is_stopped() {
            return;
        }
        let health = Instant::now() >= heartbeat;
        if health {
            svc.refresh(&app);
            heartbeat = Instant::now() + HEALTH;
            svc.publish(&app);
        }
        let now = Instant::now();
        let (eligible, due, epoch, threshold) = {
            let g = svc.lock();
            let eligible = g.ready.is_some_and(|t| now >= t + FIRST_CHECK)
                && g.snapshot.enabled
                && g.snapshot.snoozed_until <= Local::now().timestamp_millis();
            (
                eligible,
                g.dirty.due().is_some_and(|t| now >= t),
                svc.epoch.load(Ordering::SeqCst),
                g.snapshot.threshold_mb,
            )
        };
        if eligible && !busy() && (health || due || epoch != last_epoch) {
            let current_gate = gate(&path).ok();
            let expired = last_scan.is_none_or(|t| t.elapsed() >= CACHE_TTL);
            if due
                || expired
                || epoch != last_epoch
                || current_gate.is_none()
                || current_gate != last_gate
            {
                svc.lock().dirty = Debounce::default();
                let result = measure(&path, threshold.saturating_mul(1024 * 1024), || {
                    svc.is_stopped() || svc.epoch.load(Ordering::SeqCst) != epoch
                });
                last_scan = Some(Instant::now());
                last_gate = if result == Measurement::Unknown {
                    None
                } else {
                    current_gate
                };
                last_epoch = epoch;
                svc.commit_measurement(epoch, result, busy());
                svc.publish(&app);
            }
        }
        let wait = {
            let g = svc.lock();
            let mut next = heartbeat;
            if let Some(ready) = g.ready {
                if ready + FIRST_CHECK > Instant::now() {
                    next = next.min(ready + FIRST_CHECK);
                } else if g.snapshot.enabled && g.snapshot.snoozed_until == 0 && !busy() {
                    if let Some(due) = g.dirty.due() {
                        next = next.min(due);
                    }
                }
            }
            next.saturating_duration_since(Instant::now())
                .max(Duration::from_millis(10))
        };
        let _ = rx.recv_timeout(wait);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_service() -> Service {
        let (wake, _) = bounded(1);
        let mut inner = Inner::default();
        inner.snapshot.enabled = true;
        Service {
            inner: Mutex::new(inner),
            settings_lock: Mutex::new(()),
            epoch: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            wake,
            native_stop: Mutex::new(None),
        }
    }
    #[test]
    fn candidate_is_reused_until_ack_or_invalidated() {
        let svc = test_service();
        svc.commit_measurement(0, Measurement::Over, false);
        let first = svc.snapshot(false);
        assert_eq!(first.state, Phase::Candidate);
        svc.commit_measurement(0, Measurement::Over, false);
        assert_eq!(svc.snapshot(false).reminder_id, first.reminder_id);
        assert_eq!(svc.snapshot(false).revision, first.revision);
        // show/事件投递失败没有 ACK，不消费候选；设置更新后旧结果不能复活卡片。
        svc.invalidate(&mut svc.lock());
        svc.commit_measurement(0, Measurement::Over, false);
        assert_eq!(svc.snapshot(false).state, Phase::Hidden);
    }
    #[test]
    fn unknown_is_not_below_and_recovery_creates_new_candidate() {
        let svc = test_service();
        svc.commit_measurement(0, Measurement::Unknown, false);
        assert_eq!(
            svc.snapshot(false).error_code.as_deref(),
            Some("TRASH_UNREADABLE")
        );
        svc.commit_measurement(0, Measurement::Below, false);
        assert!(svc.snapshot(false).error_code.is_none());
        svc.commit_measurement(0, Measurement::Over, false);
        assert_eq!(svc.snapshot(false).state, Phase::Candidate);
    }
    #[test]
    fn disabled_snoozed_busy_do_not_show() {
        let svc = test_service();
        svc.commit_measurement(0, Measurement::Over, true);
        assert_eq!(svc.snapshot(false).state, Phase::Hidden);
        svc.lock().snapshot.enabled = false;
        svc.commit_measurement(0, Measurement::Over, false);
        assert_eq!(svc.snapshot(false).state, Phase::Hidden);
        svc.lock().snapshot.enabled = true;
        svc.lock().snapshot.snoozed_until = next_midnight(&Local::now()).unwrap();
        svc.commit_measurement(0, Measurement::Over, false);
        assert_eq!(svc.snapshot(false).state, Phase::Hidden);
    }
    #[test]
    fn snooze_failure_is_honest_and_retry_only_saves() {
        let svc = test_service();
        svc.commit_measurement(0, Measurement::Over, false);
        let id = svc.snapshot(false).reminder_id.unwrap();
        // 保存失败：诚实置 Error，不伪装已暂缓
        assert_eq!(
            svc.save_snooze_with(&id, |_| Err("disk full".into()))
                .unwrap_err(),
            "TRASH_SAVE_FAILED"
        );
        let failed = svc.snapshot(false);
        assert_eq!(failed.state, Phase::Error);
        assert_eq!(failed.snoozed_until, 0);
        // 重试仅重新保存，成功后隐藏
        let saved = svc
            .save_snooze_with(&id, |until| {
                assert!(until > Local::now().timestamp_millis());
                Ok(())
            })
            .unwrap();
        assert_eq!(saved.state, Phase::Hidden);
        // 已隐藏后重复暂缓是幂等 no-op，不再触发保存
        svc.save_snooze_with(&id, |_| panic!("重复暂缓不能再次保存"))
            .unwrap();
    }
    #[test]
    fn stale_snooze_cannot_write() {
        let svc = test_service();
        svc.commit_measurement(0, Measurement::Over, false);
        let id = svc.snapshot(false).reminder_id.unwrap();
        svc.invalidate(&mut svc.lock());
        assert_eq!(
            svc.save_snooze_with(&id, |_| panic!("过期提醒不能保存"))
                .unwrap_err(),
            "TRASH_STALE_REMINDER"
        );
    }
    #[test]
    fn stop_before_native_registration_wakes_new_listener() {
        let svc = test_service();
        svc.stop();
        let called = Arc::new(AtomicBool::new(false));
        let signal = called.clone();
        svc.install_native_stop(Box::new(move || {
            signal.store(true, Ordering::SeqCst);
        }));
        assert!(called.load(Ordering::SeqCst));
    }
    #[test]
    fn settings_are_fail_closed() {
        assert_eq!(
            parse_settings(&serde_json::json!({})).unwrap().0,
            Config::default()
        );
        for value in [
            serde_json::json!(null),
            serde_json::json!({"trashReminder": {"enabled": true, "threshold": 0}}),
            serde_json::json!({"trashReminder": {"enabled": "true", "threshold": 1}}),
        ] {
            assert!(parse_settings(&value).is_err());
        }
        for threshold in [512, 1024, 2048, 4096, 10240, 20480] {
            assert!(
                Config {
                    enabled: true,
                    threshold
                }
                .validate()
                .is_ok()
            );
        }
        // 旧版小容量阈值（1/10/50 MB）已下架：写入校验拒绝，读路径归一化为默认 4096
        for threshold in [1, 10, 50] {
            assert!(
                Config {
                    enabled: true,
                    threshold
                }
                .validate()
                .is_err()
            );
            let (config, _) = parse_settings(&serde_json::json!({
                "trashReminder": {"enabled": true, "threshold": threshold}
            }))
            .unwrap();
            assert_eq!(config.threshold, 4096);
        }
    }
    #[test]
    fn midnight_is_calendar_boundary() {
        let now = chrono::FixedOffset::east_opt(8 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 3, 2, 23, 58, 0)
            .unwrap();
        assert_eq!(
            next_midnight(&now).unwrap() - now.timestamp_millis(),
            120_000
        );
        assert_eq!(
            effective_snooze(&now, now.timestamp_millis() + 86_400_000),
            0
        );
        assert_eq!(
            effective_snooze(&now, next_midnight(&now).unwrap()),
            next_midnight(&now).unwrap()
        );
    }
    #[test]
    fn debounce_is_bounded() {
        let start = Instant::now();
        let mut d = Debounce::default();
        d.mark(start);
        assert_eq!(d.due(), Some(start + Duration::from_secs(1)));
        for second in 1..10 {
            d.mark(start + Duration::from_secs(second));
        }
        assert_eq!(d.due(), Some(start + Duration::from_secs(5)));
    }
    #[test]
    fn threshold_and_unknown_and_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(measure(dir.path(), 1, || false), Measurement::Below);
        std::fs::write(dir.path().join("item"), vec![0; 1024 * 1024]).unwrap();
        assert_eq!(
            measure(dir.path(), 1024 * 1024, || false),
            Measurement::Over
        );
        assert_eq!(
            measure(dir.path(), 1024 * 1024 + 1, || false),
            Measurement::Below
        );
        assert_eq!(
            measure(&dir.path().join("missing"), 1, || false),
            Measurement::Unknown
        );
        assert_eq!(measure(dir.path(), 1, || true), Measurement::Cancelled);
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_do_not_follow_targets() {
        let dir = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        std::fs::write(external.path().join("large"), vec![0; 1024 * 1024]).unwrap();
        std::os::unix::fs::symlink(external.path(), dir.path().join("link")).unwrap();
        assert_eq!(
            measure(dir.path(), 1024 * 1024, || false),
            Measurement::Below
        );
    }
}
