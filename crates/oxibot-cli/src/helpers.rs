//! Shared CLI helpers — path expansion, response printing, version banner.

use std::path::PathBuf;

use colored::Colorize;

/// Expand `~` at the start of a path to the user's home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs_next::home_dir() {
            return home.join(rest);
        }
    }
    if path == "~" {
        if let Some(home) = dirs_next::home_dir() {
            return home;
        }
    }
    PathBuf::from(path)
}

/// Print an agent response to stdout.
pub fn print_response(response: &str, _render_markdown: bool) {
    // TODO: add termimad or similar markdown renderer when render_markdown=true
    println!();
    println!("{}", "🦀 Oxibot".cyan().bold());
    if response.is_empty() {
        println!("{}", "(no response)".dimmed());
    } else {
        println!("{response}");
    }
    println!();
}

/// Print the banner shown at REPL start.
pub fn print_banner() {
    let version = env!("CARGO_PKG_VERSION");
    println!();
    println!(
        "{}  v{}",
        "🦀 Oxibot".cyan().bold(),
        version.dimmed()
    );
    println!(
        "{}",
        "Type a message, or \"exit\" to quit.".dimmed()
    );
    println!();
}

/// Print a "thinking" spinner placeholder (for non-log mode).
pub fn print_thinking() {
    eprint!("{}", "⠿ thinking...".dimmed());
}

/// Clear the "thinking" placeholder.
pub fn clear_thinking() {
    eprint!("\r{}\r", " ".repeat(40));
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_tilde_home() {
        let result = expand_tilde("~/foo/bar");
        assert!(result.ends_with("foo/bar"));
        assert!(!result.starts_with("~"));
    }

    #[test]
    fn expand_tilde_no_tilde() {
        let result = expand_tilde("/absolute/path");
        assert_eq!(result, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn expand_tilde_bare() {
        let result = expand_tilde("~");
        assert!(!result.to_string_lossy().contains('~'));
    }

    #[test]
    fn expand_tilde_relative() {
        let result = expand_tilde("relative/path");
        assert_eq!(result, PathBuf::from("relative/path"));
    }
}
