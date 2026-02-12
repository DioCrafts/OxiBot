//! Bus event types — messages flowing between channels and the agent loop.
//!
//! Replaces nanobot's `bus/events.py` `InboundMessage` / `OutboundMessage` dataclasses.

use crate::types::MediaAttachment;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// An inbound message from a channel to the agent.
#[derive(Clone, Debug)]
pub struct InboundMessage {
    /// Channel name (e.g. "telegram", "discord", "cli").
    pub channel: String,
    /// Sender identifier within the channel.
    pub sender_id: String,
    /// Chat/conversation identifier.
    pub chat_id: String,
    /// Text content of the message.
    pub content: String,
    /// When the message was received.
    pub timestamp: DateTime<Utc>,
    /// Attached media (photos, voice, documents).
    pub media: Vec<MediaAttachment>,
    /// Channel-specific metadata (e.g. message_id, username).
    pub metadata: HashMap<String, String>,
}

impl InboundMessage {
    /// Create a new inbound message with minimal required fields.
    pub fn new(
        channel: impl Into<String>,
        sender_id: impl Into<String>,
        chat_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        InboundMessage {
            channel: channel.into(),
            sender_id: sender_id.into(),
            chat_id: chat_id.into(),
            content: content.into(),
            timestamp: Utc::now(),
            media: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Session key combining channel and chat_id (e.g. "telegram:123456").
    ///
    /// Used as the key for session persistence and history lookup.
    pub fn session_key(&self) -> String {
        format!("{}:{}", self.channel, self.chat_id)
    }
}

/// An outbound message from the agent to a channel.
#[derive(Clone, Debug)]
pub struct OutboundMessage {
    /// Target channel name.
    pub channel: String,
    /// Target chat/conversation identifier.
    pub chat_id: String,
    /// Text content to send.
    pub content: String,
    /// Optional message ID to reply to.
    pub reply_to: Option<String>,
    /// Attached media to send.
    pub media: Vec<MediaAttachment>,
    /// Channel-specific metadata.
    pub metadata: HashMap<String, String>,
}

impl OutboundMessage {
    /// Create a new outbound message.
    pub fn new(
        channel: impl Into<String>,
        chat_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        OutboundMessage {
            channel: channel.into(),
            chat_id: chat_id.into(),
            content: content.into(),
            reply_to: None,
            media: Vec::new(),
            metadata: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inbound_message_creation() {
        let msg = InboundMessage::new("telegram", "user_42", "chat_99", "Hello Oxibot!");

        assert_eq!(msg.channel, "telegram");
        assert_eq!(msg.sender_id, "user_42");
        assert_eq!(msg.chat_id, "chat_99");
        assert_eq!(msg.content, "Hello Oxibot!");
        assert!(msg.media.is_empty());
        assert!(msg.metadata.is_empty());
    }

    #[test]
    fn test_session_key() {
        let msg = InboundMessage::new("discord", "user_1", "channel_abc", "test");
        assert_eq!(msg.session_key(), "discord:channel_abc");
    }

    #[test]
    fn test_session_key_format_cli() {
        let msg = InboundMessage::new("cli", "local", "default", "hello");
        assert_eq!(msg.session_key(), "cli:default");
    }

    #[test]
    fn test_outbound_message_creation() {
        let msg = OutboundMessage::new("telegram", "chat_99", "Here's your answer!");

        assert_eq!(msg.channel, "telegram");
        assert_eq!(msg.chat_id, "chat_99");
        assert_eq!(msg.content, "Here's your answer!");
        assert!(msg.reply_to.is_none());
        assert!(msg.media.is_empty());
    }

    #[test]
    fn test_inbound_with_metadata() {
        let mut msg = InboundMessage::new("telegram", "user_1", "chat_1", "hi");
        msg.metadata
            .insert("message_id".to_string(), "12345".to_string());
        msg.metadata
            .insert("username".to_string(), "torrefacto".to_string());

        assert_eq!(msg.metadata.get("username").unwrap(), "torrefacto");
        assert_eq!(msg.metadata.get("message_id").unwrap(), "12345");
    }

    #[test]
    fn test_inbound_with_media() {
        let mut msg = InboundMessage::new("telegram", "user_1", "chat_1", "check this");
        msg.media.push(MediaAttachment {
            mime_type: "image/jpeg".to_string(),
            path: "/tmp/photo.jpg".to_string(),
            filename: Some("photo.jpg".to_string()),
            size: Some(102400),
        });

        assert_eq!(msg.media.len(), 1);
        assert_eq!(msg.media[0].mime_type, "image/jpeg");
        assert_eq!(msg.media[0].size, Some(102400));
    }
}
