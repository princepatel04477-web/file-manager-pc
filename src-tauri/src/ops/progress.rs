//! Long-running operation bookkeeping: progress snapshots and cancellation flags.
//!
//! The registry holds no Tauri types, so the emission policy lives in the command
//! layer and the bookkeeping stays unit-testable.

use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PROGRESS_EVENT: &str = "ops://progress";
/// How much of a file to copy between cancellation checks.
pub const CANCEL_CHECK_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum OpsKind {
    Copy,
    Move,
    Rename,
    Delete,
    /// A Clean tab scan (junk walk, duplicate hashing).
    Scan,
    /// Moving Clean tab selections to the Recycle Bin.
    Clean,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum OpsState {
    Running,
    Completed,
    Cancelled,
    Failed,
}

impl OpsState {
    pub fn is_finished(self) -> bool {
        !matches!(self, OpsState::Running)
    }
}

/// Snapshot pushed to the frontend on the `ops://progress` channel.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OperationProgress {
    pub job_id: String,
    pub kind: OpsKind,
    pub state: OpsState,
    pub items_total: u64,
    pub items_done: u64,
    pub items_skipped: u64,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub current: String,
    pub destination: String,
    pub started_unix: u64,
    pub finished_unix: Option<u64>,
    pub error: Option<String>,
}

impl OperationProgress {
    /// 0..=100, derived from bytes when known and items otherwise.
    pub fn percent(&self) -> u8 {
        if self.bytes_total > 0 {
            let ratio = self.bytes_done.min(self.bytes_total) as f64 / self.bytes_total as f64;
            return (ratio * 100.0).round().clamp(0.0, 100.0) as u8;
        }
        if self.items_total > 0 {
            let ratio = self.items_done.min(self.items_total) as f64 / self.items_total as f64;
            return (ratio * 100.0).round().clamp(0.0, 100.0) as u8;
        }
        if matches!(self.state, OpsState::Completed) { 100 } else { 0 }
    }
}

struct Job {
    progress: OperationProgress,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct OpsRegistry {
    jobs: Mutex<HashMap<String, Arc<Job>>>,
    next_id: AtomicU64,
}

#[derive(Clone)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn requested(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

impl OpsRegistry {
    pub fn start(&self, kind: OpsKind, items_total: u64, bytes_total: u64, destination: String) -> (String, CancelToken, OperationProgress) {
        let job_id = format!("op-{}", self.next_id.fetch_add(1, Ordering::SeqCst) + 1);
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = OperationProgress {
            job_id: job_id.clone(),
            kind,
            state: OpsState::Running,
            items_total,
            items_done: 0,
            items_skipped: 0,
            bytes_total,
            bytes_done: 0,
            current: String::new(),
            destination,
            started_unix: now_unix(),
            finished_unix: None,
            error: None,
        };
        let job = Arc::new(Job { progress: progress.clone(), cancel: cancel.clone() });
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.insert(job_id.clone(), job);
        }
        (job_id, CancelToken { flag: cancel }, progress)
    }

    /// Record the item that is about to start.
    pub fn set_current(&self, job_id: &str, current: &str) -> Option<OperationProgress> {
        self.mutate(job_id, |progress| progress.current = current.to_owned())
    }

    pub fn advance_bytes(&self, job_id: &str, delta: u64) -> Option<OperationProgress> {
        self.mutate(job_id, |progress| {
            progress.bytes_done = progress.bytes_done.saturating_add(delta);
        })
    }

    pub fn finish_item(&self, job_id: &str, bytes: u64) -> Option<OperationProgress> {
        self.mutate(job_id, |progress| {
            progress.items_done = progress.items_done.saturating_add(1);
            progress.bytes_done = progress.bytes_done.saturating_add(bytes);
            progress.current.clear();
        })
    }

    pub fn skip_item(&self, job_id: &str) -> Option<OperationProgress> {
        self.mutate(job_id, |progress| {
            progress.items_skipped = progress.items_skipped.saturating_add(1);
        })
    }

    /// True when the job exists and cancellation has not already been requested.
    pub fn request_cancel(&self, job_id: &str) -> bool {
        let Ok(jobs) = self.jobs.lock() else { return false };
        let Some(job) = jobs.get(job_id) else { return false };
        if job.progress.state.is_finished() {
            return false;
        }
        !job.cancel.swap(true, Ordering::SeqCst)
    }

    pub fn cancel_requested(&self, job_id: &str) -> bool {
        self.jobs
            .lock()
            .ok()
            .and_then(|jobs| jobs.get(job_id).map(|job| job.cancel.load(Ordering::SeqCst)))
            .unwrap_or(false)
    }

    pub fn snapshot(&self, job_id: &str) -> Option<OperationProgress> {
        self.jobs.lock().ok()?.get(job_id).map(|job| job.progress.clone())
    }

