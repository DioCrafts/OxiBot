//! Session persistence and caching.
//!
//! File format: JSONL in `~/.oxibot/sessions/{safe_key}.jsonl`
//! - Line 1: `{"_type":"metadata","created_at":"...","updated_at":"...","metadata":{}}`
//! - Line 2+: `{"role":"user","content":"hello","timestamp":"..."}`

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::RwLock;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::types::{Message, Session};
use crate::utils;

// ─────────────────────────────────────────────
// Session metadata (first line of JSONL)
// ─────────────────────────────────────────────

/// Metadata header written as the first line of each JSONL session file.
#[derive(Debug, Serialize, Deserialize)]
struct SessionMetadata {
    #[serde(rename = "_type")]
    record_type: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

// ─────────────────────────────────────────────
// SessionManager
// ─────────────────────────────────────────────

/// Manages conversation sessions with in-memory caching and JSONL persistence.
///
/// Thread-safe via `RwLock` — multiple readers, exclusive writer.
pub struct SessionManager {
    /// Directory where `.jsonl` session files are stored.
    sessions_dir: PathBuf,
    /// In-memory cache of active sessions.
    cache: RwLock<HashMap<String, Session>>,
}

impl SessionManager {
    /// Create a new session manager.
    ///
    /// `sessions_dir` defaults to `~/.oxibot/sessions/` if `None`.
    /// The directory is created if it doesn't exist.
    pub fn new(sessions_dir: Option<PathBuf>) -> std::io::Result<Self> {
        let dir = sessions_dir.unwrap_or_else(utils::get_sessions_path);
        std::fs::create_dir_all(&dir)?;

        Ok(SessionManager {
            sessions_dir: dir,
            cache: RwLock::new(HashMap::new()),
        })
    }

    /// Get an existing session or create a new one.
    ///
    /// 1. Check in-memory cache
    /// 2. Try to load from disk
    /// 3. Create new empty session
    pub fn get_or_create(&self, key: &str) -> Session {
        // Check cache first
        {
            let cache = self.cache.read().unwrap();
            if let Some(session) = cache.get(key) {
                return session.clone();
            }
        }

        // Try loading from disk
        if let Some(session) = self.load_from_disk(key) {
            let mut cache = self.cache.write().unwrap();
            cache.insert(key.to_string(), session.clone());
            return session;
        }

        // Create new empty session
        let session = Session::new(key);
        let mut cache = self.cache.write().unwrap();
        cache.insert(key.to_string(), session.clone());
        session
    }

    /// Add a message to a session and persist to disk.
    pub fn add_message(&self, key: &str, message: Message) {
        let mut session = self.get_or_create(key);
        session.messages.push(message);
        session.updated_at = Utc::now();

        // Update cache and save
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(key.to_string(), session.clone());
        }

        if let Err(e) = self.save_to_disk(&session) {
            warn!("Failed to persist session {}: {}", key, e);
        }
    }

    /// Get the last `max_messages` from a session's history.
    ///
    /// Returns messages in LLM format (role + content).
    pub fn get_history(&self, key: &str, max_messages: usize) -> Vec<Message> {
        let session = self.get_or_create(key);
        let len = session.messages.len();
        if len <= max_messages {
            session.messages
        } else {
            session.messages[len - max_messages..].to_vec()
        }
    }

    /// Clear all messages in a session (reset conversation).
    pub fn clear(&self, key: &str) {
        let mut session = self.get_or_create(key);
        session.messages.clear();
        session.updated_at = Utc::now();

        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(key.to_string(), session.clone());
        }

