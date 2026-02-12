//! Provider registry — static specs for all 12 supported LLM providers.
//!
//! Ports nanobot's `providers/registry.py` PROVIDERS list.
//! Each `ProviderSpec` describes how to connect to a provider:
//! keywords for model matching, env var names, API bases, quirks, etc.

use std::collections::HashMap;

// ─────────────────────────────────────────────
// ProviderSpec — static metadata for one provider
// ─────────────────────────────────────────────

/// Static specification describing one LLM provider.
///
/// Used by the matching logic to figure out which provider to use for a given model.
#[derive(Clone, Debug)]
pub struct ProviderSpec {
    /// Internal name (e.g. `"openrouter"`).
    pub name: &'static str,
    /// Keywords to match in model names (lowercase). E.g. `&["claude", "anthropic"]`.
    pub keywords: &'static [&'static str],
    /// Environment variable for the API key. E.g. `"OPENROUTER_API_KEY"`.
    pub env_key: &'static str,
    /// Human-readable name for logs. E.g. `"OpenRouter"`.
    pub display_name: &'static str,
    /// Prefix to prepend to model names for API routing.
    /// E.g. `Some("deepseek")` → model becomes `"deepseek/deepseek-chat"`.
    pub prefix: Option<&'static str>,
    /// Prefixes that, if already present, mean we skip prepending.
    /// E.g. `&["deepseek/"]` — if model is `"deepseek/xxx"` don't re-prefix.
    pub skip_prefixes: &'static [&'static str],
    /// Whether this is a gateway/aggregator (OpenRouter, AiHubMix).
    /// Gateways are used as fallback when no direct match is found.
    pub is_gateway: bool,
    /// Whether this is a local/self-hosted provider (vLLM).
    pub is_local: bool,
    /// If the API key starts with this prefix, auto-detect this provider.
    /// E.g. `Some("sk-or-")` for OpenRouter.
    pub detect_by_key_prefix: Option<&'static str>,
    /// If the API base URL contains this substring, auto-detect.
    /// E.g. `Some("aihubmix")`.
    pub detect_by_base_keyword: Option<&'static str>,
    /// Default API base URL. Used for gateways and providers with non-standard endpoints.
    pub default_api_base: Option<&'static str>,
    /// Whether to strip existing model prefix before re-prefixing (AiHubMix quirk).
    /// E.g. `"anthropic/claude-3"` → strip to `"claude-3"` → prefix to `"openai/claude-3"`.
    pub strip_model_prefix: bool,
    /// Per-model overrides. `(pattern, key, value)` — if `pattern` appears in model name
    /// (lowercase), force that key to that f64 value in the request.
    /// E.g. Kimi K2.5 requires `temperature >= 1.0`.
    pub model_overrides: &'static [ModelOverride],
}

/// A per-model parameter override.
#[derive(Clone, Debug)]
pub struct ModelOverride {
    /// Substring to match in the lowercase model name.
    pub pattern: &'static str,
    /// The field to override (currently only "temperature" is supported).
    pub field: OverrideField,
    /// The value to set.
    pub value: f64,
}

/// Fields that can be overridden per model.
#[derive(Clone, Debug)]
pub enum OverrideField {
    Temperature,
}

// ─────────────────────────────────────────────
// All 12 providers (in priority order, matching nanobot)
// ─────────────────────────────────────────────

