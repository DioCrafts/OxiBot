//! WhatsApp channel — WebSocket client connecting to a Baileys-based Node.js bridge.
//!
//! Port of nanobot's `channels/whatsapp.py`.
//!
//! Architecture:
//! - A Node.js bridge process (`@whiskeysockets/baileys`) speaks WhatsApp Web protocol
//! - This channel connects as a WebSocket **client** to the bridge (default `ws://localhost:3001`)
//! - Inbound: bridge pushes `{"type":"message", ...}` JSON over WS
//! - Outbound: we send `{"type":"send", "to":"...", "text":"..."}` JSON over WS
//!
//! Features:
//! - Auto-reconnect with backoff
//! - Allow-list by phone number
//! - Group message support (pass-through via metadata)
//! - Voice/image/video/document placeholders from bridge

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::{Mutex, Notify};
use tracing::{debug, error, info, warn};

use oxibot_core::bus::queue::MessageBus;
use oxibot_core::bus::types::{InboundMessage, OutboundMessage};

use crate::base::Channel;

// ─────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────

/// Default bridge WebSocket URL.
const DEFAULT_BRIDGE_URL: &str = "ws://localhost:3001";

/// Reconnect backoff (seconds).
const RECONNECT_DELAY_SECS: u64 = 5;

// ─────────────────────────────────────────────
// WhatsAppChannel
// ─────────────────────────────────────────────

/// WhatsApp channel — connects to a Baileys bridge via WebSocket.
pub struct WhatsAppChannel {
    /// Bridge WebSocket URL.
    bridge_url: String,
    /// Message bus for inbound/outbound.
    bus: Arc<MessageBus>,
    /// Allow-list of phone numbers (the part before `@`). Empty = allow everyone.
    allowed_users: Vec<String>,
    /// Shutdown signal.
    shutdown: Arc<Notify>,
    /// Active WebSocket write half (for sending outbound messages).
    ws_write: Arc<Mutex<Option<WsSender>>>,
    /// Whether bridge reports connected to WhatsApp.
    connected: Arc<Mutex<bool>>,
}

/// Type alias for the WebSocket sink.
type WsSender = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::Message,
>;

impl WhatsAppChannel {
    /// Create a new WhatsApp channel.
    pub fn new(
        bridge_url: String,
        bus: Arc<MessageBus>,
        allowed_users: Vec<String>,
    ) -> Self {
        let url = if bridge_url.is_empty() {
            DEFAULT_BRIDGE_URL.to_string()
        } else {
            bridge_url
        };

        Self {
            bridge_url: url,
            bus,
            allowed_users,
            shutdown: Arc::new(Notify::new()),
            ws_write: Arc::new(Mutex::new(None)),
            connected: Arc::new(Mutex::new(false)),
        }
    }

    /// Check if a sender is allowed.
    fn is_allowed(&self, sender_id: &str) -> bool {
        if self.allowed_users.is_empty() {
            return true;
        }
        if self.allowed_users.iter().any(|u| u == sender_id) {
            return true;
        }
        for part in sender_id.split('|') {
            if !part.is_empty() && self.allowed_users.iter().any(|u| u == part) {
                return true;
            }
        }
        false
    }

