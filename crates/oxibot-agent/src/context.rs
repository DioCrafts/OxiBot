//! Context builder — constructs the system prompt and conversation messages.
//!
//! Port of nanobot's `agent/context.py`.
//! Builds the system prompt from identity, bootstrap files, memory, and skills,
//! then assembles the full message list for an LLM call.

use std::path::PathBuf;

use chrono::Utc;
use oxibot_core::types::{ContentPart, ImageUrl, Message};
use tracing::debug;

use crate::memory::MemoryStore;
use crate::skills::SkillsLoader;

// ─────────────────────────────────────────────
// Bootstrap / identity files
// ─────────────────────────────────────────────

/// Files that are automatically injected into the system prompt when present
/// in the workspace root.
const BOOTSTRAP_FILES: &[&str] = &[
    "AGENTS.md",
    "SOUL.md",
    "USER.md",
    "TOOLS.md",
    "IDENTITY.md",
];

// ─────────────────────────────────────────────
// Context builder
// ─────────────────────────────────────────────

/// Builds system prompts and conversation message lists for the agent loop.
pub struct ContextBuilder {
    /// Root workspace directory.
    workspace: PathBuf,
    /// Agent identity name (for the system prompt).
    agent_name: String,
    /// Memory store for long-term + daily notes.
    memory: MemoryStore,
    /// Skills loader for discovering and loading skill files.
    skills: SkillsLoader,
}

impl ContextBuilder {
    /// Create a new context builder.
    pub fn new(workspace: impl Into<PathBuf>, agent_name: impl Into<String>) -> Self {
        let workspace = workspace.into();
        let memory = MemoryStore::new_lazy(&workspace);
        let skills = SkillsLoader::new(&workspace, None);
        Self {
            workspace,
            agent_name: agent_name.into(),
            memory,
            skills,
        }
    }

    /// Set the built-in skills directory (builder pattern).
    pub fn with_builtin_skills(mut self, path: PathBuf) -> Self {
        self.skills = SkillsLoader::new(&self.workspace, Some(path));
        self
    }

    /// Get a reference to the memory store.
    pub fn memory(&self) -> &MemoryStore {
        &self.memory
    }

    /// Get a reference to the skills loader.
    pub fn skills(&self) -> &SkillsLoader {
        &self.skills
    }

    // ────────────── System prompt ──────────────

