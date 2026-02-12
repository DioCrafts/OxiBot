//! Discord channel — raw Gateway WebSocket + REST API.
//!
//! Port of nanobot's `channels/discord.py`.
//!
//! Uses the raw Discord Gateway (WebSocket) for receiving messages
//! and the REST API for sending. No heavy Discord library required.
//!
//! Features:
//! - Gateway v10 WebSocket with heartbeat + resume
//! - Text and attachment handling
//! - Typing indicator while agent processes
//! - Allow-list by Discord user ID
//! - Message chunking for >2000 char responses
//! - Rate-limit retry (HTTP 429)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::{Mutex, Notify, RwLock};
use tracing::{debug, error, info, warn};

use oxibot_core::bus::queue::MessageBus;
use oxibot_core::bus::types::{InboundMessage, OutboundMessage};

use crate::base::Channel;

// ─────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────

/// Discord REST API base URL.
const DISCORD_API_BASE: &str = "https://discord.com/api/v10";

/// Default Gateway WebSocket URL.
const DEFAULT_GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";

/// Discord message length limit.
const DISCORD_MAX_LEN: usize = 2000;

/// Maximum attachment download size (20 MB).
const MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024;

/// Typing indicator refresh interval (Discord typing lasts ~10s).
const TYPING_INTERVAL_SECS: u64 = 8;

/// Default intents: GUILDS(1) + GUILD_MESSAGES(512) + DMs(4096) + MESSAGE_CONTENT(32768).
const DEFAULT_INTENTS: u64 = 1 + 512 + 4096 + 32768;

// Gateway opcodes
const OP_DISPATCH: u64 = 0;
const OP_HEARTBEAT: u64 = 1;
const OP_IDENTIFY: u64 = 2;
const OP_RESUME: u64 = 6;
const OP_RECONNECT: u64 = 7;
const OP_INVALID_SESSION: u64 = 9;
const OP_HELLO: u64 = 10;
const OP_HEARTBEAT_ACK: u64 = 11;

// ─────────────────────────────────────────────
// DiscordChannel
// ─────────────────────────────────────────────

/// Discord channel using raw Gateway WebSocket + REST API.
pub struct DiscordChannel {
    /// Bot token from Discord Developer Portal.
    token: String,
    /// Message bus for inbound/outbound.
    bus: Arc<MessageBus>,
    /// Allow-list of Discord user IDs. Empty = allow everyone.
    allowed_users: Vec<String>,
    /// Gateway WebSocket URL.
    gateway_url: String,
    /// Gateway intents bitmask.
    intents: u64,
    /// Shutdown signal.
    shutdown: Arc<Notify>,
    /// HTTP client for REST API calls.
    http: reqwest::Client,
    /// Active typing indicator tasks keyed by channel_id.
    typing_tasks: Arc<RwLock<HashMap<String, tokio::task::JoinHandle<()>>>>,
    /// Gateway sequence number for heartbeats.
    seq: Arc<Mutex<Option<u64>>>,
    /// Whether last heartbeat was ACKed (zombie detection).
    heartbeat_acked: Arc<Mutex<bool>>,
    /// Session ID for resume.
    session_id: Arc<Mutex<Option<String>>>,
    /// Resume gateway URL.
    resume_url: Arc<Mutex<Option<String>>>,
}

