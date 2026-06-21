//! Integration tests for voltron-ractor using mock providers.
//!
//! These tests verify:
//! - Agent actor spawns and processes messages
//! - ActorAgentHandle API works end-to-end
//! - ActorRuntime topic dispatch routes correctly
//! - Shutdown works gracefully

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use voltron_core::{
    LLMProvider, LLMResponse, MemoryStore, MemoryRecord, Message, SkillExecutor,
    SkillManifest, SkillResult, VoltronError,
};
use voltron_ractor::{
    actor::AgentConfig,
    handle::ActorAgentHandle,
    runtime::ActorRuntime,
};

// ── Mock LLM Provider ─────────────────────────────────────────────

struct MockLLM {
    prefix: String,
    calls: Mutex<Vec<Vec<Message>>>,
}

impl MockLLM {
    fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_string(),
            calls: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl LLMProvider for MockLLM {
    async fn generate(
        &self,
        messages: &[Message],
        _tools: &[voltron_core::ToolDefinition],
    ) -> Result<LLMResponse, VoltronError> {
        self.calls.lock().unwrap().push(messages.to_vec());
        Ok(LLMResponse {
            content: format!("{}-response", self.prefix),
            tool_calls: vec![],
            finish_reason: Some("stop".to_string()),
            metadata: HashMap::new(),
        })
    }

    fn provider_name(&self) -> &str {
        "mock"
    }
}

// ── Mock Memory Store ─────────────────────────────────────────────

struct MockMemory {
    records: Mutex<HashMap<String, MemoryRecord>>,
}

impl MockMemory {
    fn new() -> Self {
        Self {
            records: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl MemoryStore for MockMemory {
    async fn put(&self, record: MemoryRecord) -> Result<(), VoltronError> {
        self.records.lock().unwrap().insert(record.id.clone(), record);
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<MemoryRecord>, VoltronError> {
        Ok(self.records.lock().unwrap().get(id).cloned())
    }

    async fn search(&self, tags: &[String]) -> Result<Vec<MemoryRecord>, VoltronError> {
        let records = self.records.lock().unwrap();
        let results: Vec<_> = records
            .values()
            .filter(|r| tags.iter().all(|t| r.tags.contains(t)))
            .cloned()
            .collect();
        Ok(results)
    }

    async fn delete(&self, _id: &str) -> Result<(), VoltronError> {
        Ok(())
    }
}

// ── Mock Skill Executor ───────────────────────────────────────────

struct MockSkills;

#[async_trait]
impl SkillExecutor for MockSkills {
    async fn execute(
        &self,
        _skill_id: &str,
        _args: serde_json::Value,
    ) -> Result<SkillResult, VoltronError> {
        Ok(SkillResult {
            output: serde_json::json!({"result": "mock"}),
            elapsed_ms: 1,
            success: true,
            error: None,
        })
    }

    fn manifests(&self) -> Vec<SkillManifest> {
        vec![]
    }

    fn manifest(&self, _skill_id: &str) -> Option<SkillManifest> {
        None
    }
}

// ── Helper ────────────────────────────────────────────────────────

fn test_config(agent_id: &str) -> AgentConfig {
    AgentConfig {
        agent_id: agent_id.to_string(),
        description: format!("Test agent {}", agent_id),
        system_prompt: "You are a test agent.".to_string(),
        max_history: 10,
        topics: vec!["test".to_string()],
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_spawn_and_process_message() {
    let llm = Arc::new(MockLLM::new("alpha"));
    let memory = Arc::new(MockMemory::new());
    let skills = Arc::new(MockSkills);

    let handle = ActorAgentHandle::spawn(
        test_config("agent-a"),
        llm.clone(),
        memory,
        skills,
    )
    .await
    .expect("spawn should succeed");

    assert_eq!(handle.agent_id(), "agent-a");

    let response = handle
        .process_message(Message::user("Hello, world!"))
        .await
        .expect("process_message should succeed");

    assert_eq!(response.content, "alpha-response");
    assert!(response.tool_calls.is_empty());
    assert!(response.finished);
}

#[tokio::test]
async fn test_message_persisted_to_memory() {
    let llm = Arc::new(MockLLM::new("beta"));
    let memory = Arc::new(MockMemory::new());
    let skills = Arc::new(MockSkills);

    let handle = ActorAgentHandle::spawn(
        test_config("agent-b"),
        llm,
        memory.clone(),
        skills,
    )
    .await
    .expect("spawn should succeed");

    handle
        .process_message(Message::user("Store this"))
        .await
        .expect("should succeed");

    // Search for messages tagged with this agent
    let records = memory
        .search(&["agent:agent-b".to_string()])
        .await
        .expect("search should succeed");

    assert!(!records.is_empty(), "should have stored messages");
}

#[tokio::test]
async fn test_shutdown_rejects_new_messages() {
    let llm = Arc::new(MockLLM::new("gamma"));
    let memory = Arc::new(MockMemory::new());
    let skills = Arc::new(MockSkills);

    let handle = ActorAgentHandle::spawn(
        test_config("agent-c"),
        llm,
        memory,
        skills,
    )
    .await
    .expect("spawn should succeed");

    // Shutdown
    handle.shutdown().await.expect("shutdown should succeed");

    // Subsequent messages should be rejected
    let result = handle.process_message(Message::user("after shutdown")).await;
    assert!(result.is_err(), "should reject message after shutdown");
}

#[tokio::test]
async fn test_actor_runtime_publish_to_topic() {
    let mut runtime = ActorRuntime::new();

    let llm1 = Arc::new(MockLLM::new("one"));
    let memory1 = Arc::new(MockMemory::new());
    let skills1 = Arc::new(MockSkills);

    let llm2 = Arc::new(MockLLM::new("two"));
    let memory2 = Arc::new(MockMemory::new());
    let skills2 = Arc::new(MockSkills);

    // Register two agents on the "publish-test" topic
    runtime
        .register(
            AgentConfig {
                agent_id: "pub-a".to_string(),
                description: "Publisher A".into(),
                system_prompt: "A".into(),
                max_history: 5,
                topics: vec!["publish-test".to_string()],
            },
            llm1,
            memory1,
            skills1,
        )
        .await
        .expect("register a");

    runtime
        .register(
            AgentConfig {
                agent_id: "pub-b".to_string(),
                description: "Publisher B".into(),
                system_prompt: "B".into(),
                max_history: 5,
                topics: vec!["publish-test".to_string()],
            },
            llm2,
            memory2,
            skills2,
        )
        .await
        .expect("register b");

    assert_eq!(runtime.agent_count(), 2);

    let results = runtime
        .publish("publish-test", Message::user("broadcast"))
        .await;

    assert_eq!(results.len(), 2);
    for (_, result) in &results {
        assert!(result.is_ok(), "agent should respond successfully");
    }
}

#[tokio::test]
async fn test_actor_runtime_send_to_specific_agent() {
    let mut runtime = ActorRuntime::new();

    let llm = Arc::new(MockLLM::new("direct"));
    let memory = Arc::new(MockMemory::new());
    let skills = Arc::new(MockSkills);

    runtime
        .register(
            AgentConfig {
                agent_id: "direct-agent".to_string(),
                description: "Direct target".into(),
                system_prompt: "Direct".into(),
                max_history: 5,
                topics: vec!["other".to_string()],
            },
            llm,
            memory,
            skills,
        )
        .await
        .expect("register");

    let response = runtime
        .send_to("direct-agent", Message::user("ping"))
        .await
        .expect("send_to should succeed");

    assert_eq!(response.content, "direct-response");
}

#[tokio::test]
async fn test_publish_falls_back_to_default_agent() {
    let mut runtime = ActorRuntime::new();

    let llm = Arc::new(MockLLM::new("default"));
    let memory = Arc::new(MockMemory::new());
    let skills = Arc::new(MockSkills);

    runtime
        .register(
            AgentConfig {
                agent_id: "fallback-agent".to_string(),
                description: "Fallback".into(),
                system_prompt: "FB".into(),
                max_history: 5,
                topics: vec![], // subscribes to nothing
            },
            llm,
            memory,
            skills,
        )
        .await
        .expect("register");

    runtime.set_default_agent("fallback-agent");

    // Publish to a topic with no subscribers
    let results = runtime
        .publish("unused-topic", Message::user("hi"))
        .await;

    assert_eq!(results.len(), 1);
    assert!(results.contains_key("fallback-agent"));
}

#[tokio::test]
async fn test_reload_history_from_memory() {
    let llm = Arc::new(MockLLM::new("reload"));
    let memory = Arc::new(MockMemory::new());
    let skills = Arc::new(MockSkills);

    let handle = ActorAgentHandle::spawn(
        AgentConfig {
            agent_id: "reload-agent".to_string(),
            description: "reloader".into(),
            system_prompt: "Reload".into(),
            max_history: 20,
            topics: vec!["test".to_string()],
        },
        llm.clone(),
        memory.clone(),
        skills,
    )
    .await
    .expect("spawn should succeed");

    // Process a message to populate memory
    handle
        .process_message(Message::user("first message"))
        .await
        .expect("first message should succeed");

    handle
        .process_message(Message::user("second message"))
        .await
        .expect("second message should succeed");

    // Reload should succeed (verifies no panic, even with partial history)
    handle.reload().await.expect("reload should succeed");
}

#[tokio::test]
async fn test_runtime_send_to_nonexistent_agent() {
    let runtime = ActorRuntime::new();

    let result = runtime
        .send_to("ghost", Message::user("hello"))
        .await;

    assert!(result.is_err(), "should error for nonexistent agent");
}

#[tokio::test]
async fn test_runtime_publish_no_subscribers_no_default() {
    let runtime = ActorRuntime::new();

    let results = runtime
        .publish("orphan-topic", Message::user("anyone?"))
        .await;

    assert!(results.is_empty(), "should be empty with no subscribers and no default");
}

#[tokio::test]
async fn test_multiple_topics_single_agent() {
    let mut runtime = ActorRuntime::new();

    let llm = Arc::new(MockLLM::new("multi"));
    let memory = Arc::new(MockMemory::new());
    let skills = Arc::new(MockSkills);

    runtime
        .register(
            AgentConfig {
                agent_id: "multi-topic".to_string(),
                description: "multi".into(),
                system_prompt: "multi".into(),
                max_history: 5,
                topics: vec!["topic-a".to_string(), "topic-b".to_string()],
            },
            llm,
            memory,
            skills,
        )
        .await
        .expect("register");

    // Both topics should route to the same agent
    let results_a = runtime
        .publish("topic-a", Message::user("from a"))
        .await;
    assert_eq!(results_a.len(), 1);

    let results_b = runtime
        .publish("topic-b", Message::user("from b"))
        .await;
    assert_eq!(results_b.len(), 1);
}

#[tokio::test]
async fn test_runtime_shutdown_all() {
    let mut runtime = ActorRuntime::new();

    for i in 0..3 {
        let llm = Arc::new(MockLLM::new(&format!("sd{}", i)));
        let memory = Arc::new(MockMemory::new());
        let skills = Arc::new(MockSkills);

        runtime
            .register(
                AgentConfig {
                    agent_id: format!("sdp-{}", i),
                    description: format!("shutdown-{}", i),
                    system_prompt: "sd".into(),
                    max_history: 5,
                    topics: vec![],
                },
                llm,
                memory,
                skills,
            )
            .await
            .expect("register");
    }

    assert_eq!(runtime.agent_count(), 3);
    runtime.shutdown_all().await;

    // After shutdown, sending to any agent should fail
    let result = runtime
        .send_to("sdp-0", Message::user("post shutdown"))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_agent_ids_returns_registered_agents() {
    let mut runtime = ActorRuntime::new();

    let llm = Arc::new(MockLLM::new("id1"));
    let memory = Arc::new(MockMemory::new());
    let skills = Arc::new(MockSkills);

    runtime
        .register(
            AgentConfig {
                agent_id: "alice".to_string(),
                description: "a".into(),
                system_prompt: "a".into(),
                max_history: 5,
                topics: vec![],
            },
            llm,
            memory,
            skills,
        )
        .await
        .expect("register alice");

    let ids = runtime.agent_ids();
    assert!(ids.contains(&"alice"));
}

#[tokio::test]
async fn test_process_multiple_messages_builds_history() {
    let llm = Arc::new(MockLLM::new("hist"));
    let memory = Arc::new(MockMemory::new());
    let skills = Arc::new(MockSkills);

    let handle = ActorAgentHandle::spawn(
        AgentConfig {
            agent_id: "hist-agent".to_string(),
            description: "history".into(),
            system_prompt: "History bot".into(),
            max_history: 10,
            topics: vec![],
        },
        llm.clone(),
        memory.clone(),
        skills,
    )
    .await
    .expect("spawn");

    // Send multiple messages — all should succeed, verifying history tracking works
    for i in 0..5 {
        let resp = handle
            .process_message(Message::user(&format!("msg {}", i)))
            .await
            .expect(&format!("msg {} should succeed", i));
        assert_eq!(resp.content, "hist-response");
    }
}
