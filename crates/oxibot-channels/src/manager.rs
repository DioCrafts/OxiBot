//! Channel Manager — orchestrates channel lifecycle and message routing.
//!
//! Port of nanobot's `channels/manager.py`.
//!
//! Responsibilities:
//! - Register enabled channels
//! - Start/stop all channels concurrently via `tokio::spawn`
//! - Dispatch outbound messages from the bus to the correct channel
//! - Report channel status

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Notify;
use tracing::{debug, error, info, warn};

use oxibot_core::bus::queue::MessageBus;

use crate::base::Channel;

// ─────────────────────────────────────────────
// ChannelManager
// ─────────────────────────────────────────────

/// Manages the lifecycle and message routing for all chat channels.
///
/// Channels are registered with `register()`, started concurrently with
/// `start_all()`, and stopped with `stop_all()`. An outbound dispatcher
/// task reads from the message bus and routes responses to the correct
/// channel.
pub struct ChannelManager {
    /// Registered channels, keyed by name.
    channels: HashMap<String, Arc<dyn Channel>>,
    /// Message bus for outbound message consumption.
    bus: Arc<MessageBus>,
    /// Shutdown signal.
    shutdown: Arc<Notify>,
}

impl ChannelManager {
    /// Create a new channel manager.
    pub fn new(bus: Arc<MessageBus>) -> Self {
        Self {
            channels: HashMap::new(),
            bus,
            shutdown: Arc::new(Notify::new()),
        }
    }

    /// Register a channel. Overwrites any previous channel with the same name.
    pub fn register(&mut self, channel: Arc<dyn Channel>) {
        let name = channel.name().to_string();
        info!(channel = %name, "registered channel");
        self.channels.insert(name, channel);
    }

    /// Unregister a channel by name.
    pub fn unregister(&mut self, name: &str) -> Option<Arc<dyn Channel>> {
        let removed = self.channels.remove(name);
        if removed.is_some() {
            info!(channel = %name, "unregistered channel");
        }
        removed
    }

