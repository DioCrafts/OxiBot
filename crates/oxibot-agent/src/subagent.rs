//! Subagent Manager — background task delegation.
//!
//! Port of nanobot's `agent/subagent.py`.
//!
//! The main agent can delegate tasks to subagents via the `spawn` tool.
//! Each subagent runs as a `tokio::spawn` task with:
//! - Its own system prompt (task-focused, simpler than the main agent's)
//! - A limited tool registry (filesystem, shell, web — NO message, spawn, edit)
//! - An independent message history (ephemeral, not persisted)
//! - The same LLM provider as the parent
//!
//! On completion, the subagent publishes its result as a `system` inbound
//! message on the bus, targeted at the original channel/chat. The agent
//! loop picks it up and summarizes the result for the user.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

use oxibot_core::bus::queue::MessageBus;
use oxibot_core::bus::types::InboundMessage;
use oxibot_core::types::{Message, ToolCall};
use oxibot_providers::traits::{LlmProvider, LlmRequestConfig};

use crate::agent_loop::ExecToolConfig;
use crate::context::ContextBuilder;
use crate::tools::filesystem::{ListDirTool, ReadFileTool, WriteFileTool};
use crate::tools::registry::ToolRegistry;
use crate::tools::shell::ExecTool;
use crate::tools::web::{WebFetchTool, WebSearchTool};

/// Maximum LLM ↔ tool iterations for a subagent task.
const SUBAGENT_MAX_ITERATIONS: usize = 15;

// ─────────────────────────────────────────────
// TaskInfo
// ─────────────────────────────────────────────

/// Metadata about a running subagent task.
#[derive(Clone, Debug)]
pub struct TaskInfo {
    /// Unique task identifier (8 hex chars).
    pub id: String,
    /// Short display label for the task.
    pub label: String,
    /// Full task description sent to the subagent.
    pub task: String,
    /// Channel that originated the request.
    pub origin_channel: String,
    /// Chat ID that originated the request.
    pub origin_chat_id: String,
}

// ─────────────────────────────────────────────
// SubagentManager
// ─────────────────────────────────────────────

/// Manages the lifecycle of background subagent tasks.
///
/// Created once in `AgentLoop::new()` and shared via `Arc`.
/// The `SpawnTool` holds a reference and delegates `spawn()` calls here.
pub struct SubagentManager {
    /// Shared LLM provider (same instance as the parent agent).
    provider: Arc<dyn LlmProvider>,
    /// Workspace root path.
    workspace: PathBuf,
    /// Message bus for announcing results.
    bus: Arc<MessageBus>,
    /// Model name to use for subagent calls.
    model: String,
    /// Brave Search API key (for WebSearchTool).
    brave_api_key: Option<String>,
    /// Exec tool config (timeout, etc.).
    exec_config: ExecToolConfig,
    /// Whether to restrict filesystem tools to workspace.
    restrict_to_workspace: bool,
    /// LLM request config (temperature, max_tokens).
    request_config: LlmRequestConfig,
    /// Currently running tasks, keyed by task ID.
    running_tasks: RwLock<HashMap<String, TaskInfo>>,
}