/// Complete list of supported provider specifications, in matching priority order.
pub static PROVIDERS: &[ProviderSpec] = &[
    // 1. OpenRouter — gateway, matched by key prefix "sk-or-"
    ProviderSpec {
        name: "openrouter",
        keywords: &["openrouter"],
        env_key: "OPENROUTER_API_KEY",
        display_name: "OpenRouter",
        prefix: Some("openrouter"),
        skip_prefixes: &[],
        is_gateway: true,
        is_local: false,
        detect_by_key_prefix: Some("sk-or-"),
        detect_by_base_keyword: Some("openrouter"),
        default_api_base: Some("https://openrouter.ai/api/v1"),
        strip_model_prefix: false,
        model_overrides: &[],
    },
    // 2. AiHubMix — gateway, strips model prefix then re-prefixes with "openai"
    ProviderSpec {
        name: "aihubmix",
        keywords: &["aihubmix"],
        env_key: "OPENAI_API_KEY",
        display_name: "AiHubMix",
        prefix: Some("openai"),
        skip_prefixes: &[],
        is_gateway: true,
        is_local: false,
        detect_by_key_prefix: None,
        detect_by_base_keyword: Some("aihubmix"),
        default_api_base: Some("https://aihubmix.com/v1"),
        strip_model_prefix: true,
        model_overrides: &[],
    },
    // 3. Anthropic
    ProviderSpec {
        name: "anthropic",
        keywords: &["anthropic", "claude"],
        env_key: "ANTHROPIC_API_KEY",
        display_name: "Anthropic",
        prefix: None,
        skip_prefixes: &[],
        is_gateway: false,
        is_local: false,
        detect_by_key_prefix: None,
        detect_by_base_keyword: None,
        default_api_base: None,
        strip_model_prefix: false,
        model_overrides: &[],
    },
    // 4. OpenAI
    ProviderSpec {
        name: "openai",
        keywords: &["openai", "gpt"],
        env_key: "OPENAI_API_KEY",
        display_name: "OpenAI",
        prefix: None,
        skip_prefixes: &[],
        is_gateway: false,
        is_local: false,
        detect_by_key_prefix: None,
        detect_by_base_keyword: None,
        default_api_base: None,
        strip_model_prefix: false,
        model_overrides: &[],
    },
    // 5. DeepSeek
    ProviderSpec {
        name: "deepseek",
        keywords: &["deepseek"],
        env_key: "DEEPSEEK_API_KEY",
        display_name: "DeepSeek",
        prefix: Some("deepseek"),
        skip_prefixes: &["deepseek/"],
        is_gateway: false,
        is_local: false,
        detect_by_key_prefix: None,
        detect_by_base_keyword: None,
        default_api_base: None,
        strip_model_prefix: false,
        model_overrides: &[],
    },
    // 6. Gemini
    ProviderSpec {
        name: "gemini",
        keywords: &["gemini"],
        env_key: "GEMINI_API_KEY",
        display_name: "Gemini",
        prefix: Some("gemini"),
        skip_prefixes: &["gemini/"],
        is_gateway: false,
        is_local: false,
        detect_by_key_prefix: None,
        detect_by_base_keyword: None,
        default_api_base: None,
        strip_model_prefix: false,
        model_overrides: &[],
    },
    // 7. ZhiPu (GLM)
    ProviderSpec {
        name: "zhipu",
        keywords: &["zhipu", "glm", "zai"],
        env_key: "ZAI_API_KEY",
        display_name: "ZhiPu",
        prefix: Some("zai"),
        skip_prefixes: &["zhipu/", "zai/", "openrouter/", "hosted_vllm/"],
        is_gateway: false,
        is_local: false,
        detect_by_key_prefix: None,
        detect_by_base_keyword: None,
        default_api_base: None,
        strip_model_prefix: false,
        model_overrides: &[],
    },
    // 8. DashScope (Qwen)
    ProviderSpec {
        name: "dashscope",
        keywords: &["qwen", "dashscope"],
        env_key: "DASHSCOPE_API_KEY",
        display_name: "DashScope",
        prefix: Some("dashscope"),
        skip_prefixes: &["dashscope/", "openrouter/"],
        is_gateway: false,
        is_local: false,
        detect_by_key_prefix: None,
        detect_by_base_keyword: None,
        default_api_base: None,
        strip_model_prefix: false,
        model_overrides: &[],
    },
    // 9. Moonshot (Kimi) — Kimi K2.5 forces temperature=1.0
    ProviderSpec {
        name: "moonshot",
        keywords: &["moonshot", "kimi"],
        env_key: "MOONSHOT_API_KEY",
        display_name: "Moonshot",
        prefix: Some("moonshot"),
        skip_prefixes: &["moonshot/", "openrouter/"],
        is_gateway: false,
        is_local: false,
        detect_by_key_prefix: None,
        detect_by_base_keyword: None,
        default_api_base: Some("https://api.moonshot.ai/v1"),
        strip_model_prefix: false,
        model_overrides: &[ModelOverride {
            pattern: "kimi-k2.5",
            field: OverrideField::Temperature,
            value: 1.0,
        }],
    },
    // 10. MiniMax
    ProviderSpec {
        name: "minimax",
        keywords: &["minimax"],
        env_key: "MINIMAX_API_KEY",
        display_name: "MiniMax",
        prefix: Some("minimax"),
        skip_prefixes: &["minimax/", "openrouter/"],
        is_gateway: false,
        is_local: false,
        detect_by_key_prefix: None,
        detect_by_base_keyword: None,
        default_api_base: Some("https://api.minimax.io/v1"),
        strip_model_prefix: false,
        model_overrides: &[],
    },
    // 11. vLLM (self-hosted)
    ProviderSpec {
        name: "vllm",
        keywords: &["vllm"],
        env_key: "HOSTED_VLLM_API_KEY",
        display_name: "vLLM",
        prefix: Some("hosted_vllm"),
        skip_prefixes: &[],
        is_gateway: false,
        is_local: true,
        detect_by_key_prefix: None,
        detect_by_base_keyword: None,
        default_api_base: None,
        strip_model_prefix: false,
        model_overrides: &[],
    },
    // 12. Groq
    ProviderSpec {
        name: "groq",
        keywords: &["groq"],
        env_key: "GROQ_API_KEY",
        display_name: "Groq",
        prefix: Some("groq"),
        skip_prefixes: &["groq/"],
        is_gateway: false,
        is_local: false,
        detect_by_key_prefix: None,
        detect_by_base_keyword: None,
        default_api_base: None,
        strip_model_prefix: false,
        model_overrides: &[],
    },
];

