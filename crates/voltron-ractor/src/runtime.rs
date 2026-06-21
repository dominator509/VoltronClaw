//! Typed multi-agent runtime with topic-based pub/sub dispatch.
//!
//! The `ActorRuntime` manages a collection of `ActorAgentHandle`s
//! and routes incoming messages to the appropriate agents based on
//! topic subscriptions.

use std::collections::HashMap;
use std::sync::Arc;

use ractor::SpawnErr;
use tracing::{debug, error, info, warn};

use voltron_core::traits::{LLMProvider, MemoryStore, SkillExecutor};
use voltron_core::types::Message;

use crate::actor::AgentConfig;
use crate::handle::ActorAgentHandle;
use crate::AgentError;

/// Manages multiple agent actors with topic-based routing.
pub struct ActorRuntime {
    /// Agent handles keyed by agent_id.
    agents: HashMap<String, ActorAgentHandle>,
    /// Topic → list of agent_ids that subscribe to it.
    topic_subscribers: HashMap<String, Vec<String>>,
    /// Default agent to use when no topic matches.
    default_agent: Option<String>,
}

impl ActorRuntime {
    /// Create a new empty runtime.
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            topic_subscribers: HashMap::new(),
            default_agent: None,
        }
    }

    /// Set the default agent (used when no topic matches).
    pub fn set_default_agent(&mut self, agent_id: &str) {
        self.default_agent = Some(agent_id.to_string());
    }

    /// Register a new agent in the runtime.
    ///
    /// The agent will be spawned as a ractor actor.
    pub async fn register(
        &mut self,
        config: AgentConfig,
        llm: Arc<dyn LLMProvider>,
        memory: Arc<dyn MemoryStore>,
        skills: Arc<dyn SkillExecutor>,
    ) -> Result<(), SpawnErr> {
        let agent_id = config.agent_id.clone();
        let topics = config.topics.clone();

        let handle = ActorAgentHandle::spawn(config, llm, memory, skills).await?;

        // Register topic subscriptions
        for topic in &topics {
            self.topic_subscribers
                .entry(topic.clone())
                .or_default()
                .push(agent_id.clone());
        }

        info!(
            agent_id = %agent_id,
            topic_count = topics.len(),
            "Agent registered in runtime"
        );

        self.agents.insert(agent_id, handle);
        Ok(())
    }

    /// Route a message to all agents subscribed to the given topic.
    ///
    /// Returns a map of agent_id → response for each agent that
    /// processed the message.
    pub async fn publish(
        &self,
        topic: &str,
        message: Message,
    ) -> HashMap<String, Result<crate::AgentResponse, AgentError>> {
        let mut results = HashMap::new();

        let subscribers = match self.topic_subscribers.get(topic) {
            Some(agents) if !agents.is_empty() => agents.clone(),
            _ => {
                // Fall back to default agent
                if let Some(ref default_id) = self.default_agent {
                    vec![default_id.clone()]
                } else {
                    warn!(topic, "No subscribers for topic and no default agent");
                    return results;
                }
            }
        };

        for agent_id in &subscribers {
            let handle = match self.agents.get(agent_id) {
                Some(h) => h,
                None => {
                    warn!(agent_id, "Agent not found for topic subscriber");
                    continue;
                }
            };

            debug!(
                agent_id = %agent_id,
                topic,
                "Dispatching message to agent"
            );

            let response = handle.process_message(message.clone()).await;
            results.insert(agent_id.clone(), response);
        }

        results
    }

    /// Send a message directly to a specific agent (bypassing topics).
    pub async fn send_to(
        &self,
        agent_id: &str,
        message: Message,
    ) -> Result<crate::AgentResponse, AgentError> {
        let handle = self
            .agents
            .get(agent_id)
            .ok_or_else(|| AgentError::Internal(format!("Agent not found: {}", agent_id)))?;

        handle.process_message(message).await
    }

    /// Get a handle to a specific agent.
    pub fn get_agent(&self, agent_id: &str) -> Option<&ActorAgentHandle> {
        self.agents.get(agent_id)
    }

    /// List all registered agent ids.
    pub fn agent_ids(&self) -> Vec<&str> {
        self.agents.keys().map(|s| s.as_str()).collect()
    }

    /// Shutdown all agents gracefully.
    pub async fn shutdown_all(&self) {
        for (agent_id, handle) in &self.agents {
            info!(agent_id = %agent_id, "Requesting shutdown");
            if let Err(e) = handle.shutdown().await {
                error!(agent_id = %agent_id, error = %e, "Shutdown error");
            }
        }
        info!("All agents shutdown complete");
    }

    /// Number of registered agents.
    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }
}

impl Default for ActorRuntime {
    fn default() -> Self {
        Self::new()
    }
}
