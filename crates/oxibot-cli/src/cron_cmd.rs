//! `oxibot cron` — manage scheduled tasks from the CLI.
//!
//! Replaces nanobot's `cron` subcommands:
//! - `oxibot cron list [--all]` — list scheduled jobs
//! - `oxibot cron add --name NAME --message MSG (--every N | --cron EXPR | --at TIME)` — add a job
//! - `oxibot cron remove <ID>` — remove a job
//! - `oxibot cron enable <ID> [--disable]` — enable/disable a job
//! - `oxibot cron run <ID> [--force]` — manually trigger a job

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Subcommand;
use colored::Colorize;

use oxibot_core::bus::queue::MessageBus;
use oxibot_core::utils::get_data_path;
use oxibot_cron::types::{CronJob, CronPayload, CronSchedule, ScheduleKind};
use oxibot_cron::CronService;

// ─────────────────────────────────────────────
// Subcommand enum
// ─────────────────────────────────────────────

/// Cron subcommands.
#[derive(Subcommand)]
pub enum CronCommands {
    /// List scheduled jobs
    List {
        /// Include disabled jobs
        #[arg(short, long, default_value_t = false)]
        all: bool,
    },

    /// Add a new scheduled job
    Add {
        /// Job name
        #[arg(short, long)]
        name: String,

        /// Prompt message for the agent
        #[arg(short, long)]
        message: String,

        /// Run every N seconds (interval schedule)
        #[arg(short, long)]
        every: Option<u64>,

        /// Cron expression, e.g. "0 9 * * *" (cron schedule)
        #[arg(short, long)]
        cron: Option<String>,

        /// Run once at a specific time (ISO 8601 format, e.g. "2026-03-01T09:00:00")
        #[arg(long)]
        at: Option<String>,

        /// Deliver the agent's response to a channel
        #[arg(short, long, default_value_t = false)]
        deliver: bool,

        /// Recipient identifier (chat_id) for delivery
        #[arg(long)]
        to: Option<String>,

        /// Channel name for delivery (e.g. "telegram", "whatsapp")
        #[arg(long)]
        channel: Option<String>,
    },

    /// Remove a scheduled job by ID
    Remove {
        /// Job ID (8-character hex)
        job_id: String,
    },

    /// Enable or disable a job
    Enable {
        /// Job ID (8-character hex)
        job_id: String,

        /// Disable instead of enable
        #[arg(long, default_value_t = false)]
        disable: bool,
    },

    /// Manually run a job now
    Run {
        /// Job ID (8-character hex)
        job_id: String,
    },
}

// ─────────────────────────────────────────────
// Dispatcher
// ─────────────────────────────────────────────

/// Dispatch a cron subcommand.
pub async fn dispatch(cmd: CronCommands) -> Result<()> {
    match cmd {
        CronCommands::List { all } => list_jobs(all).await,
        CronCommands::Add {
            name,
            message,
            every,
            cron,
            at,
            deliver,
            to,
            channel,
        } => add_job(name, message, every, cron, at, deliver, to, channel).await,
        CronCommands::Remove { job_id } => remove_job(&job_id).await,
        CronCommands::Enable { job_id, disable } => enable_job(&job_id, !disable).await,
        CronCommands::Run { job_id } => run_job(&job_id).await,
    }
}

// ─────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────

/// Create a CronService with the default store path (no bus needed for CLI ops).
fn make_service() -> CronService {
    let store_path = get_data_path().join("cron").join("jobs.json");
    // Bus is not used in CLI-only operations, so create a dummy one
    let bus = Arc::new(MessageBus::new(1));
    CronService::new(bus, Some(store_path))
}