    pub fn active(&self) -> Vec<OperationProgress> {
        let Ok(jobs) = self.jobs.lock() else { return Vec::new() };
        let mut running: Vec<OperationProgress> = jobs
            .values()
            .filter(|job| !job.progress.state.is_finished())
            .map(|job| job.progress.clone())
            .collect();
        running.sort_by_key(|left| left.started_unix);
        running
    }

    pub fn finish(&self, job_id: &str, state: OpsState, error: Option<String>) -> Option<OperationProgress> {
        self.mutate(job_id, |progress| {
            progress.state = state;
            progress.error = error;
            progress.finished_unix = Some(now_unix());
            if matches!(state, OpsState::Completed) {
                progress.bytes_done = progress.bytes_total.max(progress.bytes_done);
                progress.items_done = progress.items_total.max(progress.items_done);
                progress.current.clear();
            }
        })
    }

    /// Drop finished jobs so the map does not grow for the lifetime of the process.
    pub fn prune_finished(&self, keep: usize) {
        let Ok(mut jobs) = self.jobs.lock() else { return };
        let mut finished: Vec<(u64, String)> = jobs
            .iter()
            .filter(|(_, job)| job.progress.state.is_finished())
            .map(|(id, job)| (job.progress.finished_unix.unwrap_or(0), id.clone()))
            .collect();
        finished.sort_unstable();
        let overflow = finished.len().saturating_sub(keep);
        for (_, id) in finished.into_iter().take(overflow) {
            jobs.remove(&id);
        }
    }

    fn mutate(&self, job_id: &str, edit: impl FnOnce(&mut OperationProgress)) -> Option<OperationProgress> {
        let mut jobs = self.jobs.lock().ok()?;
        let job = jobs.get(job_id)?.clone();
        let mut progress = job.progress.clone();
        edit(&mut progress);
        *jobs.get_mut(job_id)? = Arc::new(Job { progress: progress.clone(), cancel: job.cancel.clone() });
        Some(progress)
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_start_running_and_report_progress() {
        let registry = OpsRegistry::default();
        let (id, _token, initial) = registry.start(OpsKind::Copy, 4, 1_000, "C:\\Users\\u\\Documents".to_owned());
        assert_eq!(initial.state, OpsState::Running);
        assert_eq!(registry.active().len(), 1);

        registry.set_current(&id, "report.pdf");
        registry.advance_bytes(&id, 250);
        let snapshot = registry.snapshot(&id).expect("job tracked");
        assert_eq!(snapshot.bytes_done, 250);
        assert_eq!(snapshot.percent(), 25);
        assert_eq!(snapshot.current, "report.pdf");

        registry.finish_item(&id, 250);
        let snapshot = registry.snapshot(&id).expect("job tracked");
        assert_eq!(snapshot.items_done, 1);
        assert_eq!(snapshot.bytes_done, 500);
        assert!(snapshot.current.is_empty());
    }

    #[test]
    fn cancellation_is_visible_to_the_worker_and_only_once() {
        let registry = OpsRegistry::default();
        let (id, token, _) = registry.start(OpsKind::Delete, 2, 0, String::new());
        assert!(!token.requested());
        assert!(registry.request_cancel(&id), "first request is honoured");
        assert!(token.requested(), "worker sees the flag");
        assert!(!registry.request_cancel(&id), "second request is a no-op");
        assert!(!registry.request_cancel("op-missing"));
    }

    #[test]
    fn finished_jobs_leave_the_active_list_and_stop_accepting_cancellation() {
        let registry = OpsRegistry::default();
        let (id, _, _) = registry.start(OpsKind::Move, 1, 10, String::new());
        let finished = registry.finish(&id, OpsState::Cancelled, Some("Stopped by you.".to_owned())).expect("job tracked");
        assert_eq!(finished.state, OpsState::Cancelled);
        assert!(finished.finished_unix.is_some());
        assert!(registry.active().is_empty());
        assert!(!registry.request_cancel(&id));
    }

    #[test]
    fn percent_prefers_bytes_and_caps_at_one_hundred() {
        let registry = OpsRegistry::default();
        let (id, _, _) = registry.start(OpsKind::Copy, 2, 100, String::new());
        registry.advance_bytes(&id, 1_000);
        assert_eq!(registry.snapshot(&id).expect("job").percent(), 100);

        let (items_only, _, _) = registry.start(OpsKind::Delete, 4, 0, String::new());
        registry.finish_item(&items_only, 0);
        assert_eq!(registry.snapshot(&items_only).expect("job").percent(), 25);
    }

    #[test]
    fn pruning_keeps_only_the_most_recent_finished_jobs() {
        let registry = OpsRegistry::default();
        for index in 0..5 {
            let (id, _, _) = registry.start(OpsKind::Copy, 1, 1, format!("C:\\dst{index}"));
            registry.finish(&id, OpsState::Completed, None);
        }
        registry.prune_finished(2);
        assert_eq!(registry.active().len(), 0);
        let tracked = registry.jobs.lock().expect("lock").len();
        assert_eq!(tracked, 2);
    }
}
