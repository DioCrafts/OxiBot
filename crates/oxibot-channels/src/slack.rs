//! Slack channel — Socket Mode WebSocket + REST API.
//!
//! Port of nanobot's `channels/slack.py`.
//!
//! Uses Slack's Socket Mode (WebSocket) for receiving events and
//! the Web API (REST) for sending messages. No Bolt framework.
//!
//! Features:
//! - Socket Mode WebSocket with envelope ACKs
//! - Two-tiered access: DM policy + channel/group policy
//! - De-duplication of `message` vs `app_mention` events
//! - Thread support (DMs skip thread_ts, channels use it)
//! - `:eyes:` reaction as acknowledgment indicator
//! - Bot-mention stripping
//! - Message chunking for >4000 char responses
//! - Auto-reconnect with backoff

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{Mutex, Notify, RwLock};
use tracing::{debug, error, info, warn};

use oxibot_core::bus::queue::MessageBus;
use oxibot_core::bus::types::{InboundMessage, OutboundMessage};
use oxibot_core::config::schema::SlackConfig;

use crate::base::Channel;

// ─────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────

/// Slack Web API base URL.
const SLACK_API_BASE: &str = "https://slack.com/api";

/// Slack message length limit for `chat.postMessage`.
const SLACK_MAX_LEN: usize = 4000;

/// Reconnect backoff (seconds).
const RECONNECT_DELAY_SECS: u64 = 5;

/// Maximum reconnect attempts before giving up.
const MAX_RECONNECT_ATTEMPTS: u32 = 10;

// ─────────────────────────────────────────────
// Socket Mode types
// ─────────────────────────────────────────────

/// Socket Mode envelope received from Slack.
#[derive(Debug, Clone, Deserialize)]
struct SocketEnvelope {
    /// Envelope ID — must be ACKed immediately.
    envelope_id: String,
    /// Envelope type: `"events_api"`, `"slash_commands"`, `"interactive"`.
    #[serde(rename = "type")]
    envelope_type: String,
    /// The payload (events_api wraps an event callback).
    #[serde(default)]
    payload: Value,
}

/// ACK response sent back to Slack.
#[derive(Debug, Serialize)]
struct SocketAck {
    envelope_id: String,
}

// ─────────────────────────────────────────────
// SlackChannel
// ─────────────────────────────────────────────

/// Slack channel using Socket Mode + Web API.
pub struct SlackChannel {
    /// Full config (tokens, policies, etc.).
    config: SlackConfig,
    /// Message bus for inbound/outbound.
    bus: Arc<MessageBus>,
    /// Shutdown signal.
    shutdown: Arc<Notify>,
    /// HTTP client for Web API calls.
    http: reqwest::Client,
    /// Bot's own user ID (resolved via `auth.test`).
    bot_user_id: Arc<RwLock<Option<String>>>,
    /// Active WebSocket write half (for sending ACKs).
    ws_write: Arc<Mutex<Option<WsSender>>>,
}

/// Type alias for the WebSocket sink.
type WsSender = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::Message,
>;

impl SlackChannel {
    /// Create a new Slack channel from config.
    pub fn new(config: SlackConfig, bus: Arc<MessageBus>) -> Self {
        Self {
            config,
            bus,
            shutdown: Arc::new(Notify::new()),
            http: reqwest::Client::new(),
            bot_user_id: Arc::new(RwLock::new(None)),
            ws_write: Arc::new(Mutex::new(None)),
        }
    }

    // ─────────────────────────────────────────
    // Connection helpers
    // ─────────────────────────────────────────

