//! Memory system — long-term memory and daily notes.
//!
//! Port of nanobot's `agent/memory.py`.
//!
//! The agent's memory is file-based:
//! - **Long-term memory**: `workspace/memory/MEMORY.md` — persistent facts, prefs
//! - **Daily notes**: `workspace/memory/YYYY-MM-DD.md` — ephemeral daily context
//!
//! The context builder reads memory on every prompt build (passive read).
//! The agent writes memory via the filesystem tools (active write).

use std::path::{Path, PathBuf};

use chrono::Utc;
use tracing::debug;

// ─────────────────────────────────────────────
// MemoryStore
// ─────────────────────────────────────────────

/// File-based memory store for the agent.
///
/// Manages `memory/MEMORY.md` (long-term) and `memory/YYYY-MM-DD.md` (daily).
pub struct MemoryStore {
    /// The `memory/` directory inside the workspace.
    memory_dir: PathBuf,
    /// Path to the long-term memory file.
    memory_file: PathBuf,
}

impl MemoryStore {
    /// Create a new memory store, creating the `memory/` directory if needed.
    pub fn new(workspace: &Path) -> std::io::Result<Self> {
        let memory_dir = workspace.join("memory");
        if !memory_dir.exists() {
            std::fs::create_dir_all(&memory_dir)?;
            debug!(dir = %memory_dir.display(), "created memory directory");
        }
        let memory_file = memory_dir.join("MEMORY.md");
        Ok(Self {
            memory_dir,
            memory_file,
        })
    }

    /// Create a MemoryStore without creating the directory (for read-only checks).
    pub fn new_lazy(workspace: &Path) -> Self {
        let memory_dir = workspace.join("memory");
        let memory_file = memory_dir.join("MEMORY.md");
        Self {
            memory_dir,
            memory_file,
        }
    }

    // ────────────── Long-term memory ──────────────

    /// Read the long-term memory file. Returns empty string if absent.
    pub fn read_long_term(&self) -> String {
        std::fs::read_to_string(&self.memory_file).unwrap_or_default()
    }

    /// Overwrite the entire long-term memory file.
    pub fn write_long_term(&self, content: &str) -> std::io::Result<()> {
        self.ensure_dir()?;
        std::fs::write(&self.memory_file, content)
    }

    // ────────────── Daily notes ──────────────

    /// Path to today's daily notes file.
    pub fn today_file(&self) -> PathBuf {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        self.memory_dir.join(format!("{today}.md"))
    }

    /// Read today's daily notes. Returns empty string if absent.
    pub fn read_today(&self) -> String {
        std::fs::read_to_string(self.today_file()).unwrap_or_default()
    }

    /// Append content to today's daily notes.
    ///
    /// If the file doesn't exist, creates it with a date header first.
    pub fn append_today(&self, content: &str) -> std::io::Result<()> {
        self.ensure_dir()?;
        let path = self.today_file();
        if path.exists() {
            let mut existing = std::fs::read_to_string(&path)?;
            existing.push('\n');
            existing.push_str(content);
            std::fs::write(&path, existing)
        } else {
            let today = Utc::now().format("%Y-%m-%d").to_string();
            let initial = format!("# {today}\n\n{content}");
            std::fs::write(&path, initial)
        }
    }

    // ────────────── Aggregation ──────────────

    /// List daily note files, newest first.
    pub fn list_memory_files(&self) -> Vec<PathBuf> {
        let pattern = self.memory_dir.join("????-??-??.md");
        let pattern_str = pattern.to_string_lossy().to_string();

        let mut files: Vec<PathBuf> = glob_simple(&self.memory_dir)
            .into_iter()
            .collect();
        files.sort();
        files.reverse(); // newest first
        let _ = pattern_str; // suppress unused
        files
    }

    /// Read the last N days of daily notes, joined by `---` separators.
    pub fn get_recent_memories(&self, days: usize) -> String {
        let files = self.list_memory_files();
        let parts: Vec<String> = files
            .into_iter()
            .take(days)
            .filter_map(|f| std::fs::read_to_string(&f).ok())
            .filter(|c| !c.trim().is_empty())
            .collect();
        parts.join("\n\n---\n\n")
    }

    /// Build the memory context string for the system prompt.
    ///
    /// Returns `None` if no memory exists.
    /// Format:
    /// ```text
    /// # Memory
    ///
    /// ## Long-term Memory
    /// <content of MEMORY.md>
    ///
    /// ## Today's Notes (YYYY-MM-DD)
    /// <content of today's daily file>
    /// ```
    pub fn get_memory_context(&self) -> Option<String> {
        let mut sections = Vec::new();

        // Long-term memory
        let long_term = self.read_long_term();
        if !long_term.trim().is_empty() {
            sections.push(format!("## Long-term Memory\n\n{long_term}"));
        }

        // Today's daily notes
        let today_content = self.read_today();
        if !today_content.trim().is_empty() {
            let today = Utc::now().format("%Y-%m-%d").to_string();
            sections.push(format!("## Today's Notes ({today})\n\n{today_content}"));
        }

        if sections.is_empty() {
            None
        } else {
            Some(format!("# Memory\n\n{}", sections.join("\n\n")))
        }
    }

    /// Path to the memory directory.
    pub fn memory_dir(&self) -> &Path {
        &self.memory_dir
    }

    /// Path to the long-term memory file.
    pub fn memory_file(&self) -> &Path {
        &self.memory_file
    }

