//! LLM provider layer for Oxibot.
//!
//! Replaces nanobot's LiteLLM dependency with direct HTTP clients.
//!
//! # Architecture
//!
//! - [`traits::LlmProvider`] — trait that all providers implement
//! - [`registry`] — static specs for all 12 supported providers + matching logic
//! - [`http_provider::HttpProvider`] — generic OpenAI-compatible HTTP client
//! - [`http_provider::create_provider`] — convenience builder from model name + config

pub mod http_provider;
pub mod registry;
pub mod traits;
pub mod transcription;

// Re-export main types for convenience
pub use http_provider::{create_provider, HttpProvider};
pub use registry::{ProviderConfig, ProviderSpec, PROVIDERS};
pub use traits::{LlmProvider, LlmRequestConfig};
pub use transcription::{GroqTranscriber, TranscriptionProvider};
