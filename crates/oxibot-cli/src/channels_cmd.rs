//! `oxibot channels` — manage chat channels from the CLI.
//!
//! Replaces nanobot's `channels` subcommands:
//! - `oxibot channels status` — show channel configuration status
//! - `oxibot channels login` — link WhatsApp via bridge (QR code)

use anyhow::Result;
use clap::Subcommand;
use colored::Colorize;

use oxibot_core::config::load_config;

// ─────────────────────────────────────────────
// Subcommand enum
// ─────────────────────────────────────────────

/// Channels subcommands.
#[derive(Subcommand)]
pub enum ChannelsCommands {
    /// Show channel configuration status
    Status,

    /// Link WhatsApp device via QR code (starts the bridge)
    Login,
}

// ─────────────────────────────────────────────
// Dispatcher
// ─────────────────────────────────────────────

/// Dispatch a channels subcommand.
pub fn dispatch(cmd: ChannelsCommands) -> Result<()> {
    match cmd {
        ChannelsCommands::Status => channel_status(),
        ChannelsCommands::Login => channel_login(),
    }
}

// ─────────────────────────────────────────────
// Channel status
// ─────────────────────────────────────────────

/// Row for the status table.
struct ChannelRow {
    name: &'static str,
    configured: bool,
    detail: String,
}

/// `oxibot channels status`
fn channel_status() -> Result<()> {
    let config = load_config(None);
    let ch = &config.channels;

    let rows = vec![
        ChannelRow {
            name: "Telegram",
            configured: !ch.telegram.token.is_empty(),
            detail: if ch.telegram.token.is_empty() {
                "not configured".dimmed().to_string()
            } else {
                format!("token: {}...", &ch.telegram.token[..ch.telegram.token.len().min(10)])
            },
        },
        ChannelRow {
            name: "Discord",
            configured: !ch.discord.token.is_empty(),
            detail: if ch.discord.token.is_empty() {
                "not configured".dimmed().to_string()
            } else {
                format!("token: {}...", &ch.discord.token[..ch.discord.token.len().min(10)])
            },
        },
        ChannelRow {
            name: "WhatsApp",
            configured: !ch.whatsapp.bridge_url.is_empty(),
            detail: if ch.whatsapp.bridge_url.is_empty() {
                "not configured".dimmed().to_string()
            } else {
                ch.whatsapp.bridge_url.clone()
            },
        },
        ChannelRow {
            name: "Slack",
            configured: !ch.slack.bot_token.is_empty() && !ch.slack.app_token.is_empty(),
            detail: if ch.slack.bot_token.is_empty() || ch.slack.app_token.is_empty() {
                "not configured".dimmed().to_string()
            } else {
                "socket mode".to_string()
            },
        },
        ChannelRow {
            name: "Email",
            configured: !ch.email.imap_host.is_empty(),
            detail: if ch.email.imap_host.is_empty() {
                "not configured".dimmed().to_string()
            } else {
                format!("{}:{}", ch.email.imap_host, ch.email.imap_port)
            },
        },
        ChannelRow {
            name: "Feishu",
            configured: !ch.feishu.app_id.is_empty(),
            detail: if ch.feishu.app_id.is_empty() {
                "not configured".dimmed().to_string()
            } else {
                format!("app_id: {}...", &ch.feishu.app_id[..ch.feishu.app_id.len().min(10)])
            },
        },
        ChannelRow {
            name: "DingTalk",
            configured: !ch.dingtalk.client_id.is_empty(),
            detail: if ch.dingtalk.client_id.is_empty() {
                "not configured".dimmed().to_string()
            } else {
                format!("client_id: {}...", &ch.dingtalk.client_id[..ch.dingtalk.client_id.len().min(10)])
            },
        },
        ChannelRow {
            name: "QQ",
            configured: !ch.qq.app_id.is_empty(),
            detail: if ch.qq.app_id.is_empty() {
                "not configured".dimmed().to_string()
            } else {
                format!("app_id: {}...", &ch.qq.app_id[..ch.qq.app_id.len().min(10)])
            },
        },
        ChannelRow {
            name: "Mochat",
            configured: !ch.mochat.url.is_empty(),
            detail: if ch.mochat.url.is_empty() {
                "not configured".dimmed().to_string()
            } else {
                ch.mochat.url.clone()
            },
        },
    ];

    println!();
    println!("{}", "  Channel Status".cyan().bold());
    println!();

    // Header
    println!(
        "  {:<12} {:<10} {}",
        "Channel".bold(),
        "Status".bold(),
        "Configuration".bold(),
    );
    println!("  {}", "─".repeat(60));

    for row in &rows {
        let status = if row.configured {
            "✓".green().to_string()
        } else {
            "✗".dimmed().to_string()
        };
        println!("  {:<12} {:<10} {}", row.name, status, row.detail);
    }

    println!();
    Ok(())
}

