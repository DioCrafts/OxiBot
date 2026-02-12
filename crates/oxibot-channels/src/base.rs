//! Channel trait — the abstract interface every chat channel must implement.
//!
//! Port of nanobot's `channels/base.py` `BaseChannel` ABC.
//!
//! Each channel (Telegram, Discord, etc.) implements this trait to:
//! - `start()` — begin listening for incoming messages (long-running)
//! - `stop()` — graceful shutdown
//! - `send()` — deliver an outbound message to the channel
//! - `name()` — channel identifier matching config keys

use async_trait::async_trait;
use oxibot_core::bus::types::OutboundMessage;

/// Every chat channel implements this trait.
///
/// The `ChannelManager` holds `Box<dyn Channel>` and orchestrates
/// start/stop/send across all enabled channels.
#[async_trait]
pub trait Channel: Send + Sync {
    /// Unique channel name (e.g. "telegram", "discord", "slack").
    ///
    /// Must match the key used in config and in `OutboundMessage.channel`.
    fn name(&self) -> &str;

    /// Start listening for incoming messages.
    ///
    /// This should be a long-running task that publishes `InboundMessage`s
    /// to the message bus. It runs until `stop()` is called or the
    /// shutdown signal is received.
    async fn start(&self) -> anyhow::Result<()>;

    /// Graceful shutdown — stop listening and clean up resources.
    async fn stop(&self) -> anyhow::Result<()>;

    /// Send an outbound message to this channel.
    ///
    /// Called by the `ChannelManager`'s outbound dispatcher when
    /// it receives a message targeted at this channel.
    async fn send(&self, msg: &OutboundMessage) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// A mock channel for testing.
    struct MockChannel {
        started: Arc<AtomicBool>,
        stopped: Arc<AtomicBool>,
        sent: Arc<tokio::sync::Mutex<Vec<String>>>,
    }

    impl MockChannel {
        fn new() -> Self {
            Self {
                started: Arc::new(AtomicBool::new(false)),
                stopped: Arc::new(AtomicBool::new(false)),
                sent: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl Channel for MockChannel {
        fn name(&self) -> &str {
            "mock"
        }

        async fn start(&self) -> anyhow::Result<()> {
            self.started.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn stop(&self) -> anyhow::Result<()> {
            self.stopped.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn send(&self, msg: &OutboundMessage) -> anyhow::Result<()> {
            let mut sent = self.sent.lock().await;
            sent.push(msg.content.clone());
            Ok(())
        }
    }

    #[test]
    fn test_mock_channel_name() {
        let ch = MockChannel::new();
        assert_eq!(ch.name(), "mock");
    }

    #[tokio::test]
    async fn test_mock_channel_start() {
        let ch = MockChannel::new();
        ch.start().await.unwrap();
        assert!(ch.started.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_mock_channel_stop() {
        let ch = MockChannel::new();
        ch.stop().await.unwrap();
        assert!(ch.stopped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_mock_channel_send() {
        let ch = MockChannel::new();
        let msg = OutboundMessage::new("mock", "chat_1", "Hello!");
        ch.send(&msg).await.unwrap();

        let sent = ch.sent.lock().await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], "Hello!");
    }
}