    /// Call `apps.connections.open` to get a WebSocket URL for Socket Mode.
    async fn get_ws_url(&self) -> anyhow::Result<String> {
        let resp = self
            .http
            .post(format!("{}/apps.connections.open", SLACK_API_BASE))
            .bearer_auth(&self.config.app_token)
            .send()
            .await?;

        let body: Value = resp.json().await?;
        if body["ok"].as_bool() != Some(true) {
            let err = body["error"].as_str().unwrap_or("unknown");
            anyhow::bail!("apps.connections.open failed: {}", err);
        }

        let url = body["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("no url in apps.connections.open response"))?;

        Ok(url.to_string())
    }

    /// Call `auth.test` to resolve the bot's own user ID.
    async fn resolve_bot_id(&self) -> anyhow::Result<String> {
        let resp = self
            .http
            .post(format!("{}/auth.test", SLACK_API_BASE))
            .bearer_auth(&self.config.bot_token)
            .send()
            .await?;

        let body: Value = resp.json().await?;
        if body["ok"].as_bool() != Some(true) {
            let err = body["error"].as_str().unwrap_or("unknown");
            anyhow::bail!("auth.test failed: {}", err);
        }

        let user_id = body["user_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("no user_id in auth.test response"))?;

        Ok(user_id.to_string())
    }

    // ─────────────────────────────────────────
    // Access control
    // ─────────────────────────────────────────

    /// Check if a sender is allowed in the given context.
    ///
    /// Two-tiered policy:
    /// - DMs: `dm.enabled` → `dm.policy` → `dm.allow_from`
    /// - Channels/groups: `allowed_users` (flat list)
    fn is_allowed(&self, sender_id: &str, _chat_id: &str, channel_type: &str) -> bool {
        if channel_type == "im" {
            // DM policy
            if !self.config.dm.enabled {
                return false;
            }
            match self.config.dm.policy.as_str() {
                "allowlist" => self.config.dm.allow_from.iter().any(|u| u == sender_id),
                _ => true, // "open" or unrecognized → allow all
            }
        } else {
            // Channel/group: flat allow-list
            if self.config.allowed_users.is_empty() {
                return true;
            }
            self.config.allowed_users.iter().any(|u| u == sender_id)
        }
    }

    /// Check whether the bot should respond in a channel/group message.
    ///
    /// Policy:
    /// - `"open"` — respond to all messages
    /// - `"mention"` — only respond to `app_mention` or messages containing `<@BOT_ID>`
    /// - `"allowlist"` — only respond in channels listed in `group_allow_from`
    fn should_respond_in_channel(
        &self,
        event_type: &str,
        text: &str,
        chat_id: &str,
        bot_id: &str,
    ) -> bool {
        match self.config.group_policy.as_str() {
            "open" => true,
            "allowlist" => self.config.group_allow_from.iter().any(|c| c == chat_id),
            _ => {
                // "mention" (default)
                event_type == "app_mention"
                    || text.contains(&format!("<@{}>", bot_id))
            }
        }
    }

    /// Strip `<@BOT_ID>` mention from text.
    fn strip_bot_mention(text: &str, bot_id: &str) -> String {
        let pattern = format!("<@{}>", bot_id);
        text.replace(&pattern, "").trim().to_string()
    }

    // ─────────────────────────────────────────
    // Web API helpers
    // ─────────────────────────────────────────

    /// Add a reaction to a message (best-effort).
    async fn add_reaction(&self, channel: &str, timestamp: &str, emoji: &str) {
        let resp = self
            .http
            .post(format!("{}/reactions.add", SLACK_API_BASE))
            .bearer_auth(&self.config.bot_token)
            .json(&json!({
                "channel": channel,
                "timestamp": timestamp,
                "name": emoji,
            }))
            .send()
            .await;

        match resp {
            Ok(r) => {
                if let Ok(body) = r.json::<Value>().await {
                    if body["ok"].as_bool() != Some(true) {
                        debug!(
                            error = %body["error"].as_str().unwrap_or("unknown"),
                            "reaction add failed (non-fatal)"
                        );
                    }
                }
            }
            Err(e) => debug!(error = %e, "reaction add HTTP error (non-fatal)"),
        }
    }

    /// Send a chat message via `chat.postMessage`.
    async fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut body = json!({
            "channel": channel,
            "text": text,
        });

        if let Some(ts) = thread_ts {
            body["thread_ts"] = json!(ts);
        }

        let resp = self
            .http
            .post(format!("{}/chat.postMessage", SLACK_API_BASE))
            .bearer_auth(&self.config.bot_token)
            .json(&body)
            .send()
            .await?;

        let resp_body: Value = resp.json().await?;
        if resp_body["ok"].as_bool() != Some(true) {
            let err = resp_body["error"].as_str().unwrap_or("unknown");
            anyhow::bail!("chat.postMessage failed: {}", err);
        }

        Ok(())
    }

    /// Split a long message into chunks of up to `SLACK_MAX_LEN` characters.
    fn split_message(text: &str) -> Vec<String> {
        if text.len() <= SLACK_MAX_LEN {
            return vec![text.to_string()];
        }

        let mut chunks = Vec::new();
        let mut remaining = text;

        while !remaining.is_empty() {
            if remaining.len() <= SLACK_MAX_LEN {
                chunks.push(remaining.to_string());
                break;
            }

            // Try to split at a newline within the limit
            let slice = &remaining[..SLACK_MAX_LEN];
            let split_at = slice.rfind('\n').unwrap_or(SLACK_MAX_LEN);
            let split_at = if split_at == 0 { SLACK_MAX_LEN } else { split_at };

            chunks.push(remaining[..split_at].to_string());
            remaining = remaining[split_at..].trim_start_matches('\n');
        }

        chunks
    }

    // ─────────────────────────────────────────
    // Socket Mode event processing
    // ─────────────────────────────────────────

    /// Process a Socket Mode envelope.
    async fn process_envelope(&self, envelope: SocketEnvelope) {
        // Only handle events_api envelopes
        if envelope.envelope_type != "events_api" {
            debug!(
                envelope_type = %envelope.envelope_type,
                "ignoring non-events_api envelope"
            );
            return;
        }

        let event = &envelope.payload["event"];
        let event_type = event["type"].as_str().unwrap_or("");

        // Only handle `message` and `app_mention`
        if event_type != "message" && event_type != "app_mention" {
            debug!(event_type = %event_type, "ignoring event type");
            return;
        }

        // Skip messages with subtypes (edits, joins, bot_messages, etc.)
        if event_type == "message" && event.get("subtype").is_some() {
            debug!("ignoring message with subtype");
            return;
        }

        let sender_id = event["user"].as_str().unwrap_or("").to_string();
        let chat_id = event["channel"].as_str().unwrap_or("").to_string();
        let text = event["text"].as_str().unwrap_or("").to_string();
        let ts = event["ts"].as_str().unwrap_or("").to_string();
        let thread_ts = event
            .get("thread_ts")
            .and_then(|v| v.as_str())
            .unwrap_or(&ts)
            .to_string();
        let channel_type = event["channel_type"]
            .as_str()
            .unwrap_or("channel")
            .to_string();

        // Get bot user ID
        let bot_id = {
            let guard = self.bot_user_id.read().await;
            guard.clone().unwrap_or_default()
        };

        // Skip bot's own messages
        if sender_id == bot_id {
            debug!("ignoring bot's own message");
            return;
        }

        // De-duplicate: if event is `message` and text mentions the bot,
        // skip it — the `app_mention` event will handle it instead.
        if event_type == "message" && text.contains(&format!("<@{}>", bot_id)) {
            debug!("skipping message with mention (app_mention will handle)");
            return;
        }

        // Access control
        if !self.is_allowed(&sender_id, &chat_id, &channel_type) {
            warn!(
                sender = %sender_id,
                chat = %chat_id,
                "access denied by policy"
            );
            return;
        }

        // Channel/group response policy (DMs always respond if allowed)
        if channel_type != "im"
            && !self.should_respond_in_channel(event_type, &text, &chat_id, &bot_id)
        {
            debug!("not responding in channel per group_policy");
            return;
        }

        // Strip bot mention from text
        let clean_text = if !bot_id.is_empty() {
            Self::strip_bot_mention(&text, &bot_id)
        } else {
            text.clone()
        };

        if clean_text.is_empty() {
            debug!("empty message after mention stripping, ignoring");
            return;
        }

        // Add :eyes: reaction as acknowledgment
        self.add_reaction(&chat_id, &ts, "eyes").await;

        // Build metadata
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("channel_type".to_string(), channel_type.clone());
        metadata.insert("thread_ts".to_string(), thread_ts.clone());
        metadata.insert("ts".to_string(), ts);

        // Publish inbound message
        let inbound = InboundMessage {
            sender_id: sender_id.clone(),
            chat_id: chat_id.clone(),
            channel: "slack".to_string(),
            content: clean_text,
            timestamp: chrono::Utc::now(),
            media: Vec::new(),
            metadata,
        };

        if let Err(e) = self.bus.publish_inbound(inbound).await {
            error!(error = %e, "failed to publish inbound message");
        }
    }

    // ─────────────────────────────────────────
    // WebSocket loop
    // ─────────────────────────────────────────

    /// Main Socket Mode loop — connects, receives events, ACKs envelopes.
    async fn run_socket_loop(&self) -> anyhow::Result<()> {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message as WsMessage;

        let mut attempts: u32 = 0;

        loop {
            // Check shutdown
            if attempts > 0 {
                let delay = Duration::from_secs(RECONNECT_DELAY_SECS * (attempts as u64).min(6));
                info!(
                    attempt = attempts,
                    delay_secs = delay.as_secs(),
                    "reconnecting to Slack Socket Mode..."
                );
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = self.shutdown.notified() => {
                        info!("shutdown during reconnect backoff");
                        return Ok(());
                    }
                }
            }

            if attempts >= MAX_RECONNECT_ATTEMPTS {
                anyhow::bail!(
                    "exceeded max reconnect attempts ({})",
                    MAX_RECONNECT_ATTEMPTS
                );
            }

            // Get WebSocket URL via apps.connections.open
            let ws_url = match self.get_ws_url().await {
                Ok(url) => {
                    debug!(url = %url, "got Socket Mode URL");
                    url
                }
                Err(e) => {
                    error!(error = %e, "failed to get Socket Mode URL");
                    attempts += 1;
                    continue;
                }
            };

            // Connect WebSocket
            let ws_stream = match tokio_tungstenite::connect_async(&ws_url).await {
                Ok((stream, _)) => {
                    info!("connected to Slack Socket Mode");
                    attempts = 0;
                    stream
                }
                Err(e) => {
                    error!(error = %e, "WebSocket connect failed");
                    attempts += 1;
                    continue;
                }
            };

            let (write, mut read) = ws_stream.split();
            {
                let mut guard = self.ws_write.lock().await;
                *guard = Some(write);
            }

            // Read loop
            loop {
                tokio::select! {
                    msg = read.next() => {
                        match msg {
                            Some(Ok(WsMessage::Text(text))) => {
                                self.handle_ws_message(&text).await;
                            }
                            Some(Ok(WsMessage::Ping(data))) => {
                                let mut guard = self.ws_write.lock().await;
                                if let Some(ref mut w) = *guard {
                                    let _ = w.send(WsMessage::Pong(data)).await;
                                }
                            }
                            Some(Ok(WsMessage::Close(_))) => {
                                info!("Slack WebSocket closed by server");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!(error = %e, "Slack WebSocket error");
                                break;
                            }
                            None => {
                                info!("Slack WebSocket stream ended");
                                break;
                            }
                            _ => {} // Binary, etc.
                        }
                    }
                    _ = self.shutdown.notified() => {
                        info!("shutdown signal received");
                        let mut guard = self.ws_write.lock().await;
                        if let Some(ref mut w) = *guard {
                            let _ = w.close().await;
                        }
                        *guard = None;
                        return Ok(());
                    }
                }
            }

            // Clean up write half before reconnect
            {
                let mut guard = self.ws_write.lock().await;
                *guard = None;
            }
            attempts += 1;
        }
    }