    /// Build the full system prompt.
    pub fn build_system_prompt(&self) -> String {
        let mut parts: Vec<String> = Vec::new();

        // 1) Identity
        parts.push(self.build_identity());

        // 2) Bootstrap files
        for filename in BOOTSTRAP_FILES {
            let path = self.workspace.join(filename);
            if path.is_file() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    debug!(file = filename, "loaded bootstrap file");
                    parts.push(format!("## {filename}\n\n{content}"));
                }
            }
        }

        // 3) Memory context (via MemoryStore)
        if let Some(memory) = self.memory.get_memory_context() {
            parts.push(memory);
        }

        // 4) Always-on skills (full body injected)
        let always_skills = self.skills.get_always_skills();
        if !always_skills.is_empty() {
            let always_content = self.skills.load_skills_for_context(&always_skills);
            if !always_content.is_empty() {
                parts.push(format!("# Active Skills\n\n{always_content}"));
            }
        }

        // 5) Skills summary (XML catalogue — agent uses read_file for on-demand loading)
        let skills_summary = self.skills.build_skills_summary();
        if !skills_summary.is_empty() {
            parts.push(format!(
                "# Skills\n\n\
                 The following skills extend your capabilities. \
                 To use a skill, read its SKILL.md file using the `read_file` tool.\n\
                 Skills with available=\"false\" need dependencies installed first.\n\n\
                 {skills_summary}"
            ));
        }

        parts.join("\n\n---\n\n")
    }

    /// Core identity block.
    fn build_identity(&self) -> String {
        let now = Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let workspace = self.workspace.display();
        let memory_file = self.memory.memory_file().display();
        let today = Utc::now().format("%Y-%m-%d");

        format!(
            "# Identity\n\n\
             You are **{name}**, an AI assistant.\n\n\
             - **Date/time**: {now}\n\
             - **Runtime**: Rust on {os}/{arch}\n\
             - **Workspace**: `{workspace}`\n\n\
             You have access to tools. Use them when needed to answer questions, \
             read/write files, run commands, search the web, and more.\n\
             Always prefer using tools over guessing. \
             Be concise and helpful.\n\n\
             ## Memory\n\n\
             When you learn something important about the user or the project, \
             persist it by writing to `{memory_file}` using the `write_file` or `edit_file` tool.\n\
             For daily notes, write to `{workspace}/memory/{today}.md`.",
            name = self.agent_name,
        )
    }

    // ────────────── Message building ──────────────

    /// Build the full message list for an LLM call.
    ///
    /// 1. System prompt
    /// 2. Session history
    /// 3. Current user message
    pub fn build_messages(
        &self,
        history: &[Message],
        user_text: &str,
        media: &[String],
        channel: &str,
        chat_id: &str,
    ) -> Vec<Message> {
        let mut messages = Vec::new();

        // System prompt + session info
        let mut system = self.build_system_prompt();
        system.push_str(&format!(
            "\n\n## Current Session\nChannel: {channel}\nChat ID: {chat_id}"
        ));
        messages.push(Message::system(system));

        // History
        messages.extend_from_slice(history);

        // Current user message
        if media.is_empty() {
            messages.push(Message::user(user_text));
        } else {
            messages.push(build_multimodal_user_message(user_text, media));
        }

        messages
    }

    /// Add a tool result to the message list (convenience wrapper).
    pub fn add_tool_result(messages: &mut Vec<Message>, tool_call_id: &str, result: &str) {
        messages.push(Message::tool_result(tool_call_id, result));
    }

    /// Add an assistant message (with optional tool calls) to the message list.
    pub fn add_assistant_message(
        messages: &mut Vec<Message>,
        content: Option<String>,
        tool_calls: Vec<oxibot_core::types::ToolCall>,
    ) {
        if tool_calls.is_empty() {
            if let Some(text) = content {
                messages.push(Message::assistant(text));
            }
        } else {
            messages.push(Message::assistant_tool_calls(tool_calls));
        }
    }
}

// ─────────────────────────────────────────────
// Multimodal helpers
// ─────────────────────────────────────────────

/// Build a user message with base64-encoded images.
///
/// Audio files are skipped — their transcription is already in the text content.
fn build_multimodal_user_message(text: &str, media_paths: &[String]) -> Message {
    let mut parts = Vec::new();

    for path in media_paths {
        // Skip audio files — transcription text is already in `content`
        if is_audio_extension(path) {
            continue;
        }
        if let Ok(data) = std::fs::read(path) {
            let mime = guess_mime(path);
            let b64 = base64_encode(&data);
            parts.push(ContentPart::ImageUrl {
                image_url: ImageUrl {
                    url: format!("data:{mime};base64,{b64}"),
                    detail: None,
                },
            });
        }
    }

    parts.push(ContentPart::Text {
        text: text.to_string(),
    });

    Message::user_parts(parts)
}

/// Check if a file path has an audio extension.
fn is_audio_extension(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".ogg")
        || lower.ends_with(".oga")
        || lower.ends_with(".opus")
        || lower.ends_with(".mp3")
        || lower.ends_with(".m4a")
        || lower.ends_with(".wav")
        || lower.ends_with(".flac")
        || lower.ends_with(".aac")
        || lower.ends_with(".wma")
        || lower.ends_with(".webm")
}

/// Simple MIME guesser based on extension.
fn guess_mime(path: &str) -> &str {
    let lower = path.to_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "image/jpeg"
    }
}

