//! Agent actor implementation — wraps core Voltron traits in a ractor actor.
//!
//! Each `AgentActor` owns:
//! - An `Arc<dyn LLMProvider>` for LLM calls
//! - An `Arc<dyn MemoryStore>` for conversation persistence
//! - An `Arc<dyn SkillExecutor>` for tool execution
//!
//! The actor processes `AgentTask` messages sequentially (ractor guarantees
//! single-threaded message processing per actor).

use std::collections::HashMap;
use std::sync::Arc;

use ractor::{Actor, ActorProcessingErr, ActorRef, SpawnErr};
use tracing::{debug, error, info, warn};

use voltron_core::traits::{LLMProvider, MemoryStore, SkillExecutor};
use voltron_core::types::{MemoryRecord, Message};

use crate::{AgentError, AgentResponse, AgentTask};

// ── Agent configuration ───────────────────────────────────────────

/// Configuration for an AgentActor.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Unique agent identifier (e.g., "ip-man", "deziray").
    pub agent_id: String,
    /// Human-readable description of the agent's role.
    pub description: String,
    /// System prompt injected before every conversation turn.
    pub system_prompt: String,
    /// Maximum conversation history messages to retain.
    pub max_history: usize,
    /// Topics this agent subscribes to for task dispatch.
    pub topics: Vec<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            agent_id: uuid::Uuid::new_v4().to_string(),
            description: String::new(),
            system_prompt: String::new(),
            max_history: 100,
            topics: Vec::new(),
        }
    }
}

// ── Agent state ───────────────────────────────────────────────────

/// Internal state of the agent actor.
pub struct AgentState {
    pub config: AgentConfig,
    pub llm: Arc<dyn LLMProvider>,
    pub memory: Arc<dyn MemoryStore>,
    pub skills: Arc<dyn SkillExecutor>,
    /// Conversation history (most recent first).
    pub history: Vec<Message>,
    /// Whether the agent is shutting down.
    pub shutting_down: bool,
}

// ── Startup arguments ─────────────────────────────────────────────

/// Arguments passed to `pre_start` to initialize the agent state.
pub struct AgentArgs {
    pub config: AgentConfig,
    pub llm: Arc<dyn LLMProvider>,
    pub memory: Arc<dyn MemoryStore>,
    pub skills: Arc<dyn SkillExecutor>,
}

// ── Actor definition ──────────────────────────────────────────────

pub struct AgentActor;

impl AgentActor {
    /// Spawn a new agent actor and return an `ActorRef` to it.
    pub async fn spawn(
        config: AgentConfig,
        llm: Arc<dyn LLMProvider>,
        memory: Arc<dyn MemoryStore>,
        skills: Arc<dyn SkillExecutor>,
    ) -> Result<ActorRef<AgentTask>, SpawnErr> {
        let agent_id = config.agent_id.clone();

        let args = AgentArgs {
            config,
            llm,
            memory,
            skills,
        };

        let (actor_ref, _handle) = <AgentActor as Actor>::spawn(
            Some(format!("agent-{}", agent_id)),
            AgentActor,
            args,
        )
        .await?;

        info!(
            agent_id = %agent_id,
            "Agent actor spawned"
        );

        Ok(actor_ref)
    }

    /// Build conversation messages from history + system prompt + new message.
    fn build_messages(state: &AgentState, incoming: &Message) -> Vec<Message> {
        let mut messages = Vec::with_capacity(state.history.len() + 2);

        // System prompt
        messages.push(Message {
            role: "system".to_string(),
            content: state.config.system_prompt.clone(),
            name: None,
            tool_calls: vec![],
            tool_call_id: None,
        });

        // History (oldest first, since we store newest first)
        for msg in state.history.iter().rev() {
            messages.push(msg.clone());
        }

        // Incoming message
        messages.push(incoming.clone());

        messages
    }

    /// Persist a conversation message to memory.
    async fn record_message(
        memory: &Arc<dyn MemoryStore>,
        agent_id: &str,
        msg: &Message,
    ) -> Result<(), AgentError> {
        let record = MemoryRecord {
            id: uuid::Uuid::new_v4().to_string(),
            content: msg.content.clone(),
            tags: vec![
                format!("agent:{}", agent_id),
                format!("role:{}", msg.role),
            ],
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            metadata: HashMap::new(),
        };

        memory
            .put(record)
            .await
            .map_err(|e| AgentError::MemoryError(e.to_string()))?;
        Ok(())
    }

