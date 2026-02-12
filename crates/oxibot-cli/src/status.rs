//! `oxibot status` — show configuration and provider status.
//!
//! Replaces nanobot's `status` command:
//! - Shows config path, workspace, model
//! - Shows API key status for each provider

use anyhow::Result;
use colored::Colorize;

use oxibot_core::config::load_config;
use oxibot_core::utils::get_data_path;
use oxibot_providers::registry::PROVIDERS;

/// Run the status command.
pub fn run() -> Result<()> {
    let config = load_config(None);
    let data_dir = get_data_path();
    let config_path = data_dir.join("config.json");

    println!();
    println!("{}", "🦀 Oxibot Status".cyan().bold());
    println!();

    // Config
    let config_exists = config_path.exists();
    println!(
        "  {:<18} {} {}",
        "Config:".bold(),
        config_path.display(),
        if config_exists {
            "✓".green().to_string()
        } else {
            "(not found)".red().to_string()
        }
    );

    // Workspace
    let workspace = crate::helpers::expand_tilde(&config.agents.defaults.workspace);
    let ws_exists = workspace.exists();
    println!(
        "  {:<18} {} {}",
        "Workspace:".bold(),
        workspace.display(),
        if ws_exists {
            "✓".green().to_string()
        } else {
            "(not found)".red().to_string()
        }
    );

    // Model
    println!(
        "  {:<18} {}",
        "Model:".bold(),
        config.agents.defaults.model
    );

    // Temperature & tokens
    println!(
        "  {:<18} {} | max_tokens: {}",
        "Parameters:".bold(),
        format!("temp: {}", config.agents.defaults.temperature).dimmed(),
        format!("{}", config.agents.defaults.max_tokens).dimmed(),
    );

    // Providers
    println!();
    println!("  {}", "Providers:".bold());
    let providers_map = config.providers.to_map();

    for spec in PROVIDERS {
        let status = if let Some(prov_config) = providers_map.get(spec.name) {
            if prov_config.is_configured() {
                format!("{} (key set)", "✓".green())
            } else {
                format!("{}", "· not configured".dimmed())
            }
        } else {
            format!("{}", "· not configured".dimmed())
        };
        println!("    {:<20} {}", spec.display_name, status);
    }

    // Brave Search
    println!();
    let brave_status = if config.tools.web.search.api_key.is_empty() {
        format!("{}", "· not configured".dimmed())
    } else {
        format!("{} (key set)", "✓".green())
    };
    println!("  {:<18} {}", "Brave Search:".bold(), brave_status);

    println!();

    Ok(())
}
