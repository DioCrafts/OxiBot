//! Core types for Oxibot — typed replacements for nanobot's `dict[str, Any]` messages.
//!
//! These types model the OpenAI chat completions API format used by all LLM providers.
//! In nanobot (Python), messages are untyped `list[dict[str, Any]]`. Here, we use
//! Rust enums to catch format errors at compile time instead of runtime.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─────────────────────────────────────────────
// Messages (OpenAI chat completions format)
// ─────────────────────────────────────────────

/// A chat message in the OpenAI format.
///
/// Replaces nanobot's `list[dict[str, Any]]` with a typed enum.
/// Each variant maps to a `role` field value.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "role")]
pub enum Message {
    #[serde(rename = "system")]
    System { content: String },

    #[serde(rename = "user")]
    User { content: MessageContent },

    #[serde(rename = "assistant")]
    Assistant {
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<ToolCall>>,
        /// Reasoning/thinking content from models like DeepSeek-R1 or Kimi.
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
    },

    #[serde(rename = "tool")]
    Tool {
        content: String,
        tool_call_id: String,
    },
}

impl Message {
    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Message::System {
            content: content.into(),
        }
    }

    /// Create a user message with text content.
    pub fn user(content: impl Into<String>) -> Self {
        Message::User {
            content: MessageContent::Text(content.into()),
        }
    }

    /// Create a user message with multipart content (text + images).
    pub fn user_parts(parts: Vec<ContentPart>) -> Self {
        Message::User {
            content: MessageContent::Parts(parts),
        }
    }

    /// Create an assistant message with text content.
    pub fn assistant(content: impl Into<String>) -> Self {
        Message::Assistant {
            content: Some(content.into()),
            tool_calls: None,
            reasoning_content: None,
        }
    }

    /// Create an assistant message with tool calls (no text content).
    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Message::Assistant {
            content: None,
            tool_calls: Some(tool_calls),
            reasoning_content: None,
        }
    }

    /// Create a tool result message.
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Message::Tool {
            content: content.into(),
            tool_call_id: tool_call_id.into(),
        }
    }
}

// ─────────────────────────────────────────────
// Message Content (text or multipart/vision)
// ─────────────────────────────────────────────

/// User message content — either plain text or multipart (for vision/images).
///
/// When serialized: text becomes a plain string, parts become an array of objects.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum MessageContent {
    /// Simple text content (most common case).
    Text(String),
    /// Multipart content with text and/or images (for vision models).
    Parts(Vec<ContentPart>),
}

/// A single part of a multipart message.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ContentPart {
    /// Text part.
    #[serde(rename = "text")]
    Text { text: String },
    /// Image URL part (can be a URL or base64 data URI).
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrl },
}

/// Image URL payload.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ImageUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

// ─────────────────────────────────────────────
// Tool Calls (function calling)
// ─────────────────────────────────────────────

/// A tool call from the assistant, requesting execution of a function.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    /// Unique ID for this tool call (used to match results).
    pub id: String,
    /// Always "function" in current OpenAI API.
    #[serde(rename = "type")]
    pub call_type: String,
    /// The function to call.
    pub function: FunctionCall,
}

impl ToolCall {
    /// Create a new tool call.
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: impl Into<String>) -> Self {
        ToolCall {
            id: id.into(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: name.into(),
                arguments: arguments.into(),
            },
        }
    }
}

/// The function name and arguments within a tool call.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FunctionCall {
    /// Name of the function/tool to call.
    pub name: String,
    /// JSON-encoded arguments string.
    pub arguments: String,
}

// ─────────────────────────────────────────────
// Tool Definitions (for LLM requests)
// ─────────────────────────────────────────────

/// Definition of a tool, sent to the LLM so it knows what tools are available.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolDefinition {
    /// Always "function".
    #[serde(rename = "type")]
    pub tool_type: String,
    /// The function schema.
    pub function: FunctionDefinition,
}

/// Schema of a function tool.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolDefinition {
    /// Create a new tool definition.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        ToolDefinition {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

// ─────────────────────────────────────────────
// LLM Response
// ─────────────────────────────────────────────

