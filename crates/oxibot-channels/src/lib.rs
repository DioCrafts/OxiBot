//! Oxibot Channels — chat channel integrations.
//!
//! This crate provides:
//! - **base**: The `Channel` trait that all channel implementations must satisfy
//! - **manager**: `ChannelManager` — lifecycle orchestration and outbound message routing
//!
//! Individual channel implementations (Telegram, Discord, etc.) will be added
//! as feature-gated modules.

pub mod base;
pub mod formatting;
pub mod manager;

#[cfg(feature = "telegram")]
pub mod telegram;

#[cfg(feature = "discord")]
pub mod discord;

#[cfg(feature = "whatsapp")]
pub mod whatsapp;

#[cfg(feature = "slack")]
pub mod slack;

#[cfg(feature = "email")]
pub mod email;

pub use base::Channel;
pub use manager::ChannelManager;