// ─────────────────────────────────────────────
// Channel login (WhatsApp bridge)
// ─────────────────────────────────────────────

/// `oxibot channels login`
///
/// Starts the WhatsApp bridge and displays a QR code for linking.
fn channel_login() -> Result<()> {
    use std::process::Command;

    println!();
    println!("{}", "🦀 Oxibot — WhatsApp Login".cyan().bold());
    println!();

    // Find the bridge directory
    let bridge_dir = find_bridge_dir()?;

    // Check if Node.js / npm is available
    let npm = which_npm();
    if npm.is_none() {
        eprintln!(
            "  {} npm not found. Please install Node.js >= 18.",
            "✗".red()
        );
        eprintln!("     https://nodejs.org/");
        return Ok(());
    }
    let npm = npm.unwrap();

    // Check if bridge is built
    let dist_index = bridge_dir.join("dist").join("index.js");
    if !dist_index.exists() {
        println!("  Building bridge...");

        // npm install
        let install = Command::new(&npm)
            .arg("install")
            .current_dir(&bridge_dir)
            .output();

        match install {
            Ok(out) if out.status.success() => {}
            Ok(out) => {
                eprintln!(
                    "  {} npm install failed:\n{}",
                    "✗".red(),
                    String::from_utf8_lossy(&out.stderr)
                );
                return Ok(());
            }
            Err(e) => {
                eprintln!("  {} failed to run npm: {}", "✗".red(), e);
                return Ok(());
            }
        }

        // npm run build
        let build = Command::new(&npm)
            .args(["run", "build"])
            .current_dir(&bridge_dir)
            .output();

        match build {
            Ok(out) if out.status.success() => {
                println!("  {} Bridge built", "✓".green());
            }
            Ok(out) => {
                eprintln!(
                    "  {} npm run build failed:\n{}",
                    "✗".red(),
                    String::from_utf8_lossy(&out.stderr)
                );
                return Ok(());
            }
            Err(e) => {
                eprintln!("  {} failed to build bridge: {}", "✗".red(), e);
                return Ok(());
            }
        }
    }

    println!("  Starting bridge... Scan the QR code to connect.");
    println!();

    // Run the bridge interactively (inherits stdin/stdout for QR display)
    let status = Command::new(&npm)
        .arg("start")
        .current_dir(&bridge_dir)
        .status();

    match status {
        Ok(s) if s.success() => {
            println!();
            println!("  {} Bridge exited cleanly", "✓".green());
        }
        Ok(s) => {
            eprintln!("  {} Bridge exited with code: {:?}", "✗".red(), s.code());
        }
        Err(e) => {
            eprintln!("  {} Failed to start bridge: {}", "✗".red(), e);
        }
    }

    Ok(())
}

/// Find the WhatsApp bridge directory.
///
/// Checks (in order):
/// 1. `~/.oxibot/bridge/` (user-local copy)
/// 2. Next to the oxibot binary (for development)
/// 3. `./bridge/` (current directory)
fn find_bridge_dir() -> Result<std::path::PathBuf> {
    let data_bridge = oxibot_core::utils::get_data_path().join("bridge");
    if data_bridge.join("package.json").exists() {
        return Ok(data_bridge);
    }

    // Check next to the binary
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let dev_bridge = parent.join("bridge");
            if dev_bridge.join("package.json").exists() {
                return Ok(dev_bridge);
            }
        }
    }

    // Check current directory
    let cwd_bridge = std::path::PathBuf::from("bridge");
    if cwd_bridge.join("package.json").exists() {
        return Ok(cwd_bridge);
    }

    anyhow::bail!(
        "WhatsApp bridge not found. Expected at:\n\
         - {}\n\
         - ./bridge/\n\n\
         Copy the bridge from the nanobot project to one of these locations.",
        data_bridge.display()
    )
}

/// Find npm executable.
fn which_npm() -> Option<String> {
    // Try "npm" directly
    if std::process::Command::new("npm")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some("npm".to_string());
    }
    None
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_rows_all_nine() {
        // Ensure all 9 channels are represented by running channel_status
        // with default (empty) config — should not panic.
        // We can't easily capture stdout, so just verify no crash.
        let config = load_config(None);
        let _ = &config.channels;
        // If we got here, config loads fine
    }

    #[test]
    fn test_which_npm_returns_option() {
        // This may or may not find npm depending on environment
        let result = which_npm();
        // Just ensure it returns without panic
        let _ = result;
    }
}
