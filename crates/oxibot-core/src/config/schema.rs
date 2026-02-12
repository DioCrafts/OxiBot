//! Configuration schema — typed replacements for nanobot's Pydantic models.
//!
//! Hierarchy: `Config` → `AgentsConfig`, `ProvidersConfig`, `ChannelsConfig`,
//! `ToolsConfig`, `GatewayConfig`.
//!
//! JSON on disk uses **camelCase** keys; Rust uses snake_case.
//! We use `#[serde(rename_all = "camelCase")]` to handle the conversion.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─────────────────────────────────────────────
// Root Config
// ─────────────────────────────────────────────

/// Root configuration — loaded from `~/.oxibot/config.json` + env vars.
///
/// Replaces nanobot's `Config(BaseSettings)`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub agents: AgentsConfig,
    pub providers: ProvidersConfig,
    pub channels: ChannelsConfig,
    pub tools: ToolsConfig,
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub transcription: TranscriptionConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            agents: AgentsConfig::default(),
            providers: ProvidersConfig::default(),
            channels: ChannelsConfig::default(),
            tools: ToolsConfig::default(),
            gateway: GatewayConfig::default(),
            transcription: TranscriptionConfig::default(),
        }
    }
}

// ─────────────────────────────────────────────
// Agents
// ─────────────────────────────────────────────

/// Agent configuration container.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentsConfig {
    pub defaults: AgentDefaults,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            defaults: AgentDefaults::default(),
        }
    }
}

/// Default agent settings.
///
/// Replaces nanobot's `AgentDefaults`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentDefaults {
    /// Default workspace directory.
    pub workspace: String,
    /// Default LLM model identifier.
    pub model: String,
    /// Maximum tokens to generate per response.
    pub max_tokens: u32,
    /// Sampling temperature (0.0 – 2.0).
    pub temperature: f64,
    /// Maximum tool-calling loop iterations before forcing a response.
    pub max_tool_iterations: u32,
}

impl Default for AgentDefaults {
    fn default() -> Self {
        Self {
            workspace: "~/.oxibot/workspace".to_string(),
            model: "anthropic/claude-sonnet-4-20250514".to_string(),
            max_tokens: 8192,
            temperature: 0.7,
            max_tool_iterations: 20,
        }
    }
}

// ─────────────────────────────────────────────
// Providers
// ─────────────────────────────────────────────

/// Configuration for a single LLM provider (API key, base URL, headers).
///
/// Replaces nanobot's `ProviderConfig`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderConfig {
    /// API key for authentication.
    #[serde(default)]
    pub api_key: String,
    /// Custom API base URL (overrides provider default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
    /// Extra HTTP headers to send with each request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_headers: Option<HashMap<String, String>>,
}

impl ProviderConfig {
    /// Whether this provider has a configured API key.
    pub fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }
}

/// All provider configurations.
///
/// One `ProviderConfig` per supported LLM backend.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProvidersConfig {
    #[serde(default)]
    pub anthropic: ProviderConfig,
    #[serde(default)]
    pub openai: ProviderConfig,
    #[serde(default)]
    pub openrouter: ProviderConfig,
    #[serde(default)]
    pub deepseek: ProviderConfig,
    #[serde(default)]
    pub groq: ProviderConfig,
    #[serde(default)]
    pub zhipu: ProviderConfig,
    #[serde(default)]
    pub dashscope: ProviderConfig,
    #[serde(default)]
    pub vllm: ProviderConfig,
    #[serde(default)]
    pub gemini: ProviderConfig,
    #[serde(default)]
    pub moonshot: ProviderConfig,
    #[serde(default)]
    pub minimax: ProviderConfig,
    #[serde(default)]
    pub aihubmix: ProviderConfig,
}

impl ProvidersConfig {
    /// Get a provider config by name (e.g. `"anthropic"`).
    pub fn get_by_name(&self, name: &str) -> Option<&ProviderConfig> {
        match name {
            "anthropic" => Some(&self.anthropic),
            "openai" => Some(&self.openai),
            "openrouter" => Some(&self.openrouter),
            "deepseek" => Some(&self.deepseek),
            "groq" => Some(&self.groq),
            "zhipu" => Some(&self.zhipu),
            "dashscope" => Some(&self.dashscope),
            "vllm" => Some(&self.vllm),
            "gemini" => Some(&self.gemini),
            "moonshot" => Some(&self.moonshot),
            "minimax" => Some(&self.minimax),
            "aihubmix" => Some(&self.aihubmix),
            _ => None,
        }
    }

