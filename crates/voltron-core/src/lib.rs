//! voltron-core — trait definitions, error types, and data structures
//! for the Voltron Claw composite agent.
//!
//! This crate defines the five core async traits (`LLMProvider`,
//! `MemoryStore`, `SkillExecutor`, `ChannelAdapter`, `AuditSink`)
//! and the shared types (`Message`, `LLMResponse`, `MemoryRecord`,
//! `SkillManifest`, `SkillResult`, `AuditEntry`) that every other
//! crate depends on.
//!
//! # Version
//! Workspace version — see root `Cargo.toml`.

pub mod error;
pub mod traits;
pub mod types;

// Re-export everything so downstream crates can write
// `use voltron_core::*;` or `use voltron_core::traits::*;`.
pub use error::VoltronError;
pub use traits::{AuditSink, ChannelAdapter, LLMProvider, MemoryStore, SkillExecutor};
pub use types::{
    AuditEntry, LLMResponse, MemoryRecord, Message, SkillManifest, SkillResult, ToolCall,
    ToolDefinition,
};