/// Response from an LLM provider after a chat completion call.
///
/// Replaces nanobot's `LLMResponse` dataclass.
#[derive(Clone, Debug, Default)]
pub struct LlmResponse {
    /// Text content from the assistant (None if only tool calls).
    pub content: Option<String>,
    /// Tool calls requested by the assistant.
    pub tool_calls: Vec<ToolCall>,
    /// Why the model stopped generating.
    pub finish_reason: Option<String>,
    /// Token usage statistics.
    pub usage: Option<UsageInfo>,
    /// Reasoning/thinking content (DeepSeek-R1, Kimi).
    pub reasoning_content: Option<String>,
}

impl LlmResponse {
    /// Create an error response (error message as content, no tool calls).
    pub fn error(msg: impl Into<String>) -> Self {
        LlmResponse {
            content: Some(msg.into()),
            ..Default::default()
        }
    }

    /// Whether the response contains tool calls.
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

/// Token usage statistics from the LLM.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UsageInfo {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

// ─────────────────────────────────────────────
// Media attachments
// ─────────────────────────────────────────────

/// A media attachment (photo, voice, document) from a channel message.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MediaAttachment {
    /// MIME type (e.g. "image/jpeg", "audio/ogg").
    pub mime_type: String,
    /// Local file path or URL to the media.
    pub path: String,
    /// Optional filename.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// File size in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

// ─────────────────────────────────────────────
// Provider-related types
// ─────────────────────────────────────────────

/// Raw chat completion response from an OpenAI-compatible API.
/// Used internally for deserialization.
#[derive(Debug, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: Option<String>,
    pub choices: Vec<ChatChoice>,
    pub usage: Option<UsageInfo>,
}

/// A single choice in a chat completion response.
#[derive(Debug, Deserialize)]
pub struct ChatChoice {
    pub message: AssistantMessage,
    pub finish_reason: Option<String>,
}

/// The assistant message within a chat completion choice.
#[derive(Debug, Deserialize)]
pub struct AssistantMessage {
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
}

impl From<ChatCompletionResponse> for LlmResponse {
    fn from(resp: ChatCompletionResponse) -> Self {
        let choice = resp.choices.into_iter().next();
        match choice {
            Some(c) => LlmResponse {
                content: c.message.content,
                tool_calls: c.message.tool_calls.unwrap_or_default(),
                finish_reason: c.finish_reason,
                usage: resp.usage,
                reasoning_content: c.message.reasoning_content,
            },
            None => LlmResponse::error("No choices in response"),
        }
    }
}

// ─────────────────────────────────────────────
// Chat completion request (for building API calls)
// ─────────────────────────────────────────────

/// Request body for an OpenAI-compatible chat completion API.
#[derive(Debug, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
}

// ─────────────────────────────────────────────
// Session types
// ─────────────────────────────────────────────

