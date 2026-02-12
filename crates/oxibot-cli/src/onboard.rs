//! `oxibot onboard` — initialize configuration and workspace.
//!
//! Replaces nanobot's `onboard` command:
//! - Creates `~/.oxibot/config.json` with defaults
//! - Creates workspace directory with template files

use anyhow::Result;
use colored::Colorize;

use oxibot_core::config::{load_config, save_config};
use oxibot_core::utils::{get_data_path, get_default_workspace_path};

/// Run the onboard command.
pub fn run() -> Result<()> {
    println!();
    println!("{}", "🦀 Oxibot — Setup".cyan().bold());
    println!();

    let data_dir = get_data_path();
    let config_path = data_dir.join("config.json");

    // 1. Create config if it doesn't exist
    if config_path.exists() {
        println!(
            "  {} config already exists at {}",
            "✓".green(),
            config_path.display()
        );
    } else {
        let config = load_config(None); // defaults
        save_config(&config, Some(&config_path))?;
        println!(
            "  {} created config at {}",
            "✓".green(),
            config_path.display()
        );
    }

    // 2. Ensure workspace directory
    let workspace = get_default_workspace_path();
    std::fs::create_dir_all(&workspace)?;
    println!(
        "  {} workspace at {}",
        "✓".green(),
        workspace.display()
    );

    // 3. Create memory directory
    let memory_dir = workspace.join("memory");
    std::fs::create_dir_all(&memory_dir)?;
    println!("  {} memory dir at {}", "✓".green(), memory_dir.display());

    // 4. Create template files if they don't exist
    create_template(&workspace.join("AGENTS.md"), AGENTS_TEMPLATE)?;
    create_template(&workspace.join("SOUL.md"), SOUL_TEMPLATE)?;
    create_template(&workspace.join("USER.md"), USER_TEMPLATE)?;
    create_template(&workspace.join("HEARTBEAT.md"), HEARTBEAT_TEMPLATE)?;
    create_template(&memory_dir.join("MEMORY.md"), MEMORY_TEMPLATE)?;

    // 5. Create skills directory with skill-creator
    let skills_dir = workspace.join("skills");
    std::fs::create_dir_all(&skills_dir)?;
    let sc_dir = skills_dir.join("skill-creator");
    if !sc_dir.exists() {
        std::fs::create_dir_all(&sc_dir)?;
        std::fs::write(sc_dir.join("SKILL.md"), SKILL_CREATOR_TEMPLATE)?;
        println!("  {} created skill: skill-creator", "✓".green());
    } else {
        println!("  {} skill-creator already exists", "✓".green());
    }

    // 6. Create sessions + history directories
    let sessions_dir = data_dir.join("sessions");
    std::fs::create_dir_all(&sessions_dir)?;
    let history_dir = data_dir.join("history");
    std::fs::create_dir_all(&history_dir)?;

    println!();
    println!(
        "{}",
        "  Setup complete! Run `oxibot agent` to start chatting.".green()
    );
    println!();

    Ok(())
}

/// Create a template file if it doesn't exist.
fn create_template(path: &std::path::Path, content: &str) -> Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if path.exists() {
        println!("  {} {} already exists", "✓".green(), name);
    } else {
        std::fs::write(path, content)?;
        println!("  {} created {}", "✓".green(), name);
    }
    Ok(())
}

// ─────────────────────────────────────────────
// Templates
// ─────────────────────────────────────────────

const AGENTS_TEMPLATE: &str = r#"# Agents

Configuration and personality for your AI agents.

## Default Agent: Oxibot

- **Name**: Oxibot
- **Role**: Personal AI assistant
- **Style**: Concise, helpful, technical when needed
"#;

const USER_TEMPLATE: &str = r#"# User Profile

Tell Oxibot about yourself so it can personalize its responses.

## About Me

- **Name**: (your name)
- **Role**: (your role/profession)
- **Preferences**: (communication preferences)
"#;

const SOUL_TEMPLATE: &str = r#"# Soul

I am Oxibot, a lightweight AI assistant built in Rust.

## Personality

- Helpful and friendly
- Concise and to the point
- Curious and eager to learn

## Values

- Accuracy over speed
- User privacy and safety
- Transparency in actions
"#;

const HEARTBEAT_TEMPLATE: &str = r#"# Heartbeat Tasks

This file is checked every 30 minutes by your Oxibot agent.
Add tasks below that you want the agent to work on periodically.

If this file has no tasks (only headers and comments), the agent will skip the heartbeat.

## Active Tasks

<!-- Add your periodic tasks below this line -->


## Completed

<!-- Move completed tasks here or delete them -->
"#;

const MEMORY_TEMPLATE: &str = r#"# Long-term Memory

Oxibot persists important information here automatically.
You can also edit this file directly.
"#;

const SKILL_CREATOR_TEMPLATE: &str = include_str!("../../oxibot-agent/skills/skill-creator/SKILL.md");

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn create_template_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TEST.md");
        create_template(&path, "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn create_template_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TEST.md");
        std::fs::write(&path, "original").unwrap();
        create_template(&path, "new content").unwrap();
        // Should NOT overwrite
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
    }

    #[test]
    fn templates_not_empty() {
        assert!(!AGENTS_TEMPLATE.is_empty());
        assert!(!USER_TEMPLATE.is_empty());
        assert!(!MEMORY_TEMPLATE.is_empty());
    }
}
