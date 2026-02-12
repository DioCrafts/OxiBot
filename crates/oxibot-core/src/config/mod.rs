//! Configuration system — schema, loading, and env var overrides.
//!
//! # Usage
//! ```no_run
//! use oxibot_core::config;
//!
//! let cfg = config::load_config(None);
//! println!("Model: {}", cfg.agents.defaults.model);
//! ```

pub mod loader;
pub mod schema;

// Re-export key types
pub use loader::{get_config_path, load_config, save_config};
pub use schema::Config;