// ─────────────────────────────────────────────
// Matching functions
// ─────────────────────────────────────────────

/// Find a provider spec by matching keywords against a model name.
///
/// Skips gateways and local providers — those are fallback only.
/// Returns the first match in priority order.
pub fn find_by_model(model: &str) -> Option<&'static ProviderSpec> {
    let model_lower = model.to_lowercase();
    PROVIDERS.iter().find(|spec| {
        !spec.is_gateway
            && !spec.is_local
            && spec
                .keywords
                .iter()
                .any(|kw| model_lower.contains(kw))
    })
}

/// Find a provider spec by exact name.
pub fn find_by_name(name: &str) -> Option<&'static ProviderSpec> {
    PROVIDERS.iter().find(|spec| spec.name == name)
}

/// Try to auto-detect a gateway/local provider from key prefix or base URL.
///
/// Priority:
/// 1. If `provider_name` matches a gateway/local spec → return it.
/// 2. If `api_key` starts with a spec's `detect_by_key_prefix` → return it.
/// 3. If `api_base` contains a spec's `detect_by_base_keyword` → return it.
pub fn find_gateway(
    provider_name: Option<&str>,
    api_key: Option<&str>,
    api_base: Option<&str>,
) -> Option<&'static ProviderSpec> {
    // 1. Exact name match among gateways/locals
    if let Some(name) = provider_name {
        if let Some(spec) = PROVIDERS
            .iter()
            .find(|s| s.name == name && (s.is_gateway || s.is_local))
        {
            return Some(spec);
        }
    }

    // 2. Detect by API key prefix
    if let Some(key) = api_key {
        if let Some(spec) = PROVIDERS.iter().find(|s| {
            s.detect_by_key_prefix
                .map_or(false, |pfx| key.starts_with(pfx))
        }) {
            return Some(spec);
        }
    }

    // 3. Detect by API base URL keyword
    if let Some(base) = api_base {
        let base_lower = base.to_lowercase();
        if let Some(spec) = PROVIDERS.iter().find(|s| {
            s.detect_by_base_keyword
                .map_or(false, |kw| base_lower.contains(kw))
        }) {
            return Some(spec);
        }
    }

    None
}

/// Resolve the model name for API calls, applying prefix and strip logic.
///
/// This replaces nanobot's `_resolve_model()`.
///
/// Rules:
/// - If `strip_model_prefix` is true (AiHubMix), strip everything before the last `/`.
/// - If a prefix is defined and the model doesn't already start with a skip_prefix, prepend it.
pub fn resolve_model_name(model: &str, spec: &ProviderSpec) -> String {
    let mut resolved = model.to_string();

    // Strip existing prefix (AiHubMix quirk)
    if spec.strip_model_prefix {
        if let Some(pos) = resolved.rfind('/') {
            resolved = resolved[pos + 1..].to_string();
        }
    }

    // Apply prefix if needed
    if let Some(prefix) = spec.prefix {
        let already_prefixed = spec
            .skip_prefixes
            .iter()
            .any(|sp| resolved.starts_with(sp));
        if !already_prefixed {
            resolved = format!("{}/{}", prefix, resolved);
        }
    }

    resolved
}

