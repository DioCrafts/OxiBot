//! Utility helpers — path resolution, date formatting, string manipulation.
//!
//! Replaces nanobot's `utils/helpers.py`.

use std::path::PathBuf;

/// Get the Oxibot data directory (e.g. `~/.oxibot/`).
pub fn get_data_path() -> PathBuf {
    let home = dirs_next().unwrap_or_else(|| PathBuf::from("."));
    home.join(".oxibot")
}

/// Get the sessions directory (e.g. `~/.oxibot/sessions/`).
pub fn get_sessions_path() -> PathBuf {
    get_data_path().join("sessions")
}

/// Get the default workspace path (e.g. `~/.oxibot/workspace/`).
pub fn get_default_workspace_path() -> PathBuf {
    get_data_path().join("workspace")
}

/// Get today's date as YYYY-MM-DD.
pub fn today_date() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Get current ISO 8601 timestamp.
pub fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Truncate a string to `max_len` characters, adding "..." if truncated.
/// Unicode-safe.
pub fn truncate_string(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

/// Sanitize a string for use as a filename.
pub fn safe_filename(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Expand `~` to the home directory in a path string.
pub fn expand_home(path: &str) -> PathBuf {
    if path.starts_with("~/") || path == "~" {
        let home = dirs_next().unwrap_or_else(|| PathBuf::from("."));
        home.join(&path[2..])
    } else {
        PathBuf::from(path)
    }
}

/// Helper to get home directory.
fn dirs_next() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::var("USERPROFILE").ok().map(PathBuf::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_short_string() {
        assert_eq!(truncate_string("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_exact_length() {
        assert_eq!(truncate_string("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_long_string() {
        let result = truncate_string("hello world, this is a long string", 15);
        assert_eq!(result, "hello world,...");
        assert!(result.len() <= 15);
    }

    #[test]
    fn test_truncate_unicode() {
        let result = truncate_string("こんにちは世界です", 5);
        assert_eq!(result, "こん...");
    }

    #[test]
    fn test_safe_filename() {
        assert_eq!(safe_filename("hello world!"), "hello_world_");
        assert_eq!(safe_filename("file.txt"), "file.txt");
        assert_eq!(safe_filename("a/b/c"), "a_b_c");
        assert_eq!(safe_filename("test@2024"), "test_2024");
    }

    #[test]
    fn test_safe_filename_preserves_valid() {
        assert_eq!(safe_filename("my-file_v2.txt"), "my-file_v2.txt");
    }

    #[test]
    fn test_expand_home_tilde() {
        let expanded = expand_home("~/test/path");
        assert!(!expanded.starts_with("~"));
        assert!(expanded.to_str().unwrap().ends_with("test/path"));
    }

    #[test]
    fn test_expand_home_absolute() {
        let expanded = expand_home("/absolute/path");
        assert_eq!(expanded, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn test_today_date_format() {
        let date = today_date();
        // Should match YYYY-MM-DD pattern
        assert_eq!(date.len(), 10);
        assert_eq!(date.chars().nth(4), Some('-'));
        assert_eq!(date.chars().nth(7), Some('-'));
    }

    #[test]
    fn test_timestamp_is_valid() {
        let ts = timestamp();
        // Should be parseable as RFC 3339
        chrono::DateTime::parse_from_rfc3339(&ts).unwrap();
    }

    #[test]
    fn test_data_path_ends_with_oxibot() {
        let path = get_data_path();
        assert!(path.ends_with(".oxibot"));
    }

    #[test]
    fn test_sessions_path() {
        let path = get_sessions_path();
        assert!(path.ends_with("sessions"));
        assert!(path.parent().unwrap().ends_with(".oxibot"));
    }
}
