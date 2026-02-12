//! Cron service — custom async scheduler with job persistence.
//!
//! Port of nanobot's `cron/service.py`.
//!
//! Architecture:
//! - Jobs stored in `~/.oxibot/cron/jobs.json`
//! - Timer sleeps until the nearest `next_run_at_ms`, then fires due jobs
//! - Job execution invokes a callback (typically `agent.process_direct()`)
//! - Results optionally delivered to a channel via the message bus
//!
//! No APScheduler. Fully custom async timer using `tokio::time::sleep`.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::{Mutex, Notify};
use tracing::{debug, error, info, warn};

use oxibot_core::bus::queue::MessageBus;
use oxibot_core::bus::types::OutboundMessage;

use crate::types::{
    compute_next_run_from, CronJob, CronStore, JobStatus, ScheduleKind,
};

// ─────────────────────────────────────────────
// Job callback type
// ─────────────────────────────────────────────

/// Callback invoked when a job fires.
///
/// Receives the job reference and returns the agent's response text.
/// In the gateway, this typically wraps `agent.process_direct()`.
pub type OnJobFn =
    Arc<dyn Fn(CronJob) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>> + Send + Sync>;

// ─────────────────────────────────────────────
// CronService
// ─────────────────────────────────────────────

/// Cron scheduler — manages jobs, persistence, and timed execution.
pub struct CronService {
    /// Path to the jobs JSON file.
    store_path: PathBuf,
    /// In-memory job store (protected by mutex for async safety).
    store: Arc<Mutex<CronStore>>,
    /// Message bus for outbound delivery.
    bus: Arc<MessageBus>,
    /// Callback for job execution (agent.process_direct).
    on_job: Arc<Mutex<Option<OnJobFn>>>,
    /// Shutdown signal.
    shutdown: Arc<Notify>,
    /// Re-arm signal (when jobs are added/modified).
    rearm: Arc<Notify>,
}

impl CronService {
    /// Create a new cron service.
    ///
    /// If `store_path` is `None`, defaults to `~/.oxibot/cron/jobs.json`.
    pub fn new(bus: Arc<MessageBus>, store_path: Option<PathBuf>) -> Self {
        let path = store_path.unwrap_or_else(|| {
            let data_dir = oxibot_core::utils::get_data_path();
            data_dir.join("cron").join("jobs.json")
        });

        Self {
            store_path: path,
            store: Arc::new(Mutex::new(CronStore::new())),
            bus,
            on_job: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(Notify::new()),
            rearm: Arc::new(Notify::new()),
        }
    }

    /// Set the on-job callback.
    pub async fn set_on_job(&self, callback: OnJobFn) {
        let mut on_job = self.on_job.lock().await;
        *on_job = Some(callback);
    }

    // ─────────────────────────────────────────
    // Persistence
    // ─────────────────────────────────────────

    /// Load the store from disk.
    pub async fn load(&self) -> anyhow::Result<()> {
        if !self.store_path.exists() {
            debug!(path = %self.store_path.display(), "no cron store file, starting empty");
            return Ok(());
        }

        let data = tokio::fs::read_to_string(&self.store_path).await?;
        let loaded: CronStore = serde_json::from_str(&data)
            .map_err(|e| anyhow::anyhow!("failed to parse cron store: {}", e))?;

        let mut store = self.store.lock().await;
        *store = loaded;
        info!(
            path = %self.store_path.display(),
            jobs = store.jobs.len(),
            "loaded cron store"
        );
        Ok(())
    }