/// Base64-encode bytes (no padding issues, uses standard alphabet).
fn base64_encode(data: &[u8]) -> String {
    use std::io::Write;
    // Simple base64 encoder without external dependency
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 0x3F) as usize]);
        out.push(ALPHABET[((triple >> 12) & 0x3F) as usize]);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3F) as usize]);
        } else {
            out.push(b'=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3F) as usize]);
        } else {
            out.push(b'=');
        }
    }
    let _ = out.flush();
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guess_mime() {
        assert_eq!(guess_mime("photo.png"), "image/png");
        assert_eq!(guess_mime("photo.PNG"), "image/png");
        assert_eq!(guess_mime("photo.jpg"), "image/jpeg");
        assert_eq!(guess_mime("photo.gif"), "image/gif");
        assert_eq!(guess_mime("photo.webp"), "image/webp");
        assert_eq!(guess_mime("photo.unknown"), "image/jpeg");
    }

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b"Hello"), "SGVsbG8=");
        assert_eq!(base64_encode(b"Hi"), "SGk=");
        assert_eq!(base64_encode(b"ABC"), "QUJD");
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn test_build_identity() {
        let ctx = ContextBuilder::new("/tmp/workspace", "TestBot");
        let identity = ctx.build_identity();
        assert!(identity.contains("TestBot"));
        assert!(identity.contains("/tmp/workspace"));
        assert!(identity.contains("Rust on"));
    }

    #[test]
    fn test_build_system_prompt_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ContextBuilder::new(dir.path(), "Oxibot");
        let prompt = ctx.build_system_prompt();
        assert!(prompt.contains("Oxibot"));
        // No bootstrap files → no "---" separator for them
    }

    #[test]
    fn test_build_system_prompt_with_bootstrap() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "# Agent config\nBe helpful.").unwrap();
        let ctx = ContextBuilder::new(dir.path(), "Oxibot");
        let prompt = ctx.build_system_prompt();
        assert!(prompt.contains("Be helpful."));
        assert!(prompt.contains("## AGENTS.md"));
    }

    #[test]
    fn test_build_system_prompt_with_memory() {
        let dir = tempfile::tempdir().unwrap();
        let mem_dir = dir.path().join("memory");
        std::fs::create_dir(&mem_dir).unwrap();
        std::fs::write(mem_dir.join("MEMORY.md"), "User prefers dark mode.").unwrap();
        let ctx = ContextBuilder::new(dir.path(), "Oxibot");
        let prompt = ctx.build_system_prompt();
        assert!(prompt.contains("User prefers dark mode."));
        assert!(prompt.contains("Long-term Memory"));
    }

    #[test]
    fn test_build_messages_text_only() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ContextBuilder::new(dir.path(), "Oxibot");
        let history = vec![
            Message::user("previous question"),
            Message::assistant("previous answer"),
        ];
        let msgs = ctx.build_messages(&history, "new question", &[], "cli", "direct");
        // system + 2 history + 1 user = 4
        assert_eq!(msgs.len(), 4);
    }

    #[test]
    fn test_build_messages_with_session_info() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ContextBuilder::new(dir.path(), "Oxibot");
        let msgs = ctx.build_messages(&[], "hello", &[], "telegram", "chat_42");
        // The system message should contain channel/chat info
        if let Message::System { content } = &msgs[0] {
            assert!(content.contains("Channel: telegram"));
            assert!(content.contains("Chat ID: chat_42"));
        } else {
            panic!("First message should be System");
        }
    }

    #[test]
    fn test_add_tool_result() {
        let mut msgs = vec![Message::user("test")];
        ContextBuilder::add_tool_result(&mut msgs, "call_1", "result data");
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn test_add_assistant_message_text() {
        let mut msgs = Vec::new();
        ContextBuilder::add_assistant_message(&mut msgs, Some("hello".into()), vec![]);
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn test_add_assistant_message_tool_calls() {
        use oxibot_core::types::ToolCall;
        let mut msgs = Vec::new();
        let tc = ToolCall::new("id1", "read_file", r#"{"path":"foo"}"#);
        ContextBuilder::add_assistant_message(&mut msgs, None, vec![tc]);
        assert_eq!(msgs.len(), 1);
    }
}
