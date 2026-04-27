use serde::Serialize;
use std::time::Instant;

use super::metrics_process::ProcessInfo;

#[derive(Debug, Clone)]
pub struct ProcessWatchOptions {
    pub enabled: bool,
    pub cpu_threshold: f64,
    pub window_secs: f64,
}

impl Default for ProcessWatchOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            cpu_threshold: 100.0,
            window_secs: 300.0,
        }
    }
}

impl ProcessWatchOptions {
    pub fn snapshot_config(&self) -> ProcessWatchConfig {
        let total_secs = self.window_secs as u64;
        let m = total_secs / 60;
        let s = total_secs % 60;
        ProcessWatchConfig {
            enabled: self.enabled,
            cpu_threshold: self.cpu_threshold,
            window: format!("{}m{}s", m, s),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessWatchConfig {
    pub enabled: bool,
    pub cpu_threshold: f64,
    pub window: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessAlert {
    pub pid: i32,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub command: String,
    pub cpu: f64,
    pub threshold: f64,
    pub window: String,
    pub triggered_at: String,
    pub status: String,
}

#[derive(Debug)]
struct TrackedProcess {
    info: ProcessInfo,
    first_above: Option<Instant>,
    triggered_at: Option<Instant>,
    current_above: bool,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct ProcessIdentity {
    pid: i32,
    ppid: i32,
    command: String,
}

pub struct ProcessWatcher {
    options: ProcessWatchOptions,
    tracks: std::collections::HashMap<ProcessIdentity, TrackedProcess>,
}

impl ProcessWatcher {
    pub fn new(options: ProcessWatchOptions) -> Self {
        Self {
            options,
            tracks: std::collections::HashMap::new(),
        }
    }

    pub fn update(&mut self, now: Instant, processes: &[ProcessInfo]) -> Vec<ProcessAlert> {
        if !self.options.enabled {
            return Vec::new();
        }

        let mut seen = std::collections::HashSet::new();
        for proc in processes {
            if proc.pid <= 0 {
                continue;
            }
            let key = ProcessIdentity {
                pid: proc.pid,
                ppid: proc.ppid,
                command: proc.command.clone(),
            };
            seen.insert(key.clone());

            let track = self.tracks.entry(key).or_insert_with(|| TrackedProcess {
                info: proc.clone(),
                first_above: None,
                triggered_at: None,
                current_above: false,
            });

            track.info = proc.clone();
            track.current_above = proc.cpu >= self.options.cpu_threshold;

            if track.current_above {
                if track.first_above.is_none() {
                    track.first_above = Some(now);
                }
                let window = std::time::Duration::from_secs_f64(self.options.window_secs);
                if now.duration_since(track.first_above.unwrap()) >= window
                    && track.triggered_at.is_none()
                {
                    track.triggered_at = Some(now);
                }
            } else {
                track.first_above = None;
                track.triggered_at = None;
            }
        }

        self.tracks.retain(|k, _| seen.contains(k));
        self.snapshot()
    }

    pub(crate) fn snapshot(&self) -> Vec<ProcessAlert> {
        if !self.options.enabled {
            return Vec::new();
        }

        let mut alerts: Vec<ProcessAlert> = self
            .tracks
            .values()
            .filter(|t| t.current_above && t.triggered_at.is_some())
            .map(|t| {
                let triggered = t.triggered_at.unwrap();
                let elapsed = Instant::now().duration_since(triggered);
                let seconds = elapsed.as_secs();
                let ts = format!(
                    "{}",
                    chrono::Utc::now() - chrono::Duration::seconds(seconds as i64)
                );
                ProcessAlert {
                    pid: t.info.pid,
                    name: t.info.name.clone(),
                    command: t.info.command.clone(),
                    cpu: t.info.cpu,
                    threshold: self.options.cpu_threshold,
                    window: format!("{}s", self.options.window_secs),
                    triggered_at: ts,
                    status: "active".into(),
                }
            })
            .collect();

        alerts.sort_by(|a, b| {
            b.cpu
                .partial_cmp(&a.cpu)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.pid.cmp(&b.pid))
        });

        alerts
    }
}
