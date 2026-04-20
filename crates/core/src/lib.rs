//! thclaws-core: native Rust AI coding agent library.
//!
//! Module layout follows the phased port plan in `dev-log/007-native-port-plan.md`.
//! Phase 5 lands the foundations: errors, types, config, token estimation.
//! Higher layers (providers, tools, context, agent, repl) land in later phases.

pub mod agent;
pub mod agent_defs;
pub mod commands;
pub mod compaction;
pub mod config;
pub mod context;
pub mod dotenv;
pub mod endpoints;
pub mod error;
#[cfg(feature = "gui")]
pub mod gui;
pub mod hooks;
pub mod kms;
pub mod mcp;
pub mod memory;
pub mod oauth;
pub mod permissions;
pub mod plugins;
pub mod prompts;
pub mod providers;
pub mod repl;
pub mod sandbox;
pub mod secrets;
pub mod session;
pub mod skills;
pub mod subagent;
pub mod team;
pub mod tokens;
pub mod tools;
pub mod types;
pub mod usage;
pub mod version;

pub use error::{Error, Result};
