//! Generic HTTP-based LLM provider for OpenAI-compatible APIs.
//!
//! This is the **most important component** of the migration — it replaces LiteLLM
//! by talking directly to any OpenAI-compatible `/chat/completions` endpoint.
//!
//! Covers: OpenAI, Anthropic (via OpenRouter), DeepSeek, Groq, Gemini, ZhiPu,
//!         DashScope, Moonshot, MiniMax, vLLM, AiHubMix, OpenRouter.

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use tracing::{debug, error, warn};

use oxibot_core::types::{
    ChatCompletionRequest, ChatCompletionResponse, LlmResponse, Message, ToolDefinition,
};

use crate::registry::{
    apply_model_overrides, resolve_model_name, ProviderConfig, ProviderSpec,
};
use crate::traits::{LlmProvider, LlmRequestConfig};

// ─────────────────────────────────────────────
// HttpProvider
// ─────────────────────────────────────────────

/// A generic LLM provider that talks to any OpenAI-compatible HTTP API.
///
/// Replaces nanobot's `LiteLLMProvider` — instead of routing through LiteLLM,
/// we make direct HTTP requests via `reqwest`.
pub struct HttpProvider {
    /// HTTP client (shared, connection-pooled).
    client: reqwest::Client,
    /// API base URL (e.g. `"https://api.openai.com/v1"`).
    api_base: String,
    /// API key for Bearer authentication.
    api_key: String,
    /// Default model for this provider instance.
    default_model: String,
    /// Extra headers to send with each request (e.g. AiHubMix X-App-Code).
    extra_headers: HeaderMap,
    /// Reference to the provider spec for model resolution and overrides.
    spec: &'static ProviderSpec,
}

impl std::fmt::Debug for HttpProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpProvider")
            .field("api_base", &self.api_base)
            .field("default_model", &self.default_model)
            .field("provider", &self.spec.display_name)
            .finish()
    }
}

impl HttpProvider {
    /// Create a new HttpProvider from a provider config and spec.
    ///
    /// # Arguments
    /// * `config`  — User's config (api_key, api_base, extra_headers)
    /// * `spec`    — Static provider spec from the registry
    /// * `model`   — The default model to use
    pub fn new(config: &ProviderConfig, spec: &'static ProviderSpec, model: &str) -> Self {
        // Resolve API base: config > spec default > standard OpenAI path
        let api_base = config
            .api_base
            .clone()
            .or_else(|| spec.default_api_base.map(String::from))
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

        // Build extra headers
        let mut extra_headers = HeaderMap::new();
        if let Some(ref headers) = config.extra_headers {
            for (key, value) in headers {
                if let (Ok(name), Ok(val)) = (
                    HeaderName::from_bytes(key.as_bytes()),
                    HeaderValue::from_str(value),
                ) {
                    extra_headers.insert(name, val);
                } else {
                    warn!("Invalid header: {}={}", key, value);
                }
            }
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("Failed to build HTTP client");

        HttpProvider {
            client,
            api_base,
            api_key: config.api_key.clone(),
            default_model: model.to_string(),
            extra_headers,
            spec,
        }
    }

    /// Build the full chat completions URL.
    fn completions_url(&self) -> String {
        let base = self.api_base.trim_end_matches('/');
        format!("{}/chat/completions", base)
    }

    /// Resolve the model name for this provider (apply prefix/strip logic).
    fn resolve_model(&self, model: &str) -> String {
        resolve_model_name(model, self.spec)
    }
}

#[async_trait]
impl LlmProvider for HttpProvider {
    async fn chat(
        &self,
        messages: &[Message],
        tools: Option<&[ToolDefinition]>,
        model: &str,
        config: &LlmRequestConfig,
    ) -> LlmResponse {
        let resolved_model = self.resolve_model(model);
        let temperature = apply_model_overrides(model, self.spec, config.temperature);

        debug!(
            provider = self.spec.display_name,
            model = %resolved_model,
            messages = messages.len(),
            tools = tools.map_or(0, |t| t.len()),
            "Calling LLM"
        );

        let request_body = ChatCompletionRequest {
            model: resolved_model.clone(),
            messages: messages.to_vec(),
            tools: tools.map(|t| t.to_vec()),
            tool_choice: tools.map(|_| "auto".to_string()),
            max_tokens: Some(config.max_tokens),
            temperature: Some(temperature),
        };

        let url = self.completions_url();

        let result = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .headers(self.extra_headers.clone())
            .json(&request_body)
            .send()
            .await;

        let response = match result {
            Ok(resp) => resp,
            Err(e) => {
                error!(provider = self.spec.display_name, error = %e, "HTTP request failed");
                return LlmResponse::error(format!("Error calling LLM: {}", e));
            }
        };

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Failed to read error body".to_string());
            error!(
                provider = self.spec.display_name,
                status = %status,
                body = %error_text,
                "API error"
            );
            return LlmResponse::error(format!(
                "Error calling LLM: {} — {}",
                status, error_text
            ));
        }

