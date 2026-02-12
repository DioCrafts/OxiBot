//! Message tool — lets the agent proactively send messages to channels.
//!
//! Port of nanobot's `agent/tools/message.py`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tracing::debug;

use oxibot_core::bus::types::OutboundMessage;

use super::base::{optional_string, require_string, Tool};

/// Callback type for sending outbound messages.
pub type SendCallback = Arc<dyn Fn(OutboundMessage) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> + Send + Sync>;

// ─────────────────────────────────────────────
// MessageTool
// ─────────────────────────────────────────────

/// Allows the agent to send messages to channels.
///
/// The agent loop calls `set_context` before each interaction to set
/// the default channel/chat_id for the current conversation.
pub struct MessageTool {
    send_callback: Option<SendCallback>,
    /// Default channel / chat_id set per-interaction by the agent loop.
    context: Mutex<(String, String)>,
}

impl MessageTool {
    /// Create a new message tool with a send callback.
    pub fn new(send_callback: Option<SendCallback>) -> Self {
        Self {
            send_callback,
            context: Mutex::new(("cli".into(), "direct".into())),
        }
    }

    /// Set the current context (called by the agent loop per-message).
    pub async fn set_context(&self, channel: &str, chat_id: &str) {
        let mut ctx = self.context.lock().await;
        *ctx = (channel.to_string(), chat_id.to_string());
    }
}

#[async_trait]
impl Tool for MessageTool {
    fn name(&self) -> &str {
        "message"
    }

    fn description(&self) -> &str {
        "Send a message to a channel. By default sends to the current conversation. \
         Can optionally specify a different channel and chat_id to send to."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The message content to send"
                },
                "channel": {
                    "type": "string",
                    "description": "Target channel (optional, defaults to current)"
                },
                "chat_id": {
                    "type": "string",
                    "description": "Target chat ID (optional, defaults to current)"
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, params: HashMap<String, Value>) -> anyhow::Result<String> {
        let content = require_string(&params, "content")?;
        let param_channel = optional_string(&params, "channel");
        let param_chat_id = optional_string(&params, "chat_id");

        let ctx = self.context.lock().await;
        let channel = param_channel.unwrap_or_else(|| ctx.0.clone());
        let chat_id = param_chat_id.unwrap_or_else(|| ctx.1.clone());
        drop(ctx);

        debug!(channel = %channel, chat_id = %chat_id, "sending message via tool");

        let msg = OutboundMessage::new(&channel, &chat_id, &content);

        if let Some(cb) = &self.send_callback {
            cb(msg).await.map_err(|e| anyhow::anyhow!("Failed to send message: {e}"))?;
        } else {
            // No callback — just a no-op (CLI mode / tests)
            debug!("No send callback configured; message discarded");
        }

        Ok(format!("Message sent to {channel}:{chat_id}"))
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_definition() {
        let tool = MessageTool::new(None);
        let def = tool.to_definition();
        assert_eq!(def.function.name, "message");
        assert_eq!(def.tool_type, "function");
    }

    #[tokio::test]
    async fn test_set_context() {
        let tool = MessageTool::new(None);
        tool.set_context("telegram", "chat_42").await;
        let ctx = tool.context.lock().await;
        assert_eq!(ctx.0, "telegram");
        assert_eq!(ctx.1, "chat_42");
    }

    #[tokio::test]
    async fn test_execute_no_callback() {
        let tool = MessageTool::new(None);
        tool.set_context("discord", "guild_1").await;
        let mut params = HashMap::new();
        params.insert("content".into(), json!("Hello from agent"));
        let result = tool.execute(params).await.unwrap();
        assert_eq!(result, "Message sent to discord:guild_1");
    }

    #[tokio::test]
    async fn test_execute_with_channel_override() {
        let tool = MessageTool::new(None);
        tool.set_context("cli", "direct").await;
        let mut params = HashMap::new();
        params.insert("content".into(), json!("Hello"));
        params.insert("channel".into(), json!("slack"));
        params.insert("chat_id".into(), json!("C12345"));
        let result = tool.execute(params).await.unwrap();
        assert_eq!(result, "Message sent to slack:C12345");
    }

    #[tokio::test]
    async fn test_execute_missing_content() {
        let tool = MessageTool::new(None);
        let params = HashMap::new();
        let result = tool.execute(params).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_with_callback() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();

        let callback: SendCallback = Arc::new(move |_msg| {
            let called = called_clone.clone();
            Box::pin(async move {
                called.store(true, Ordering::SeqCst);
                Ok(())
            })
        });

        let tool = MessageTool::new(Some(callback));
        let mut params = HashMap::new();
        params.insert("content".into(), json!("ping"));
        let result = tool.execute(params).await.unwrap();
        assert!(result.contains("Message sent"));
        assert!(called.load(Ordering::SeqCst));
    }
}