    /// Handle a single WebSocket text message.
    async fn handle_ws_message(&self, text: &str) {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message as WsMessage;

        // Check for hello/disconnect messages
        if let Ok(msg) = serde_json::from_str::<Value>(text) {
            if msg["type"].as_str() == Some("hello") {
                info!("received Socket Mode hello");
                return;
            }
            if msg["type"].as_str() == Some("disconnect") {
                let reason = msg["reason"].as_str().unwrap_or("unknown");
                info!(reason = %reason, "Slack requested disconnect");
                // The read loop will handle reconnection
                return;
            }
        }

        // Parse as envelope
        let envelope: SocketEnvelope = match serde_json::from_str(text) {
            Ok(e) => e,
            Err(e) => {
                debug!(error = %e, "failed to parse Socket Mode envelope");
                return;
            }
        };

        // ACK immediately
        let ack = SocketAck {
            envelope_id: envelope.envelope_id.clone(),
        };
        if let Ok(ack_json) = serde_json::to_string(&ack) {
            let mut guard = self.ws_write.lock().await;
            if let Some(ref mut w) = *guard {
                if let Err(e) = w.send(WsMessage::Text(ack_json.into())).await {
                    warn!(error = %e, "failed to send ACK");
                }
            }
        }

        // Process the envelope asynchronously
        self.process_envelope(envelope).await;
    }
}