/// Apply per-model overrides to request parameters.
///
/// Returns overridden values for temperature (and potentially other fields).
/// E.g. Kimi K2.5 forces `temperature = 1.0`.
pub fn apply_model_overrides(
    model: &str,
    spec: &ProviderSpec,
    temperature: f64,
) -> f64 {
    let model_lower = model.to_lowercase();
    let mut temp = temperature;

    for ovr in spec.model_overrides {
        if model_lower.contains(ovr.pattern) {
            match ovr.field {
                OverrideField::Temperature => temp = ovr.value,
            }
        }
    }

    temp
}

/// Re-export the provider config from core — single source of truth.
pub use oxibot_core::config::schema::ProviderConfig;

/// Match a model name to a configured provider.
///
/// This replaces nanobot's `Config._match_provider()`.
///
/// 1. Find by keyword match, only if that provider has an API key.
/// 2. Fallback to the first configured gateway.
pub fn match_provider<'a>(
    model: &str,
    providers: &'a HashMap<String, ProviderConfig>,
) -> Option<(&'a ProviderConfig, &'static ProviderSpec)> {
    // 1. Direct keyword match
    if let Some(spec) = find_by_model(model) {
        if let Some(config) = providers.get(spec.name) {
            if config.is_configured() {
                return Some((config, spec));
            }
        }
    }

    // 2. Fallback to first configured gateway
    PROVIDERS
        .iter()
        .filter(|s| s.is_gateway)
        .find_map(|spec| {
            providers
                .get(spec.name)
                .filter(|c| c.is_configured())
                .map(|c| (c, spec))
        })
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_by_model_claude() {
        let spec = find_by_model("claude-sonnet-4-20250514").unwrap();
        assert_eq!(spec.name, "anthropic");
    }

    #[test]
    fn test_find_by_model_gpt() {
        let spec = find_by_model("gpt-4o-mini").unwrap();
        assert_eq!(spec.name, "openai");
    }

    #[test]
    fn test_find_by_model_deepseek() {
        let spec = find_by_model("deepseek-chat").unwrap();
        assert_eq!(spec.name, "deepseek");
    }

    #[test]
    fn test_find_by_model_qwen() {
        let spec = find_by_model("qwen-turbo").unwrap();
        assert_eq!(spec.name, "dashscope");
    }

    #[test]
    fn test_find_by_model_gemini() {
        let spec = find_by_model("gemini-2.0-flash").unwrap();
        assert_eq!(spec.name, "gemini");
    }

    #[test]
    fn test_find_by_model_groq() {
        let spec = find_by_model("groq/llama-3.3-70b").unwrap();
        assert_eq!(spec.name, "groq");
    }

    #[test]
    fn test_find_by_model_kimi() {
        let spec = find_by_model("kimi-k2.5-preview").unwrap();
        assert_eq!(spec.name, "moonshot");
    }

    #[test]
    fn test_find_by_model_glm() {
        let spec = find_by_model("glm-4-flash").unwrap();
        assert_eq!(spec.name, "zhipu");
    }

    #[test]
    fn test_find_by_model_skips_gateway() {
        // "openrouter" is a gateway — should NOT match directly
        let spec = find_by_model("openrouter/anthropic/claude-3");
        // This matches anthropic, not openrouter (gateways are skipped)
        assert_eq!(spec.unwrap().name, "anthropic");
    }

    #[test]
    fn test_find_by_model_unknown() {
        let spec = find_by_model("some-random-model-xyz");
        assert!(spec.is_none());
    }

    #[test]
    fn test_find_by_name() {
        let spec = find_by_name("deepseek").unwrap();
        assert_eq!(spec.display_name, "DeepSeek");
        assert_eq!(spec.env_key, "DEEPSEEK_API_KEY");
    }

    #[test]
    fn test_find_gateway_by_key_prefix() {
        let spec = find_gateway(None, Some("sk-or-abc123"), None).unwrap();
        assert_eq!(spec.name, "openrouter");
    }

    #[test]
    fn test_find_gateway_by_name() {
        let spec = find_gateway(Some("aihubmix"), None, None).unwrap();
        assert_eq!(spec.name, "aihubmix");
    }

    #[test]
    fn test_find_gateway_by_base_keyword() {
        let spec = find_gateway(None, None, Some("https://aihubmix.com/v1")).unwrap();
        assert_eq!(spec.name, "aihubmix");
    }

    #[test]
    fn test_find_gateway_none() {
        let spec = find_gateway(None, Some("sk-regular-key"), None);
        assert!(spec.is_none());
    }

    // ── resolve_model_name ──

    #[test]
    fn test_resolve_model_basic_prefix() {
        let spec = find_by_name("deepseek").unwrap();
        // "deepseek-chat" → "deepseek/deepseek-chat"
        assert_eq!(resolve_model_name("deepseek-chat", spec), "deepseek/deepseek-chat");
    }

    #[test]
    fn test_resolve_model_skip_prefix() {
        let spec = find_by_name("deepseek").unwrap();
        // Already has "deepseek/" → don't re-prefix
        assert_eq!(resolve_model_name("deepseek/deepseek-chat", spec), "deepseek/deepseek-chat");
    }

    #[test]
    fn test_resolve_model_no_prefix() {
        let spec = find_by_name("anthropic").unwrap();
        // Anthropic has no prefix
        assert_eq!(resolve_model_name("claude-sonnet-4-20250514", spec), "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_resolve_model_strip_and_reprefix() {
        let spec = find_by_name("aihubmix").unwrap();
        // AiHubMix strips prefix, then adds "openai/"
        // "anthropic/claude-3" → strip → "claude-3" → "openai/claude-3"
        assert_eq!(resolve_model_name("anthropic/claude-3", spec), "openai/claude-3");
    }

    #[test]
    fn test_resolve_model_strip_no_slash() {
        let spec = find_by_name("aihubmix").unwrap();
        // No slash to strip → just prefix
        assert_eq!(resolve_model_name("gpt-4o", spec), "openai/gpt-4o");
    }

    // ── apply_model_overrides ──

    #[test]
    fn test_model_override_kimi_k25() {
        let spec = find_by_name("moonshot").unwrap();
        let temp = apply_model_overrides("kimi-k2.5-preview", spec, 0.7);
        assert_eq!(temp, 1.0);
    }

    #[test]
    fn test_model_override_no_match() {
        let spec = find_by_name("moonshot").unwrap();
        let temp = apply_model_overrides("moonshot-v1", spec, 0.7);
        assert_eq!(temp, 0.7); // Unchanged
    }

    #[test]
    fn test_model_override_not_applicable() {
        let spec = find_by_name("openai").unwrap();
        let temp = apply_model_overrides("gpt-4o", spec, 0.5);
        assert_eq!(temp, 0.5); // No overrides for OpenAI
    }

    // ── match_provider ──

    #[test]
    fn test_match_provider_direct() {
        let mut providers = HashMap::new();
        providers.insert(
            "anthropic".to_string(),
            ProviderConfig {
                api_key: "sk-ant-123".to_string(),
                ..Default::default()
            },
        );

        let (config, spec) = match_provider("claude-sonnet-4-20250514", &providers).unwrap();
        assert_eq!(spec.name, "anthropic");
        assert_eq!(config.api_key, "sk-ant-123");
    }

    #[test]
    fn test_match_provider_gateway_fallback() {
        let mut providers = HashMap::new();
        // Unknown model, but OpenRouter is configured
        providers.insert(
            "openrouter".to_string(),
            ProviderConfig {
                api_key: "sk-or-fallback".to_string(),
                ..Default::default()
            },
        );

        let (config, spec) = match_provider("some-unknown-model", &providers).unwrap();
        assert_eq!(spec.name, "openrouter");
        assert_eq!(config.api_key, "sk-or-fallback");
    }

    #[test]
    fn test_match_provider_no_key() {
        let mut providers = HashMap::new();
        providers.insert(
            "anthropic".to_string(),
            ProviderConfig {
                api_key: "".to_string(), // Empty = not configured
                ..Default::default()
            },
        );

        let result = match_provider("claude-3", &providers);
        assert!(result.is_none());
    }

    // ── PROVIDERS static array ──

    #[test]
    fn test_all_providers_have_unique_names() {
        let names: Vec<&str> = PROVIDERS.iter().map(|s| s.name).collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(names.len(), unique.len(), "Duplicate provider names found");
    }

    #[test]
    fn test_provider_count() {
        assert_eq!(PROVIDERS.len(), 12);
    }
}
