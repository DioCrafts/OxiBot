//! Tool Registry — thread-safe store matching nanobot's `ToolRegistry`.
//!
//! The agent loop registers tools here and dispatches LLM tool-call requests
//! by name.

use std::collections::HashMap;
use std::sync::Arc;

use oxibot_core::types::ToolDefinition;
use tracing::{info, warn};

use super::base::Tool;

// ─────────────────────────────────────────────
// Registry
// ─────────────────────────────────────────────

/// Stores tools keyed by name and dispatches calls.
///
/// Owns `Arc<dyn Tool>` so tools can be shared across threads.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool. Overwrites any previous tool with the same name.
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        info!(tool = tool.name(), "registered tool");
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Unregister a tool by name. Returns the removed tool, if any.
    pub fn unregister(&mut self, name: &str) -> Option<Arc<dyn Tool>> {
        let removed = self.tools.remove(name);
        if removed.is_some() {
            info!(tool = name, "unregistered tool");
        }
        removed
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// Check if a tool is registered.
    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Names of all registered tools, sorted for determinism.
    pub fn tool_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort();
        names
    }

    /// Get the LLM-facing definitions for all registered tools.
    pub fn get_definitions(&self) -> Vec<ToolDefinition> {
        let mut defs: Vec<ToolDefinition> = self.tools.values().map(|t| t.to_definition()).collect();
        defs.sort_by(|a, b| a.function.name.cmp(&b.function.name));
        defs
    }

    /// Execute a tool by name with the given parameters.
    ///
    /// Mirrors nanobot's error-string convention: the LLM always gets a
    /// `String` back, even on failure.
    pub async fn execute(&self, name: &str, params: HashMap<String, serde_json::Value>) -> String {
        let tool = match self.tools.get(name) {
            Some(t) => t,
            None => {
                warn!(tool = name, "tool not found");
                return format!("Error: Tool '{name}' not found");
            }
        };

        match tool.execute(params).await {
            Ok(result) => result,
            Err(e) => {
                warn!(tool = name, error = %e, "tool execution failed");
                format!("Error executing {name}: {e}")
            }
        }
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;

    /// Minimal test tool.
    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echoes back the input"
        }
        fn parameters(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Text to echo" }
                },
                "required": ["text"]
            })
        }
        async fn execute(&self, params: HashMap<String, serde_json::Value>) -> anyhow::Result<String> {
            let text = params
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("(empty)");
            Ok(format!("Echo: {text}"))
        }
    }

    /// Tool that always fails.
    struct FailTool;

    #[async_trait]
    impl Tool for FailTool {
        fn name(&self) -> &str {
            "fail"
        }
        fn description(&self) -> &str {
            "Always fails"
        }
        fn parameters(&self) -> serde_json::Value {
            json!({"type": "object", "properties": {}, "required": []})
        }
        async fn execute(&self, _params: HashMap<String, serde_json::Value>) -> anyhow::Result<String> {
            anyhow::bail!("intentional failure")
        }
    }

    #[test]
    fn test_register_and_lookup() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(EchoTool));
        assert!(reg.has("echo"));
        assert!(!reg.has("nope"));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn test_unregister() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(EchoTool));
        assert!(reg.unregister("echo").is_some());
        assert!(!reg.has("echo"));
        assert!(reg.is_empty());
    }

    #[test]
    fn test_tool_names_sorted() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(FailTool));
        reg.register(Arc::new(EchoTool));
        assert_eq!(reg.tool_names(), vec!["echo", "fail"]);
    }

    #[test]
    fn test_get_definitions() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(EchoTool));
        let defs = reg.get_definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].function.name, "echo");
        assert_eq!(defs[0].tool_type, "function");
    }

    #[tokio::test]
    async fn test_execute_success() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(EchoTool));
        let mut params = HashMap::new();
        params.insert("text".into(), json!("hello"));
        let result = reg.execute("echo", params).await;
        assert_eq!(result, "Echo: hello");
    }

    #[tokio::test]
    async fn test_execute_not_found() {
        let reg = ToolRegistry::new();
        let result = reg.execute("missing", HashMap::new()).await;
        assert!(result.starts_with("Error: Tool 'missing' not found"));
    }

    #[tokio::test]
    async fn test_execute_error_caught() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(FailTool));
        let result = reg.execute("fail", HashMap::new()).await;
        assert!(result.starts_with("Error executing fail:"));
        assert!(result.contains("intentional failure"));
    }

    #[test]
    fn test_default() {
        let reg = ToolRegistry::default();
        assert!(reg.is_empty());
    }
}