/// A conversation session with message history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub key: String,
    pub messages: Vec<Message>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl Session {
    /// Create a new empty session.
    pub fn new(key: impl Into<String>) -> Self {
        let now = chrono::Utc::now();
        Session {
            key: key.into(),
            messages: Vec::new(),
            created_at: now,
            updated_at: now,
            metadata: HashMap::new(),
        }
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Message serialization ──

    #[test]
    fn test_system_message_serialization() {
        let msg = Message::system("You are a helpful assistant.");
        let json = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["role"], "system");
        assert_eq!(json["content"], "You are a helpful assistant.");
    }

    #[test]
    fn test_user_text_message_serialization() {
        let msg = Message::user("Hello, world!");
        let json = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "Hello, world!");
    }

    #[test]
    fn test_user_multipart_message_serialization() {
        let msg = Message::user_parts(vec![
            ContentPart::Text {
                text: "What's in this image?".to_string(),
            },
            ContentPart::ImageUrl {
                image_url: ImageUrl {
                    url: "data:image/png;base64,abc123".to_string(),
                    detail: Some("high".to_string()),
                },
            },
        ]);
        let json = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["role"], "user");
        let content = json["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "What's in this image?");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,abc123");
        assert_eq!(content[1]["image_url"]["detail"], "high");
    }

    #[test]
    fn test_assistant_text_message_serialization() {
        let msg = Message::assistant("The answer is 42.");
        let json = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["role"], "assistant");
        assert_eq!(json["content"], "The answer is 42.");
        // tool_calls and reasoning_content should be absent (not null)
        assert!(json.get("tool_calls").is_none());
        assert!(json.get("reasoning_content").is_none());
    }

    #[test]
    fn test_assistant_tool_calls_serialization() {
        let tool_calls = vec![ToolCall::new(
            "call_123",
            "web_search",
            r#"{"query": "Rust programming"}"#,
        )];
        let msg = Message::assistant_tool_calls(tool_calls);
        let json = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["role"], "assistant");
        assert!(json.get("content").is_none());

        let calls = json["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "call_123");
        assert_eq!(calls[0]["type"], "function");
        assert_eq!(calls[0]["function"]["name"], "web_search");
        assert_eq!(
            calls[0]["function"]["arguments"],
            r#"{"query": "Rust programming"}"#
        );
    }

    #[test]
    fn test_tool_result_serialization() {
        let msg = Message::tool_result("call_123", "Search results: Rust is great!");
        let json = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["role"], "tool");
        assert_eq!(json["content"], "Search results: Rust is great!");
        assert_eq!(json["tool_call_id"], "call_123");
    }

    // ── Message deserialization (from API responses) ──

    #[test]
    fn test_system_message_deserialization() {
        let json = json!({"role": "system", "content": "Be helpful."});
        let msg: Message = serde_json::from_value(json).unwrap();

        match msg {
            Message::System { content } => assert_eq!(content, "Be helpful."),
            _ => panic!("Expected System message"),
        }
    }

    #[test]
    fn test_user_text_deserialization() {
        let json = json!({"role": "user", "content": "Hi there"});
        let msg: Message = serde_json::from_value(json).unwrap();

        match msg {
            Message::User {
                content: MessageContent::Text(text),
            } => assert_eq!(text, "Hi there"),
            _ => panic!("Expected User text message"),
        }
    }

    #[test]
    fn test_assistant_with_tool_calls_deserialization() {
        let json = json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "call_abc",
                "type": "function",
                "function": {
                    "name": "read_file",
                    "arguments": "{\"path\": \"/tmp/test.txt\"}"
                }
            }]
        });
        let msg: Message = serde_json::from_value(json).unwrap();

        match msg {
            Message::Assistant {
                content,
                tool_calls,
                ..
            } => {
                assert!(content.is_none());
                let calls = tool_calls.unwrap();
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].function.name, "read_file");
            }
            _ => panic!("Expected Assistant message"),
        }
    }

    // ── Round-trip: serialize then deserialize ──

    #[test]
    fn test_message_round_trip() {
        let messages = vec![
            Message::system("You are Oxibot."),
            Message::user("What is 2+2?"),
            Message::assistant("The answer is 4."),
            Message::tool_result("call_1", "done"),
        ];

        let json_str = serde_json::to_string(&messages).unwrap();
        let deserialized: Vec<Message> = serde_json::from_str(&json_str).unwrap();

        assert_eq!(messages, deserialized);
    }

    // ── ToolDefinition ──

    #[test]
    fn test_tool_definition_serialization() {
        let tool_def = ToolDefinition::new(
            "read_file",
            "Read the contents of a file",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to read"
                    }
                },
                "required": ["path"]
            }),
        );
        let json = serde_json::to_value(&tool_def).unwrap();

        assert_eq!(json["type"], "function");
        assert_eq!(json["function"]["name"], "read_file");
        assert_eq!(json["function"]["description"], "Read the contents of a file");
        assert_eq!(json["function"]["parameters"]["type"], "object");
        assert!(json["function"]["parameters"]["properties"]["path"].is_object());
    }

    // ── ChatCompletionResponse → LlmResponse ──

    #[test]
    fn test_chat_completion_response_parsing() {
        let api_json = json!({
            "id": "chatcmpl-abc123",
            "choices": [{
                "message": {
                    "content": "Hello! How can I help?",
                    "tool_calls": null
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 8,
                "total_tokens": 18
            }
        });

        let resp: ChatCompletionResponse = serde_json::from_value(api_json).unwrap();
        let llm_resp: LlmResponse = resp.into();

        assert_eq!(llm_resp.content.as_deref(), Some("Hello! How can I help?"));
        assert!(!llm_resp.has_tool_calls());
        assert_eq!(llm_resp.finish_reason.as_deref(), Some("stop"));
        assert_eq!(llm_resp.usage.as_ref().unwrap().total_tokens, 18);
    }

    #[test]
    fn test_chat_completion_with_tool_calls_parsing() {
        let api_json = json!({
            "id": "chatcmpl-xyz",
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_42",
                        "type": "function",
                        "function": {
                            "name": "exec",
                            "arguments": "{\"command\": \"ls -la\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 50,
                "completion_tokens": 20,
                "total_tokens": 70
            }
        });

        let resp: ChatCompletionResponse = serde_json::from_value(api_json).unwrap();
        let llm_resp: LlmResponse = resp.into();

        assert!(llm_resp.content.is_none());
        assert!(llm_resp.has_tool_calls());
        assert_eq!(llm_resp.tool_calls.len(), 1);
        assert_eq!(llm_resp.tool_calls[0].function.name, "exec");
        assert_eq!(llm_resp.finish_reason.as_deref(), Some("tool_calls"));
    }

    #[test]
    fn test_chat_completion_empty_choices() {
        let api_json = json!({
            "id": "chatcmpl-empty",
            "choices": [],
            "usage": null
        });

        let resp: ChatCompletionResponse = serde_json::from_value(api_json).unwrap();
        let llm_resp: LlmResponse = resp.into();

        assert_eq!(
            llm_resp.content.as_deref(),
            Some("No choices in response")
        );
    }

    // ── ChatCompletionRequest serialization ──

    #[test]
    fn test_chat_request_serialization() {
        let request = ChatCompletionRequest {
            model: "anthropic/claude-opus-4-5".to_string(),
            messages: vec![
                Message::system("You are Oxibot."),
                Message::user("Hello"),
            ],
            tools: None,
            tool_choice: None,
            max_tokens: Some(4096),
            temperature: Some(0.7),
        };

        let json = serde_json::to_value(&request).unwrap();

        assert_eq!(json["model"], "anthropic/claude-opus-4-5");
        assert_eq!(json["messages"].as_array().unwrap().len(), 2);
        assert_eq!(json["max_tokens"], 4096);
        assert_eq!(json["temperature"], 0.7);
        // tools and tool_choice should not appear when None
        assert!(json.get("tools").is_none());
        assert!(json.get("tool_choice").is_none());
    }

    #[test]
    fn test_chat_request_with_tools() {
        let tool_def = ToolDefinition::new(
            "web_search",
            "Search the web",
            json!({"type": "object", "properties": {"query": {"type": "string"}}}),
        );

        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![Message::user("Search for Rust")],
            tools: Some(vec![tool_def]),
            tool_choice: Some("auto".to_string()),
            max_tokens: None,
            temperature: None,
        };

        let json = serde_json::to_value(&request).unwrap();

        assert!(json.get("tools").is_some());
        assert_eq!(json["tool_choice"], "auto");
        // max_tokens and temperature should not appear when None
        assert!(json.get("max_tokens").is_none());
        assert!(json.get("temperature").is_none());
    }

    // ── LlmResponse helpers ──

    #[test]
    fn test_llm_response_error() {
        let resp = LlmResponse::error("Something went wrong");

        assert_eq!(resp.content.as_deref(), Some("Something went wrong"));
        assert!(!resp.has_tool_calls());
    }

    // ── Session ──

    #[test]
    fn test_session_creation() {
        let session = Session::new("telegram:123456");

        assert_eq!(session.key, "telegram:123456");
        assert!(session.messages.is_empty());
        assert!(session.metadata.is_empty());
    }

    #[test]
    fn test_session_serialization_round_trip() {
        let mut session = Session::new("discord:789");
        session.messages.push(Message::user("Hello"));
        session.messages.push(Message::assistant("Hi there!"));
        session
            .metadata
            .insert("channel".to_string(), "discord".to_string());

        let json_str = serde_json::to_string(&session).unwrap();
        let deserialized: Session = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.key, "discord:789");
        assert_eq!(deserialized.messages.len(), 2);
        assert_eq!(
            deserialized.metadata.get("channel").map(|s| s.as_str()),
            Some("discord")
        );
    }
}
