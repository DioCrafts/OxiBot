//! Heartbeat service — periodic agent wake-up to check for tasks.
//!
//! Port of nanobot's `heartbeat/service.py`.
//!
//! The agent reads `HEARTBEAT.md` from the workspace and executes any
//! tasks listed there. If nothing needs attention, it replies `HEARTBEAT_OK`.
//! If `HEARTBEAT.md` is empty or contains only headers, the tick is skipped.

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::future::Future;

use tokio::sync::Notify;
use tracing::{debug, error, info};

// ─────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────

/// Default interval: 30 minutes.
pub const DEFAULT_HEARTBEAT_INTERVAL_S: u64 = 30 * 60;

/// The prompt sent to the agent during a heartbeat tick.
pub const HEARTBEAT_PROMPT: &str = r#"Read HEARTBEAT.md in your workspace (if it exists).
Follow any instructions or tasks listed there.
If nothing needs attention, reply with just: HEARTBEAT_OK"#;

/// Token that indicates "nothing to do".
const HEARTBEAT_OK_TOKEN: &str = "HEARTBEAT_OK";

// ─────────────────────────────────────────────
// Callback type
// ─────────────────────────────────────────────

/// Callback invoked on each heartbeat tick.
///
/// Receives the heartbeat prompt and returns the agent's response.
pub type OnHeartbeatFn = Arc<
    dyn Fn(String) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send>>
        + Send
        + Sync,
>;

// ─────────────────────────────────────────────
// HeartbeatService
// ─────────────────────────────────────────────

/// Periodic heartbeat that wakes the agent to check `HEARTBEAT.md`.
pub struct HeartbeatService {
    /// Workspace root (where `HEARTBEAT.md` lives).
    workspace: PathBuf,
    /// Callback to invoke (typically `agent.process_direct()`).
    on_heartbeat: Option<OnHeartbeatFn>,
    /// Interval in seconds between heartbeats.
    interval_s: u64,
    /// Whether the service is enabled.
    enabled: bool,
    /// Shutdown signal.
    shutdown: Arc<Notify>,
}

impl HeartbeatService {
    /// Create a new heartbeat service.
    pub fn new(
        workspace: PathBuf,
        on_heartbeat: Option<OnHeartbeatFn>,
        interval_s: Option<u64>,
        enabled: bool,
    ) -> Self {
        Self {
            workspace,
            on_heartbeat,
            interval_s: interval_s.unwrap_or(DEFAULT_HEARTBEAT_INTERVAL_S),
            enabled,
            shutdown: Arc::new(Notify::new()),
        }
    }

    /// Set the heartbeat callback.
    pub fn set_on_heartbeat(&mut self, callback: OnHeartbeatFn) {
        self.on_heartbeat = Some(callback);
    }

    /// Path to `HEARTBEAT.md`.
    fn heartbeat_file(&self) -> PathBuf {
        self.workspace.join("HEARTBEAT.md")
    }

    /// Read `HEARTBEAT.md` content, returning `None` if it doesn't exist.
    fn read_heartbeat_file(&self) -> Option<String> {
        let path = self.heartbeat_file();
        if path.exists() {
            std::fs::read_to_string(&path).ok()
        } else {
            None
        }
    }

    /// Check if `HEARTBEAT.md` has no actionable content.
    ///
    /// Lines that are empty, headers (#), HTML comments, or checkboxes
    /// are not considered actionable.
    fn is_heartbeat_empty(content: Option<&str>) -> bool {
        let content = match content {
            Some(c) if !c.is_empty() => c,
            _ => return true,
        };

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty()
                || trimmed.starts_with('#')
                || trimmed.starts_with("<!--")
                || trimmed == "- [ ]"
                || trimmed == "* [ ]"
                || trimmed == "- [x]"
                || trimmed == "* [x]"
            {
                continue;
            }
            // Found actionable content
            return false;
        }

