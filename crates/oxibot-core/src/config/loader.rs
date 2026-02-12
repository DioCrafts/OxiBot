//! Config loader — reads `~/.oxibot/config.json`, merges env vars, and
//! applies legacy migrations.
//!
//! Replaces nanobot's `config/loader.py`.
//!
//! # Loading precedence
//! 1. Defaults (from `Config::default()`)
//! 2. JSON file at `~/.oxibot/config.json`
//! 3. Environment variables `OXIBOT_<SECTION>__<FIELD>` (override JSON)

use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use super::schema::Config;

/// Default config file path.
pub fn get_config_path() -> PathBuf {
    crate::utils::get_data_path().join("config.json")
}

/// Load configuration from the default path + env vars.
///
/// Falls back to `Config::default()` if the file doesn't exist or can't be parsed.
pub fn load_config(path: Option<&Path>) -> Config {
    let config_path = path
        .map(PathBuf::from)
        .unwrap_or_else(get_config_path);

    load_config_from_path(&config_path)
}

/// Load config from a specific file path.
fn load_config_from_path(path: &Path) -> Config {
    if !path.exists() {
        info!("No config file found at {}, using defaults", path.display());
        return apply_env_overrides(Config::default());
    }

    debug!("Loading config from {}", path.display());

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read config file {}: {}", path.display(), e);
            return apply_env_overrides(Config::default());
        }
    };

    // Parse JSON → Value first for migration
    let mut raw: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            warn!("Failed to parse config JSON: {}", e);
            return apply_env_overrides(Config::default());
        }
    };

    // Apply legacy migrations
    migrate_config(&mut raw);

    // Deserialize into typed Config
    let config: Config = match serde_json::from_value(raw) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to deserialize config: {}", e);
            return apply_env_overrides(Config::default());
        }
    };

    apply_env_overrides(config)
}

/// Save configuration to disk (pretty-printed JSON with camelCase keys).
pub fn save_config(config: &Config, path: Option<&Path>) -> std::io::Result<()> {
    let config_path = path
        .map(PathBuf::from)
        .unwrap_or_else(get_config_path);

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    std::fs::write(&config_path, json)?;
    debug!("Config saved to {}", config_path.display());
    Ok(())
}

/// Apply legacy config migrations.
///
/// Moves `tools.exec.restrictToWorkspace` → `tools.restrictToWorkspace`.
fn migrate_config(raw: &mut serde_json::Value) {
    // Migration: tools.exec.restrictToWorkspace → tools.restrictToWorkspace
    if let Some(tools) = raw.get_mut("tools") {
        if let Some(exec) = tools.get("exec") {
            if let Some(restrict) = exec.get("restrictToWorkspace") {
                if tools.get("restrictToWorkspace").is_none() {
                    let val = restrict.clone();
                    tools["restrictToWorkspace"] = val;
                    debug!("Migrated tools.exec.restrictToWorkspace → tools.restrictToWorkspace");
                }
            }
        }
    }
}

