//! Session manager — in-memory cache + JSONL file persistence.
//!
//! Replaces nanobot's `session/manager.py`.
//!
//! # Disk format (JSONL)
//!
//! Each session is a `.jsonl` file under `~/.oxibot/sessions/`.
//! - Line 1: metadata `{"_type": "metadata", "created_at": "...", "updated_at": "...", "metadata": {}}`
//! - Lines 2+: messages `{"role": "user", "content": "hello", "timestamp": "..."}`

pub mod manager;

pub use manager::SessionManager;
