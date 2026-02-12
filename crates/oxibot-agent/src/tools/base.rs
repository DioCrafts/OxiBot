//! Tool trait — the abstract interface every agent tool must implement.
//!
//! Port of nanobot's `agent/tools/base.py` `Tool` ABC.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

use oxibot_core::types::ToolDefinition;

// ─────────────────────────────────────────────
// Tool trait
// ─────────────────────────────────────────────

/// Every agent tool implements this trait.
///
/// The agent loop discovers tools via `name()`, sends their schemas to the LLM
/// via `to_definition()`, and dispatches calls via `execute()`.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique name used by the LLM to call this tool (e.g. `"read_file"`).
    fn name(&self) -> &str;

    /// Human-readable description shown to the LLM.
    fn description(&self) -> &str;

    /// JSON Schema describing the parameters (as a `serde_json::Value`).
    ///
    /// Must be `{"type": "object", "properties": {...}, "required": [...]}`.
    fn parameters(&self) -> Value;

    /// Execute the tool with the given arguments.
    ///
    /// Returns the tool output as a string (the LLM reads this).
    /// On failure, return an `Err` — the registry will catch it and
    /// convert to an error string for the LLM.
    async fn execute(&self, params: HashMap<String, Value>) -> anyhow::Result<String>;

    /// Build the `ToolDefinition` sent to the LLM.
    ///
    /// Default implementation — rarely needs overriding.
    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition::new(self.name(), self.description(), self.parameters())
    }
}

// ─────────────────────────────────────────────
// Param helpers
// ─────────────────────────────────────────────

/// Extract a required `String` param, returning a user-friendly error.
pub fn require_string(params: &HashMap<String, Value>, key: &str) -> anyhow::Result<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: {key}"))
}

/// Extract an optional `String` param.
pub fn optional_string(params: &HashMap<String, Value>, key: &str) -> Option<String> {
    params.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// Extract an optional integer param.
pub fn optional_i64(params: &HashMap<String, Value>, key: &str) -> Option<i64> {
    params.get(key).and_then(|v| v.as_i64())
}

/// Extract an optional boolean param (defaults to `false` if absent).
pub fn optional_bool(params: &HashMap<String, Value>, key: &str) -> bool {
    params
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_require_string_present() {
        let mut params = HashMap::new();
        params.insert("path".into(), json!("/tmp/foo.txt"));
        assert_eq!(require_string(&params, "path").unwrap(), "/tmp/foo.txt");
    }

    #[test]
    fn test_require_string_missing() {
        let params = HashMap::new();
        assert!(require_string(&params, "path").is_err());
    }

    #[test]
    fn test_require_string_wrong_type() {
        let mut params = HashMap::new();
        params.insert("path".into(), json!(42));
        assert!(require_string(&params, "path").is_err());
    }

    #[test]
    fn test_optional_string() {
        let mut params = HashMap::new();
        params.insert("mode".into(), json!("markdown"));
        assert_eq!(optional_string(&params, "mode"), Some("markdown".into()));
        assert_eq!(optional_string(&params, "other"), None);
    }

    #[test]
    fn test_optional_i64() {
        let mut params = HashMap::new();
        params.insert("count".into(), json!(5));
        assert_eq!(optional_i64(&params, "count"), Some(5));
        assert_eq!(optional_i64(&params, "missing"), None);
    }

    #[test]
    fn test_optional_bool() {
        let mut params = HashMap::new();
        params.insert("force".into(), json!(true));
        assert!(optional_bool(&params, "force"));
        assert!(!optional_bool(&params, "missing"));
    }

    /// Verify the default `to_definition()` produces the right shape.
    #[tokio::test]
    async fn test_to_definition_default() {
        struct DummyTool;

        #[async_trait]
        impl Tool for DummyTool {
            fn name(&self) -> &str { "dummy" }
            fn description(&self) -> &str { "A test tool" }
            fn parameters(&self) -> Value {
                json!({
                    "type": "object",
                    "properties": {
                        "msg": { "type": "string" }
                    },
                    "required": ["msg"]
                })
            }
            async fn execute(&self, _params: HashMap<String, Value>) -> anyhow::Result<String> {
                Ok("ok".into())
            }
        }

        let def = DummyTool.to_definition();
        assert_eq!(def.function.name, "dummy");
        assert_eq!(def.function.description, "A test tool");
        assert_eq!(def.tool_type, "function");
    }
}