/// Apply environment variable overrides on top of a loaded config.
///
/// Env var format: `OXIBOT_<SECTION>__<FIELD>` (double underscore as delimiter).
///
/// Supported overrides:
/// - `OXIBOT_AGENTS__DEFAULTS__MODEL` → `agents.defaults.model`
/// - `OXIBOT_AGENTS__DEFAULTS__MAX_TOKENS` → `agents.defaults.max_tokens`
/// - `OXIBOT_AGENTS__DEFAULTS__TEMPERATURE` → `agents.defaults.temperature`
/// - `OXIBOT_PROVIDERS__<NAME>__API_KEY` → `providers.<name>.api_key`
/// - `OXIBOT_PROVIDERS__<NAME>__API_BASE` → `providers.<name>.api_base`
/// - `OXIBOT_GATEWAY__HOST` → `gateway.host`
/// - `OXIBOT_GATEWAY__PORT` → `gateway.port`
/// - `OXIBOT_TOOLS__RESTRICT_TO_WORKSPACE` → `tools.restrict_to_workspace`
fn apply_env_overrides(mut config: Config) -> Config {
    // Agent defaults
    if let Ok(val) = std::env::var("OXIBOT_AGENTS__DEFAULTS__MODEL") {
        config.agents.defaults.model = val;
    }
    if let Ok(val) = std::env::var("OXIBOT_AGENTS__DEFAULTS__MAX_TOKENS") {
        if let Ok(n) = val.parse::<u32>() {
            config.agents.defaults.max_tokens = n;
        }
    }
    if let Ok(val) = std::env::var("OXIBOT_AGENTS__DEFAULTS__TEMPERATURE") {
        if let Ok(t) = val.parse::<f64>() {
            config.agents.defaults.temperature = t;
        }
    }
    if let Ok(val) = std::env::var("OXIBOT_AGENTS__DEFAULTS__MAX_TOOL_ITERATIONS") {
        if let Ok(n) = val.parse::<u32>() {
            config.agents.defaults.max_tool_iterations = n;
        }
    }
    if let Ok(val) = std::env::var("OXIBOT_AGENTS__DEFAULTS__WORKSPACE") {
        config.agents.defaults.workspace = val;
    }

    // Provider API keys (by provider name)
    apply_provider_env(&mut config.providers.anthropic, "ANTHROPIC");
    apply_provider_env(&mut config.providers.openai, "OPENAI");
    apply_provider_env(&mut config.providers.openrouter, "OPENROUTER");
    apply_provider_env(&mut config.providers.deepseek, "DEEPSEEK");
    apply_provider_env(&mut config.providers.groq, "GROQ");
    apply_provider_env(&mut config.providers.zhipu, "ZHIPU");
    apply_provider_env(&mut config.providers.dashscope, "DASHSCOPE");
    apply_provider_env(&mut config.providers.vllm, "VLLM");
    apply_provider_env(&mut config.providers.gemini, "GEMINI");
    apply_provider_env(&mut config.providers.moonshot, "MOONSHOT");
    apply_provider_env(&mut config.providers.minimax, "MINIMAX");
    apply_provider_env(&mut config.providers.aihubmix, "AIHUBMIX");

    // Gateway
    if let Ok(val) = std::env::var("OXIBOT_GATEWAY__HOST") {
        config.gateway.host = val;
    }
    if let Ok(val) = std::env::var("OXIBOT_GATEWAY__PORT") {
        if let Ok(p) = val.parse::<u16>() {
            config.gateway.port = p;
        }
    }

    // Tools
    if let Ok(val) = std::env::var("OXIBOT_TOOLS__RESTRICT_TO_WORKSPACE") {
        config.tools.restrict_to_workspace = val == "true" || val == "1";
    }

    config
}