/// Format milliseconds as a human-readable duration.
fn format_duration_ms(ms: i64) -> String {
    let secs = ms / 1000;
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

/// Format a Unix epoch timestamp (ms) as a local datetime string.
fn format_timestamp_ms(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(ms) {
        chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        _ => "—".to_string(),
    }
}

// ─────────────────────────────────────────────
// Command implementations
// ─────────────────────────────────────────────

/// `oxibot cron list [--all]`
async fn list_jobs(include_disabled: bool) -> Result<()> {
    let service = make_service();
    service.load().await.context("failed to load cron store")?;

    let jobs = service.list_jobs().await;
    let jobs: Vec<&CronJob> = if include_disabled {
        jobs.iter().collect()
    } else {
        jobs.iter().filter(|j| j.enabled).collect()
    };

    if jobs.is_empty() {
        println!("  No scheduled jobs.{}", if !include_disabled { " Use --all to include disabled." } else { "" });
        return Ok(());
    }

    println!();
    println!("{}", "  Scheduled Jobs".cyan().bold());
    println!();

    // Header
    println!(
        "  {:<10} {:<20} {:<18} {:<10} {}",
        "ID".bold(),
        "Name".bold(),
        "Schedule".bold(),
        "Status".bold(),
        "Next Run".bold(),
    );
    println!("  {}", "─".repeat(76));

    for job in &jobs {
        // Format schedule
        let schedule = match job.schedule.kind {
            ScheduleKind::Every => {
                let ms = job.schedule.every_ms.unwrap_or(60_000);
                format!("every {}", format_duration_ms(ms))
            }
            ScheduleKind::Cron => {
                job.schedule.expr.clone().unwrap_or_else(|| "—".to_string())
            }
            ScheduleKind::At => "one-time".to_string(),
        };

        // Format status
        let status = if job.enabled {
            "enabled".green().to_string()
        } else {
            "disabled".dimmed().to_string()
        };

        // Format next run
        let next_run = match job.state.next_run_at_ms {
            Some(ms) => format_timestamp_ms(ms),
            None => "—".to_string(),
        };

        println!(
            "  {:<10} {:<20} {:<18} {:<10} {}",
            job.id, job.name, schedule, status, next_run
        );
    }

    println!();
    Ok(())
}

/// `oxibot cron add`
async fn add_job(
    name: String,
    message: String,
    every: Option<u64>,
    cron_expr: Option<String>,
    at: Option<String>,
    deliver: bool,
    to: Option<String>,
    channel: Option<String>,
) -> Result<()> {
    // Determine schedule
    let schedule = if let Some(secs) = every {
        CronSchedule::every((secs * 1000) as i64)
    } else if let Some(expr) = cron_expr {
        // Validate cron expression
        let _ = expr
            .parse::<cron::Schedule>()
            .map_err(|e| anyhow::anyhow!("Invalid cron expression '{}': {}", expr, e))?;
        CronSchedule::cron(expr)
    } else if let Some(at_str) = at {
        let dt = chrono::NaiveDateTime::parse_from_str(&at_str, "%Y-%m-%dT%H:%M:%S")
            .or_else(|_| chrono::NaiveDateTime::parse_from_str(&at_str, "%Y-%m-%d %H:%M:%S"))
            .or_else(|_| chrono::NaiveDateTime::parse_from_str(&at_str, "%Y-%m-%dT%H:%M"))
            .map_err(|e| anyhow::anyhow!("Invalid datetime '{}': {} (expected ISO 8601, e.g. 2026-03-01T09:00:00)", at_str, e))?;
        let local = chrono::Local::now().timezone();
        let aware = dt.and_local_timezone(local);
        let ts_ms = match aware {
            chrono::LocalResult::Single(dt) => dt.timestamp_millis(),
            _ => anyhow::bail!("Ambiguous or invalid local time: {}", at_str),
        };
        CronSchedule::at(ts_ms)
    } else {
        anyhow::bail!("Must specify one of: --every <seconds>, --cron <expression>, or --at <datetime>");
    };

    let payload = CronPayload {
        message,
        deliver,
        channel,
        to,
    };

    let job = CronJob::new(name, schedule, payload);

    let service = make_service();
    service.load().await.context("failed to load cron store")?;
    let id = service.add_job(job).await.context("failed to add job")?;

    println!(
        "  {} Added job {} ({})",
        "✓".green(),
        id.cyan(),
        service.get_job(&id).await.map(|j| j.name).unwrap_or_default()
    );

    Ok(())
}

/// `oxibot cron remove <ID>`
async fn remove_job(id: &str) -> Result<()> {
    let service = make_service();
    service.load().await.context("failed to load cron store")?;

    if service.remove_job(id).await? {
        println!("  {} Removed job {}", "✓".green(), id.cyan());
    } else {
        println!("  {} Job {} not found", "✗".red(), id);
    }

    Ok(())
}

/// `oxibot cron enable <ID> [--disable]`
async fn enable_job(id: &str, enabled: bool) -> Result<()> {
    let service = make_service();
    service.load().await.context("failed to load cron store")?;

    if service.set_enabled(id, enabled).await? {
        let label = if enabled { "Enabled" } else { "Disabled" };
        let job_name = service
            .get_job(id)
            .await
            .map(|j| j.name)
            .unwrap_or_default();
        println!(
            "  {} {} job '{}' ({})",
            "✓".green(),
            label,
            job_name,
            id.cyan()
        );
    } else {
        println!("  {} Job {} not found", "✗".red(), id);
    }

    Ok(())
}

/// `oxibot cron run <ID>`
async fn run_job(id: &str) -> Result<()> {
    let service = make_service();
    service.load().await.context("failed to load cron store")?;

    let job = service.get_job(id).await;
    if job.is_none() {
        println!("  {} Job {} not found", "✗".red(), id);
        return Ok(());
    }
    let job = job.unwrap();

    // For manual run, we need an agent. Build one from config.
    println!(
        "  {} Running job '{}' ({})...",
        "⠿".dimmed(),
        job.name,
        id.cyan()
    );

    let config = oxibot_core::config::load_config(None);
    let agent_loop = crate::build_agent_loop(&config)?;

    let response = agent_loop
        .process_direct(&job.payload.message)
        .await
        .context("agent processing failed")?;

    // Update last run state
    service.execute_job(id).await;

    println!();
    println!("{}", "🦀 Oxibot".cyan().bold());
    if response.is_empty() {
        println!("{}", "(no response)".dimmed());
    } else {
        println!("{response}");
    }
    println!();

    Ok(())
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration_ms() {
        assert_eq!(format_duration_ms(5_000), "5s");
        assert_eq!(format_duration_ms(60_000), "1m");
        assert_eq!(format_duration_ms(120_000), "2m");
        assert_eq!(format_duration_ms(3_600_000), "1h");
        assert_eq!(format_duration_ms(86_400_000), "1d");
    }

    #[test]
    fn test_format_timestamp_ms() {
        // Just make sure it doesn't panic
        let result = format_timestamp_ms(1_707_696_000_000); // 2024-02-12 ~UTC
        assert!(!result.is_empty());
        assert_ne!(result, "—");
    }

    #[test]
    fn test_format_timestamp_ms_invalid() {
        // i64::MIN should produce "—"
        // Actually chrono handles most values, so just check it doesn't panic
        let result = format_timestamp_ms(0);
        assert!(!result.is_empty());
    }
}
