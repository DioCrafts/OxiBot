//! Tool modules for Oxibot agent.

pub mod base;
pub mod registry;
pub mod filesystem;
pub mod shell;
pub mod web;
pub mod message;
pub mod spawn;

pub use base::{Tool, require_string, optional_string, optional_i64, optional_bool};
pub use registry::ToolRegistry;