        match response.json::<ChatCompletionResponse>().await {
            Ok(chat_resp) => {
                let llm_resp: LlmResponse = chat_resp.into();
                debug!(
                    provider = self.spec.display_name,
                    has_content = llm_resp.content.is_some(),
                    tool_calls = llm_resp.tool_calls.len(),
                    finish_reason = llm_resp.finish_reason.as_deref().unwrap_or("?"),
                    "LLM response received"
                );
                llm_resp
            }
            Err(e) => {
                error!(
                    provider = self.spec.display_name,
                    error = %e,
                    "Failed to parse LLM response"
                );
                LlmResponse::error(format!("Error parsing LLM response: {}", e))
            }
        }
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    fn display_name(&self) -> &str {
        self.spec.display_name
    }
}

// ─────────────────────────────────────────────
// Builder (convenience)
// ─────────────────────────────────────────────

/// Build an HttpProvider from a model name and a map of provider configs.
///
/// This is the main entry point — it matches the model to a provider,
/// reads the config, and creates the HttpProvider.
///
/// Replaces nanobot's CLI instantiation logic.
pub fn create_provider(
    model: &str,
    providers: &std::collections::HashMap<String, ProviderConfig>,
) -> Result<HttpProvider, String> {
    let (config, spec) = crate::registry::match_provider(model, providers)
        .ok_or_else(|| {
            format!(
                "No configured provider found for model '{}'. \
                 Set the appropriate API key (e.g. ANTHROPIC_API_KEY, OPENROUTER_API_KEY).",
                model
            )
        })?;

    debug!(
        provider = spec.display_name,
        model = model,
        api_base = config.api_base.as_deref().unwrap_or("default"),
        "Creating LLM provider"
    );

    Ok(HttpProvider::new(config, spec, model))
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::find_by_name;
    use std::collections::HashMap;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_config(api_key: &str, api_base: Option<&str>) -> ProviderConfig {
        ProviderConfig {
            api_key: api_key.to_string(),
            api_base: api_base.map(String::from),
            extra_headers: None,
        }
    }

    // ── Unit tests ──

    #[test]
    fn test_completions_url_trailing_slash() {
        let spec = find_by_name("openai").unwrap();
        let config = make_config("key", Some("https://api.openai.com/v1/"));
        let provider = HttpProvider::new(&config, spec, "gpt-4o");
        assert_eq!(
            provider.completions_url(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_completions_url_no_trailing_slash() {
        let spec = find_by_name("openai").unwrap();
        let config = make_config("key", Some("https://api.openai.com/v1"));
        let provider = HttpProvider::new(&config, spec, "gpt-4o");
        assert_eq!(
            provider.completions_url(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_default_api_base_for_gateway() {
        let spec = find_by_name("openrouter").unwrap();
        let config = make_config("sk-or-abc", None);
        let provider = HttpProvider::new(&config, spec, "meta-llama/llama-3");
        assert_eq!(provider.api_base, "https://openrouter.ai/api/v1");
    }

    #[test]
    fn test_config_overrides_default_base() {
        let spec = find_by_name("openrouter").unwrap();
        let config = make_config("sk-or-abc", Some("https://custom.proxy.com/v1"));
        let provider = HttpProvider::new(&config, spec, "meta-llama/llama-3");
        assert_eq!(provider.api_base, "https://custom.proxy.com/v1");
    }

    #[test]
    fn test_model_resolution_in_provider() {
        let spec = find_by_name("deepseek").unwrap();
        let config = make_config("key", None);
        let provider = HttpProvider::new(&config, spec, "deepseek-chat");
        assert_eq!(provider.resolve_model("deepseek-chat"), "deepseek/deepseek-chat");
    }

    #[test]
    fn test_display_name() {
        let spec = find_by_name("groq").unwrap();
        let config = make_config("key", None);
        let provider = HttpProvider::new(&config, spec, "llama-3.3-70b");
        assert_eq!(provider.display_name(), "Groq");
    }

    #[test]
    fn test_extra_headers() {
        let spec = find_by_name("aihubmix").unwrap();
        let mut headers = HashMap::new();
        headers.insert("X-App-Code".to_string(), "my-app-code".to_string());
        let config = ProviderConfig {
            api_key: "key".to_string(),
            api_base: None,
            extra_headers: Some(headers),
        };
        let provider = HttpProvider::new(&config, spec, "gpt-4o");
        assert!(provider.extra_headers.contains_key("x-app-code"));
    }

    // ── Integration tests with mock server ──

    #[tokio::test]
    async fn test_chat_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("Authorization", "Bearer test-key-123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "chatcmpl-test",
                "choices": [{
                    "message": {
                        "content": "Hello! I'm Oxibot.",
                        "tool_calls": null
                    },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 5,
                    "total_tokens": 15
                }
            })))
            .mount(&mock_server)
            .await;

        let spec = find_by_name("openai").unwrap();
        let config = make_config("test-key-123", Some(&mock_server.uri()));
        let provider = HttpProvider::new(&config, spec, "gpt-4o");

        let messages = vec![
            Message::system("You are Oxibot."),
            Message::user("Hello"),
        ];
        let req_config = LlmRequestConfig::default();

        let resp = provider.chat(&messages, None, "gpt-4o", &req_config).await;

        assert_eq!(resp.content.as_deref(), Some("Hello! I'm Oxibot."));
        assert!(!resp.has_tool_calls());
        assert_eq!(resp.finish_reason.as_deref(), Some("stop"));
        assert_eq!(resp.usage.as_ref().unwrap().total_tokens, 15);
    }

    #[tokio::test]
    async fn test_chat_with_tool_calls() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "chatcmpl-tools",
                "choices": [{
                    "message": {
                        "content": null,
                        "tool_calls": [{
                            "id": "call_abc123",
                            "type": "function",
                            "function": {
                                "name": "web_search",
                                "arguments": "{\"query\": \"Rust programming\"}"
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 15,
                    "total_tokens": 35
                }
            })))
            .mount(&mock_server)
            .await;

        let spec = find_by_name("openai").unwrap();
        let config = make_config("key", Some(&mock_server.uri()));
        let provider = HttpProvider::new(&config, spec, "gpt-4o");

        let tool_def = ToolDefinition::new(
            "web_search",
            "Search the web",
            serde_json::json!({"type": "object", "properties": {"query": {"type": "string"}}}),
        );

        let messages = vec![Message::user("Search for Rust")];
        let req_config = LlmRequestConfig::default();

        let resp = provider
            .chat(&messages, Some(&[tool_def]), "gpt-4o", &req_config)
            .await;

        assert!(resp.content.is_none());
        assert!(resp.has_tool_calls());
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].function.name, "web_search");
        assert_eq!(resp.tool_calls[0].id, "call_abc123");
    }

    #[tokio::test]
    async fn test_chat_api_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(429).set_body_json(serde_json::json!({
                    "error": {
                        "message": "Rate limit exceeded",
                        "type": "rate_limit_error"
                    }
                })),
            )
            .mount(&mock_server)
            .await;

        let spec = find_by_name("openai").unwrap();
        let config = make_config("key", Some(&mock_server.uri()));
        let provider = HttpProvider::new(&config, spec, "gpt-4o");

        let messages = vec![Message::user("Hello")];
        let req_config = LlmRequestConfig::default();

        let resp = provider.chat(&messages, None, "gpt-4o", &req_config).await;

        // Should return error message, not panic
        assert!(resp.content.is_some());
        let content = resp.content.unwrap();
        assert!(content.contains("Error calling LLM"));
        assert!(content.contains("429"));
    }