    /// Convert to a HashMap<String, ProviderConfig> for use with the provider registry.
    pub fn to_map(&self) -> HashMap<String, ProviderConfig> {
        let mut map = HashMap::new();
        let entries: &[(&str, &ProviderConfig)] = &[
            ("anthropic", &self.anthropic),
            ("openai", &self.openai),
            ("openrouter", &self.openrouter),
            ("deepseek", &self.deepseek),
            ("groq", &self.groq),
            ("zhipu", &self.zhipu),
            ("dashscope", &self.dashscope),
            ("vllm", &self.vllm),
            ("gemini", &self.gemini),
            ("moonshot", &self.moonshot),
            ("minimax", &self.minimax),
            ("aihubmix", &self.aihubmix),
        ];
        for (name, config) in entries {
            map.insert(name.to_string(), (*config).clone());
        }
        map
    }
}

// ─────────────────────────────────────────────
// Channels
// ─────────────────────────────────────────────

/// All channel configurations.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub discord: DiscordConfig,
    #[serde(default)]
    pub whatsapp: WhatsAppConfig,
    #[serde(default)]
    pub feishu: FeishuConfig,
    #[serde(default)]
    pub dingtalk: DingTalkConfig,
    #[serde(default)]
    pub slack: SlackConfig,
    #[serde(default)]
    pub email: EmailConfig,
    #[serde(default)]
    pub qq: QQConfig,
    #[serde(default)]
    pub mochat: MochatConfig,
}

/// Telegram channel config.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TelegramConfig {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

/// Discord channel config.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiscordConfig {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

/// WhatsApp channel config.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WhatsAppConfig {
    #[serde(default)]
    pub bridge_url: String,
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

/// Feishu/Lark channel config.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FeishuConfig {
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub app_secret: String,
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

/// DingTalk channel config.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DingTalkConfig {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

/// Slack channel config.
///
/// Supports two-tiered access control:
/// - DMs: controlled by `dm.enabled` + `dm.policy` + `dm.allow_from`
/// - Channels/groups: controlled by `group_policy` + `group_allow_from`
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SlackConfig {
    /// Bot token (`xoxb-...`) — required.
    #[serde(default)]
    pub bot_token: String,
    /// App-level token (`xapp-...`) — required for Socket Mode.
    #[serde(default)]
    pub app_token: String,
    /// Flat allowed-users list (user IDs). Empty = allow everyone.
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Group/channel response policy: `"mention"` (default), `"open"`, or `"allowlist"`.
    #[serde(default = "default_group_policy")]
    pub group_policy: String,
    /// Channel IDs allowed when `group_policy = "allowlist"`.
    #[serde(default)]
    pub group_allow_from: Vec<String>,
    /// DM-specific settings.
    #[serde(default)]
    pub dm: SlackDMConfig,
}

fn default_group_policy() -> String {
    "mention".to_string()
}

/// Slack DM-specific settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SlackDMConfig {
    /// Whether DMs are enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// DM access policy: `"open"` (default) or `"allowlist"`.
    #[serde(default = "default_dm_policy")]
    pub policy: String,
    /// User IDs allowed when `policy = "allowlist"`.
    #[serde(default)]
    pub allow_from: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_dm_policy() -> String {
    "open".to_string()
}

impl Default for SlackDMConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            policy: "open".to_string(),
            allow_from: Vec::new(),
        }
    }
}

/// Email channel config.
///
/// Supports IMAP polling for inbound + SMTP for outbound.
/// Thread tracking via subject prefix + In-Reply-To headers.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EmailConfig {
    // ── IMAP settings ──
    /// IMAP server hostname.
    #[serde(default)]
    pub imap_host: String,
    /// IMAP server port (default 993 for IMAPS).
    #[serde(default = "default_imap_port")]
    pub imap_port: u16,
    /// IMAP login username.
    #[serde(default)]
    pub imap_username: String,
    /// IMAP login password.
    #[serde(default)]
    pub imap_password: String,
    /// IMAP folder to poll (default "INBOX").
    #[serde(default = "default_imap_mailbox")]
    pub imap_mailbox: String,
    /// Use IMAPS (TLS from the start). Default true.
    #[serde(default = "default_true")]
    pub imap_use_ssl: bool,

    // ── SMTP settings ──
    /// SMTP server hostname.
    #[serde(default)]
    pub smtp_host: String,
    /// SMTP server port (default 587 for STARTTLS).
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    /// SMTP login username.
    #[serde(default)]
    pub smtp_username: String,
    /// SMTP login password.
    #[serde(default)]
    pub smtp_password: String,
    /// Use STARTTLS for SMTP (default true).
    #[serde(default = "default_true")]
    pub smtp_use_tls: bool,
    /// Use implicit TLS/SMTPS (default false, for port 465).
    #[serde(default)]
    pub smtp_use_ssl: bool,
    /// Sender address for outbound; falls back to smtp_username.
    #[serde(default)]
    pub from_address: String,

    // ── Behavior ──
    /// Poll interval in seconds (minimum 5, default 30).
    #[serde(default = "default_poll_interval")]
    pub poll_interval_seconds: u32,
    /// Mark fetched emails as \\Seen (default true).
    #[serde(default = "default_true")]
    pub mark_seen: bool,
    /// Truncate email body to this many characters (default 12000).
    #[serde(default = "default_max_body_chars")]
    pub max_body_chars: u32,
    /// Subject prefix for replies (default "Re: ").
    #[serde(default = "default_subject_prefix")]
    pub subject_prefix: String,
    /// Allowed sender emails (empty = allow everyone).
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