    /// Run the WebSocket connection with auto-reconnect.
    async fn run_bridge_loop(&self) -> anyhow::Result<()> {
        loop {
            match self.bridge_session().await {
                Ok(()) => {
                    info!("whatsapp bridge session ended normally");
                    break;
                }
                Err(e) => {
                    *self.connected.lock().await = false;
                    *self.ws_write.lock().await = None;
                    warn!(error = %e, "whatsapp bridge error, reconnecting in {RECONNECT_DELAY_SECS}s");
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(RECONNECT_DELAY_SECS)) => {}
                        _ = self.shutdown.notified() => {
                            info!("whatsapp shutdown during reconnect wait");
                            return Ok(());
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Single WebSocket session to the bridge.
    async fn bridge_session(&self) -> anyhow::Result<()> {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message as WsMessage;

        debug!(url = %self.bridge_url, "connecting to whatsapp bridge");
        let (ws_stream, _) = tokio_tungstenite::connect_async(&self.bridge_url).await?;
        info!("connected to whatsapp bridge");

        let (write, mut read) = ws_stream.split();
        *self.ws_write.lock().await = Some(write);

        loop {
            tokio::select! {
                msg = read.next() => {
                    let msg = match msg {
                        Some(Ok(m)) => m,
                        Some(Err(e)) => {
                            error!(error = %e, "whatsapp ws read error");
                            return Err(e.into());
                        }
                        None => {
                            debug!("whatsapp ws stream ended");
                            return Ok(());
                        }
                    };

                    let text = match msg {
                        WsMessage::Text(t) => t.to_string(),
                        WsMessage::Close(_) => {
                            info!("whatsapp bridge closed connection");
                            return Ok(());
                        }
                        _ => continue,
                    };

                    if let Err(e) = self.handle_bridge_message(&text).await {
                        warn!(error = %e, "failed to handle bridge message");
                    }
                }
                _ = self.shutdown.notified() => {
                    info!("whatsapp shutdown signal received");
                    // Close WS gracefully
                    if let Some(mut write) = self.ws_write.lock().await.take() {
                        let _ = write.send(WsMessage::Close(None)).await;
                    }
                    return Ok(());
                }
            }
        }
    }

    /// Parse and handle a JSON message from the bridge.
    async fn handle_bridge_message(&self, raw: &str) -> anyhow::Result<()> {
        let payload: Value = serde_json::from_str(raw)?;
        let msg_type = payload["type"].as_str().unwrap_or("");

        match msg_type {
            "message" => {
                self.handle_incoming_message(&payload).await;
            }
            "status" => {
                let status = payload["status"].as_str().unwrap_or("unknown");
                let was_connected = *self.connected.lock().await;
                let now_connected = status == "connected";
                *self.connected.lock().await = now_connected;
                if now_connected && !was_connected {
                    info!("whatsapp bridge: connected to WhatsApp");
                } else if !now_connected && was_connected {
                    warn!(status = status, "whatsapp bridge: disconnected");
                } else {
                    debug!(status = status, "whatsapp bridge status update");
                }
            }
            "qr" => {
                info!("whatsapp: scan QR code in the bridge terminal to authenticate");
            }
            "sent" => {
                let to = payload["to"].as_str().unwrap_or("?");
                debug!(to = to, "whatsapp message sent confirmation");
            }
            "error" => {
                let err = payload["error"].as_str().unwrap_or("unknown");
                error!(error = err, "whatsapp bridge error");
            }
            _ => {
                debug!(msg_type = msg_type, "whatsapp bridge: unknown message type");
            }
        }

        Ok(())
    }

    /// Handle an incoming `"message"` event from the bridge.
    async fn handle_incoming_message(&self, payload: &Value) {
        // Extract sender: prefer `pn` (phone-based JID) over `sender` (LID-based JID)
        let raw_sender = payload["pn"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| payload["sender"].as_str())
            .unwrap_or("");

        if raw_sender.is_empty() {
            debug!("whatsapp message with no sender, ignoring");
            return;
        }

        // Extract sender_id: part before '@' (phone number)
        let sender_id = raw_sender
            .split('@')
            .next()
            .unwrap_or(raw_sender)
            .to_string();

        // chat_id: use `sender` field (full LID) for replies
        let chat_id = payload["sender"]
            .as_str()
            .unwrap_or(raw_sender)
            .to_string();

        // Check allow-list
        if !self.is_allowed(&sender_id) {
            warn!(
                sender = %sender_id,
                "whatsapp message from unauthorized user, ignoring"
            );
            return;
        }

        // Content
        let content = payload["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        if content.is_empty() {
            debug!("whatsapp empty message, ignoring");
            return;
        }

        let is_group = payload["isGroup"].as_bool().unwrap_or(false);

        debug!(
            sender = %sender_id,
            chat_id = %chat_id,
            content_len = content.len(),
            is_group = is_group,
            "whatsapp inbound message"
        );

        // Build inbound message
        let mut inbound = InboundMessage::new("whatsapp", &sender_id, &chat_id, &content);
        if let Some(msg_id) = payload["id"].as_str() {
            inbound.metadata.insert("message_id".into(), msg_id.to_string());
        }
        if let Some(ts) = payload["timestamp"].as_i64() {
            inbound.metadata.insert("timestamp".into(), ts.to_string());
        }
        inbound.metadata.insert("is_group".into(), is_group.to_string());

        if let Err(e) = self.bus.publish_inbound(inbound).await {
            error!(error = %e, "failed to publish whatsapp message to bus");
        }
    }
}

#[async_trait]
impl Channel for WhatsAppChannel {
    fn name(&self) -> &str {
        "whatsapp"
    }

    async fn start(&self) -> anyhow::Result<()> {
        info!(url = %self.bridge_url, "starting whatsapp channel");
        self.run_bridge_loop().await
    }

    async fn stop(&self) -> anyhow::Result<()> {
        info!("stopping whatsapp channel");
        self.shutdown.notify_waiters();
        *self.connected.lock().await = false;
        *self.ws_write.lock().await = None;
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> anyhow::Result<()> {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message as WsMessage;

        let mut guard = self.ws_write.lock().await;
        let write = match guard.as_mut() {
            Some(w) => w,
            None => {
                warn!("whatsapp bridge not connected, dropping outbound message");
                return Ok(());
            }
        };

        let frame = json!({
            "type": "send",
            "to": msg.chat_id,
            "text": msg.content
        })
        .to_string();

        write.send(WsMessage::text(frame)).await?;
        debug!(chat_id = %msg.chat_id, "whatsapp message sent");
        Ok(())
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_channel() -> WhatsAppChannel {
        let bus = Arc::new(MessageBus::new(32));
        WhatsAppChannel::new(String::new(), bus, vec![])
    }

    fn create_restricted_channel() -> WhatsAppChannel {
        let bus = Arc::new(MessageBus::new(32));
        WhatsAppChannel::new(
            String::new(),
            bus,
            vec!["34612345678".into(), "1555123456".into()],
        )
    }

    #[test]
    fn test_channel_name() {
        let ch = create_test_channel();
        assert_eq!(ch.name(), "whatsapp");
    }

    #[test]
    fn test_default_bridge_url() {
        let ch = create_test_channel();
        assert_eq!(ch.bridge_url, "ws://localhost:3001");
    }

    #[test]
    fn test_custom_bridge_url() {
        let bus = Arc::new(MessageBus::new(32));
        let ch = WhatsAppChannel::new("ws://mybridge:9000".into(), bus, vec![]);
        assert_eq!(ch.bridge_url, "ws://mybridge:9000");
    }

    #[test]
    fn test_is_allowed_empty_list() {
        let ch = create_test_channel();
        assert!(ch.is_allowed("anyone"));
        assert!(ch.is_allowed("34612345678"));
    }

    #[test]
    fn test_is_allowed_by_phone() {
        let ch = create_restricted_channel();
        assert!(ch.is_allowed("34612345678"));
        assert!(ch.is_allowed("1555123456"));
    }

    #[test]
    fn test_is_allowed_denied() {
        let ch = create_restricted_channel();
        assert!(!ch.is_allowed("0000000000"));
    }

    #[test]
    fn test_is_allowed_pipe_split() {
        let ch = create_restricted_channel();
        assert!(ch.is_allowed("34612345678|someuser"));
        assert!(!ch.is_allowed("000|stranger"));
    }

    #[tokio::test]
    async fn test_handle_bridge_message_status() {
        let ch = create_test_channel();
        let msg = r#"{"type":"status","status":"connected"}"#;
        ch.handle_bridge_message(msg).await.unwrap();
        assert!(*ch.connected.lock().await);
    }

    #[tokio::test]
    async fn test_handle_bridge_message_status_disconnected() {
        let ch = create_test_channel();
        // First connect
        ch.handle_bridge_message(r#"{"type":"status","status":"connected"}"#)
            .await
            .unwrap();
        assert!(*ch.connected.lock().await);
        // Then disconnect
        ch.handle_bridge_message(r#"{"type":"status","status":"disconnected"}"#)
            .await
            .unwrap();
        assert!(!*ch.connected.lock().await);
    }

    #[tokio::test]
    async fn test_handle_bridge_message_qr() {
        let ch = create_test_channel();
        // Should not panic
        ch.handle_bridge_message(r#"{"type":"qr","qr":"data"}"#)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_handle_bridge_message_error() {
        let ch = create_test_channel();
        ch.handle_bridge_message(r#"{"type":"error","error":"something went wrong"}"#)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_handle_bridge_message_invalid_json() {
        let ch = create_test_channel();
        let result = ch.handle_bridge_message("not json").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_handle_incoming_message_publishes() {
        let bus = Arc::new(MessageBus::new(32));
        let ch = WhatsAppChannel::new(String::new(), bus.clone(), vec![]);

        let payload = json!({
            "type": "message",
            "id": "msg1",
            "sender": "12345@lid",
            "pn": "12345@s.whatsapp.net",
            "content": "hello from whatsapp",
            "timestamp": 1700000000,
            "isGroup": false
        });

        ch.handle_incoming_message(&payload).await;

        let msg = bus.consume_inbound().await;
        assert!(msg.is_some());
        let msg = msg.unwrap();
        assert_eq!(msg.channel, "whatsapp");
        assert_eq!(msg.sender_id, "12345");
        assert_eq!(msg.chat_id, "12345@lid");
        assert_eq!(msg.content, "hello from whatsapp");
        assert_eq!(msg.metadata.get("message_id").unwrap(), "msg1");
        assert_eq!(msg.metadata.get("timestamp").unwrap(), "1700000000");
        assert_eq!(msg.metadata.get("is_group").unwrap(), "false");
    }

    #[tokio::test]
    async fn test_handle_incoming_message_prefers_pn() {
        let bus = Arc::new(MessageBus::new(32));
        let ch = WhatsAppChannel::new(String::new(), bus.clone(), vec![]);

        let payload = json!({
            "sender": "lid123@lid",
            "pn": "5551234@s.whatsapp.net",
            "content": "test"
        });

        ch.handle_incoming_message(&payload).await;

        let msg = bus.consume_inbound().await.unwrap();
        // sender_id should be phone part from pn
        assert_eq!(msg.sender_id, "5551234");
        // chat_id should be the sender (LID) for replies
        assert_eq!(msg.chat_id, "lid123@lid");
    }

    #[tokio::test]
    async fn test_handle_incoming_message_falls_back_to_sender() {
        let bus = Arc::new(MessageBus::new(32));
        let ch = WhatsAppChannel::new(String::new(), bus.clone(), vec![]);

        let payload = json!({
            "sender": "67890@s.whatsapp.net",
            "content": "test"
        });

        ch.handle_incoming_message(&payload).await;

        let msg = bus.consume_inbound().await.unwrap();
        assert_eq!(msg.sender_id, "67890");
    }

    #[tokio::test]
    async fn test_handle_incoming_message_empty_content() {
        let bus = Arc::new(MessageBus::new(32));
        let ch = WhatsAppChannel::new(String::new(), bus.clone(), vec![]);

        let payload = json!({
            "sender": "12345@s.whatsapp.net",
            "content": ""
        });

        ch.handle_incoming_message(&payload).await;

        // Empty content should be ignored (not published)
        // Try a non-blocking recv
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            bus.consume_inbound(),
        )
        .await;
        assert!(result.is_err()); // timeout = no message
    }

    #[tokio::test]
    async fn test_handle_incoming_message_unauthorized() {
        let bus = Arc::new(MessageBus::new(32));
        let ch = WhatsAppChannel::new(
            String::new(),
            bus.clone(),
            vec!["allowed_phone".into()],
        );

        let payload = json!({
            "sender": "unauthorized@s.whatsapp.net",
            "content": "hello"
        });

        ch.handle_incoming_message(&payload).await;

        // Should be silently ignored
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            bus.consume_inbound(),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_handle_incoming_message_no_sender() {
        let bus = Arc::new(MessageBus::new(32));
        let ch = WhatsAppChannel::new(String::new(), bus.clone(), vec![]);

        let payload = json!({
            "content": "orphan message"
        });

        ch.handle_incoming_message(&payload).await;

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            bus.consume_inbound(),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_handle_incoming_message_group() {
        let bus = Arc::new(MessageBus::new(32));
        let ch = WhatsAppChannel::new(String::new(), bus.clone(), vec![]);

        let payload = json!({
            "sender": "group123@g.us",
            "pn": "34612@s.whatsapp.net",
            "content": "group msg",
            "isGroup": true
        });

        ch.handle_incoming_message(&payload).await;

        let msg = bus.consume_inbound().await.unwrap();
        assert_eq!(msg.metadata.get("is_group").unwrap(), "true");
    }

    #[tokio::test]
    async fn test_send_without_connection() {
        let ch = create_test_channel();
        let msg = OutboundMessage::new("whatsapp", "12345@lid", "hello");
        // Should not error, just warn and drop
        let result = ch.send(&msg).await;
        assert!(result.is_ok());
    }
}