    /// Get a registered channel by name.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Channel>> {
        self.channels.get(name)
    }

    /// Get the names of all registered channels, sorted.
    pub fn channel_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.channels.keys().cloned().collect();
        names.sort();
        names
    }

    /// Number of registered channels.
    pub fn len(&self) -> usize {
        self.channels.len()
    }

    /// Whether there are no registered channels.
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Start all channels + the outbound dispatcher.
    ///
    /// Each channel's `start()` is spawned as a `tokio::spawn` task.
    /// The outbound dispatcher runs as an additional task that routes
    /// messages from the bus to the correct channel.
    ///
    /// This method blocks until shutdown is signaled.
    pub async fn start_all(&self) -> Result<()> {
        if self.channels.is_empty() {
            warn!("no channels registered, nothing to start");
            return Ok(());
        }

        info!(
            channels = ?self.channel_names(),
            "starting {} channel(s)",
            self.channels.len()
        );

        let mut handles = Vec::new();

        // Spawn each channel's start() as a background task
        for (name, channel) in &self.channels {
            let ch = channel.clone();
            let ch_name = name.clone();

            let handle = tokio::spawn(async move {
                info!(channel = %ch_name, "channel starting");
                if let Err(e) = ch.start().await {
                    error!(channel = %ch_name, error = %e, "channel start failed");
                }
                info!(channel = %ch_name, "channel stopped");
            });

            handles.push(handle);
        }

        // Spawn the outbound dispatcher
        let bus = self.bus.clone();
        let channels = self.channels.clone();
        let shutdown = self.shutdown.clone();

        let dispatcher_handle = tokio::spawn(async move {
            Self::dispatch_outbound(bus, channels, shutdown).await;
        });

        handles.push(dispatcher_handle);

        // Wait for shutdown signal
        self.shutdown.notified().await;

        info!("channel manager shutting down");
        Ok(())
    }

    /// Stop all channels and the outbound dispatcher.
    pub async fn stop_all(&self) {
        info!("stopping all channels");

        // Signal shutdown to the dispatcher
        self.shutdown.notify_waiters();

        // Stop each channel
        for (name, channel) in &self.channels {
            debug!(channel = %name, "stopping channel");
            if let Err(e) = channel.stop().await {
                error!(channel = %name, error = %e, "channel stop failed");
            }
        }

        info!("all channels stopped");
    }

    /// Signal the manager to shut down.
    pub fn signal_shutdown(&self) {
        self.shutdown.notify_waiters();
    }

    /// Outbound message dispatcher — routes agent responses to the correct channel.
    ///
    /// Runs as a background task, polling the bus outbound queue.
    async fn dispatch_outbound(
        bus: Arc<MessageBus>,
        channels: HashMap<String, Arc<dyn Channel>>,
        shutdown: Arc<Notify>,
    ) {
        info!("outbound dispatcher started");

        loop {
            tokio::select! {
                msg = bus.consume_outbound() => {
                    match msg {
                        Some(outbound) => {
                            debug!(
                                channel = %outbound.channel,
                                chat_id = %outbound.chat_id,
                                content_len = outbound.content.len(),
                                "dispatching outbound message"
                            );

                            if let Some(channel) = channels.get(&outbound.channel) {
                                if let Err(e) = channel.send(&outbound).await {
                                    error!(
                                        channel = %outbound.channel,
                                        error = %e,
                                        "failed to send outbound message"
                                    );
                                }
                            } else {
                                warn!(
                                    channel = %outbound.channel,
                                    "no channel registered for outbound message"
                                );
                            }
                        }
                        None => {
                            info!("outbound bus closed, dispatcher exiting");
                            break;
                        }
                    }
                }
                _ = shutdown.notified() => {
                    info!("dispatcher received shutdown signal");
                    break;
                }
            }
        }
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::Channel;
    use oxibot_core::bus::types::OutboundMessage;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// Mock channel for testing.
    struct MockChannel {
        channel_name: String,
        started: Arc<AtomicBool>,
        stopped: Arc<AtomicBool>,
        send_count: Arc<AtomicUsize>,
    }

    impl MockChannel {
        fn new(name: &str) -> Self {
            Self {
                channel_name: name.into(),
                started: Arc::new(AtomicBool::new(false)),
                stopped: Arc::new(AtomicBool::new(false)),
                send_count: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait::async_trait]
    impl Channel for MockChannel {
        fn name(&self) -> &str {
            &self.channel_name
        }

        async fn start(&self) -> anyhow::Result<()> {
            self.started.store(true, Ordering::SeqCst);
            // Simulate a long-running listener
            tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            Ok(())
        }

        async fn stop(&self) -> anyhow::Result<()> {
            self.stopped.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn send(&self, _msg: &OutboundMessage) -> anyhow::Result<()> {
            self.send_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn test_new_manager_empty() {
        let bus = Arc::new(MessageBus::new(32));
        let mgr = ChannelManager::new(bus);
        assert!(mgr.is_empty());
        assert_eq!(mgr.len(), 0);
    }

    #[test]
    fn test_register_channel() {
        let bus = Arc::new(MessageBus::new(32));
        let mut mgr = ChannelManager::new(bus);

        let ch = Arc::new(MockChannel::new("telegram"));
        mgr.register(ch);

        assert_eq!(mgr.len(), 1);
        assert!(!mgr.is_empty());
        assert!(mgr.get("telegram").is_some());
        assert!(mgr.get("discord").is_none());
    }

    #[test]
    fn test_register_multiple_channels() {
        let bus = Arc::new(MessageBus::new(32));
        let mut mgr = ChannelManager::new(bus);

        mgr.register(Arc::new(MockChannel::new("telegram")));
        mgr.register(Arc::new(MockChannel::new("discord")));
        mgr.register(Arc::new(MockChannel::new("slack")));

        assert_eq!(mgr.len(), 3);
        assert_eq!(mgr.channel_names(), vec!["discord", "slack", "telegram"]);
    }

    #[test]
    fn test_unregister_channel() {
        let bus = Arc::new(MessageBus::new(32));
        let mut mgr = ChannelManager::new(bus);

        mgr.register(Arc::new(MockChannel::new("telegram")));
        assert_eq!(mgr.len(), 1);

        let removed = mgr.unregister("telegram");
        assert!(removed.is_some());
        assert_eq!(mgr.len(), 0);
    }

    #[test]
    fn test_unregister_nonexistent() {
        let bus = Arc::new(MessageBus::new(32));
        let mut mgr = ChannelManager::new(bus);

        let removed = mgr.unregister("nonexistent");
        assert!(removed.is_none());
    }

    #[test]
    fn test_register_overwrites() {
        let bus = Arc::new(MessageBus::new(32));
        let mut mgr = ChannelManager::new(bus);

        mgr.register(Arc::new(MockChannel::new("telegram")));
        mgr.register(Arc::new(MockChannel::new("telegram")));

        assert_eq!(mgr.len(), 1); // overwritten, not duplicated
    }

    #[test]
    fn test_channel_names_sorted() {
        let bus = Arc::new(MessageBus::new(32));
        let mut mgr = ChannelManager::new(bus);

        mgr.register(Arc::new(MockChannel::new("slack")));
        mgr.register(Arc::new(MockChannel::new("discord")));
        mgr.register(Arc::new(MockChannel::new("telegram")));

        let names = mgr.channel_names();
        assert_eq!(names, vec!["discord", "slack", "telegram"]);
    }

    #[tokio::test]
    async fn test_start_all_empty() {
        let bus = Arc::new(MessageBus::new(32));
        let mgr = ChannelManager::new(bus);

        // Should return immediately with no channels
        let result = mgr.start_all().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_stop_all() {
        let bus = Arc::new(MessageBus::new(32));
        let mut mgr = ChannelManager::new(bus);

        let ch = Arc::new(MockChannel::new("test"));
        let stopped = ch.stopped.clone();
        mgr.register(ch);

        mgr.stop_all().await;
        assert!(stopped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_dispatch_outbound_routes_correctly() {
        let bus = Arc::new(MessageBus::new(32));

        let ch1 = Arc::new(MockChannel::new("telegram"));
        let ch2 = Arc::new(MockChannel::new("discord"));
        let ch1_count = ch1.send_count.clone();
        let ch2_count = ch2.send_count.clone();

        let mut channels: HashMap<String, Arc<dyn Channel>> = HashMap::new();
        channels.insert("telegram".into(), ch1);
        channels.insert("discord".into(), ch2);

        let shutdown = Arc::new(Notify::new());

        // Spawn the dispatcher
        let bus_clone = bus.clone();
        let shutdown_clone = shutdown.clone();
        let handle = tokio::spawn(async move {
            ChannelManager::dispatch_outbound(bus_clone, channels, shutdown_clone).await;
        });

        // Send messages
        bus.publish_outbound(OutboundMessage::new("telegram", "chat_1", "Hello TG"))
            .await
            .unwrap();
        bus.publish_outbound(OutboundMessage::new("discord", "guild_1", "Hello DC"))
            .await
            .unwrap();
        bus.publish_outbound(OutboundMessage::new("telegram", "chat_2", "Again TG"))
            .await
            .unwrap();

        // Give dispatcher time to process
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Signal shutdown
        shutdown.notify_waiters();
        let _ = handle.await;

        assert_eq!(ch1_count.load(Ordering::SeqCst), 2); // telegram got 2
        assert_eq!(ch2_count.load(Ordering::SeqCst), 1); // discord got 1
    }

    #[tokio::test]
    async fn test_dispatch_outbound_unknown_channel() {
        let bus = Arc::new(MessageBus::new(32));
        let channels: HashMap<String, Arc<dyn Channel>> = HashMap::new();
        let shutdown = Arc::new(Notify::new());

        let bus_clone = bus.clone();
        let shutdown_clone = shutdown.clone();
        let handle = tokio::spawn(async move {
            ChannelManager::dispatch_outbound(bus_clone, channels, shutdown_clone).await;
        });

        // Send to a channel that doesn't exist
        bus.publish_outbound(OutboundMessage::new("unknown", "chat", "msg"))
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        shutdown.notify_waiters();

        // Should complete without panic
        let _ = handle.await;
    }

    #[tokio::test]
    async fn test_signal_shutdown() {
        let bus = Arc::new(MessageBus::new(32));
        let mgr = ChannelManager::new(bus);

        // Register a channel that sleeps in start()
        // Signal shutdown should wake up start_all
        let _mgr_shutdown = Arc::new(Notify::new());

        // Just verify signal_shutdown doesn't panic
        mgr.signal_shutdown();
    }
}