        if let Err(e) = self.save_to_disk(&session) {
            warn!("Failed to persist cleared session {}: {}", key, e);
        }
    }

    /// Delete a session entirely (from cache and disk).
    ///
    /// Returns `true` if the session file existed on disk.
    pub fn delete(&self, key: &str) -> bool {
        // Remove from cache
        {
            let mut cache = self.cache.write().unwrap();
            cache.remove(key);
        }

        // Remove from disk
        let path = self.session_path(key);
        if path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                warn!("Failed to delete session file: {}", e);
                return false;
            }
            debug!("Deleted session file: {}", path.display());
            true
        } else {
            false
        }
    }

    /// List all sessions from disk.
    ///
    /// Returns a list of session summaries sorted by `updated_at` (newest first).
    pub fn list_sessions(&self) -> Vec<SessionSummary> {
        let mut summaries = Vec::new();

        let entries = match std::fs::read_dir(&self.sessions_dir) {
            Ok(entries) => entries,
            Err(e) => {
                warn!("Failed to read sessions directory: {}", e);
                return summaries;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(true, |ext| ext != "jsonl") {
                continue;
            }

            // Read first line (metadata)
            if let Ok(file) = std::fs::File::open(&path) {
                let reader = std::io::BufReader::new(file);
                if let Some(Ok(line)) = reader.lines().next() {
                    if let Ok(meta) = serde_json::from_str::<SessionMetadata>(&line) {
                        let key = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .map(|s| s.replace('_', ":"))
                            .unwrap_or_default();

                        summaries.push(SessionSummary {
                            key,
                            created_at: meta.created_at,
                            updated_at: meta.updated_at,
                            path: path.clone(),
                        });
                    }
                }
            }
        }

        // Sort by updated_at descending
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        summaries
    }

    /// Get the JSONL file path for a session key.
    fn session_path(&self, key: &str) -> PathBuf {
        let safe_key = utils::safe_filename(&key.replace(':', "_"));
        self.sessions_dir.join(format!("{}.jsonl", safe_key))
    }

    /// Load a session from a JSONL file.
    fn load_from_disk(&self, key: &str) -> Option<Session> {
        let path = self.session_path(key);
        if !path.exists() {
            return None;
        }

        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                warn!("Failed to open session file {}: {}", path.display(), e);
                return None;
            }
        };

        let reader = std::io::BufReader::new(file);
        let mut session = Session::new(key);
        let mut messages = Vec::new();

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };

            if line.trim().is_empty() {
                continue;
            }

            // Try as metadata first
            if let Ok(meta) = serde_json::from_str::<SessionMetadata>(&line) {
                if meta.record_type == "metadata" {
                    session.created_at = meta.created_at;
                    session.updated_at = meta.updated_at;
                    session.metadata = meta.metadata;
                    continue;
                }
            }

            // Try as message
            if let Ok(msg) = serde_json::from_str::<Message>(&line) {
                messages.push(msg);
            }
        }

        session.messages = messages;
        debug!(
            "Loaded session '{}' with {} messages from disk",
            key,
            session.messages.len()
        );
        Some(session)
    }

    /// Save a session to a JSONL file (overwrite).
    fn save_to_disk(&self, session: &Session) -> std::io::Result<()> {
        let path = self.session_path(&session.key);

        let mut file = std::fs::File::create(&path)?;

        // Write metadata line
        let meta = SessionMetadata {
            record_type: "metadata".to_string(),
            created_at: session.created_at,
            updated_at: session.updated_at,
            metadata: session.metadata.clone(),
        };
        writeln!(file, "{}", serde_json::to_string(&meta)?)?;

        // Write each message
        for msg in &session.messages {
            writeln!(file, "{}", serde_json::to_string(msg)?)?;
        }

        debug!(
            "Saved session '{}' ({} messages) to {}",
            session.key,
            session.messages.len(),
            path.display()
        );
        Ok(())
    }
}

/// Summary of a session for listing purposes.
#[derive(Clone, Debug)]
pub struct SessionSummary {
    /// Session key (e.g. `"telegram:12345"`).
    pub key: String,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// When the session was last updated.
    pub updated_at: DateTime<Utc>,
    /// Path to the JSONL file.
    pub path: PathBuf,
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_manager() -> (SessionManager, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let mgr = SessionManager::new(Some(dir.path().to_path_buf())).unwrap();
        (mgr, dir)
    }

    #[test]
    fn test_get_or_create_new_session() {
        let (mgr, _dir) = make_manager();
        let session = mgr.get_or_create("telegram:12345");
        assert_eq!(session.key, "telegram:12345");
        assert!(session.messages.is_empty());
    }

    #[test]
    fn test_get_or_create_returns_cached() {
        let (mgr, _dir) = make_manager();
        mgr.add_message("test:1", Message::user("hello"));
        let session = mgr.get_or_create("test:1");
        assert_eq!(session.messages.len(), 1);
    }

    #[test]
    fn test_add_message() {
        let (mgr, _dir) = make_manager();
        mgr.add_message("test:1", Message::user("hello"));
        mgr.add_message("test:1", Message::assistant("hi there!"));

        let session = mgr.get_or_create("test:1");
        assert_eq!(session.messages.len(), 2);
    }