// ─────────────────────────────────────────────
// Channel trait implementation
// ─────────────────────────────────────────────

#[async_trait]
impl Channel for SlackChannel {
    fn name(&self) -> &str {
        "slack"
    }

    async fn start(&self) -> anyhow::Result<()> {
        // Validate tokens
        if self.config.bot_token.is_empty() {
            warn!("slack bot_token is empty, channel will not start");
            return Ok(());
        }
        if self.config.app_token.is_empty() {
            warn!("slack app_token is empty (required for Socket Mode), channel will not start");
            return Ok(());
        }

        // Resolve bot user ID
        match self.resolve_bot_id().await {
            Ok(id) => {
                info!(bot_user_id = %id, "resolved Slack bot user ID");
                let mut guard = self.bot_user_id.write().await;
                *guard = Some(id);
            }
            Err(e) => {
                warn!(error = %e, "could not resolve bot user ID (mention detection may not work)");
            }
        }

        info!(
            group_policy = %self.config.group_policy,
            dm_enabled = self.config.dm.enabled,
            "starting Slack Socket Mode channel"
        );

        // Enter Socket Mode loop
        self.run_socket_loop().await
    }

    async fn stop(&self) -> anyhow::Result<()> {
        info!("stopping Slack channel");
        self.shutdown.notify_waiters();

        // Close WebSocket
        {
            use futures_util::SinkExt;

            let mut guard = self.ws_write.lock().await;
            if let Some(ref mut w) = *guard {
                let _ = w.close().await;
            }
            *guard = None;
        }

        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> anyhow::Result<()> {
        let channel_type = msg
            .metadata
            .get("channel_type")
            .map(|s| s.as_str())
            .unwrap_or("channel");

        // Thread support: use thread_ts for channels, skip for DMs
        let thread_ts = if channel_type != "im" {
            msg.metadata.get("thread_ts").map(|s| s.as_str())
        } else {
            None
        };

        // Split long messages
        let chunks = Self::split_message(&msg.content);

        for chunk in &chunks {
            if let Err(e) = self.post_message(&msg.chat_id, chunk, thread_ts).await {
                error!(error = %e, "failed to send Slack message");
                return Err(e);
            }
        }

        Ok(())
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> SlackConfig {
        SlackConfig {
            bot_token: "xoxb-test-token".into(),
            app_token: "xapp-test-token".into(),
            allowed_users: Vec::new(),
            group_policy: "mention".into(),
            group_allow_from: Vec::new(),
            dm: oxibot_core::config::schema::SlackDMConfig {
                enabled: true,
                policy: "open".into(),
                allow_from: Vec::new(),
            },
        }
    }

    fn make_bus() -> Arc<MessageBus> {
        Arc::new(MessageBus::new(10))
    }

    // ── Channel trait ──

    #[test]
    fn test_channel_name() {
        let ch = SlackChannel::new(make_config(), make_bus());
        assert_eq!(ch.name(), "slack");
    }

    #[tokio::test]
    async fn test_stop_without_start() {
        let ch = SlackChannel::new(make_config(), make_bus());
        // Should not panic
        ch.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_start_empty_bot_token() {
        let mut cfg = make_config();
        cfg.bot_token = String::new();
        let ch = SlackChannel::new(cfg, make_bus());
        // Should return Ok without starting
        ch.start().await.unwrap();
    }

    #[tokio::test]
    async fn test_start_empty_app_token() {
        let mut cfg = make_config();
        cfg.app_token = String::new();
        let ch = SlackChannel::new(cfg, make_bus());
        ch.start().await.unwrap();
    }

    // ── Access control ──

    #[test]
    fn test_dm_allowed_open_policy() {
        let ch = SlackChannel::new(make_config(), make_bus());
        assert!(ch.is_allowed("U123", "D456", "im"));
    }

    #[test]
    fn test_dm_disabled() {
        let mut cfg = make_config();
        cfg.dm.enabled = false;
        let ch = SlackChannel::new(cfg, make_bus());
        assert!(!ch.is_allowed("U123", "D456", "im"));
    }

    #[test]
    fn test_dm_allowlist_allowed() {
        let mut cfg = make_config();
        cfg.dm.policy = "allowlist".into();
        cfg.dm.allow_from = vec!["U123".into()];
        let ch = SlackChannel::new(cfg, make_bus());
        assert!(ch.is_allowed("U123", "D456", "im"));
    }

    #[test]
    fn test_dm_allowlist_denied() {
        let mut cfg = make_config();
        cfg.dm.policy = "allowlist".into();
        cfg.dm.allow_from = vec!["U999".into()];
        let ch = SlackChannel::new(cfg, make_bus());
        assert!(!ch.is_allowed("U123", "D456", "im"));
    }

    #[test]
    fn test_channel_allowed_no_list() {
        let ch = SlackChannel::new(make_config(), make_bus());
        assert!(ch.is_allowed("U123", "C456", "channel"));
    }

    #[test]
    fn test_channel_allowed_in_list() {
        let mut cfg = make_config();
        cfg.allowed_users = vec!["U123".into()];
        let ch = SlackChannel::new(cfg, make_bus());
        assert!(ch.is_allowed("U123", "C456", "channel"));
    }

    #[test]
    fn test_channel_denied_not_in_list() {
        let mut cfg = make_config();
        cfg.allowed_users = vec!["U999".into()];
        let ch = SlackChannel::new(cfg, make_bus());
        assert!(!ch.is_allowed("U123", "C456", "channel"));
    }

    // ── Group policy ──

    #[test]
    fn test_should_respond_open() {
        let mut cfg = make_config();
        cfg.group_policy = "open".into();
        let ch = SlackChannel::new(cfg, make_bus());
        assert!(ch.should_respond_in_channel("message", "hello", "C123", "BBOT"));
    }

    #[test]
    fn test_should_respond_mention_with_mention() {
        let ch = SlackChannel::new(make_config(), make_bus());
        assert!(ch.should_respond_in_channel(
            "message",
            "hey <@BBOT> do stuff",
            "C123",
            "BBOT"
        ));
    }

    #[test]
    fn test_should_respond_mention_without_mention() {
        let ch = SlackChannel::new(make_config(), make_bus());
        assert!(!ch.should_respond_in_channel("message", "hello world", "C123", "BBOT"));
    }

    #[test]
    fn test_should_respond_mention_app_mention_event() {
        let ch = SlackChannel::new(make_config(), make_bus());
        assert!(ch.should_respond_in_channel("app_mention", "hello", "C123", "BBOT"));
    }

    #[test]
    fn test_should_respond_allowlist_allowed() {
        let mut cfg = make_config();
        cfg.group_policy = "allowlist".into();
        cfg.group_allow_from = vec!["C123".into()];
        let ch = SlackChannel::new(cfg, make_bus());
        assert!(ch.should_respond_in_channel("message", "hello", "C123", "BBOT"));
    }

    #[test]
    fn test_should_respond_allowlist_denied() {
        let mut cfg = make_config();
        cfg.group_policy = "allowlist".into();
        cfg.group_allow_from = vec!["C999".into()];
        let ch = SlackChannel::new(cfg, make_bus());
        assert!(!ch.should_respond_in_channel("message", "hello", "C123", "BBOT"));
    }

    // ── Bot mention stripping ──

    #[test]
    fn test_strip_bot_mention() {
        let result = SlackChannel::strip_bot_mention("<@BBOT> hello world", "BBOT");
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_strip_bot_mention_middle() {
        let result = SlackChannel::strip_bot_mention("hey <@BBOT> do stuff", "BBOT");
        assert_eq!(result, "hey  do stuff");
    }

    #[test]
    fn test_strip_bot_mention_no_mention() {
        let result = SlackChannel::strip_bot_mention("hello world", "BBOT");
        assert_eq!(result, "hello world");
    }

    // ── Message splitting ──

    #[test]
    fn test_split_message_short() {
        let chunks = SlackChannel::split_message("hello");
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn test_split_message_long() {
        let msg = "x".repeat(SLACK_MAX_LEN + 100);
        let chunks = SlackChannel::split_message(&msg);
        assert!(chunks.len() >= 2);
        assert!(chunks[0].len() <= SLACK_MAX_LEN);
        // All content preserved
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, msg.len());
    }

    #[test]
    fn test_split_message_at_newline() {
        let mut msg = "a".repeat(SLACK_MAX_LEN - 10);
        msg.push('\n');
        msg.push_str(&"b".repeat(20));
        let chunks = SlackChannel::split_message(&msg);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], "a".repeat(SLACK_MAX_LEN - 10));
    }

    // ── Envelope processing ──

    #[tokio::test]
    async fn test_process_envelope_non_events_api() {
        let ch = SlackChannel::new(make_config(), make_bus());
        let envelope = SocketEnvelope {
            envelope_id: "eid123".into(),
            envelope_type: "slash_commands".into(),
            payload: json!({}),
        };
        // Should not panic, just skip
        ch.process_envelope(envelope).await;
    }

    #[tokio::test]
    async fn test_process_envelope_unknown_event_type() {
        let ch = SlackChannel::new(make_config(), make_bus());
        let envelope = SocketEnvelope {
            envelope_id: "eid123".into(),
            envelope_type: "events_api".into(),
            payload: json!({
                "event": {
                    "type": "reaction_added",
                    "user": "U123",
                    "channel": "C456"
                }
            }),
        };
        ch.process_envelope(envelope).await;
    }

    #[tokio::test]
    async fn test_process_envelope_message_with_subtype() {
        let ch = SlackChannel::new(make_config(), make_bus());
        let envelope = SocketEnvelope {
            envelope_id: "eid123".into(),
            envelope_type: "events_api".into(),
            payload: json!({
                "event": {
                    "type": "message",
                    "subtype": "bot_message",
                    "user": "U123",
                    "channel": "C456",
                    "text": "hello"
                }
            }),
        };
        ch.process_envelope(envelope).await;
        // Should be filtered out (no inbound message published)
    }

    #[tokio::test]
    async fn test_process_envelope_skips_bot_own_message() {
        let ch = SlackChannel::new(make_config(), make_bus());
        {
            let mut guard = ch.bot_user_id.write().await;
            *guard = Some("BBOT".into());
        }
        let envelope = SocketEnvelope {
            envelope_id: "eid123".into(),
            envelope_type: "events_api".into(),
            payload: json!({
                "event": {
                    "type": "message",
                    "user": "BBOT",
                    "channel": "D456",
                    "channel_type": "im",
                    "text": "my own message",
                    "ts": "1234567890.123456"
                }
            }),
        };
        ch.process_envelope(envelope).await;
    }

    #[tokio::test]
    async fn test_process_envelope_deduplicates_mention() {
        let ch = SlackChannel::new(make_config(), make_bus());
        {
            let mut guard = ch.bot_user_id.write().await;
            *guard = Some("BBOT".into());
        }
        let envelope = SocketEnvelope {
            envelope_id: "eid123".into(),
            envelope_type: "events_api".into(),
            payload: json!({
                "event": {
                    "type": "message",
                    "user": "U123",
                    "channel": "C456",
                    "channel_type": "channel",
                    "text": "<@BBOT> hello",
                    "ts": "1234567890.123456"
                }
            }),
        };
        // Should be skipped (app_mention will handle it)
        ch.process_envelope(envelope).await;
    }

    #[tokio::test]
    async fn test_process_envelope_dm_disabled() {
        let mut cfg = make_config();
        cfg.dm.enabled = false;
        let ch = SlackChannel::new(cfg, make_bus());
        let envelope = SocketEnvelope {
            envelope_id: "eid123".into(),
            envelope_type: "events_api".into(),
            payload: json!({
                "event": {
                    "type": "message",
                    "user": "U123",
                    "channel": "D456",
                    "channel_type": "im",
                    "text": "hello",
                    "ts": "1234567890.123456"
                }
            }),
        };
        ch.process_envelope(envelope).await;
        // Should be filtered by DM policy
    }

    // ── Socket Mode types ──

    #[test]
    fn test_socket_envelope_deserialize() {
        let json = r#"{
            "envelope_id": "abc123",
            "type": "events_api",
            "payload": {"event": {"type": "message"}}
        }"#;
        let envelope: SocketEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.envelope_id, "abc123");
        assert_eq!(envelope.envelope_type, "events_api");
    }

    #[test]
    fn test_socket_ack_serialize() {
        let ack = SocketAck {
            envelope_id: "abc123".into(),
        };
        let json = serde_json::to_string(&ack).unwrap();
        assert!(json.contains("abc123"));
    }

    // ── Handle WS message ──

    #[tokio::test]
    async fn test_handle_ws_hello() {
        let ch = SlackChannel::new(make_config(), make_bus());
        // Should not crash
        ch.handle_ws_message(r#"{"type":"hello"}"#).await;
    }

    #[tokio::test]
    async fn test_handle_ws_disconnect() {
        let ch = SlackChannel::new(make_config(), make_bus());
        ch.handle_ws_message(r#"{"type":"disconnect","reason":"refresh_requested"}"#)
            .await;
    }

    #[tokio::test]
    async fn test_handle_ws_invalid_json() {
        let ch = SlackChannel::new(make_config(), make_bus());
        ch.handle_ws_message("not json at all").await;
    }
}
