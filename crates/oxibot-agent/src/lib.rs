//! Oxibot Agent — core loop, tools, and context builder.
//!
//! This crate contains:
//! - **tools**: Tool trait, registry, and built-in tools (filesystem, shell, web, message)
//! - **context**: System prompt and message list construction
//! - **agent_loop**: The LLM ↔ tool-calling main loop

pub mod tools;
pub mod context;
pub mod memory;
pub mod skills;
pub mod subagent;
pub mod agent_loop;

pub use agent_loop::{AgentLoop, ExecToolConfig};
pub use context::ContextBuilder;
pub use memory::MemoryStore;
pub use skills::SkillsLoader;
pub use subagent::SubagentManager;
pub use tools::{Tool, ToolRegistry};
