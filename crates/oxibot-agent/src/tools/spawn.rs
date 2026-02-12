//! Spawn tool — delegate tasks to background subagents.
//!
//! Port of nanobot's `agent/tools/spawn.py`.
//!
//! When the LLM calls this tool, a subagent is spawned via `tokio::spawn`
//! with an isolated context, limited tools, and its own message history.
//! The tool returns an immediate confirmation to the LLM; when the
//! subagent finishes, it announces the result back via the message bus.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use super::base::{optional_string, require_string, Tool};
use crate::subagent::SubagentManager;

// ─────────────────────────────────────────────
// SpawnTool
// ─────────────────────────────────────────────

/// Tool that allows the agent to spawn background subagent tasks.
///
/// The agent loop calls `set_context` before each interaction to set
/// the current channel/chat_id so subagent results route back correctly.
pub struct SpawnTool {
    /// Reference to the subagent manager.
    manager: Arc<SubagentManager>,
    /// Current origin context (channel, chat_id) — set per-interaction.
    context: Mutex<(String, String)>,
}

impl SpawnTool {
    /// Create a new spawn tool.
    pub fn new(manager: Arc<SubagentManager>) -> Self {
        Self {
            manager,
            context: Mutex::new(("cli".into(), "direct".into())),
        }
    }

    /// Set the current context (called by the agent loop per-message).
    ///
    /// This ensures subagent results are routed back to the correct
    /// channel/chat that originated the spawn request.
    pub async fn set_context(&self, channel: &str, chat_id: &str) {
        let mut ctx = self.context.lock().await;
        *ctx = (channel.to_string(), chat_id.to_string());
    }
}

#[async_trait]
impl Tool for SpawnTool {
    fn name(&self) -> &str {
        "spawn"
    }

    fn description(&self) -> &str {
        "Spawn a subagent to handle a task in the background. Use this for complex \
         or time-consuming tasks that can run independently. The subagent will \
         complete the task and report back when done."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The task for the subagent to complete"
                },
                "label": {
                    "type": "string",
                    "description": "Optional short label for the task (for display)"
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, params: HashMap<String, Value>) -> anyhow::Result<String> {
        let task = require_string(&params, "task")?;
        let label = optional_string(&params, "label");

        let ctx = self.context.lock().await;
        let origin_channel = ctx.0.clone();
        let origin_chat_id = ctx.1.clone();
        drop(ctx);

        let confirmation = self
            .manager
            .spawn(task, label, origin_channel, origin_chat_id)
            .await;

        Ok(confirmation)
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_loop::ExecToolConfig;
    use async_trait::async_trait;
    use oxibot_core::bus::queue::MessageBus;
    use oxibot_core::types::{LlmResponse, Message, ToolDefinition};
    use oxibot_providers::traits::{LlmProvider, LlmRequestConfig};

    struct MockProvider;

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn chat(
            &self,
            _messages: &[Message],
            _tools: Option<&[ToolDefinition]>,
            _model: &str,
            _config: &LlmRequestConfig,
        ) -> LlmResponse {
            LlmResponse {
                content: Some("Subagent done.".into()),
                ..Default::default()
            }
        }

        fn default_model(&self) -> &str {
            "mock"
        }

        fn display_name(&self) -> &str {
            "Mock"
        }
    }

    fn create_test_spawn_tool() -> SpawnTool {
        let bus = Arc::new(MessageBus::new(32));
        let workspace = std::env::temp_dir().join("oxibot_test_spawn_tool");
        let _ = std::fs::create_dir_all(&workspace);

        let mgr = Arc::new(SubagentManager::new(
            Arc::new(MockProvider),
            workspace,
            bus,
            "mock".into(),
            None,
            ExecToolConfig::default(),
            false,
            LlmRequestConfig::default(),
        ));

        SpawnTool::new(mgr)
    }

    #[test]
    fn test_spawn_tool_name() {
        let tool = create_test_spawn_tool();
        assert_eq!(tool.name(), "spawn");
    }

    #[test]
    fn test_spawn_tool_description() {
        let tool = create_test_spawn_tool();
        assert!(tool.description().contains("subagent"));
        assert!(tool.description().contains("background"));
    }

    #[test]
    fn test_spawn_tool_parameters_schema() {
        let tool = create_test_spawn_tool();
        let params = tool.parameters();

        assert_eq!(params["type"], "object");
        assert!(params["properties"]["task"].is_object());
        assert!(params["properties"]["label"].is_object());

        let required = params["required"].as_array().unwrap();
        assert!(required.contains(&json!("task")));
        assert!(!required.contains(&json!("label")));
    }

    #[test]
    fn test_spawn_tool_definition() {
        let tool = create_test_spawn_tool();
        let def = tool.to_definition();

        assert_eq!(def.function.name, "spawn");
        assert!(def.function.description.contains("subagent"));
    }

    #[tokio::test]
    async fn test_spawn_tool_set_context() {
        let tool = create_test_spawn_tool();

        tool.set_context("telegram", "chat_42").await;

        let ctx = tool.context.lock().await;
        assert_eq!(ctx.0, "telegram");
        assert_eq!(ctx.1, "chat_42");
    }

    #[tokio::test]
    async fn test_spawn_tool_execute() {
        let tool = create_test_spawn_tool();
        tool.set_context("discord", "guild_1").await;

        let mut params = HashMap::new();
        params.insert("task".into(), json!("Find all TODO items in the codebase"));
        params.insert("label".into(), json!("todos"));

        let result = tool.execute(params).await.unwrap();
        assert!(result.contains("Subagent [todos] started"));
        assert!(result.contains("I'll notify you when it completes"));
    }

    #[tokio::test]
    async fn test_spawn_tool_execute_no_label() {
        let tool = create_test_spawn_tool();

        let mut params = HashMap::new();
        params.insert("task".into(), json!("Short task"));

        let result = tool.execute(params).await.unwrap();
        assert!(result.contains("Subagent [Short task] started"));
    }

    #[tokio::test]
    async fn test_spawn_tool_execute_missing_task() {
        let tool = create_test_spawn_tool();
        let params = HashMap::new();

        let result = tool.execute(params).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_spawn_tool_default_context() {
        let tool = create_test_spawn_tool();

        // Without calling set_context, defaults to cli/direct
        let ctx = tool.context.lock().await;
        assert_eq!(ctx.0, "cli");
        assert_eq!(ctx.1, "direct");
    }
}
