//! Interactive REPL — replaces nanobot's prompt_toolkit loop.
//!
//! Uses `rustyline` for readline-style editing with persistent history.

use anyhow::Result;
use rustyline::config::Configurer;
use rustyline::history::DefaultHistory;
use rustyline::{DefaultEditor, Editor};
use tracing::debug;

use oxibot_agent::AgentLoop;

use crate::helpers;

/// Exit commands (case-insensitive match).
const EXIT_COMMANDS: &[&str] = &["exit", "quit", "/exit", "/quit", ":q"];

/// Run the interactive REPL loop.
pub async fn run(
    agent: AgentLoop,
    session_id: &str,
    render_markdown: bool,
    _show_logs: bool,
) -> Result<()> {
    helpers::print_banner();

    let mut editor = create_editor()?;

    loop {
        // Read input
        let input = match editor.readline("You: ") {
            Ok(line) => line,
            Err(rustyline::error::ReadlineError::Interrupted) => {
                // Ctrl-C — exit cleanly
                break;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
                // Ctrl-D — exit cleanly
                break;
            }
            Err(e) => {
                eprintln!("Input error: {e}");
                break;
            }
        };

        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Check exit commands
        if is_exit_command(trimmed) {
            println!("\nGoodbye! 👋");
            break;
        }

        // Add to history
        let _ = editor.add_history_entry(&input);

        // Process message
        debug!(session = session_id, input = trimmed, "processing input");
        helpers::print_thinking();

        match agent.process_direct(trimmed).await {
            Ok(response) => {
                helpers::clear_thinking();
                helpers::print_response(&response, render_markdown);
            }
            Err(e) => {
                helpers::clear_thinking();
                eprintln!("\n❌ Error: {e}\n");
            }
        }
    }

    // Save history
    save_history(&mut editor);

    Ok(())
}

/// Create a rustyline editor with history.
fn create_editor() -> Result<Editor<(), DefaultHistory>> {
    let mut editor = DefaultEditor::new()?;
    editor.set_max_history_size(1000)?;

    // Load history from ~/.oxibot/history/cli_history
    let history_path = history_path();
    if history_path.exists() {
        let _ = editor.load_history(&history_path);
        debug!("loaded REPL history from {}", history_path.display());
    }

    Ok(editor)
}

/// Save history to disk.
fn save_history(editor: &mut Editor<(), DefaultHistory>) {
    let path = history_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = editor.save_history(&path) {
        debug!("failed to save history: {e}");
    }
}

/// Path to the history file.
fn history_path() -> std::path::PathBuf {
    oxibot_core::utils::get_data_path().join("history").join("cli_history")
}

/// Check if input is an exit command.
fn is_exit_command(input: &str) -> bool {
    let lower = input.to_lowercase();
    EXIT_COMMANDS.contains(&lower.as_str())
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_commands() {
        assert!(is_exit_command("exit"));
        assert!(is_exit_command("EXIT"));
        assert!(is_exit_command("/quit"));
        assert!(is_exit_command(":q"));
        assert!(!is_exit_command("hello"));
        assert!(!is_exit_command(""));
    }

    #[test]
    fn history_path_under_data_dir() {
        let path = history_path();
        assert!(path.to_string_lossy().contains(".oxibot"));
        assert!(path.to_string_lossy().contains("cli_history"));
    }
}
