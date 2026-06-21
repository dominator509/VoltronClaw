//! `voltron-ractor` — Ractor-based actor model for multi-agent orchestration.
//!
//! ## Architecture
//!
//! Each agent runs as a [`ractor::Actor`] with its own mailbox, state, and
//! lifecycle hooks. Agents subscribe to named `Topic<AgentTask>` channels
//! and process tasks concurrently.
//!
//! ```text
//!                    ┌──────────────────┐
//!                    │   AgentRegistry  │
//!                    │  (pub/sub hub)   │
//!                    └───┬───────┬──────┘
//!                        │       │
//!              ┌─────────┘       └─────────┐
//!              ▼                           ▼
//!     ┌────────────────┐          ┌────────────────┐
//!     │  AgentActor A  │          │  AgentActor B  │
//!     │  (mailbox)     │          │  (mailbox)     │
//!     │  LLMProvider   │          │  LLMProvider   │
//!     │  MemoryStore   │          │  MemoryStore   │
//!     └────────────────┘          └────────────────┘
//! ```
//!
//! ## Features
//!
//! - Topic-based task dispatch (any agent can subscribe to any topic)
//! - Typed task messages with request/reply pattern
//! - Lifecycle hooks: `on_start`, `on_shutdown`, `on_error`
//! - Concurrent task processing via ractor's actor model
//! - Graceful shutdown with drain timeout

use voltron_core::types::Message;

pub mod actor;
pub mod handle;
pub mod runtime;

// ── Task messages ─────────────────────────────────────────────────

/// The task message envelope passed through ractor mailboxes.
///
/// Each variant carries a `reply_to` address for the result.
#[derive(Debug)]
pub enum AgentTask {
    /// Process a user/conversation message and produce a response.
    ProcessMessage {
        /// The incoming message to process.
        message: Message,
        /// Reply channel for the processed response.
        reply_to: tokio::sync::oneshot::Sender<Result<AgentResponse, AgentError>>,
    },
    /// Load the agent's state from memory (e.g., after restart).
    Reload {
        reply_to: tokio::sync::oneshot::Sender<Result<(), AgentError>>,
    },
    /// Shutdown the agent gracefully.
    Shutdown {
        reply_to: tokio::sync::oneshot::Sender<Result<(), AgentError>>,
    },
}

/// The response from processing a message.
#[derive(Debug, Clone)]
pub struct AgentResponse {
    /// The agent's textual response.
    pub content: String,
    /// Any tool calls the agent wants to execute.
    pub tool_calls: Vec<voltron_core::types::ToolCall>,
    /// Whether the agent considers this conversation turn complete.
    pub finished: bool,
}

/// Errors that can occur during agent task processing.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AgentError {
    #[error("LLM provider error: {0}")]
    LLMError(String),
    #[error("Memory error: {0}")]
    MemoryError(String),
    #[error("Skill execution error: {0}")]
    SkillError(String),
    #[error("Agent is shutting down")]
    ShuttingDown,
    #[error("Task cancelled")]
    Cancelled,
    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<voltron_core::VoltronError> for AgentError {
    fn from(e: voltron_core::VoltronError) -> Self {
        AgentError::Internal(e.to_string())
    }
}