fn default_imap_port() -> u16 { 993 }
fn default_smtp_port() -> u16 { 587 }
fn default_imap_mailbox() -> String { "INBOX".to_string() }
fn default_poll_interval() -> u32 { 30 }
fn default_max_body_chars() -> u32 { 12000 }
fn default_subject_prefix() -> String { "Re: ".to_string() }

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            imap_host: String::new(),
            imap_port: 993,
            imap_username: String::new(),
            imap_password: String::new(),
            imap_mailbox: "INBOX".to_string(),
            imap_use_ssl: true,
            smtp_host: String::new(),
            smtp_port: 587,
            smtp_username: String::new(),
            smtp_password: String::new(),
            smtp_use_tls: true,
            smtp_use_ssl: false,
            from_address: String::new(),
            poll_interval_seconds: 30,
            mark_seen: true,
            max_body_chars: 12000,
            subject_prefix: "Re: ".to_string(),
            allowed_users: Vec::new(),
        }
    }
}

/// QQ channel config.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QQConfig {
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub app_secret: String,
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

/// Mochat channel config.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MochatConfig {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub allowed_users: Vec<String>,
    #[serde(default)]
    pub mention: MochatMentionConfig,
    #[serde(default)]
    pub groups: HashMap<String, MochatGroupRule>,
}

/// Mochat mention settings.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MochatMentionConfig {
    #[serde(default)]
    pub enabled: bool,
}

/// Mochat group rule.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MochatGroupRule {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mention_only: bool,
}

// ─────────────────────────────────────────────
// Tools
// ─────────────────────────────────────────────

/// Tool configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolsConfig {
    /// Web tools configuration (search, fetch).
    #[serde(default)]
    pub web: WebToolsConfig,
    /// Shell exec tool configuration.
    #[serde(default)]
    pub exec: ExecToolConfig,
    /// Whether to restrict file/exec operations to the workspace directory.
    #[serde(default)]
    pub restrict_to_workspace: bool,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            web: WebToolsConfig::default(),
            exec: ExecToolConfig::default(),
            restrict_to_workspace: false,
        }
    }
}

/// Web tools configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebToolsConfig {
    #[serde(default)]
    pub search: WebSearchConfig,
}

/// Web search configuration (Brave API).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebSearchConfig {
    /// Brave Search API key.
    #[serde(default)]
    pub api_key: String,
    /// Maximum number of search results to return.
    pub max_results: u32,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            max_results: 5,
        }
    }
}

/// Shell exec tool configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecToolConfig {
    /// Timeout in seconds for shell commands.
    pub timeout: u64,
}

impl Default for ExecToolConfig {
    fn default() -> Self {
        Self { timeout: 60 }
    }
}

// ─────────────────────────────────────────────
// Gateway
// ─────────────────────────────────────────────

/// Voice transcription configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TranscriptionConfig {
    /// Whether voice transcription is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Transcription provider (currently only "groq" supported).
    #[serde(default = "default_groq")]
    pub provider: String,
    /// API key for the transcription provider.
    /// Falls back to GROQ_API_KEY env var if empty.
    #[serde(default)]
    pub api_key: String,
    /// Whisper model name.
    #[serde(default = "default_whisper_model")]
    pub model: String,
}

fn default_groq() -> String { "groq".into() }
fn default_whisper_model() -> String { "whisper-large-v3".into() }

impl Default for TranscriptionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: "groq".into(),
            api_key: String::new(),
            model: "whisper-large-v3".into(),
        }
    }
}