        true
    }

    /// Start the heartbeat service (blocking async loop).
    ///
    /// Returns when `stop()` is called.
    pub async fn start(&self) -> anyhow::Result<()> {
        if !self.enabled {
            info!("heartbeat disabled");
            // Park until shutdown
            self.shutdown.notified().await;
            return Ok(());
        }

        info!(interval_s = self.interval_s, "heartbeat service started");

        loop {
            let sleep_duration = std::time::Duration::from_secs(self.interval_s);

            tokio::select! {
                _ = tokio::time::sleep(sleep_duration) => {
                    self.tick().await;
                }
                _ = self.shutdown.notified() => {
                    info!("heartbeat service shutting down");
                    return Ok(());
                }
            }
        }
    }

    /// Stop the heartbeat service.
    pub fn stop(&self) {
        info!("stopping heartbeat service");
        self.shutdown.notify_waiters();
    }

    /// Execute a single heartbeat tick.
    async fn tick(&self) {
        let content = self.read_heartbeat_file();

        // Skip if HEARTBEAT.md is empty or doesn't exist
        if Self::is_heartbeat_empty(content.as_deref()) {
            debug!("heartbeat: no tasks (HEARTBEAT.md empty)");
            return;
        }

        info!("heartbeat: checking for tasks...");

        if let Some(ref callback) = self.on_heartbeat {
            match callback(HEARTBEAT_PROMPT.to_string()).await {
                Ok(response) => {
                    // Normalize: remove underscores and compare case-insensitively
                    let normalized = response.to_uppercase().replace('_', "");
                    let token = HEARTBEAT_OK_TOKEN.replace('_', "");
                    if normalized.contains(&token) {
                        info!("heartbeat: OK (no action needed)");
                    } else {
                        info!("heartbeat: completed task");
                    }
                }
                Err(e) => {
                    error!(error = %e, "heartbeat execution failed");
                }
            }
        }
    }

    /// Manually trigger a heartbeat (for CLI or testing).
    pub async fn trigger_now(&self) -> Option<anyhow::Result<String>> {
        if let Some(ref callback) = self.on_heartbeat {
            Some(callback(HEARTBEAT_PROMPT.to_string()).await)
        } else {
            None
        }
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_heartbeat_empty_none() {
        assert!(HeartbeatService::is_heartbeat_empty(None));
    }

    #[test]
    fn test_is_heartbeat_empty_blank() {
        assert!(HeartbeatService::is_heartbeat_empty(Some("")));
        assert!(HeartbeatService::is_heartbeat_empty(Some("  \n  \n")));
    }

    #[test]
    fn test_is_heartbeat_empty_headers_only() {
        let content = "# Heartbeat Tasks\n\n## Active\n\n<!-- comment -->\n";
        assert!(HeartbeatService::is_heartbeat_empty(Some(content)));
    }

    #[test]
    fn test_is_heartbeat_not_empty() {
        let content = "# Tasks\n\nCheck the deployments\n";
        assert!(!HeartbeatService::is_heartbeat_empty(Some(content)));
    }

    #[test]
    fn test_is_heartbeat_empty_checkboxes() {
        let content = "# Tasks\n- [ ]\n* [x]\n";
        assert!(HeartbeatService::is_heartbeat_empty(Some(content)));
    }

    #[test]
    fn test_is_heartbeat_not_empty_with_task() {
        let content = "# Tasks\n- [ ] Deploy v2.0\n";
        assert!(!HeartbeatService::is_heartbeat_empty(Some(content)));
    }

    #[tokio::test]
    async fn test_trigger_now_no_callback() {
        let service = HeartbeatService::new(
            PathBuf::from("/tmp/test-heartbeat"),
            None,
            Some(60),
            true,
        );
        let result = service.trigger_now().await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_trigger_now_with_callback() {
        let callback: OnHeartbeatFn = Arc::new(|_prompt| {
            Box::pin(async { Ok("HEARTBEAT_OK".to_string()) })
        });
        let service = HeartbeatService::new(
            PathBuf::from("/tmp/test-heartbeat"),
            Some(callback),
            Some(60),
            true,
        );
        let result = service.trigger_now().await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().unwrap(), "HEARTBEAT_OK");
    }

    #[tokio::test]
    async fn test_stop_exits_loop() {
        let service = Arc::new(HeartbeatService::new(
            PathBuf::from("/tmp/test-heartbeat"),
            None,
            Some(1), // 1 second interval for test speed
            true,
        ));

        let svc = service.clone();
        let handle = tokio::spawn(async move {
            svc.start().await
        });

        // Stop after a brief delay
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        service.stop();

        let result = handle.await.unwrap();
        assert!(result.is_ok());
    }
}