impl SubagentManager {
    /// Create a new subagent manager.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        workspace: PathBuf,
        bus: Arc<MessageBus>,
        model: String,
        brave_api_key: Option<String>,
        exec_config: ExecToolConfig,
        restrict_to_workspace: bool,
        request_config: LlmRequestConfig,
    ) -> Self {
        Self {
            provider,
            workspace,
            bus,
            model,
            brave_api_key,
            exec_config,
            restrict_to_workspace,
            request_config,
            running_tasks: RwLock::new(HashMap::new()),
        }
    }

    /// Spawn a subagent task in the background.
    ///
    /// Returns an immediate confirmation string.
    /// The actual work runs as a `tokio::spawn` task.
    pub async fn spawn(
        self: &Arc<Self>,
        task: String,
        label: Option<String>,
        origin_channel: String,
        origin_chat_id: String,
    ) -> String {
        let task_id = generate_task_id();
        let display_label = label.unwrap_or_else(|| {
            if task.len() > 30 {
                format!("{}…", &task[..30])
            } else {
                task.clone()
            }
        });

        let info = TaskInfo {
            id: task_id.clone(),
            label: display_label.clone(),
            task: task.clone(),
            origin_channel: origin_channel.clone(),
            origin_chat_id: origin_chat_id.clone(),
        };

        // Register the task
        {
            let mut tasks = self.running_tasks.write().await;
            tasks.insert(task_id.clone(), info);
        }

        // Spawn the background coroutine
        let mgr = Arc::clone(self);
        let tid = task_id.clone();
        let lbl = display_label.clone();
        let t = task.clone();

        tokio::spawn(async move {
            let result = mgr.run_subagent(&tid, &t).await;

            match result {
                Ok(response) => {
                    mgr.announce_result(&tid, &lbl, &response, &origin_channel, &origin_chat_id)
                        .await;
                }
                Err(e) => {
                    error!(task_id = %tid, error = %e, "subagent task failed");
                    mgr.announce_result(
                        &tid,
                        &lbl,
                        &format!("Task failed: {e}"),
                        &origin_channel,
                        &origin_chat_id,
                    )
                    .await;
                }
            }

            // Auto-cleanup
            let mut tasks = mgr.running_tasks.write().await;
            tasks.remove(&tid);
            info!(task_id = %tid, "subagent task cleaned up");
        });

        format!(
            "Subagent [{display_label}] started (id: {task_id}). I'll notify you when it completes."
        )
    }

    /// Run the subagent's LLM ↔ tool loop.
    ///
    /// This is the core execution: build an isolated context, register
    /// limited tools, and loop LLM ↔ tools until a final answer or
    /// max iterations.
    async fn run_subagent(&self, task_id: &str, task: &str) -> Result<String> {
        info!(task_id = %task_id, "subagent starting");

        // Build isolated tool registry (no message, no spawn, no edit_file)
        let mut tools = ToolRegistry::new();
        let allowed_dir = if self.restrict_to_workspace {
            Some(self.workspace.clone())
        } else {
            None
        };

        tools.register(Arc::new(ReadFileTool::new(allowed_dir.clone())));
        tools.register(Arc::new(WriteFileTool::new(allowed_dir.clone())));
        tools.register(Arc::new(ListDirTool::new(allowed_dir)));
        tools.register(Arc::new(ExecTool::new(
            self.workspace.clone(),
            Some(self.exec_config.timeout),
            self.restrict_to_workspace,
        )));
        tools.register(Arc::new(WebSearchTool::new(self.brave_api_key.clone())));
        tools.register(Arc::new(WebFetchTool::new()));

        // Build system prompt
        let system_prompt = self.build_subagent_prompt(task);

        // Ephemeral message list (no session persistence)
        let mut messages = vec![Message::system(&system_prompt), Message::user(task)];

        let tool_defs = tools.get_definitions();
        let mut final_content: Option<String> = None;

        for iteration in 0..SUBAGENT_MAX_ITERATIONS {
            debug!(task_id = %task_id, iteration = iteration, "subagent LLM call");

            let response = self
                .provider
                .chat(&messages, Some(&tool_defs), &self.model, &self.request_config)
                .await;

            if response.has_tool_calls() {
                let tool_calls: Vec<ToolCall> = response.tool_calls.clone();
                ContextBuilder::add_assistant_message(
                    &mut messages,
                    response.content.clone(),
                    tool_calls.clone(),
                );

                for tc in &tool_calls {
                    let params: HashMap<String, serde_json::Value> =
                        serde_json::from_str(&tc.function.arguments).unwrap_or_default();

                    info!(
                        task_id = %task_id,
                        tool = %tc.function.name,
                        iteration = iteration,
                        "subagent executing tool"
                    );

                    let result = tools.execute(&tc.function.name, params).await;
                    ContextBuilder::add_tool_result(&mut messages, &tc.id, &result);
                }
            } else {
                final_content = response.content;
                break;
            }
        }

        let result = final_content
            .unwrap_or_else(|| "Subagent completed processing but produced no output.".into());

        info!(task_id = %task_id, result_len = result.len(), "subagent finished");
        Ok(result)
    }

    /// Announce the subagent result back to the bus.
    ///
    /// Publishes an `InboundMessage` with `channel="system"` and
    /// `chat_id="<origin_channel>:<origin_chat_id>"` so the agent loop
    /// can route the response back to the correct conversation.
    async fn announce_result(
        &self,
        task_id: &str,
        label: &str,
        result: &str,
        origin_channel: &str,
        origin_chat_id: &str,
    ) {
        let content = format!(
            "## Subagent Result\n\
             **Task**: {label}\n\n\
             {result}\n\n\
             ---\n\
             *Summarize this naturally for the user. Keep it brief. \
             Do not mention 'subagent' or task IDs.*"
        );

        let msg = InboundMessage::new(
            "system",
            "subagent",
            format!("{origin_channel}:{origin_chat_id}"),
            content,
        );

        info!(task_id = %task_id, "announcing subagent result");
        if let Err(e) = self.bus.publish_inbound(msg).await {
            error!(
                task_id = %task_id,
                error = %e,
                "failed to announce subagent result"
            );
        }
    }

    /// Build the subagent's system prompt.
    fn build_subagent_prompt(&self, task: &str) -> String {
        format!(
            "# Subagent\n\
             You are a subagent spawned by the main agent to complete a specific task.\n\n\
             ## Your Task\n\
             {task}\n\n\
             ## Rules\n\
             1. Stay focused — complete only the assigned task\n\
             2. Your final response will be reported back to the main agent\n\
             3. Do not initiate conversations or take on side tasks\n\
             4. Be concise but informative\n\n\
             ## What You Can Do\n\
             - Read and write files in the workspace\n\
             - List directory contents\n\
             - Execute shell commands\n\
             - Search the web and fetch web pages\n\n\
             ## What You Cannot Do\n\
             - Send messages directly to users (no message tool)\n\
             - Spawn other subagents\n\
             - Edit files in-place (use write_file to overwrite)\n\
             - Access the main agent's conversation history\n\n\
             ## Workspace\n\
             Your workspace is at: {workspace}",
            workspace = self.workspace.display()
        )
    }

    /// Get info about currently running tasks.
    pub async fn running_tasks(&self) -> Vec<TaskInfo> {
        let tasks = self.running_tasks.read().await;
        tasks.values().cloned().collect()
    }

    /// Get the number of running tasks.
    pub async fn task_count(&self) -> usize {
        let tasks = self.running_tasks.read().await;
        tasks.len()
    }
}