    #[test]
    fn test_get_history() {
        let (mgr, _dir) = make_manager();
        for i in 0..10 {
            mgr.add_message("test:1", Message::user(format!("msg {}", i)));
        }

        let history = mgr.get_history("test:1", 3);
        assert_eq!(history.len(), 3);
        // Should be the last 3
        match &history[0] {
            Message::User { content: crate::types::MessageContent::Text(text), .. } => {
                assert_eq!(text, "msg 7");
            }
            _ => panic!("Expected user message"),
        }
    }

    #[test]
    fn test_get_history_less_than_max() {
        let (mgr, _dir) = make_manager();
        mgr.add_message("test:1", Message::user("one"));
        mgr.add_message("test:1", Message::user("two"));

        let history = mgr.get_history("test:1", 50);
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_clear_session() {
        let (mgr, _dir) = make_manager();
        mgr.add_message("test:1", Message::user("hello"));
        mgr.add_message("test:1", Message::assistant("hi"));

        mgr.clear("test:1");

        let session = mgr.get_or_create("test:1");
        assert!(session.messages.is_empty());
    }

    #[test]
    fn test_delete_session() {
        let (mgr, _dir) = make_manager();
        mgr.add_message("test:1", Message::user("hello"));

        let existed = mgr.delete("test:1");
        assert!(existed);

        // After delete, get_or_create returns fresh
        let session = mgr.get_or_create("test:1");
        assert!(session.messages.is_empty());
    }

    #[test]
    fn test_delete_nonexistent() {
        let (mgr, _dir) = make_manager();
        let existed = mgr.delete("nonexistent:key");
        assert!(!existed);
    }

    #[test]
    fn test_persistence_round_trip() {
        let dir = tempdir().unwrap();

        // Create manager, add messages
        {
            let mgr = SessionManager::new(Some(dir.path().to_path_buf())).unwrap();
            mgr.add_message("telegram:42", Message::system("You are Oxibot."));
            mgr.add_message("telegram:42", Message::user("Hello"));
            mgr.add_message("telegram:42", Message::assistant("Hi! How can I help?"));
        }

        // New manager (empty cache) should load from disk
        {
            let mgr = SessionManager::new(Some(dir.path().to_path_buf())).unwrap();
            let session = mgr.get_or_create("telegram:42");
            assert_eq!(session.messages.len(), 3);
            assert_eq!(session.key, "telegram:42");
        }
    }

    #[test]
    fn test_session_file_format() {
        let dir = tempdir().unwrap();
        let mgr = SessionManager::new(Some(dir.path().to_path_buf())).unwrap();

        mgr.add_message("cli:local", Message::user("test message"));

        // Check the JSONL file exists and has correct format
        let path = dir.path().join("cli_local.jsonl");
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 2); // 1 metadata + 1 message

        // First line is metadata
        let meta: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(meta["_type"], "metadata");

        // Second line is the message
        let msg: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(msg["role"], "user");
        assert_eq!(msg["content"], "test message");
    }

    #[test]
    fn test_list_sessions() {
        let dir = tempdir().unwrap();
        let mgr = SessionManager::new(Some(dir.path().to_path_buf())).unwrap();

        mgr.add_message("telegram:1", Message::user("a"));
        mgr.add_message("discord:2", Message::user("b"));
        mgr.add_message("cli:3", Message::user("c"));

        let sessions = mgr.list_sessions();
        assert_eq!(sessions.len(), 3);
        // Should contain all keys
        let keys: Vec<&str> = sessions.iter().map(|s| s.key.as_str()).collect();
        assert!(keys.contains(&"telegram:1"));
        assert!(keys.contains(&"discord:2"));
        assert!(keys.contains(&"cli:3"));
    }

    #[test]
    fn test_multiple_sessions_independent() {
        let (mgr, _dir) = make_manager();
        mgr.add_message("a:1", Message::user("hello a"));
        mgr.add_message("b:2", Message::user("hello b"));
        mgr.add_message("b:2", Message::user("hello b again"));

        assert_eq!(mgr.get_history("a:1", 50).len(), 1);
        assert_eq!(mgr.get_history("b:2", 50).len(), 2);
    }

    #[test]
    fn test_clear_persists_to_disk() {
        let dir = tempdir().unwrap();

        {
            let mgr = SessionManager::new(Some(dir.path().to_path_buf())).unwrap();
            mgr.add_message("test:1", Message::user("hello"));
            mgr.add_message("test:1", Message::assistant("hi"));
            mgr.clear("test:1");
        }

        // Reload from disk
        {
            let mgr = SessionManager::new(Some(dir.path().to_path_buf())).unwrap();
            let session = mgr.get_or_create("test:1");
            assert!(session.messages.is_empty());
        }
    }
}