    /// Ensure the memory directory exists.
    fn ensure_dir(&self) -> std::io::Result<()> {
        if !self.memory_dir.exists() {
            std::fs::create_dir_all(&self.memory_dir)?;
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────

/// Simple glob for `YYYY-MM-DD.md` files in a directory.
fn glob_simple(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Match YYYY-MM-DD.md pattern
            name.len() == 13
                && name.ends_with(".md")
                && name.as_bytes()[4] == b'-'
                && name.as_bytes()[7] == b'-'
                && name[..4].chars().all(|c| c.is_ascii_digit())
                && name[5..7].chars().all(|c| c.is_ascii_digit())
                && name[8..10].chars().all(|c| c.is_ascii_digit())
        })
        .collect()
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_creates_dir() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("workspace");
        std::fs::create_dir(&ws).unwrap();

        let store = MemoryStore::new(&ws).unwrap();
        assert!(store.memory_dir().exists());
        assert!(store.memory_dir().is_dir());
    }

    #[test]
    fn test_read_long_term_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path()).unwrap();
        assert_eq!(store.read_long_term(), "");
    }

    #[test]
    fn test_write_and_read_long_term() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path()).unwrap();

        store.write_long_term("User likes Rust.").unwrap();
        assert_eq!(store.read_long_term(), "User likes Rust.");

        // Overwrite
        store.write_long_term("User prefers dark mode.").unwrap();
        assert_eq!(store.read_long_term(), "User prefers dark mode.");
    }

    #[test]
    fn test_read_today_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path()).unwrap();
        assert_eq!(store.read_today(), "");
    }

    #[test]
    fn test_append_today_creates_with_header() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path()).unwrap();

        store.append_today("Did some coding.").unwrap();
        let content = store.read_today();
        let today = Utc::now().format("%Y-%m-%d").to_string();
        assert!(content.starts_with(&format!("# {today}")));
        assert!(content.contains("Did some coding."));
    }

    #[test]
    fn test_append_today_appends() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path()).unwrap();

        store.append_today("First note.").unwrap();
        store.append_today("Second note.").unwrap();

        let content = store.read_today();
        assert!(content.contains("First note."));
        assert!(content.contains("Second note."));
    }

    #[test]
    fn test_list_memory_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path()).unwrap();

        // Create some daily files
        std::fs::write(store.memory_dir().join("2026-01-10.md"), "day 1").unwrap();
        std::fs::write(store.memory_dir().join("2026-01-11.md"), "day 2").unwrap();
        std::fs::write(store.memory_dir().join("2026-01-12.md"), "day 3").unwrap();
        // Non-matching files should be ignored
        std::fs::write(store.memory_dir().join("MEMORY.md"), "long term").unwrap();
        std::fs::write(store.memory_dir().join("notes.txt"), "other").unwrap();

        let files = store.list_memory_files();
        assert_eq!(files.len(), 3);
        // Should be newest first
        assert!(files[0].to_string_lossy().contains("2026-01-12"));
        assert!(files[2].to_string_lossy().contains("2026-01-10"));
    }

    #[test]
    fn test_get_recent_memories() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path()).unwrap();

        std::fs::write(store.memory_dir().join("2026-01-10.md"), "# 2026-01-10\n\nDay 1").unwrap();
        std::fs::write(store.memory_dir().join("2026-01-11.md"), "# 2026-01-11\n\nDay 2").unwrap();
        std::fs::write(store.memory_dir().join("2026-01-12.md"), "# 2026-01-12\n\nDay 3").unwrap();

        let recent = store.get_recent_memories(2);
        assert!(recent.contains("Day 3"));
        assert!(recent.contains("Day 2"));
        assert!(!recent.contains("Day 1")); // only latest 2
    }

    #[test]
    fn test_get_memory_context_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path()).unwrap();
        assert!(store.get_memory_context().is_none());
    }

    #[test]
    fn test_get_memory_context_long_term_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path()).unwrap();

        store.write_long_term("User prefers dark mode.").unwrap();
        let ctx = store.get_memory_context().unwrap();
        assert!(ctx.contains("# Memory"));
        assert!(ctx.contains("## Long-term Memory"));
        assert!(ctx.contains("User prefers dark mode."));
        assert!(!ctx.contains("Today's Notes"));
    }

    #[test]
    fn test_get_memory_context_with_daily() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path()).unwrap();

        store.write_long_term("Important fact.").unwrap();
        store.append_today("Today's work.").unwrap();

        let ctx = store.get_memory_context().unwrap();
        assert!(ctx.contains("Long-term Memory"));
        assert!(ctx.contains("Important fact."));
        assert!(ctx.contains("Today's Notes"));
        assert!(ctx.contains("Today's work."));
    }

    #[test]
    fn test_get_memory_context_empty_files_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path()).unwrap();

        store.write_long_term("   \n  \n  ").unwrap(); // whitespace-only
        assert!(store.get_memory_context().is_none());
    }

    #[test]
    fn test_new_lazy_no_create() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("noexist");
        let store = MemoryStore::new_lazy(&ws);
        // Memory dir should NOT be created
        assert!(!store.memory_dir().exists());
        // But reading should return empty, not error
        assert_eq!(store.read_long_term(), "");
        assert_eq!(store.read_today(), "");
    }

    #[test]
    fn test_glob_pattern_strict() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path()).unwrap();

        // Valid patterns
        std::fs::write(store.memory_dir().join("2026-01-15.md"), "ok").unwrap();
        // Invalid patterns — should be excluded
        std::fs::write(store.memory_dir().join("2026-1-15.md"), "bad").unwrap();
        std::fs::write(store.memory_dir().join("notes-01-15.md"), "bad").unwrap();
        std::fs::write(store.memory_dir().join("2026-01-15.txt"), "bad").unwrap();

        let files = store.list_memory_files();
        assert_eq!(files.len(), 1);
    }
}
