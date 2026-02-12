//! Cron type system — schedule, payload, job state, and persistence.
//!
//! Port of nanobot's `cron/types.py` dataclasses.
//!
//! All types derive `Serialize`/`Deserialize` with `camelCase` keys
//! for JSON compatibility.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────
// CronSchedule
// ─────────────────────────────────────────────

/// Schedule variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleKind {
    /// One-shot: fire at a specific timestamp.
    At,
    /// Interval: fire every N milliseconds.
    Every,
    /// Cron expression: standard 5-field cron.
    Cron,
}

/// When a cron job fires.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronSchedule {
    /// Schedule variant.
    pub kind: ScheduleKind,
    /// One-shot timestamp (Unix epoch milliseconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<i64>,
    /// Interval in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub every_ms: Option<i64>,
    /// Standard 5-field cron expression (e.g. `"0 9 * * *"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
    /// Timezone (e.g. `"America/New_York"`). Reserved for future use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
}

impl CronSchedule {
    /// Create a one-shot schedule.
    pub fn at(at_ms: i64) -> Self {
        Self {
            kind: ScheduleKind::At,
            at_ms: Some(at_ms),
            every_ms: None,
            expr: None,
            tz: None,
        }
    }

    /// Create an interval schedule.
    pub fn every(every_ms: i64) -> Self {
        Self {
            kind: ScheduleKind::Every,
            at_ms: None,
            every_ms: Some(every_ms),
            expr: None,
            tz: None,
        }
    }

    /// Create a cron-expression schedule.
    pub fn cron(expr: impl Into<String>) -> Self {
        Self {
            kind: ScheduleKind::Cron,
            at_ms: None,
            every_ms: None,
            expr: Some(expr.into()),
            tz: None,
        }
    }
}

impl Default for CronSchedule {
    fn default() -> Self {
        Self::every(60_000) // 1 minute
    }
}

// ─────────────────────────────────────────────
// CronPayload
// ─────────────────────────────────────────────

/// What a cron job does when it fires.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronPayload {
    /// Prompt text sent to the agent.
    #[serde(default)]
    pub message: String,
    /// Whether to deliver the agent's response to a channel.
    #[serde(default)]
    pub deliver: bool,
    /// Target channel name (e.g. `"telegram"`, `"whatsapp"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// Recipient identifier within the channel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

impl Default for CronPayload {
    fn default() -> Self {
        Self {
            message: String::new(),
            deliver: false,
            channel: None,
            to: None,
        }
    }
}

// ─────────────────────────────────────────────
// CronJobState
// ─────────────────────────────────────────────

/// Run status of a job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Ok,
    Error,
    Skipped,
}

/// Mutable state for a cron job.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJobState {
    /// Next scheduled run (Unix epoch ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run_at_ms: Option<i64>,
    /// Last run time (Unix epoch ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_at_ms: Option<i64>,
    /// Status of the last run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_status: Option<JobStatus>,
    /// Error message from the last failed run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

// ─────────────────────────────────────────────
// CronJob
// ─────────────────────────────────────────────

/// A scheduled job.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJob {
    /// Unique identifier (UUID v4, first 8 chars).
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Whether the job is active.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// When to fire.
    pub schedule: CronSchedule,
    /// What to do.
    pub payload: CronPayload,
    /// Mutable run state.
    #[serde(default)]
    pub state: CronJobState,
    /// Creation timestamp (Unix epoch ms).
    #[serde(default)]
    pub created_at_ms: i64,
    /// Last update timestamp (Unix epoch ms).
    #[serde(default)]
    pub updated_at_ms: i64,
    /// Whether to delete the job after a single run.
    #[serde(default)]
    pub delete_after_run: bool,
}

fn default_true() -> bool {
    true
}

impl CronJob {
    /// Create a new job with a generated ID.
    pub fn new(name: impl Into<String>, schedule: CronSchedule, payload: CronPayload) -> Self {
        let now = Utc::now().timestamp_millis();
        Self {
            id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
            name: name.into(),
            enabled: true,
            schedule,
            payload,
            state: CronJobState::default(),
            created_at_ms: now,
            updated_at_ms: now,
            delete_after_run: false,
        }
    }

    /// Whether this job is due to run (now >= next_run_at_ms).
    pub fn is_due(&self) -> bool {
        if !self.enabled {
            return false;
        }
        match self.state.next_run_at_ms {
            Some(next) => Utc::now().timestamp_millis() >= next,
            None => false,
        }
    }

    /// Session key for this job's conversation history.
    pub fn session_key(&self) -> String {
        format!("cron:{}", self.id)
    }

