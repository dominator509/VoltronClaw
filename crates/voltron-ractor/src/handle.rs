//! Actor agent handle — public API for interacting with a spawned agent.
//!
//! Wraps an `ractor::ActorRef<AgentTask>` and provides ergonomic
//! async methods for message processing, reload, and shutdown.

use ractor::ActorRef;
use tokio::sync::oneshot;

use crate::actor::{AgentActor, AgentConfig};
use crate::{AgentError, AgentResponse, AgentTask};
use std::sync::Arc;
use voltron_core::traits::{LLMProvider, MemoryStore, SkillExecutor};
use voltron_core::types::Message;

/// A handle to a spawned agent actor.
///
/// Cloning is cheap — it clones the internal `ActorRef`.
#[derive(Clone)]
pub struct ActorAgentHandle {
    actor_ref: ActorRef<AgentTask>,
    agent_id: String,
}

impl ActorAgentHandle {
    /// Spawn a new agent actor and return a handle to it.
    pub async fn spawn(
        config: AgentConfig,
        llm: Arc<dyn LLMProvider>,
        memory: Arc<dyn MemoryStore>,
        skills: Arc<dyn SkillExecutor>,
    ) -> Result<Self, ractor::SpawnErr> {
        let agent_id = config.agent_id.clone();
        let actor_ref = AgentActor::spawn(config, llm, memory, skills).await?;
        Ok(Self {
            actor_ref,
            agent_id,
        })
    }

    /// Get the agent's identifier.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Send a message to the agent for processing.
    ///
    /// Returns the agent's response, including any tool calls.
    pub async fn process_message(&self, message: Message) -> Result<AgentResponse, AgentError> {
        let (tx, rx) = oneshot::channel();

        self.actor_ref
            .cast(AgentTask::ProcessMessage {
                message,
                reply_to: tx,
            })
            .map_err(|e| AgentError::Internal(format!("failed to send task: {}", e)))?;

        rx.await
            .map_err(|_| AgentError::Cancelled)?
    }

    /// Reload the agent's conversation history from memory.
    pub async fn reload(&self) -> Result<(), AgentError> {
        let (tx, rx) = oneshot::channel();

        self.actor_ref
            .cast(AgentTask::Reload {
                reply_to: tx,
            })
            .map_err(|e| AgentError::Internal(format!("failed to send reload: {}", e)))?;

        rx.await
            .map_err(|_| AgentError::Cancelled)?
    }

    /// Initiate graceful shutdown of the agent.
    ///
    /// After shutdown, the agent will reject new `ProcessMessage` tasks.
    /// Existing tasks in the mailbox will still be processed.
    pub async fn shutdown(&self) -> Result<(), AgentError> {
        let (tx, rx) = oneshot::channel();

        self.actor_ref
            .cast(AgentTask::Shutdown { reply_to: tx })
            .map_err(|e| AgentError::Internal(format!("failed to send shutdown: {}", e)))?;

        rx.await
            .map_err(|_| AgentError::Cancelled)?
    }

    /// Check if the agent actor is still alive.
    ///
    /// Sends a lightweight no-op ping and returns true if the actor responds.
    /// Returns false if the actor is dead or unresponsive.
    pub async fn is_alive(&self) -> bool {
        let (tx, rx) = oneshot::channel();
        if self
            .actor_ref
            .cast(AgentTask::Reload { reply_to: tx })
            .is_err()
        {
            return false;
        }
        // Don't block waiting — if the actor responds at all, it's alive
        tokio::time::timeout(std::time::Duration::from_millis(100), rx)
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false)
    }
}