    /// Save the store to disk.
    pub async fn save(&self) -> anyhow::Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.store_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let store = self.store.lock().await;
        let json = serde_json::to_string_pretty(&*store)?;
        tokio::fs::write(&self.store_path, json).await?;
        debug!(path = %self.store_path.display(), "saved cron store");
        Ok(())
    }

    // ─────────────────────────────────────────
    // Job management
    // ─────────────────────────────────────────

    /// Add a job. Computes next run time and saves.
    pub async fn add_job(&self, mut job: CronJob) -> anyhow::Result<String> {
        // Compute initial next_run
        let now_ms = Utc::now().timestamp_millis();
        job.state.next_run_at_ms = compute_next_run_from(&job.schedule, now_ms);

        let id = job.id.clone();
        {
            let mut store = self.store.lock().await;
            store.add(job);
        }
        self.save().await?;
        self.rearm.notify_one();
        info!(id = %id, "added cron job");
        Ok(id)
    }

    /// Remove a job by ID.
    pub async fn remove_job(&self, id: &str) -> anyhow::Result<bool> {
        let removed = {
            let mut store = self.store.lock().await;
            store.remove(id)
        };
        if removed {
            self.save().await?;
            self.rearm.notify_one();
            info!(id = %id, "removed cron job");
        }
        Ok(removed)
    }

    /// Enable or disable a job.
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> anyhow::Result<bool> {
        let found = {
            let mut store = self.store.lock().await;
            if let Some(job) = store.find_mut(id) {
                job.enabled = enabled;
                job.updated_at_ms = Utc::now().timestamp_millis();
                if enabled {
                    let now = Utc::now().timestamp_millis();
                    job.state.next_run_at_ms = compute_next_run_from(&job.schedule, now);
                }
                true
            } else {
                false
            }
        };
        if found {
            self.save().await?;
            self.rearm.notify_one();
        }
        Ok(found)
    }

    /// List all jobs (snapshot).
    pub async fn list_jobs(&self) -> Vec<CronJob> {
        let store = self.store.lock().await;
        store.jobs.clone()
    }

    /// Get a single job by ID.
    pub async fn get_job(&self, id: &str) -> Option<CronJob> {
        let store = self.store.lock().await;
        store.find(id).cloned()
    }

    // ─────────────────────────────────────────
    // Timer loop
    // ─────────────────────────────────────────

    /// Start the scheduler loop.
    ///
    /// Loads the store, then enters a loop:
    /// 1. Find nearest `next_run_at_ms`
    /// 2. Sleep until that time (or shutdown/rearm signal)
    /// 3. Execute all due jobs
    /// 4. Recompute and repeat
    pub async fn start(&self) -> anyhow::Result<()> {
        // Load persisted jobs
        if let Err(e) = self.load().await {
            warn!(error = %e, "failed to load cron store, starting empty");
        }

        info!("cron service started");

        loop {
            // Find how long to sleep
            let sleep_ms = {
                let store = self.store.lock().await;
                Self::next_wake_ms(&store)
            };

            let sleep_duration = if let Some(ms) = sleep_ms {
                let delay = (ms - Utc::now().timestamp_millis()).max(0) as u64;
                std::time::Duration::from_millis(delay)
            } else {
                // No scheduled jobs — sleep a long time, rearm will wake us
                std::time::Duration::from_secs(3600)
            };

            debug!(sleep_ms = sleep_duration.as_millis() as u64, "cron timer armed");

            tokio::select! {
                _ = tokio::time::sleep(sleep_duration) => {
                    // Timer fired — execute due jobs
                    self.execute_due_jobs().await;
                }
                _ = self.rearm.notified() => {
                    debug!("cron timer re-armed (job added/modified)");
                    // Loop back to recalculate sleep
                }
                _ = self.shutdown.notified() => {
                    info!("cron service shutting down");
                    return Ok(());
                }
            }
        }
    }

    /// Stop the scheduler.
    pub async fn stop(&self) {
        info!("stopping cron service");
        self.shutdown.notify_waiters();
    }

    /// Find the nearest next_run_at_ms across all enabled jobs.
    fn next_wake_ms(store: &CronStore) -> Option<i64> {
        store
            .jobs
            .iter()
            .filter(|j| j.enabled)
            .filter_map(|j| j.state.next_run_at_ms)
            .min()
    }

    /// Execute all due jobs.
    async fn execute_due_jobs(&self) {
        // Collect due job IDs (avoid holding lock during execution)
        let due_ids: Vec<String> = {
            let store = self.store.lock().await;
            store
                .due_jobs()
                .iter()
                .map(|j| j.id.clone())
                .collect()
        };

        if due_ids.is_empty() {
            return;
        }

        debug!(count = due_ids.len(), "executing due cron jobs");

        for id in &due_ids {
            self.execute_job(id).await;
        }
    }

    /// Execute a single job by ID.
    pub async fn execute_job(&self, id: &str) {
        // Get a snapshot of the job
        let job = {
            let store = self.store.lock().await;
            store.find(id).cloned()
        };

        let job = match job {
            Some(j) => j,
            None => {
                warn!(id = %id, "cron job not found for execution");
                return;
            }
        };

        info!(id = %job.id, name = %job.name, "executing cron job");

        // Invoke callback
        let result = {
            let on_job = self.on_job.lock().await;
            if let Some(ref callback) = *on_job {
                Some(callback(job.clone()).await)
            } else {
                warn!(id = %id, "no on_job callback set, skipping execution");
                None
            }
        };

        // Update job state
        let now_ms = Utc::now().timestamp_millis();
        let mut should_delete = false;

        {
            let mut store = self.store.lock().await;
            if let Some(j) = store.find_mut(id) {
                j.state.last_run_at_ms = Some(now_ms);

                match &result {
                    Some(Ok(response)) => {
                        j.state.last_status = Some(JobStatus::Ok);
                        j.state.last_error = None;

                        // Deliver response to channel if configured
                        if j.payload.deliver {
                            if let (Some(channel), Some(to)) =
                                (j.payload.channel.as_ref(), j.payload.to.as_ref())
                            {
                                let outbound = OutboundMessage {
                                    channel: channel.clone(),
                                    chat_id: to.clone(),
                                    content: response.clone(),
                                    reply_to: None,
                                    media: Vec::new(),
                                    metadata: std::collections::HashMap::new(),
                                };
                                if let Err(e) = self.bus.publish_outbound(outbound).await {
                                    error!(error = %e, "failed to deliver cron response");
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        j.state.last_status = Some(JobStatus::Error);
                        j.state.last_error = Some(e.to_string());
                        error!(
                            id = %id,
                            name = %j.name,
                            error = %e,
                            "cron job failed"
                        );
                    }
                    None => {
                        j.state.last_status = Some(JobStatus::Skipped);
                    }
                }

                // Compute next run
                if j.schedule.kind == ScheduleKind::At && j.delete_after_run {
                    should_delete = true;
                } else if j.schedule.kind == ScheduleKind::At {
                    j.enabled = false;
                    j.state.next_run_at_ms = None;
                } else {
                    j.state.next_run_at_ms = compute_next_run_from(&j.schedule, now_ms);
                }

                j.updated_at_ms = now_ms;
            }

            // Delete one-shot jobs
            if should_delete {
                store.remove(id);
            }
        }

        // Save
        if let Err(e) = self.save().await {
            error!(error = %e, "failed to save cron store after job execution");
        }
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CronPayload, CronSchedule};
    use tempfile::TempDir;

    fn make_bus() -> Arc<MessageBus> {
        Arc::new(MessageBus::new(10))
    }

    fn make_service(dir: &TempDir) -> CronService {
        let path = dir.path().join("jobs.json");
        CronService::new(make_bus(), Some(path))
    }

    #[tokio::test]
    async fn test_add_and_list() {
        let dir = TempDir::new().unwrap();
        let svc = make_service(&dir);

        let job = CronJob::new("test", CronSchedule::every(10_000), CronPayload::default());
        let id = svc.add_job(job).await.unwrap();

        let jobs = svc.list_jobs().await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, id);
        assert_eq!(jobs[0].name, "test");
    }

    #[tokio::test]
    async fn test_remove_job() {
        let dir = TempDir::new().unwrap();
        let svc = make_service(&dir);

        let job = CronJob::new("test", CronSchedule::every(10_000), CronPayload::default());
        let id = svc.add_job(job).await.unwrap();

        assert!(svc.remove_job(&id).await.unwrap());
        assert!(svc.list_jobs().await.is_empty());
    }

    #[tokio::test]
    async fn test_remove_nonexistent() {
        let dir = TempDir::new().unwrap();
        let svc = make_service(&dir);
        assert!(!svc.remove_job("xyz").await.unwrap());
    }

    #[tokio::test]
    async fn test_set_enabled() {
        let dir = TempDir::new().unwrap();
        let svc = make_service(&dir);

        let job = CronJob::new("test", CronSchedule::every(10_000), CronPayload::default());
        let id = svc.add_job(job).await.unwrap();

        svc.set_enabled(&id, false).await.unwrap();
        let jobs = svc.list_jobs().await;
        assert!(!jobs[0].enabled);

        svc.set_enabled(&id, true).await.unwrap();
        let jobs = svc.list_jobs().await;
        assert!(jobs[0].enabled);
    }

    #[tokio::test]
    async fn test_set_enabled_nonexistent() {
        let dir = TempDir::new().unwrap();
        let svc = make_service(&dir);
        assert!(!svc.set_enabled("xyz", true).await.unwrap());
    }

    #[tokio::test]
    async fn test_get_job() {
        let dir = TempDir::new().unwrap();
        let svc = make_service(&dir);

        let job = CronJob::new("test", CronSchedule::every(10_000), CronPayload::default());
        let id = svc.add_job(job).await.unwrap();

        assert!(svc.get_job(&id).await.is_some());
        assert!(svc.get_job("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_persistence_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("jobs.json");

        // Create and save
        {
            let svc = CronService::new(make_bus(), Some(path.clone()));
            let job = CronJob::new(
                "persistent",
                CronSchedule::every(5000),
                CronPayload {
                    message: "hello".into(),
                    deliver: true,
                    channel: Some("telegram".into()),
                    to: Some("12345".into()),
                },
            );
            svc.add_job(job).await.unwrap();
        }

        // Reload
        {
            let svc = CronService::new(make_bus(), Some(path));
            svc.load().await.unwrap();
            let jobs = svc.list_jobs().await;
            assert_eq!(jobs.len(), 1);
            assert_eq!(jobs[0].name, "persistent");
            assert_eq!(jobs[0].payload.message, "hello");
        }
    }

    #[tokio::test]
    async fn test_load_nonexistent_file() {
        let dir = TempDir::new().unwrap();
        let svc = make_service(&dir);
        // Should not error, just start empty
        svc.load().await.unwrap();
        assert!(svc.list_jobs().await.is_empty());
    }

    #[tokio::test]
    async fn test_execute_job_no_callback() {
        let dir = TempDir::new().unwrap();
        let svc = make_service(&dir);

        let mut job = CronJob::new("test", CronSchedule::every(10_000), CronPayload::default());
        job.state.next_run_at_ms = Some(0);
        let id = svc.add_job(job).await.unwrap();

        // Execute without callback — should mark as skipped
        svc.execute_job(&id).await;

        let j = svc.get_job(&id).await.unwrap();
        assert_eq!(j.state.last_status, Some(JobStatus::Skipped));
        assert!(j.state.last_run_at_ms.is_some());
    }

    #[tokio::test]
    async fn test_execute_job_with_callback() {
        let dir = TempDir::new().unwrap();
        let svc = make_service(&dir);

        let callback: OnJobFn = Arc::new(|_job| {
            Box::pin(async move { Ok("done".to_string()) })
        });
        svc.set_on_job(callback).await;

        let mut job = CronJob::new(
            "test",
            CronSchedule::every(10_000),
            CronPayload {
                message: "hello".into(),
                ..Default::default()
            },
        );
        job.state.next_run_at_ms = Some(0);
        let id = svc.add_job(job).await.unwrap();

        svc.execute_job(&id).await;

        let j = svc.get_job(&id).await.unwrap();
        assert_eq!(j.state.last_status, Some(JobStatus::Ok));
        assert!(j.state.next_run_at_ms.is_some());
    }

    #[tokio::test]
    async fn test_execute_job_error() {
        let dir = TempDir::new().unwrap();
        let svc = make_service(&dir);

        let callback: OnJobFn = Arc::new(|_job| {
            Box::pin(async move { Err(anyhow::anyhow!("boom")) })
        });
        svc.set_on_job(callback).await;

        let mut job = CronJob::new("failing", CronSchedule::every(10_000), CronPayload::default());
        job.state.next_run_at_ms = Some(0);
        let id = svc.add_job(job).await.unwrap();

        svc.execute_job(&id).await;

        let j = svc.get_job(&id).await.unwrap();
        assert_eq!(j.state.last_status, Some(JobStatus::Error));
        assert_eq!(j.state.last_error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn test_execute_oneshot_deleted() {
        let dir = TempDir::new().unwrap();
        let svc = make_service(&dir);

        let callback: OnJobFn = Arc::new(|_| Box::pin(async { Ok("ok".into()) }));
        svc.set_on_job(callback).await;

        let mut job = CronJob::new("oneshot", CronSchedule::at(0), CronPayload::default());
        job.delete_after_run = true;
        job.state.next_run_at_ms = Some(0);
        let id = svc.add_job(job).await.unwrap();

        svc.execute_job(&id).await;

        // Job should be deleted
        assert!(svc.get_job(&id).await.is_none());
    }

    #[tokio::test]
    async fn test_execute_oneshot_disabled() {
        let dir = TempDir::new().unwrap();
        let svc = make_service(&dir);

        let callback: OnJobFn = Arc::new(|_| Box::pin(async { Ok("ok".into()) }));
        svc.set_on_job(callback).await;

        let mut job = CronJob::new("oneshot", CronSchedule::at(0), CronPayload::default());
        job.delete_after_run = false;
        job.state.next_run_at_ms = Some(0);
        let id = svc.add_job(job).await.unwrap();

        svc.execute_job(&id).await;

        // Job should be disabled, not deleted
        let j = svc.get_job(&id).await.unwrap();
        assert!(!j.enabled);
        assert!(j.state.next_run_at_ms.is_none());
    }

    #[tokio::test]
    async fn test_execute_delivers_to_channel() {
        use tokio::time::{timeout, Duration};

        let dir = TempDir::new().unwrap();
        let bus = make_bus();
        let path = dir.path().join("jobs.json");
        let svc = CronService::new(bus.clone(), Some(path));

        let callback: OnJobFn = Arc::new(|_| Box::pin(async { Ok("response text".into()) }));
        svc.set_on_job(callback).await;

        let job = CronJob::new(
            "deliver",
            CronSchedule::every(10_000),
            CronPayload {
                message: "prompt".into(),
                deliver: true,
                channel: Some("telegram".into()),
                to: Some("user123".into()),
            },
        );
        let id = svc.add_job(job).await.unwrap();

        // Force job to be due NOW (add_job computes next_run in the future)
        {
            let mut store = svc.store.lock().await;
            if let Some(j) = store.find_mut(&id) {
                j.state.next_run_at_ms = Some(0);
            }
        }

        svc.execute_due_jobs().await;

        // Check outbound message was published (with timeout to avoid hanging)
        let outbound = timeout(Duration::from_secs(5), bus.consume_outbound())
            .await
            .expect("timed out waiting for outbound message");
        assert!(outbound.is_some());
        let msg = outbound.unwrap();
        assert_eq!(msg.channel, "telegram");
        assert_eq!(msg.chat_id, "user123");
        assert_eq!(msg.content, "response text");
    }

    #[tokio::test]
    async fn test_next_wake_ms() {
        let mut store = CronStore::new();

        // Empty store → None
        assert!(CronService::next_wake_ms(&store).is_none());

        // One job
        let mut j1 = CronJob::new("j1", CronSchedule::every(10_000), CronPayload::default());
        j1.state.next_run_at_ms = Some(5000);
        store.add(j1);
        assert_eq!(CronService::next_wake_ms(&store), Some(5000));

        // Two jobs — picks earliest
        let mut j2 = CronJob::new("j2", CronSchedule::every(10_000), CronPayload::default());
        j2.state.next_run_at_ms = Some(3000);
        store.add(j2);
        assert_eq!(CronService::next_wake_ms(&store), Some(3000));
    }

    #[tokio::test]
    async fn test_next_wake_ms_ignores_disabled() {
        let mut store = CronStore::new();
        let mut j1 = CronJob::new("j1", CronSchedule::every(10_000), CronPayload::default());
        j1.enabled = false;
        j1.state.next_run_at_ms = Some(1000);
        store.add(j1);
        assert!(CronService::next_wake_ms(&store).is_none());
    }

    #[tokio::test]
    async fn test_stop() {
        let dir = TempDir::new().unwrap();
        let svc = make_service(&dir);
        // stop should not error even without start
        svc.stop().await;
    }
}