/// Apply env var overrides for a single provider.
fn apply_provider_env(provider: &mut super::schema::ProviderConfig, name: &str) {
    if let Ok(val) = std::env::var(format!("OXIBOT_PROVIDERS__{name}__API_KEY")) {
        provider.api_key = val;
    }
    if let Ok(val) = std::env::var(format!("OXIBOT_PROVIDERS__{name}__API_BASE")) {
        provider.api_base = Some(val);
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_temp_json(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_load_missing_file() {
        let config = load_config_from_path(Path::new("/nonexistent/path/config.json"));
        // Should return defaults
        assert_eq!(config.agents.defaults.max_tokens, 8192);
        assert_eq!(config.gateway.port, 18790);
    }

    #[test]
    fn test_load_valid_json() {
        let file = write_temp_json(r#"{
            "agents": {
                "defaults": {
                    "model": "gpt-4o",
                    "maxTokens": 2048
                }
            }
        }"#);

        let config = load_config_from_path(file.path());
        assert_eq!(config.agents.defaults.model, "gpt-4o");
        assert_eq!(config.agents.defaults.max_tokens, 2048);
        // Default preserved
        assert_eq!(config.agents.defaults.temperature, 0.7);
    }

    #[test]
    fn test_load_invalid_json_returns_defaults() {
        let file = write_temp_json("not valid json {{{");
        let config = load_config_from_path(file.path());
        assert_eq!(config.agents.defaults.max_tokens, 8192);
    }

    #[test]
    fn test_load_empty_json() {
        let file = write_temp_json("{}");
        let config = load_config_from_path(file.path());
        assert_eq!(config.agents.defaults.model, "anthropic/claude-sonnet-4-20250514");
    }

    #[test]
    fn test_save_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        let mut config = Config::default();
        config.agents.defaults.model = "deepseek-chat".to_string();
        config.providers.anthropic.api_key = "sk-ant-test".to_string();

        save_config(&config, Some(&path)).unwrap();

        let reloaded = load_config_from_path(&path);
        assert_eq!(reloaded.agents.defaults.model, "deepseek-chat");
        assert_eq!(reloaded.providers.anthropic.api_key, "sk-ant-test");
    }

    #[test]
    fn test_migrate_restrict_to_workspace() {
        let file = write_temp_json(r#"{
            "tools": {
                "exec": {
                    "restrictToWorkspace": true,
                    "timeout": 30
                }
            }
        }"#);

        let config = load_config_from_path(file.path());
        assert!(config.tools.restrict_to_workspace);
        assert_eq!(config.tools.exec.timeout, 30);
    }

    #[test]
    fn test_migrate_no_overwrite() {
        let file = write_temp_json(r#"{
            "tools": {
                "restrictToWorkspace": false,
                "exec": {
                    "restrictToWorkspace": true
                }
            }
        }"#);

        let config = load_config_from_path(file.path());
        // Existing value should NOT be overwritten by migration
        assert!(!config.tools.restrict_to_workspace);
    }

    #[test]
    fn test_env_override_model() {
        // Set env var, apply overrides
        std::env::set_var("OXIBOT_AGENTS__DEFAULTS__MODEL", "test-model");
        let config = apply_env_overrides(Config::default());
        assert_eq!(config.agents.defaults.model, "test-model");
        std::env::remove_var("OXIBOT_AGENTS__DEFAULTS__MODEL");
    }

    #[test]
    fn test_env_override_provider_key() {
        std::env::set_var("OXIBOT_PROVIDERS__ANTHROPIC__API_KEY", "sk-env-key");
        let config = apply_env_overrides(Config::default());
        assert_eq!(config.providers.anthropic.api_key, "sk-env-key");
        std::env::remove_var("OXIBOT_PROVIDERS__ANTHROPIC__API_KEY");
    }

    #[test]
    fn test_env_override_gateway_port() {
        std::env::set_var("OXIBOT_GATEWAY__PORT", "9999");
        let config = apply_env_overrides(Config::default());
        assert_eq!(config.gateway.port, 9999);
        std::env::remove_var("OXIBOT_GATEWAY__PORT");
    }

    #[test]
    fn test_saved_json_uses_camel_case() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        save_config(&Config::default(), Some(&path)).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert!(raw["agents"]["defaults"].get("maxTokens").is_some());
        assert!(raw["agents"]["defaults"].get("max_tokens").is_none());
    }

    #[test]
    fn test_full_config_with_providers() {
        let file = write_temp_json(r#"{
            "providers": {
                "anthropic": { "apiKey": "sk-ant-123" },
                "openrouter": { "apiKey": "sk-or-456", "apiBase": "https://custom.io/v1" },
                "deepseek": { "apiKey": "ds-789" }
            },
            "agents": {
                "defaults": {
                    "model": "claude-sonnet-4-20250514",
                    "maxTokens": 4096,
                    "temperature": 0.5
                }
            }
        }"#);

        let config = load_config_from_path(file.path());
        assert!(config.providers.anthropic.is_configured());
        assert!(config.providers.openrouter.is_configured());
        assert_eq!(
            config.providers.openrouter.api_base.as_deref(),
            Some("https://custom.io/v1")
        );
        assert!(config.providers.deepseek.is_configured());
        assert!(!config.providers.openai.is_configured());
    }
}
