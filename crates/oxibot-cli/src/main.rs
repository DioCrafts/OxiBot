//! Oxibot CLI — entry point.
//!
//! Replaces nanobot's `cli/commands.py` (Typer app).
//!
//! # Commands
//!
//! - `oxibot agent [-m MESSAGE] [-s SESSION]` — main chat (single-shot or REPL)
//! - `oxibot onboard` — initialize config + workspace
//! - `oxibot status` — show configuration and provider status

mod helpers;
mod onboard;
mod repl;
mod status;
mod gateway;
mod cron_cmd;
mod channels_cmd;

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::info;

use oxibot_agent::{AgentLoop, ExecToolConfig};
use oxibot_core::bus::queue::MessageBus;
use oxibot_core::config::{load_config, Config};
use oxibot_core::session::SessionManager;
use oxibot_providers::http_provider::create_provider;

// ─────────────────────────────────────────────
// CLI definition
// ─────────────────────────────────────────────

/// 🦀 Oxibot — Ultra-lightweight AI assistant in Rust
#[derive(Parser)]
#[command(name = "oxibot", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Chat with the AI agent (single-shot or interactive REPL)
    Agent {
        /// Single message (non-interactive). Omit for REPL mode.
        #[arg(short, long)]
        message: Option<String>,

        /// Session identifier (format: "channel:id")
        #[arg(short, long, default_value = "cli:default")]
        session: String,

        /// Disable Markdown rendering in output
        #[arg(long, default_value_t = false)]
        no_markdown: bool,

        /// Enable debug logging
        #[arg(long, default_value_t = false)]
        logs: bool,
    },

    /// Initialize configuration and workspace
    Onboard,

    /// Show configuration and provider status
    Status,

    /// Start the gateway (all channels + agent loop)
    Gateway {
        /// Enable debug logging
        #[arg(long, default_value_t = false)]
        logs: bool,
    },

    /// Manage scheduled tasks
    Cron {
        #[command(subcommand)]
        action: cron_cmd::CronCommands,
    },

    /// Manage chat channels
    Channels {
        #[command(subcommand)]
        action: channels_cmd::ChannelsCommands,
    },
}

// ─────────────────────────────────────────────
// Entrypoint
// ─────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Agent {
            message,
            session,
            no_markdown,
            logs,
        } => {
            init_logging(logs);
            run_agent(message, session, !no_markdown, logs).await
        }
        Commands::Onboard => onboard::run(),
        Commands::Status => status::run(),
        Commands::Gateway { logs } => {
            init_logging(logs);
            gateway::run().await
        }
        Commands::Cron { action } => {
            init_logging(false);
            cron_cmd::dispatch(action).await
        }
        Commands::Channels { action } => channels_cmd::dispatch(action),
    }
}

// ─────────────────────────────────────────────
// Agent command
// ─────────────────────────────────────────────

async fn run_agent(
    message: Option<String>,
    session_id: String,
    render_markdown: bool,
    show_logs: bool,
) -> Result<()> {
    let config = load_config(None);
    let agent_loop = build_agent_loop(&config)?;

    match message {
        Some(msg) => {
            // Single-shot mode
            info!(session = %session_id, "processing single message");
            let response = agent_loop
                .process_direct(&msg)
                .await
                .context("agent processing failed")?;
            helpers::print_response(&response, render_markdown);
        }
        None => {
            // Interactive REPL mode
            repl::run(agent_loop, &session_id, render_markdown, show_logs).await?;
        }
    }

    Ok(())
}

/// Build an `AgentLoop` from the loaded configuration.
pub fn build_agent_loop(config: &Config) -> Result<AgentLoop> {
    let defaults = &config.agents.defaults;

    // Resolve workspace path (expand ~)
    let workspace = helpers::expand_tilde(&defaults.workspace);
    std::fs::create_dir_all(&workspace)
        .with_context(|| format!("failed to create workspace: {}", workspace.display()))?;

    // Resolve model
    let model = &defaults.model;

    // Create provider
    let providers_map = config.providers.to_map();
    let provider = create_provider(model, &providers_map)
        .map_err(|e| anyhow::anyhow!(e))?;

    // Brave API key
    let brave_key = if config.tools.web.search.api_key.is_empty() {
        None
    } else {
        Some(config.tools.web.search.api_key.clone())
    };

    // Build agent loop
    let bus = Arc::new(MessageBus::new(100));
    let session_manager = SessionManager::new(None)
        .context("failed to create session manager")?;

    let agent_loop = AgentLoop::new(
        bus,
        Arc::new(provider),
        workspace,
        Some(model.to_string()),
        Some(defaults.max_tool_iterations as usize),
        None, // uses defaults for temperature/max_tokens
        brave_key,
        Some(ExecToolConfig::default()),
        config.tools.restrict_to_workspace,
        Some(session_manager),
        None, // default agent name "Oxibot"
    );

    Ok(agent_loop)
}

/// Initialize tracing/logging.
fn init_logging(verbose: bool) {
    use tracing_subscriber::EnvFilter;

    let filter = if verbose {
        EnvFilter::new("oxibot=debug,info")
    } else {
        EnvFilter::new("warn")
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}