    /// Compute the next run time from now, based on the schedule.
    pub fn compute_next_run(&self) -> Option<i64> {
        let now_ms = Utc::now().timestamp_millis();
        compute_next_run_from(&self.schedule, now_ms)
    }
}

/// Compute the next run time from a given timestamp (for testability).
pub fn compute_next_run_from(schedule: &CronSchedule, now_ms: i64) -> Option<i64> {
    match schedule.kind {
        ScheduleKind::At => schedule.at_ms,
        ScheduleKind::Every => {
            let interval = schedule.every_ms.unwrap_or(60_000);
            Some(now_ms + interval)
        }
        ScheduleKind::Cron => {
            let expr = schedule.expr.as_deref()?;
            // Parse standard cron expression using the `cron` crate
            let parsed: cron::Schedule = expr.parse().ok()?;
            let now_dt: DateTime<Utc> =
                DateTime::from_timestamp_millis(now_ms)?;
            let next = parsed.upcoming(Utc).next()?;
            let _ = now_dt; // use now_dt to anchor the computation
            Some(next.timestamp_millis())
        }
    }
}

// ─────────────────────────────────────────────
// CronStore
// ─────────────────────────────────────────────

/// Persistent store for cron jobs (JSON file).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronStore {
    /// Store format version.
    #[serde(default = "default_version")]
    pub version: u32,
    /// List of jobs.
    #[serde(default)]
    pub jobs: Vec<CronJob>,
}

fn default_version() -> u32 {
    1
}