impl DiscordChannel {
    /// Create a new Discord channel.
    pub fn new(
        token: String,
        bus: Arc<MessageBus>,
        allowed_users: Vec<String>,
    ) -> Self {
        Self {
            token,
            bus,
            allowed_users,
            gateway_url: DEFAULT_GATEWAY_URL.into(),
            intents: DEFAULT_INTENTS,
            shutdown: Arc::new(Notify::new()),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("failed to create HTTP client"),
            typing_tasks: Arc::new(RwLock::new(HashMap::new())),
            seq: Arc::new(Mutex::new(None)),
            heartbeat_acked: Arc::new(Mutex::new(true)),
            session_id: Arc::new(Mutex::new(None)),
            resume_url: Arc::new(Mutex::new(None)),
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

    /// Run the Gateway WebSocket connection with auto-reconnect.
    async fn run_gateway(&self) -> anyhow::Result<()> {
        loop {
            let result = self.gateway_session().await;
            match result {
                Ok(()) => {
                    info!("discord gateway session ended normally");
                    break;
                }
                Err(e) => {
                    warn!(error = %e, "discord gateway error, reconnecting in 5s");
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                        _ = self.shutdown.notified() => {
                            info!("discord shutdown during reconnect wait");
                            return Ok(());
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Single Gateway WebSocket session.
    async fn gateway_session(&self) -> anyhow::Result<()> {
        use tokio_tungstenite::tungstenite::Message as WsMessage;

        // Decide URL: resume URL or default
        let url = {
            let resume = self.resume_url.lock().await;
            resume
                .as_deref()
                .unwrap_or(&self.gateway_url)
                .to_string()
        };

        debug!(url = %url, "connecting to discord gateway");
        let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await?;

        use futures_util::{SinkExt, StreamExt};
        let (mut write, mut read) = ws_stream.split();

        // Heartbeat handle
        #[allow(unused_assignments)]
        let mut heartbeat_handle: Option<tokio::task::JoinHandle<()>> = None;

        loop {
            tokio::select! {
                msg = read.next() => {
                    let msg = match msg {
                        Some(Ok(m)) => m,
                        Some(Err(e)) => {
                            error!(error = %e, "discord ws read error");
                            break;
                        }
                        None => {
                            debug!("discord ws stream ended");
                            break;
                        }
                    };

                    let text = match msg {
                        WsMessage::Text(t) => t.to_string(),
                        WsMessage::Close(_) => {
                            info!("discord ws closed by server");
                            break;
                        }
                        _ => continue,
                    };

                    let payload: Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(e) => {
                            warn!(error = %e, "discord ws invalid json");
                            continue;
                        }
                    };

                    let op = payload["op"].as_u64().unwrap_or(0);
                    let seq = payload["s"].as_u64();
                    let event = payload["t"].as_str();

                    // Update sequence number
                    if let Some(s) = seq {
                        *self.seq.lock().await = Some(s);
                    }

                    match op {
                        OP_HELLO => {
                            let interval = payload["d"]["heartbeat_interval"]
                                .as_u64()
                                .unwrap_or(41250);
                            debug!(interval_ms = interval, "discord HELLO received");

                            // Start heartbeat
                            if let Some(h) = heartbeat_handle.take() {
                                h.abort();
                            }
                            let seq_ref = self.seq.clone();
                            let acked_ref = self.heartbeat_acked.clone();
                            let shutdown_ref = self.shutdown.clone();

                            // Heartbeat sender — we'll collect frames and flush them to `write`
                            let (hb_tx, mut hb_rx) =
                                tokio::sync::mpsc::channel::<String>(8);

                            heartbeat_handle = Some(tokio::spawn(async move {
                                // Initial jitter
                                let jitter = interval as f64 * rand_jitter();
                                tokio::time::sleep(Duration::from_millis(jitter as u64)).await;

                                loop {
                                    // Check ACK
                                    {
                                        let mut acked = acked_ref.lock().await;
                                        if !*acked {
                                            warn!("discord heartbeat not ACKed, requesting reconnect");
                                            break;
                                        }
                                        *acked = false;
                                    }

                                    let s = *seq_ref.lock().await;
                                    let hb = json!({"op": OP_HEARTBEAT, "d": s}).to_string();
                                    if hb_tx.send(hb).await.is_err() {
                                        break;
                                    }

                                    tokio::select! {
                                        _ = tokio::time::sleep(Duration::from_millis(interval)) => {}
                                        _ = shutdown_ref.notified() => break,
                                    }
                                }
                            }));

                            // Spawn a task to forward heartbeat messages to WS
                            let (ws_tx, mut ws_rx) =
                                tokio::sync::mpsc::channel::<String>(16);

                            // Forward heartbeats into ws_tx
                            let ws_tx_hb = ws_tx.clone();
                            tokio::spawn(async move {
                                while let Some(msg) = hb_rx.recv().await {
                                    if ws_tx_hb.send(msg).await.is_err() {
                                        break;
                                    }
                                }
                            });

                            // Send IDENTIFY or RESUME
                            let session = self.session_id.lock().await.clone();
                            let identify_msg = if let Some(ref sid) = session {
                                let s = *self.seq.lock().await;
                                json!({
                                    "op": OP_RESUME,
                                    "d": {
                                        "token": self.token,
                                        "session_id": sid,
                                        "seq": s
                                    }
                                })
                                .to_string()
                            } else {
                                json!({
                                    "op": OP_IDENTIFY,
                                    "d": {
                                        "token": self.token,
                                        "intents": self.intents,
                                        "properties": {
                                            "os": "oxibot",
                                            "browser": "oxibot",
                                            "device": "oxibot"
                                        }
                                    }
                                })
                                .to_string()
                            };

                            write.send(WsMessage::text(identify_msg)).await?;
                            *self.heartbeat_acked.lock().await = true;

                            // Process outgoing messages (heartbeats + any future writes)
                            // We'll handle them in the select below
                            // Store ws_tx for potential future use
                            // For now, handle writes in a separate select arm
                            let write_arc = Arc::new(Mutex::new(write));
                            let write_ref = write_arc.clone();

                            // Spawn ws writer
                            tokio::spawn(async move {
                                while let Some(msg) = ws_rx.recv().await {
                                    let mut w = write_ref.lock().await;
                                    if let Err(e) = w.send(WsMessage::text(msg)).await {
                                        warn!(error = %e, "discord ws write error");
                                        break;
                                    }
                                }
                            });

                            // Continue reading from the stream
                            // We need to restructure the loop since write was moved.
                            // Instead, break and reconnect with new architecture
                            // Actually, we already split read/write, so write was moved.
                            // Let's handle this by using the write_arc for the rest.
                            // But we can't reassign `write`. We need to refactor.
                            // For simplicity, let's handle everything inline.

                            // Read loop continues (write is in write_arc now)
                            loop {
                                tokio::select! {
                                    msg = read.next() => {
                                        let msg = match msg {
                                            Some(Ok(m)) => m,
                                            Some(Err(e)) => {
                                                error!(error = %e, "discord ws read error");
                                                return Err(e.into());
                                            }
                                            None => return Ok(()),
                                        };

                                        let text = match msg {
                                            WsMessage::Text(t) => t.to_string(),
                                            WsMessage::Close(_) => return Ok(()),
                                            _ => continue,
                                        };

                                        let payload: Value = match serde_json::from_str(&text) {
                                            Ok(v) => v,
                                            Err(_) => continue,
                                        };

                                        let op = payload["op"].as_u64().unwrap_or(0);
                                        if let Some(s) = payload["s"].as_u64() {
                                            *self.seq.lock().await = Some(s);
                                        }

                                        match op {
                                            OP_DISPATCH => {
                                                let event_name = payload["t"].as_str().unwrap_or("");
                                                match event_name {
                                                    "READY" => {
                                                        if let Some(sid) = payload["d"]["session_id"].as_str() {
                                                            *self.session_id.lock().await = Some(sid.to_string());
                                                        }
                                                        if let Some(url) = payload["d"]["resume_gateway_url"].as_str() {
                                                            *self.resume_url.lock().await = Some(url.to_string());
                                                        }
                                                        let user = payload["d"]["user"]["username"].as_str().unwrap_or("unknown");
                                                        info!(user = user, "discord bot READY");
                                                    }
                                                    "RESUMED" => {
                                                        info!("discord session resumed");
                                                    }
                                                    "MESSAGE_CREATE" => {
                                                        self.handle_message_create(&payload["d"]).await;
                                                    }
                                                    _ => {
                                                        debug!(event = event_name, "discord event (unhandled)");
                                                    }
                                                }
                                            }
                                            OP_HEARTBEAT_ACK => {
                                                *self.heartbeat_acked.lock().await = true;
                                            }
                                            OP_RECONNECT => {
                                                info!("discord server requested reconnect");
                                                return Err(anyhow::anyhow!("reconnect requested"));
                                            }
                                            OP_INVALID_SESSION => {
                                                let resumable = payload["d"].as_bool().unwrap_or(false);
                                                warn!(resumable = resumable, "discord invalid session");
                                                if !resumable {
                                                    *self.session_id.lock().await = None;
                                                    *self.resume_url.lock().await = None;
                                                }
                                                return Err(anyhow::anyhow!("invalid session"));
                                            }
                                            OP_HEARTBEAT => {
                                                // Server requesting immediate heartbeat
                                                let s = *self.seq.lock().await;
                                                let hb = json!({"op": OP_HEARTBEAT, "d": s}).to_string();
                                                let _ = ws_tx.send(hb).await;
                                            }
                                            _ => {}
                                        }
                                    }
                                    _ = self.shutdown.notified() => {
                                        info!("discord shutdown signal received");
                                        let mut w = write_arc.lock().await;
                                        let _ = w.send(WsMessage::Close(None)).await;
                                        return Ok(());
                                    }
                                }
                            }
                        }

                        OP_DISPATCH => {
                            // Handle events before HELLO (shouldn't happen but be safe)
                            if let Some("MESSAGE_CREATE") = event {
                                self.handle_message_create(&payload["d"]).await;
                            }
                        }

                        OP_HEARTBEAT_ACK => {
                            *self.heartbeat_acked.lock().await = true;
                        }

                        OP_RECONNECT => {
                            info!("discord server requested reconnect");
                            break;
                        }

                        OP_INVALID_SESSION => {
                            let resumable = payload["d"].as_bool().unwrap_or(false);
                            warn!(resumable = resumable, "discord invalid session");
                            if !resumable {
                                *self.session_id.lock().await = None;
                                *self.resume_url.lock().await = None;
                            }
                            break;
                        }

                        _ => {}
                    }
                }
                _ = self.shutdown.notified() => {
                    info!("discord shutdown signal during pre-hello");
                    return Ok(());
                }
            }
        }

        Ok(())
    }

    /// Handle a MESSAGE_CREATE event from the Gateway.
    async fn handle_message_create(&self, data: &Value) {
        // Ignore bot messages
        if data["author"]["bot"].as_bool().unwrap_or(false) {
            return;
        }

        let sender_id = match data["author"]["id"].as_str() {
            Some(id) => id.to_string(),
            None => return,
        };

        let channel_id = match data["channel_id"].as_str() {
            Some(id) => id.to_string(),
            None => return,
        };

        let username = data["author"]["username"]
            .as_str()
            .unwrap_or("")
            .to_string();

        // Check allow-list
        if !self.is_allowed(&sender_id) {
            warn!(
                sender = %sender_id,
                channel = %channel_id,
                "discord message from unauthorized user, ignoring"
            );
            return;
        }

        // Collect content
        let mut content_parts: Vec<String> = Vec::new();
        let mut media_paths: Vec<String> = Vec::new();

        // Text content
        if let Some(text) = data["content"].as_str() {
            if !text.is_empty() {
                content_parts.push(text.to_string());
            }
        }

        // Attachments
        if let Some(attachments) = data["attachments"].as_array() {
            for att in attachments {
                let url = match att["url"].as_str() {
                    Some(u) => u,
                    None => continue,
                };
                let filename = att["filename"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                let att_id = att["id"]
                    .as_str()
                    .unwrap_or("0")
                    .to_string();
                let size = att["size"].as_u64().unwrap_or(0);

                if size > MAX_ATTACHMENT_BYTES {
                    content_parts.push(format!("[attachment: {filename} — too large]"));
                    continue;
                }

                match self.download_attachment(url, &att_id, &filename).await {
                    Ok(path) => {
                        content_parts.push(format!("[attachment: {path}]"));
                        media_paths.push(path);
                    }
                    Err(e) => {
                        warn!(error = %e, filename = %filename, "failed to download attachment");
                        content_parts.push(format!("[attachment: {filename} — download failed]"));
                    }
                }
            }
        }

        let content = if content_parts.is_empty() {
            "[empty message]".to_string()
        } else {
            content_parts.join("\n")
        };

        debug!(
            sender = %sender_id,
            channel = %channel_id,
            content_len = content.len(),
            "discord inbound message"
        );

        // Start typing indicator
        self.start_typing(&channel_id).await;

        // Build inbound message
        let mut inbound = InboundMessage::new("discord", &sender_id, &channel_id, &content);
        for path in &media_paths {
            inbound.media.push(oxibot_core::types::MediaAttachment {
                path: path.clone(),
                mime_type: "application/octet-stream".into(),
                filename: None,
                size: None,
            });
        }
        inbound
            .metadata
            .insert("username".into(), username);
        if let Some(msg_id) = data["id"].as_str() {
            inbound
                .metadata
                .insert("message_id".into(), msg_id.to_string());
        }
        if let Some(guild_id) = data["guild_id"].as_str() {
            inbound
                .metadata
                .insert("guild_id".into(), guild_id.to_string());
        }
        // Reply reference
        if let Some(ref_msg) = data["referenced_message"]["id"].as_str() {
            inbound
                .metadata
                .insert("reply_to".into(), ref_msg.to_string());
        }

        if let Err(e) = self.bus.publish_inbound(inbound).await {
            error!(error = %e, "failed to publish discord message to bus");
        }
    }

    /// Download an attachment to local media directory.
    async fn download_attachment(
        &self,
        url: &str,
        att_id: &str,
        filename: &str,
    ) -> anyhow::Result<String> {
        let media_dir = oxibot_core::utils::get_data_path().join("media");
        std::fs::create_dir_all(&media_dir)?;

        // Sanitize filename
        let safe_name: String = filename
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let local_path = media_dir.join(format!("{att_id}_{safe_name}"));

        let resp = self
            .http
            .get(url)
            .send()
            .await?;
        let bytes = resp.bytes().await?;

        tokio::fs::write(&local_path, &bytes).await?;
        info!(path = %local_path.display(), "downloaded discord attachment");
        Ok(local_path.display().to_string())
    }

    /// Start typing indicator for a channel.
    async fn start_typing(&self, channel_id: &str) {
        // Cancel existing typing task for this channel
        self.stop_typing(channel_id).await;

        let url = format!("{DISCORD_API_BASE}/channels/{channel_id}/typing");
        let token = self.token.clone();
        let shutdown = self.shutdown.clone();
        let channel_id_owned = channel_id.to_string();

        let http = self.http.clone();
        let handle = tokio::spawn(async move {
            loop {
                let _ = http
                    .post(&url)
                    .header("Authorization", format!("Bot {token}"))
                    .send()
                    .await;

                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(TYPING_INTERVAL_SECS)) => {}
                    _ = shutdown.notified() => break,
                }
            }
            debug!(channel = %channel_id_owned, "typing indicator stopped");
        });

        self.typing_tasks.write().await.insert(channel_id.to_string(), handle);
    }

    /// Stop typing indicator for a channel.
    async fn stop_typing(&self, channel_id: &str) {
        let mut tasks = self.typing_tasks.write().await;
        if let Some(handle) = tasks.remove(channel_id) {
            handle.abort();
        }
    }

    /// Stop all typing indicators.
    async fn stop_all_typing(&self) {
        let mut tasks = self.typing_tasks.write().await;
        for (_, handle) in tasks.drain() {
            handle.abort();
        }
    }

    /// Send a message via the REST API with retry on rate-limit.
    async fn send_rest(
        &self,
        channel_id: &str,
        content: &str,
        reply_to: Option<&str>,
    ) -> anyhow::Result<()> {
        let url = format!("{DISCORD_API_BASE}/channels/{channel_id}/messages");

        let mut body = json!({ "content": content });
        if let Some(ref_id) = reply_to {
            body["message_reference"] = json!({ "message_id": ref_id });
            body["allowed_mentions"] = json!({ "replied_user": false });
        }

        let mut attempts = 0u32;
        loop {
            attempts += 1;
            let resp = self
                .http
                .post(&url)
                .header("Authorization", format!("Bot {}", self.token))
                .json(&body)
                .send()
                .await?;

            let status = resp.status();

            if status.is_success() {
                return Ok(());
            }

            if status.as_u16() == 429 {
                // Rate limited
                let body_text = resp.text().await.unwrap_or_default();
                let retry_after: f64 = serde_json::from_str::<Value>(&body_text)
                    .ok()
                    .and_then(|v| v["retry_after"].as_f64())
                    .unwrap_or(1.0);
                warn!(
                    retry_after_s = retry_after,
                    attempt = attempts,
                    "discord rate limited"
                );
                tokio::time::sleep(Duration::from_secs_f64(retry_after)).await;
                continue;
            }

            if attempts >= 3 {
                let err_text = resp.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!(
                    "discord send failed after 3 attempts (HTTP {}): {}",
                    status,
                    err_text
                ));
            }

            warn!(
                status = %status,
                attempt = attempts,
                "discord send error, retrying in 1s"
            );
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

/// Split a message into chunks respecting Discord's 2000 char limit.
/// Tries to split at newline boundaries.
pub fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Find last newline within max_len
        let split_at = remaining[..max_len]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(max_len);

        chunks.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
    }

    chunks
}

/// Simple jitter: a random fraction between 0.0 and 1.0 for heartbeat.
fn rand_jitter() -> f64 {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos as f64) / 1_000_000_000.0
}

#[async_trait]
impl Channel for DiscordChannel {
    fn name(&self) -> &str {
        "discord"
    }

    async fn start(&self) -> anyhow::Result<()> {
        if self.token.is_empty() {
            return Err(anyhow::anyhow!("discord token is empty"));
        }

        info!("starting discord channel (gateway v10)");
        self.run_gateway().await
    }

    async fn stop(&self) -> anyhow::Result<()> {
        info!("stopping discord channel");
        self.shutdown.notify_waiters();
        self.stop_all_typing().await;
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> anyhow::Result<()> {
        let reply_to = msg.metadata.get("reply_to").map(|s| s.as_str());

        // Split long messages
        let chunks = split_message(&msg.content, DISCORD_MAX_LEN);

        for (i, chunk) in chunks.iter().enumerate() {
            // Only include reply reference on the first chunk
            let ref_id = if i == 0 { reply_to } else { None };
            self.send_rest(&msg.chat_id, chunk, ref_id).await?;
        }

        // Stop typing after sending
        self.stop_typing(&msg.chat_id).await;

        debug!(chat_id = %msg.chat_id, chunks = chunks.len(), "discord message sent");
        Ok(())
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_channel() -> DiscordChannel {
        let bus = Arc::new(MessageBus::new(32));
        DiscordChannel::new("test_token".into(), bus, vec![])
    }

    fn create_restricted_channel() -> DiscordChannel {
        let bus = Arc::new(MessageBus::new(32));
        DiscordChannel::new(
            "test_token".into(),
            bus,
            vec!["123456789".into(), "987654321".into()],
        )
    }

    #[test]
    fn test_channel_name() {
        let ch = create_test_channel();
        assert_eq!(ch.name(), "discord");
    }

    #[test]
    fn test_is_allowed_empty_list() {
        let ch = create_test_channel();
        assert!(ch.is_allowed("anyone"));
        assert!(ch.is_allowed("123|user"));
    }

    #[test]
    fn test_is_allowed_by_id() {
        let ch = create_restricted_channel();
        assert!(ch.is_allowed("123456789"));
    }

    #[test]
    fn test_is_allowed_denied() {
        let ch = create_restricted_channel();
        assert!(!ch.is_allowed("000000000"));
    }

    #[test]
    fn test_is_allowed_pipe_split() {
        let ch = create_restricted_channel();
        assert!(ch.is_allowed("123456789|someuser"));
        assert!(ch.is_allowed("000|987654321"));
        assert!(!ch.is_allowed("000|stranger"));
    }

    #[test]
    fn test_split_message_short() {
        let chunks = split_message("hello", 2000);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn test_split_message_exact() {
        let msg = "a".repeat(2000);
        let chunks = split_message(&msg, 2000);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_split_message_long() {
        let line = "hello world\n";
        let msg = line.repeat(200); // 2400 chars
        let chunks = split_message(&msg, 2000);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].len() <= 2000);
        assert!(chunks[1].len() <= 2000);
    }

    #[test]
    fn test_split_message_no_newline() {
        let msg = "x".repeat(2500);
        let chunks = split_message(&msg, 2000);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 2000);
        assert_eq!(chunks[1].len(), 500);
    }

    #[test]
    fn test_split_message_at_newline() {
        let mut msg = "x".repeat(1990);
        msg.push('\n');
        msg.push_str(&"y".repeat(500));
        let chunks = split_message(&msg, 2000);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].ends_with('\n'));
    }

    #[test]
    fn test_rand_jitter_range() {
        let j = rand_jitter();
        assert!((0.0..1.0).contains(&j));
    }

    #[test]
    fn test_constants() {
        assert_eq!(DISCORD_MAX_LEN, 2000);
        assert_eq!(MAX_ATTACHMENT_BYTES, 20 * 1024 * 1024);
        assert_eq!(DEFAULT_INTENTS, 37377);
    }

    #[tokio::test]
    async fn test_handle_message_create_ignores_bots() {
        let ch = create_test_channel();
        let data = json!({
            "author": { "id": "123", "username": "bot", "bot": true },
            "channel_id": "456",
            "content": "bot says hi"
        });
        // Should not panic or publish anything
        ch.handle_message_create(&data).await;
        // No message should be on the bus (bus is empty)
    }

    #[tokio::test]
    async fn test_handle_message_create_unauthorized() {
        let ch = create_restricted_channel();
        let data = json!({
            "author": { "id": "000000000", "username": "stranger" },
            "channel_id": "456",
            "content": "hello"
        });
        ch.handle_message_create(&data).await;
        // Should be silently ignored
    }

    #[tokio::test]
    async fn test_handle_message_create_publishes() {
        let bus = Arc::new(MessageBus::new(32));
        let ch = DiscordChannel::new("test_token".into(), bus.clone(), vec![]);

        let data = json!({
            "id": "msg1",
            "author": { "id": "user1", "username": "testuser" },
            "channel_id": "ch1",
            "content": "hello oxibot",
            "guild_id": "guild1"
        });

        ch.handle_message_create(&data).await;

        // Check message was published to bus
        let msg = bus.consume_inbound().await;
        assert!(msg.is_some());
        let msg = msg.unwrap();
        assert_eq!(msg.channel, "discord");
        assert_eq!(msg.sender_id, "user1");
        assert_eq!(msg.chat_id, "ch1");
        assert_eq!(msg.content, "hello oxibot");
        assert_eq!(msg.metadata.get("username").unwrap(), "testuser");
        assert_eq!(msg.metadata.get("message_id").unwrap(), "msg1");
        assert_eq!(msg.metadata.get("guild_id").unwrap(), "guild1");
    }

    #[tokio::test]
    async fn test_handle_message_create_empty() {
        let bus = Arc::new(MessageBus::new(32));
        let ch = DiscordChannel::new("test_token".into(), bus.clone(), vec![]);

        let data = json!({
            "author": { "id": "user1", "username": "testuser" },
            "channel_id": "ch1",
            "content": ""
        });

        ch.handle_message_create(&data).await;

        let msg = bus.consume_inbound().await;
        assert!(msg.is_some());
        assert_eq!(msg.unwrap().content, "[empty message]");
    }

    #[tokio::test]
    async fn test_handle_message_create_with_reply() {
        let bus = Arc::new(MessageBus::new(32));
        let ch = DiscordChannel::new("test_token".into(), bus.clone(), vec![]);

        let data = json!({
            "author": { "id": "user1", "username": "testuser" },
            "channel_id": "ch1",
            "content": "replying",
            "referenced_message": { "id": "original_msg_123" }
        });

        ch.handle_message_create(&data).await;

        let msg = bus.consume_inbound().await.unwrap();
        assert_eq!(msg.metadata.get("reply_to").unwrap(), "original_msg_123");
    }

    #[tokio::test]
    async fn test_typing_start_stop() {
        let ch = create_test_channel();
        ch.start_typing("channel_1").await;
        {
            let tasks = ch.typing_tasks.read().await;
            assert!(tasks.contains_key("channel_1"));
        }
        ch.stop_typing("channel_1").await;
        {
            let tasks = ch.typing_tasks.read().await;
            assert!(!tasks.contains_key("channel_1"));
        }
    }

    #[tokio::test]
    async fn test_stop_all_typing() {
        let ch = create_test_channel();
        ch.start_typing("ch1").await;
        ch.start_typing("ch2").await;
        ch.stop_all_typing().await;
        let tasks = ch.typing_tasks.read().await;
        assert!(tasks.is_empty());
    }
}
