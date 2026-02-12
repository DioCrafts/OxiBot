//! Gateway command — orchestrates channels, agent loop, and message routing.
//!
//! Port of nanobot's gateway command from `cli/commands.py`.
//!
//! Startup sequence:
//! 1. Load config
//! 2. Create message bus
//! 3. Create agent loop (with provider, tools, sessions)
//! 4. Create channel manager, register enabled channels
//! 5. Run: `tokio::select!` of agent loop + channel manager
//! 6. Handle Ctrl+C for graceful shutdown

use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::info;

use oxibot_agent::{AgentLoop, ExecToolConfig};
use oxibot_channels::ChannelManager;
use oxibot_core::bus::queue::MessageBus;
use oxibot_core::bus::types::OutboundMessage;
use oxibot_core::config::load_config;
use oxibot_core::heartbeat::HeartbeatService;
use oxibot_core::session::SessionManager;
use oxibot_cron::CronService;
use oxibot_providers::http_provider::create_provider;

use crate::helpers;

/// Run the gateway — starts the agent loop + channel manager.
pub async fn run() -> Result<()> {
    println!();
    helpers::print_banner();
    println!("  Mode: Gateway");
    println!();

    // 1. Load config
    let config = load_config(None);
    let defaults = &config.agents.defaults;

    // 2. Resolve workspace
    let workspace = helpers::expand_tilde(&defaults.workspace);
    std::fs::create_dir_all(&workspace)
        .with_context(|| format!("failed to create workspace: {}", workspace.display()))?;

    // 3. Create message bus (shared between agent + channels)
    let bus = Arc::new(MessageBus::new(100));

    // 4. Create provider
    let model = &defaults.model;
    let providers_map = config.providers.to_map();
    let provider = create_provider(model, &providers_map)
        .map_err(|e| anyhow::anyhow!(e))?;

    // 5. Brave API key
    let brave_key = if config.tools.web.search.api_key.is_empty() {
        None
    } else {
        Some(config.tools.web.search.api_key.clone())
    };

    // 6. Create session manager
    let session_manager = SessionManager::new(None)
        .context("failed to create session manager")?;

    // 7. Create agent loop (Arc-wrapped for sharing with cron callback)
    let agent_loop = Arc::new(AgentLoop::new(
        bus.clone(),
        Arc::new(provider),
        workspace.clone(),
        Some(model.to_string()),
        Some(defaults.max_tool_iterations as usize),
        None,
        brave_key,
        Some(ExecToolConfig::default()),
        config.tools.restrict_to_workspace,
        Some(session_manager),
        None,
    ));

    // 8. Create cron service
    let cron_service = Arc::new(CronService::new(bus.clone(), None));
    {
        let agent = agent_loop.clone();
        let bus = bus.clone();
        cron_service
            .set_on_job(Arc::new(move |job: oxibot_cron::CronJob| {
                let agent = agent.clone();
                let bus = bus.clone();
                Box::pin(async move {
                    let response = agent
                        .process_direct(&job.payload.message)
                        .await
                        .unwrap_or_else(|e| format!("Error: {e}"));

                    // Deliver result to channel if configured
                    if job.payload.deliver {
                        if let Some(ref chat_id) = job.payload.to {
                            let channel = job.payload.channel.as_deref().unwrap_or("cli");
                            let msg = OutboundMessage::new(channel, chat_id.as_str(), &response);
                            if let Err(e) = bus.publish_outbound(msg).await {
                                tracing::error!(error = %e, "failed to deliver cron result");
                            }
                        }
                    }

                    Ok(response)
                })
            }))
            .await;
    }

    // Pre-load to show job count in banner
    if let Err(e) = cron_service.load().await {
        tracing::warn!(error = %e, "failed to pre-load cron store");
    }
    let cron_jobs = cron_service.list_jobs().await;

    // 9. Create heartbeat service
    let heartbeat = {
        let agent = agent_loop.clone();
        let callback: oxibot_core::heartbeat::OnHeartbeatFn = Arc::new(move |prompt| {
            let agent = agent.clone();
            Box::pin(async move { agent.process_direct(&prompt).await })
        });
        Arc::new(HeartbeatService::new(
            workspace.clone(),
            Some(callback),
            None, // default 30 min
            true,
        ))
    };

    // 10. Create channel manager
    // Register configured channels
    #[allow(unused_mut)]
    let mut channel_manager = ChannelManager::new(bus.clone());

    // Telegram
    #[cfg(feature = "telegram")]
    {
        let tg = &config.channels.telegram;
        if !tg.token.is_empty() {
            use oxibot_channels::telegram::TelegramChannel;
            let mut telegram = TelegramChannel::new(
                tg.token.clone(),
                bus.clone(),
                tg.allowed_users.clone(),
            );

            // Wire voice transcription if configured
            if config.transcription.enabled {
                let tc = &config.transcription;
                // Resolve API key: config > groq provider key > env var
                let transcription_key = if !tc.api_key.is_empty() {
                    tc.api_key.clone()
                } else if !config.providers.groq.api_key.is_empty() {
                    config.providers.groq.api_key.clone()
                } else {
                    String::new()
                };

                if !transcription_key.is_empty() {
                    use oxibot_providers::GroqTranscriber;
                    use oxibot_providers::TranscriptionProvider;
                    let transcriber = Arc::new(GroqTranscriber::new(&transcription_key));
                    if transcriber.is_configured() {
                        let t = transcriber.clone();
                        telegram = telegram.with_transcriber(Arc::new(move |path: String| {
                            let t = t.clone();
                            Box::pin(async move {
                                t.transcribe(std::path::Path::new(&path)).await
                            })
                        }));
                        info!("voice transcription enabled (Groq Whisper)");
                    }
                }
            }

            channel_manager.register(Arc::new(telegram));
            info!("registered telegram channel");
        }
    }

    // Discord
    #[cfg(feature = "discord")]
    {
        let dc = &config.channels.discord;
        if !dc.token.is_empty() {
            use oxibot_channels::discord::DiscordChannel;
            let discord = DiscordChannel::new(
                dc.token.clone(),
                bus.clone(),
                dc.allowed_users.clone(),
            );
            channel_manager.register(Arc::new(discord));
            info!("registered discord channel");
        }
    }

    // WhatsApp
    #[cfg(feature = "whatsapp")]
    {
        let wa = &config.channels.whatsapp;
        if !wa.bridge_url.is_empty() {
            use oxibot_channels::whatsapp::WhatsAppChannel;
            let whatsapp = WhatsAppChannel::new(
                wa.bridge_url.clone(),
                bus.clone(),
                wa.allowed_users.clone(),
            );
            channel_manager.register(Arc::new(whatsapp));
            info!("registered whatsapp channel");
        }
    }

    // Slack
    #[cfg(feature = "slack")]
    {
        let sl = &config.channels.slack;
        if !sl.bot_token.is_empty() && !sl.app_token.is_empty() {
            use oxibot_channels::slack::SlackChannel;
            let slack = SlackChannel::new(sl.clone(), bus.clone());
            channel_manager.register(Arc::new(slack));
            info!("registered slack channel");
        }
    }

    // Email
    #[cfg(feature = "email")]
    {
        let em = &config.channels.email;
        if !em.imap_host.is_empty() {
            use oxibot_channels::email::EmailChannel;
            let email = EmailChannel::new(em.clone(), bus.clone());
            channel_manager.register(Arc::new(email));
            info!("registered email channel");
        }
    }
    info!(
        model = %model,
        workspace = %workspace.display(),
        channels = ?channel_manager.channel_names(),
        "gateway starting"
    );

    println!(
        "  Model:     {}",
        model
    );
    println!(
        "  Workspace: {}",
        workspace.display()
    );
    println!(
        "  Channels:  {} registered",
        channel_manager.len()
    );
    if !cron_jobs.is_empty() {
        let enabled = cron_jobs.iter().filter(|j| j.enabled).count();
        println!("  Cron:      {} jobs ({} enabled)", cron_jobs.len(), enabled);
    }
    println!("  Heartbeat: every 30m");
    println!();

    if channel_manager.is_empty() {
        println!("  ⚠  No channels registered. The agent loop will run but");
        println!("     only process messages from the internal bus.");
        println!("     Configure channels in ~/.oxibot/config.json");
        println!();
    }

    println!("  Ctrl+C to stop");
    println!();

    // 11. Run: agent loop + channel manager + cron + heartbeat concurrently
    //     Ctrl+C triggers graceful shutdown
    tokio::select! {
        _ = agent_loop.run() => {
            info!("agent loop exited");
        }
        result = channel_manager.start_all() => {
            if let Err(e) = result {
                tracing::error!(error = %e, "channel manager error");
            }
        }
        result = cron_service.start() => {
            if let Err(e) = result {
                tracing::error!(error = %e, "cron service error");
            }
        }
        result = heartbeat.start() => {
            if let Err(e) = result {
                tracing::error!(error = %e, "heartbeat service error");
            }
        }
        _ = tokio::signal::ctrl_c() => {
            println!();
            println!("  Shutting down...");
            info!("received Ctrl+C, shutting down");
            heartbeat.stop();
            cron_service.stop().await;
            channel_manager.stop_all().await;
        }
    }

    println!("  Gateway stopped. Goodbye!");
    Ok(())
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    // Gateway integration tests would require a full runtime environment.
    // The component tests are in oxibot-channels and oxibot-agent crates.
    // Here we just verify the module compiles and the imports work.

    #[test]
    fn test_module_compiles() {
        // If this test runs, the gateway module compiles correctly
        assert!(true);
    }
}