/// HTTP gateway configuration (for incoming webhooks / REST API).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GatewayConfig {
    /// Listen address.
    pub host: String,
    /// Listen port.
    pub port: u16,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 18790,
        }
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.agents.defaults.max_tokens, 8192);
        assert_eq!(config.agents.defaults.temperature, 0.7);
        assert_eq!(config.agents.defaults.max_tool_iterations, 20);
        assert_eq!(config.gateway.port, 18790);
        assert!(!config.tools.restrict_to_workspace);
    }

    #[test]
    fn test_config_from_json_camel_case() {
        let json = serde_json::json!({
            "agents": {
                "defaults": {
                    "model": "gpt-4o",
                    "maxTokens": 4096,
                    "temperature": 0.5,
                    "maxToolIterations": 10
                }
            },
            "gateway": {
                "host": "127.0.0.1",
                "port": 9090
            }
        });

        let config: Config = serde_json::from_value(json).unwrap();
        assert_eq!(config.agents.defaults.model, "gpt-4o");
        assert_eq!(config.agents.defaults.max_tokens, 4096);
        assert_eq!(config.agents.defaults.temperature, 0.5);
        assert_eq!(config.agents.defaults.max_tool_iterations, 10);
        assert_eq!(config.gateway.host, "127.0.0.1");
        assert_eq!(config.gateway.port, 9090);
        // Defaults preserved for missing fields
        assert!(!config.tools.restrict_to_workspace);
        assert_eq!(config.tools.exec.timeout, 60);
    }

    #[test]
    fn test_config_serialization_round_trip() {
        let config = Config::default();
        let json_str = serde_json::to_string_pretty(&config).unwrap();
        let deserialized: Config = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.agents.defaults.model, config.agents.defaults.model);
        assert_eq!(deserialized.gateway.port, config.gateway.port);
    }

    #[test]
    fn test_config_json_uses_camel_case() {
        let config = Config::default();
        let json = serde_json::to_value(&config).unwrap();
        // Should use camelCase keys
        assert!(json["agents"]["defaults"].get("maxTokens").is_some());
        assert!(json["agents"]["defaults"].get("maxToolIterations").is_some());
        assert!(json["tools"].get("restrictToWorkspace").is_some());
        // Should NOT have snake_case keys
        assert!(json["agents"]["defaults"].get("max_tokens").is_none());
    }

    #[test]
    fn test_provider_config_is_configured() {
        let empty = ProviderConfig::default();
        assert!(!empty.is_configured());

        let with_key = ProviderConfig {
            api_key: "sk-123".to_string(),
            ..Default::default()
        };
        assert!(with_key.is_configured());
    }

    #[test]
    fn test_providers_get_by_name() {
        let mut providers = ProvidersConfig::default();
        providers.anthropic.api_key = "sk-ant-123".to_string();

        assert!(providers.get_by_name("anthropic").unwrap().is_configured());
        assert!(!providers.get_by_name("openai").unwrap().is_configured());
        assert!(providers.get_by_name("nonexistent").is_none());
    }

    #[test]
    fn test_partial_json_uses_defaults() {
        let json = serde_json::json!({
            "providers": {
                "anthropic": {
                    "apiKey": "sk-ant-test"
                }
            }
        });

        let config: Config = serde_json::from_value(json).unwrap();
        assert_eq!(config.providers.anthropic.api_key, "sk-ant-test");
        // All other providers should have empty defaults
        assert!(!config.providers.openai.is_configured());
        assert!(!config.providers.groq.is_configured());
        // Agent defaults still present
        assert_eq!(config.agents.defaults.max_tokens, 8192);
    }

    #[test]
    fn test_channel_config_from_json() {
        let json = serde_json::json!({
            "channels": {
                "telegram": {
                    "token": "bot123:ABC",
                    "allowedUsers": ["user1", "user2"]
                },
                "slack": {
                    "botToken": "xoxb-123",
                    "appToken": "xapp-456",
                    "dm": {
                        "enabled": true
                    }
                }
            }
        });

        let config: Config = serde_json::from_value(json).unwrap();
        assert_eq!(config.channels.telegram.token, "bot123:ABC");
        assert_eq!(config.channels.telegram.allowed_users, vec!["user1", "user2"]);
        assert_eq!(config.channels.slack.bot_token, "xoxb-123");
        assert!(config.channels.slack.dm.enabled);
    }

    #[test]
    fn test_tools_config_from_json() {
        let json = serde_json::json!({
            "tools": {
                "web": {
                    "search": {
                        "apiKey": "brave-key-123",
                        "maxResults": 10
                    }
                },
                "exec": {
                    "timeout": 120
                },
                "restrictToWorkspace": true
            }
        });

        let config: Config = serde_json::from_value(json).unwrap();
        assert_eq!(config.tools.web.search.api_key, "brave-key-123");
        assert_eq!(config.tools.web.search.max_results, 10);
        assert_eq!(config.tools.exec.timeout, 120);
        assert!(config.tools.restrict_to_workspace);
    }

    #[test]
    fn test_empty_json_gives_defaults() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(config.agents.defaults.model, "anthropic/claude-sonnet-4-20250514");
        assert_eq!(config.agents.defaults.max_tokens, 8192);
        assert_eq!(config.gateway.port, 18790);
    }
}