    /// Create a Message with required fields filled.
    fn assistant_msg(content: &str) -> Message {
        Message {
            role: "assistant".to_string(),
            content: content.to_string(),
            name: None,
            tool_calls: vec![],
            tool_call_id: None,
        }
    }

    fn user_msg_from_record(r: MemoryRecord) -> Message {
        let role = r
            .tags
            .iter()
            .find(|t| t.starts_with("role:"))
            .map(|t| t.trim_start_matches("role:").to_string())
            .unwrap_or_else(|| "user".to_string());

        Message {
            role,
            content: r.content,
            name: None,
            tool_calls: vec![],
            tool_call_id: None,
        }
    }
}

// ── Ractor Actor implementation ───────────────────────────────────

#[ractor::async_trait]
impl Actor for AgentActor {
    type Msg = AgentTask;
    type State = AgentState;
    type Arguments = AgentArgs;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        info!(
            agent_id = %args.config.agent_id,
            description = %args.config.description,
            "Agent actor starting"
        );

        Ok(AgentState {
            config: args.config,
            llm: args.llm,
            memory: args.memory,
            skills: args.skills,
            history: Vec::new(),
            shutting_down: false,
        })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        task: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match task {
            AgentTask::ProcessMessage { message, reply_to } => {
                if state.shutting_down {
                    let _ = reply_to.send(Err(AgentError::ShuttingDown));
                    return Ok(());
                }

                debug!(
                    agent_id = %state.config.agent_id,
                    role = %message.role,
                    content_len = message.content.len(),
                    "Processing message"
                );

                // Record incoming message
                if let Err(e) = Self::record_message(
                    &state.memory,
                    &state.config.agent_id,
                    &message,
                )
                .await
                {
                    warn!("Failed to record message: {}", e);
                }

                // Build conversation context
                let messages = Self::build_messages(state, &message);

                // Call LLM
                let response = match state.llm.generate(&messages, &[]).await {
                    Ok(resp) => resp,
                    Err(e) => {
                        error!("LLM call failed: {}", e);
                        let _ = reply_to.send(Err(AgentError::LLMError(e.to_string())));
                        return Ok(());
                    }
                };

                // Record assistant response
                let assistant_msg = Self::assistant_msg(&response.content);
                if let Err(e) = Self::record_message(
                    &state.memory,
                    &state.config.agent_id,
                    &assistant_msg,
                )
                .await
                {
                    warn!("Failed to record response: {}", e);
                }

                // Update history and enforce max history
                state.history.push(message);
                state.history.push(assistant_msg);

                while state.history.len() > state.config.max_history * 2 {
                    state.history.remove(0);
                    state.history.remove(0); // Remove paired user+assistant
                }

                let agent_response = AgentResponse {
                    content: response.content,
                    tool_calls: response.tool_calls,
                    finished: response.finish_reason.as_deref() == Some("stop"),
                };

                let _ = reply_to.send(Ok(agent_response));
            }

            AgentTask::Reload { reply_to } => {
                // Reload conversation history from memory
                let tags = vec![format!("agent:{}", state.config.agent_id)];
                match state.memory.search(&tags).await {
                    Ok(records) => {
                        state.history = records
                            .into_iter()
                            .map(Self::user_msg_from_record)
                            .collect();
                        info!(
                            agent_id = %state.config.agent_id,
                            history_len = state.history.len(),
                            "Agent state reloaded"
                        );
                        let _ = reply_to.send(Ok(()));
                    }
                    Err(e) => {
                        warn!("Failed to reload history: {}", e);
                        let _ = reply_to.send(Err(AgentError::MemoryError(e.to_string())));
                    }
                }
            }

            AgentTask::Shutdown { reply_to } => {
                info!(
                    agent_id = %state.config.agent_id,
                    "Agent shutting down"
                );
                state.shutting_down = true;
                let _ = reply_to.send(Ok(()));
            }
        }

        Ok(())
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        info!(
            agent_id = %state.config.agent_id,
            "Agent actor stopped"
        );
        Ok(())
    }
}