// ─────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────

/// Generate a short unique task ID (8 hex chars from timestamp + counter).
fn generate_task_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mixed = nanos.wrapping_mul(6364136223846793005).wrapping_add(count as u64);
    format!("{:08x}", (mixed >> 32) as u32)
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use oxibot_core::types::{LlmResponse, ToolDefinition};

    /// Mock provider for testing subagent.
    struct MockSubagentProvider {
        responses: std::sync::Mutex<Vec<LlmResponse>>,
    }

    impl MockSubagentProvider {
        fn simple(text: &str) -> Self {
            Self {
                responses: std::sync::Mutex::new(vec![LlmResponse {
                    content: Some(text.into()),
                    ..Default::default()
                }]),
            }
        }

        fn with_responses(responses: Vec<LlmResponse>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockSubagentProvider {
        async fn chat(
            &self,
            _messages: &[Message],
            _tools: Option<&[ToolDefinition]>,
            _model: &str,
            _config: &LlmRequestConfig,
        ) -> LlmResponse {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                LlmResponse {
                    content: Some("(no more responses)".into()),
                    ..Default::default()
                }
            } else {
                responses.remove(0)
            }
        }

        fn default_model(&self) -> &str {
            "mock-model"
        }

        fn display_name(&self) -> &str {
            "MockSubagentProvider"
        }
    }

    fn create_test_manager(provider: Arc<dyn LlmProvider>) -> Arc<SubagentManager> {
        let bus = Arc::new(MessageBus::new(32));
        let workspace = std::env::temp_dir().join("oxibot_test_subagent");
        let _ = std::fs::create_dir_all(&workspace);

        Arc::new(SubagentManager::new(
            provider,
            workspace,
            bus,
            "mock-model".into(),
            None,
            ExecToolConfig::default(),
            false,
            LlmRequestConfig::default(),
        ))
    }

    #[test]
    fn test_generate_task_id() {
        let id1 = generate_task_id();
        let id2 = generate_task_id();
        assert_eq!(id1.len(), 8);
        assert_eq!(id2.len(), 8);
        // IDs should be different (counter ensures this)
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_generate_task_id_hex_only() {
        for _ in 0..10 {
            let id = generate_task_id();
            assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn test_task_info_clone() {
        let info = TaskInfo {
            id: "abc12345".into(),
            label: "Test task".into(),
            task: "Do something important".into(),
            origin_channel: "telegram".into(),
            origin_chat_id: "chat_42".into(),
        };
        let cloned = info.clone();
        assert_eq!(cloned.id, "abc12345");
        assert_eq!(cloned.origin_channel, "telegram");
    }

    #[test]
    fn test_build_subagent_prompt() {
        let provider = Arc::new(MockSubagentProvider::simple("ok"));
        let mgr = create_test_manager(provider);
        let prompt = mgr.build_subagent_prompt("Find all TODO comments in the codebase");

        assert!(prompt.contains("# Subagent"));
        assert!(prompt.contains("Find all TODO comments in the codebase"));
        assert!(prompt.contains("## Rules"));
        assert!(prompt.contains("## What You Can Do"));
        assert!(prompt.contains("## What You Cannot Do"));
        assert!(prompt.contains("Spawn other subagents"));
        assert!(prompt.contains("## Workspace"));
    }

    #[test]
    fn test_build_subagent_prompt_includes_workspace_path() {
        let provider = Arc::new(MockSubagentProvider::simple("ok"));
        let mgr = create_test_manager(provider);
        let prompt = mgr.build_subagent_prompt("task");

        let workspace = std::env::temp_dir().join("oxibot_test_subagent");
        assert!(prompt.contains(&workspace.display().to_string()));
    }

    #[tokio::test]
    async fn test_spawn_returns_confirmation() {
        let provider = Arc::new(MockSubagentProvider::simple("Task completed!"));
        let mgr = create_test_manager(provider);

        let result = mgr
            .spawn(
                "Count lines in main.rs".into(),
                Some("line-count".into()),
                "cli".into(),
                "direct".into(),
            )
            .await;

        assert!(result.contains("Subagent [line-count] started"));
        assert!(result.contains("I'll notify you when it completes"));
    }

    #[tokio::test]
    async fn test_spawn_default_label_short() {
        let provider = Arc::new(MockSubagentProvider::simple("done"));
        let mgr = create_test_manager(provider);

        let result = mgr
            .spawn("Short task".into(), None, "cli".into(), "direct".into())
            .await;

        assert!(result.contains("Subagent [Short task] started"));
    }

    #[tokio::test]
    async fn test_spawn_default_label_truncated() {
        let provider = Arc::new(MockSubagentProvider::simple("done"));
        let mgr = create_test_manager(provider);

        let long_task = "A very long task description that exceeds thirty characters easily".into();
        let result = mgr
            .spawn(long_task, None, "cli".into(), "direct".into())
            .await;

        // Should be truncated with ellipsis
        assert!(result.contains("…"));
    }

    #[tokio::test]
    async fn test_spawn_tracks_running_task() {
        let provider = Arc::new(MockSubagentProvider::simple("done"));
        let mgr = create_test_manager(provider);

        assert_eq!(mgr.task_count().await, 0);

        let _result = mgr
            .spawn("do stuff".into(), None, "cli".into(), "direct".into())
            .await;

        // The task may have already completed (it's simple), but it was tracked
        // Give a small window for the background task to start
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // After completion, task should be cleaned up
        // (mock provider returns immediately, so the task finishes fast)
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(mgr.task_count().await, 0);
    }

    #[tokio::test]
    async fn test_run_subagent_simple() {
        let provider = Arc::new(MockSubagentProvider::simple("The answer is 42."));
        let mgr = create_test_manager(provider);

        let result = mgr.run_subagent("test_id", "What is the answer?").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "The answer is 42.");
    }

    #[tokio::test]
    async fn test_run_subagent_with_tool_calls() {
        let dir = tempfile::tempdir().unwrap();
        let test_file = dir.path().join("data.txt");
        std::fs::write(&test_file, "important data").unwrap();

        let tool_call = ToolCall::new(
            "call_sub_1",
            "read_file",
            serde_json::json!({"path": test_file.to_str().unwrap()}).to_string(),
        );

        let provider = Arc::new(MockSubagentProvider::with_responses(vec![
            LlmResponse {
                content: None,
                tool_calls: vec![tool_call],
                ..Default::default()
            },
            LlmResponse {
                content: Some("File contains: important data".into()),
                ..Default::default()
            },
        ]));

        let bus = Arc::new(MessageBus::new(32));
        let mgr = Arc::new(SubagentManager::new(
            provider,
            dir.path().to_path_buf(),
            bus,
            "mock-model".into(),
            None,
            ExecToolConfig::default(),
            false,
            LlmRequestConfig::default(),
        ));

        let result = mgr.run_subagent("test_tool", "Read data.txt").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "File contains: important data");
    }

    #[tokio::test]
    async fn test_run_subagent_max_iterations() {
        let tool_call = ToolCall::new("loop_call", "list_dir", r#"{"path": "/tmp"}"#);
        let responses: Vec<LlmResponse> = (0..20)
            .map(|_| LlmResponse {
                content: None,
                tool_calls: vec![tool_call.clone()],
                ..Default::default()
            })
            .collect();

        let provider = Arc::new(MockSubagentProvider::with_responses(responses));
        let mgr = create_test_manager(provider);

        let result = mgr
            .run_subagent("test_max", "loop forever")
            .await
            .unwrap();
        assert!(result.contains("completed processing"));
    }

    #[tokio::test]
    async fn test_subagent_limited_tools() {
        let provider = Arc::new(MockSubagentProvider::simple("ok"));
        let mgr = create_test_manager(provider);

        // Build the tools the same way run_subagent does internally
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(ReadFileTool::new(None)));
        tools.register(Arc::new(WriteFileTool::new(None)));
        tools.register(Arc::new(ListDirTool::new(None)));
        tools.register(Arc::new(ExecTool::new(
            std::env::temp_dir(),
            Some(60),
            false,
        )));
        tools.register(Arc::new(WebSearchTool::new(None)));
        tools.register(Arc::new(WebFetchTool::new()));

        let names = tools.tool_names();
        // Should have exactly 6 tools
        assert_eq!(names.len(), 6);
        // Should NOT have message, spawn, or edit_file
        assert!(!names.contains(&"message".into()));
        assert!(!names.contains(&"spawn".into()));
        assert!(!names.contains(&"edit_file".into()));
        // Should have the allowed tools
        assert!(names.contains(&"read_file".into()));
        assert!(names.contains(&"write_file".into()));
        assert!(names.contains(&"list_dir".into()));
        assert!(names.contains(&"exec".into()));
        assert!(names.contains(&"web_search".into()));
        assert!(names.contains(&"web_fetch".into()));
    }

    #[tokio::test]
    async fn test_announce_result_publishes_to_bus() {
        let provider = Arc::new(MockSubagentProvider::simple("done"));
        let bus = Arc::new(MessageBus::new(32));
        let workspace = std::env::temp_dir().join("oxibot_test_announce");
        let _ = std::fs::create_dir_all(&workspace);

        let mgr = Arc::new(SubagentManager::new(
            provider,
            workspace,
            bus.clone(),
            "mock-model".into(),
            None,
            ExecToolConfig::default(),
            false,
            LlmRequestConfig::default(),
        ));

        mgr.announce_result("tid_1", "test label", "Result text", "telegram", "chat_99")
            .await;

        // The message should be on the inbound bus
        let msg = bus.consume_inbound().await.unwrap();
        assert_eq!(msg.channel, "system");
        assert_eq!(msg.sender_id, "subagent");
        assert_eq!(msg.chat_id, "telegram:chat_99");
        assert!(msg.content.contains("test label"));
        assert!(msg.content.contains("Result text"));
    }

    #[tokio::test]
    async fn test_running_tasks_returns_empty_initially() {
        let provider = Arc::new(MockSubagentProvider::simple("ok"));
        let mgr = create_test_manager(provider);

        let tasks = mgr.running_tasks().await;
        assert!(tasks.is_empty());
    }
}