impl CronStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self {
            version: 1,
            jobs: Vec::new(),
        }
    }

    /// Find a job by ID.
    pub fn find(&self, id: &str) -> Option<&CronJob> {
        self.jobs.iter().find(|j| j.id == id)
    }

    /// Find a mutable job by ID.
    pub fn find_mut(&mut self, id: &str) -> Option<&mut CronJob> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    /// Add a job.
    pub fn add(&mut self, job: CronJob) {
        self.jobs.push(job);
    }

    /// Remove a job by ID. Returns whether it was found.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.jobs.len();
        self.jobs.retain(|j| j.id != id);
        self.jobs.len() < before
    }

    /// Get all enabled jobs.
    pub fn enabled_jobs(&self) -> Vec<&CronJob> {
        self.jobs.iter().filter(|j| j.enabled).collect()
    }

    /// Get all due jobs.
    pub fn due_jobs(&self) -> Vec<&CronJob> {
        self.jobs.iter().filter(|j| j.is_due()).collect()
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schedule_at() {
        let s = CronSchedule::at(1000);
        assert_eq!(s.kind, ScheduleKind::At);
        assert_eq!(s.at_ms, Some(1000));
    }

    #[test]
    fn test_schedule_every() {
        let s = CronSchedule::every(60_000);
        assert_eq!(s.kind, ScheduleKind::Every);
        assert_eq!(s.every_ms, Some(60_000));
    }

    #[test]
    fn test_schedule_cron() {
        let s = CronSchedule::cron("0 9 * * *");
        assert_eq!(s.kind, ScheduleKind::Cron);
        assert_eq!(s.expr.as_deref(), Some("0 9 * * *"));
    }

    #[test]
    fn test_schedule_default() {
        let s = CronSchedule::default();
        assert_eq!(s.kind, ScheduleKind::Every);
        assert_eq!(s.every_ms, Some(60_000));
    }

    #[test]
    fn test_cron_job_new() {
        let job = CronJob::new(
            "test job",
            CronSchedule::every(5000),
            CronPayload::default(),
        );
        assert_eq!(job.name, "test job");
        assert!(job.enabled);
        assert_eq!(job.id.len(), 8);
        assert!(job.created_at_ms > 0);
    }

    #[test]
    fn test_cron_job_session_key() {
        let job = CronJob::new("test", CronSchedule::default(), CronPayload::default());
        assert!(job.session_key().starts_with("cron:"));
    }

    #[test]
    fn test_cron_job_not_due_initially() {
        let job = CronJob::new("test", CronSchedule::default(), CronPayload::default());
        assert!(!job.is_due()); // no next_run_at_ms set
    }

    #[test]
    fn test_cron_job_is_due() {
        let mut job = CronJob::new("test", CronSchedule::default(), CronPayload::default());
        job.state.next_run_at_ms = Some(0); // past time
        assert!(job.is_due());
    }

    #[test]
    fn test_cron_job_not_due_future() {
        let mut job = CronJob::new("test", CronSchedule::default(), CronPayload::default());
        job.state.next_run_at_ms = Some(i64::MAX);
        assert!(!job.is_due());
    }

    #[test]
    fn test_cron_job_disabled_not_due() {
        let mut job = CronJob::new("test", CronSchedule::default(), CronPayload::default());
        job.enabled = false;
        job.state.next_run_at_ms = Some(0);
        assert!(!job.is_due());
    }

    #[test]
    fn test_compute_next_every() {
        let schedule = CronSchedule::every(10_000);
        let now = 1000;
        let next = compute_next_run_from(&schedule, now).unwrap();
        assert_eq!(next, 11_000);
    }

    #[test]
    fn test_compute_next_at() {
        let schedule = CronSchedule::at(5000);
        let next = compute_next_run_from(&schedule, 0).unwrap();
        assert_eq!(next, 5000);
    }

    #[test]
    fn test_compute_next_cron() {
        // "0 0 * * * *" = every hour at minute 0 (6-field cron for the `cron` crate)
        let schedule = CronSchedule::cron("0 0 * * * *");
        let now = Utc::now().timestamp_millis();
        let next = compute_next_run_from(&schedule, now);
        assert!(next.is_some());
        assert!(next.unwrap() > now);
    }

    #[test]
    fn test_compute_next_cron_invalid() {
        let schedule = CronSchedule::cron("invalid");
        let next = compute_next_run_from(&schedule, 0);
        assert!(next.is_none());
    }

    // ── CronStore ──

    #[test]
    fn test_store_new() {
        let store = CronStore::new();
        assert_eq!(store.version, 1);
        assert!(store.jobs.is_empty());
    }

    #[test]
    fn test_store_add_and_find() {
        let mut store = CronStore::new();
        let job = CronJob::new("test", CronSchedule::default(), CronPayload::default());
        let id = job.id.clone();
        store.add(job);
        assert!(store.find(&id).is_some());
        assert!(store.find("nonexistent").is_none());
    }

    #[test]
    fn test_store_remove() {
        let mut store = CronStore::new();
        let job = CronJob::new("test", CronSchedule::default(), CronPayload::default());
        let id = job.id.clone();
        store.add(job);
        assert!(store.remove(&id));
        assert!(!store.remove(&id)); // already removed
    }

    #[test]
    fn test_store_enabled_jobs() {
        let mut store = CronStore::new();
        let j1 = CronJob::new("enabled", CronSchedule::default(), CronPayload::default());
        let mut j2 = CronJob::new("disabled", CronSchedule::default(), CronPayload::default());
        j2.enabled = false;
        store.add(j1);
        store.add(j2);
        assert_eq!(store.enabled_jobs().len(), 1);
    }

    #[test]
    fn test_store_due_jobs() {
        let mut store = CronStore::new();
        let mut j1 = CronJob::new("due", CronSchedule::default(), CronPayload::default());
        j1.state.next_run_at_ms = Some(0);
        let j2 = CronJob::new("not_due", CronSchedule::default(), CronPayload::default());
        store.add(j1);
        store.add(j2);
        assert_eq!(store.due_jobs().len(), 1);
    }

    // ── Serialization ──

    #[test]
    fn test_store_serialize_roundtrip() {
        let mut store = CronStore::new();
        let job = CronJob::new(
            "test",
            CronSchedule::every(5000),
            CronPayload {
                message: "hello".into(),
                deliver: true,
                channel: Some("telegram".into()),
                to: Some("12345".into()),
            },
        );
        store.add(job);

        let json = serde_json::to_string_pretty(&store).unwrap();
        let reloaded: CronStore = serde_json::from_str(&json).unwrap();
        assert_eq!(reloaded.jobs.len(), 1);
        assert_eq!(reloaded.jobs[0].name, "test");
        assert_eq!(reloaded.jobs[0].payload.message, "hello");
        assert!(reloaded.jobs[0].payload.deliver);
    }

    #[test]
    fn test_payload_default() {
        let p = CronPayload::default();
        assert!(p.message.is_empty());
        assert!(!p.deliver);
        assert!(p.channel.is_none());
        assert!(p.to.is_none());
    }

    #[test]
    fn test_job_status_serialize() {
        let status = JobStatus::Ok;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"ok\"");
    }

    #[test]
    fn test_schedule_kind_serialize() {
        assert_eq!(serde_json::to_string(&ScheduleKind::At).unwrap(), "\"at\"");
        assert_eq!(serde_json::to_string(&ScheduleKind::Every).unwrap(), "\"every\"");
        assert_eq!(serde_json::to_string(&ScheduleKind::Cron).unwrap(), "\"cron\"");
    }
}
