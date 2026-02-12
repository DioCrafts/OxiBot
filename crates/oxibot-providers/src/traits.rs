//! LLM Provider trait — the core abstraction replacing LiteLLM.
//!
//! Every LLM backend (OpenAI, Anthropic, DeepSeek, Groq, …) implements this trait.
//! The `HttpProvider` in `http_provider.rs` covers all OpenAI-compatible APIs.

use async_trait::async_trait;
use oxibot_core::types::{LlmResponse, Message, ToolDefinition};

/// Configuration passed to each LLM call.
///
/// Replaces nanobot's `AgentConfig` subset used by providers.
#[derive(Clone, Debug)]
pub struct LlmRequestConfig {
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// Sampling temperature (0.0 – 2.0).
    pub temperature: f64,
}

impl Default for LlmRequestConfig {
    fn default() -> Self {
        Self {
            max_tokens: 4096,
            temperature: 0.7,
        }
    }
}

/// Trait that all LLM providers must implement.
///
/// Replaces nanobot's `LLMProvider` ABC.
/// The main implementation is `HttpProvider` which handles any OpenAI-compatible API.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Send a chat completion request.
    ///
    /// # Arguments
    /// * `messages` — Conversation history in OpenAI format.
    /// * `tools`    — Optional list of tool definitions the LLM can call.
    /// * `model`    — Model identifier (e.g. `"claude-sonnet-4-20250514"`, `"gpt-4o"`).
    /// * `config`   — Temperature, max_tokens, etc.
    ///
    /// # Returns
    /// An `LlmResponse` with content and/or tool calls.
    /// On API errors, returns `LlmResponse::error(...)` instead of propagating.
    async fn chat(
        &self,
        messages: &[Message],
        tools: Option<&[ToolDefinition]>,
        model: &str,
        config: &LlmRequestConfig,
    ) -> LlmResponse;

    /// The default model for this provider instance.
    fn default_model(&self) -> &str;

    /// Display name for logging.
    fn display_name(&self) -> &str;
}
