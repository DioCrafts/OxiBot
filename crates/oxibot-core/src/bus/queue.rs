//! Async message bus — the central nervous system of Oxibot.
//!
//! Replaces nanobot's `bus/queue.py` (asyncio.Queue-based MessageBus).
//! Uses tokio::sync::mpsc bounded channels.

use super::types::{InboundMessage, OutboundMessage};
use tokio::sync::mpsc;

/// The message bus connecting channels ↔ agent loop.
///
/// - Channels publish to `inbound` (user messages arriving)
/// - Agent loop consumes from `inbound`, processes, publishes to `outbound`
/// - Channel manager consumes from `outbound` and routes to correct channel
pub struct MessageBus {
    inbound_tx: mpsc::Sender<InboundMessage>,
    inbound_rx: tokio::sync::Mutex<mpsc::Receiver<InboundMessage>>,
    outbound_tx: mpsc::Sender<OutboundMessage>,
    outbound_rx: tokio::sync::Mutex<mpsc::Receiver<OutboundMessage>>,
}

impl MessageBus {
    /// Create a new message bus with the given buffer capacity.
    pub fn new(buffer_size: usize) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::channel(buffer_size);
        let (outbound_tx, outbound_rx) = mpsc::channel(buffer_size);

        MessageBus {
            inbound_tx,
            inbound_rx: tokio::sync::Mutex::new(inbound_rx),
            outbound_tx,
            outbound_rx: tokio::sync::Mutex::new(outbound_rx),
        }
    }

    /// Publish a message from a channel to the agent (inbound).
    pub async fn publish_inbound(&self, msg: InboundMessage) -> Result<(), mpsc::error::SendError<InboundMessage>> {
        self.inbound_tx.send(msg).await
    }

    /// Consume the next inbound message (blocks until available).
    /// Returns None if all senders are dropped.
    pub async fn consume_inbound(&self) -> Option<InboundMessage> {
        let mut rx = self.inbound_rx.lock().await;
        rx.recv().await
    }

    /// Publish a response from the agent to a channel (outbound).
    pub async fn publish_outbound(&self, msg: OutboundMessage) -> Result<(), mpsc::error::SendError<OutboundMessage>> {
        self.outbound_tx.send(msg).await
    }

    /// Consume the next outbound message (blocks until available).
    /// Returns None if all senders are dropped.
    pub async fn consume_outbound(&self) -> Option<OutboundMessage> {
        let mut rx = self.outbound_rx.lock().await;
        rx.recv().await
    }

    /// Get a clone of the inbound sender (for channels to use).
    pub fn inbound_sender(&self) -> mpsc::Sender<InboundMessage> {
        self.inbound_tx.clone()
    }

    /// Get a clone of the outbound sender (for the agent loop to use).
    pub fn outbound_sender(&self) -> mpsc::Sender<OutboundMessage> {
        self.outbound_tx.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_inbound_message_flow() {
        let bus = MessageBus::new(10);

        let msg = InboundMessage::new("telegram", "user_1", "chat_1", "Hello!");
        bus.publish_inbound(msg).await.unwrap();

        let received = bus.consume_inbound().await.unwrap();
        assert_eq!(received.channel, "telegram");
        assert_eq!(received.content, "Hello!");
    }

    #[tokio::test]
    async fn test_outbound_message_flow() {
        let bus = MessageBus::new(10);

        let msg = OutboundMessage::new("discord", "channel_42", "Response here");
        bus.publish_outbound(msg).await.unwrap();

        let received = bus.consume_outbound().await.unwrap();
        assert_eq!(received.channel, "discord");
        assert_eq!(received.content, "Response here");
    }

    #[tokio::test]
    async fn test_message_ordering() {
        let bus = MessageBus::new(10);

        // Publish 3 messages
        for i in 1..=3 {
            let msg = InboundMessage::new("cli", "local", "default", format!("msg-{}", i));
            bus.publish_inbound(msg).await.unwrap();
        }

        // Consume in order
        let m1 = bus.consume_inbound().await.unwrap();
        let m2 = bus.consume_inbound().await.unwrap();
        let m3 = bus.consume_inbound().await.unwrap();

        assert_eq!(m1.content, "msg-1");
        assert_eq!(m2.content, "msg-2");
        assert_eq!(m3.content, "msg-3");
    }

    #[tokio::test]
    async fn test_sender_clone_works() {
        let bus = MessageBus::new(10);
        let sender = bus.inbound_sender();

        // Send via cloned sender
        let msg = InboundMessage::new("slack", "user_x", "channel_y", "From clone");
        sender.send(msg).await.unwrap();

        // Receive via bus
        let received = bus.consume_inbound().await.unwrap();
        assert_eq!(received.channel, "slack");
        assert_eq!(received.content, "From clone");
    }

    #[tokio::test]
    async fn test_multiple_producers() {
        let bus = std::sync::Arc::new(MessageBus::new(10));

        // Simulate 2 channels publishing concurrently
        let bus1 = bus.clone();
        let bus2 = bus.clone();

        let h1 = tokio::spawn(async move {
            let msg = InboundMessage::new("telegram", "u1", "c1", "from telegram");
            bus1.publish_inbound(msg).await.unwrap();
        });

        let h2 = tokio::spawn(async move {
            let msg = InboundMessage::new("discord", "u2", "c2", "from discord");
            bus2.publish_inbound(msg).await.unwrap();
        });

        h1.await.unwrap();
        h2.await.unwrap();

        // Both messages should be in the queue
        let r1 = bus.consume_inbound().await.unwrap();
        let r2 = bus.consume_inbound().await.unwrap();

        let channels: Vec<&str> = vec![r1.channel.as_str(), r2.channel.as_str()];
        assert!(channels.contains(&"telegram"));
        assert!(channels.contains(&"discord"));
    }

    #[tokio::test]
    async fn test_full_round_trip() {
        // Simulate: channel → bus → agent → bus → channel
        let bus = std::sync::Arc::new(MessageBus::new(10));

        // 1. Channel publishes inbound
        let inbound = InboundMessage::new("telegram", "user_42", "chat_99", "What is 2+2?");
        bus.publish_inbound(inbound).await.unwrap();

        // 2. Agent consumes inbound
        let received = bus.consume_inbound().await.unwrap();
        assert_eq!(received.content, "What is 2+2?");

        // 3. Agent processes and publishes outbound
        let response = OutboundMessage::new(
            received.channel.clone(),
            received.chat_id.clone(),
            "The answer is 4.",
        );
        bus.publish_outbound(response).await.unwrap();

        // 4. Channel manager consumes outbound
        let outbound = bus.consume_outbound().await.unwrap();
        assert_eq!(outbound.channel, "telegram");
        assert_eq!(outbound.chat_id, "chat_99");
        assert_eq!(outbound.content, "The answer is 4.");
    }
}