    #[tokio::test]
    async fn test_chat_network_error() {
        // Point to a port that's not listening
        let spec = find_by_name("openai").unwrap();
        let config = make_config("key", Some("http://127.0.0.1:1"));
        let provider = HttpProvider::new(&config, spec, "gpt-4o");

        let messages = vec![Message::user("Hello")];
        let req_config = LlmRequestConfig::default();

        let resp = provider.chat(&messages, None, "gpt-4o", &req_config).await;

        assert!(resp.content.is_some());
        assert!(resp.content.unwrap().contains("Error calling LLM"));
    }

    #[tokio::test]
    async fn test_chat_sends_correct_body() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_partial_json(serde_json::json!({
                "model": "deepseek/deepseek-chat",
                "max_tokens": 4096
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "chatcmpl-body",
                "choices": [{
                    "message": { "content": "ok" },
                    "finish_reason": "stop"
                }],
                "usage": null
            })))
            .mount(&mock_server)
            .await;

        let spec = find_by_name("deepseek").unwrap();
        let config = make_config("ds-key", Some(&mock_server.uri()));
        let provider = HttpProvider::new(&config, spec, "deepseek-chat");

        let messages = vec![Message::user("test")];
        let req_config = LlmRequestConfig::default();

        let resp = provider
            .chat(&messages, None, "deepseek-chat", &req_config)
            .await;

        // If the body matcher fails, wiremock returns 404 → we'd get an error
        assert_eq!(resp.content.as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn test_chat_with_reasoning_content() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "chatcmpl-reasoning",
                "choices": [{
                    "message": {
                        "content": "The answer is 42.",
                        "reasoning_content": "Let me think step by step..."
                    },
                    "finish_reason": "stop"
                }],
                "usage": null
            })))
            .mount(&mock_server)
            .await;

        let spec = find_by_name("deepseek").unwrap();
        let config = make_config("key", Some(&mock_server.uri()));
        let provider = HttpProvider::new(&config, spec, "deepseek-reasoner");

        let messages = vec![Message::user("What is the meaning of life?")];
        let req_config = LlmRequestConfig::default();

        let resp = provider
            .chat(&messages, None, "deepseek-reasoner", &req_config)
            .await;

        assert_eq!(resp.content.as_deref(), Some("The answer is 42."));
        assert_eq!(
            resp.reasoning_content.as_deref(),
            Some("Let me think step by step...")
        );
    }

    // ── create_provider ──

    #[test]
    fn test_create_provider_success() {
        let mut providers = HashMap::new();
        providers.insert(
            "anthropic".to_string(),
            make_config("sk-ant-123", None),
        );

        let provider = create_provider("claude-sonnet-4-20250514", &providers).unwrap();
        assert_eq!(provider.display_name(), "Anthropic");
        assert_eq!(provider.default_model(), "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_create_provider_no_config() {
        let providers = HashMap::new();
        let err = create_provider("claude-3", &providers).unwrap_err();
        assert!(err.contains("No configured provider"));
        assert!(err.contains("claude-3"));
    }
}
