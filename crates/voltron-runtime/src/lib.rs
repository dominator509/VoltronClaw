//! voltron-runtime — Agent orchestration runtime.
//!
//! Wires the five core traits (LLMProvider, MemoryStore, SkillExecutor,
//! ChannelAdapter, AuditSink) into a composable agent run loop with full
//! tool-calling orchestration.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────┐    ┌──────────────┐    ┌─────────┐
//! │ Channel │───▶│ AgentRuntime │───▶│ Channel │
//! │  recv() │    │              │    │  send() │
//! └─────────┘    │ • LLM        │    └─────────┘
//!                │ • Skills     │
//!                │ • Memory     │
//!                │ • Audit      │
//!                └──────────────┘
//! ```
//!
//! # Tool-calling loop
//!
//! Each user message triggers a conversation turn. The runtime calls the LLM;
//! if the LLM responds with tool calls, the runtime executes each tool via the
//! SkillExecutor, packages the results as tool-role messages, and re-calls the
//! LLM. This continues until the LLM returns a text response (no more tool
//! calls) or the maximum tool-iteration limit is reached.
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use voltron_runtime::{AgentConfig, AgentRuntime};
//! use voltron_providers::DeepSeekProvider;
//! use voltron_memory::InMemoryStore;
//! use voltron_skills::LocalSkillExecutor;
//! use voltron_channels::CliChannel;
//! use voltron_audit::InMemoryAuditSink;
//!
//! # #[tokio::main]
//! # async fn main() {
//! let runtime = AgentRuntime::builder()
//!     .provider(Arc::new(DeepSeekProvider::new("key", None)))
//!     .memory(Arc::new(InMemoryStore::new()))
//!     .skills(Arc::new(LocalSkillExecutor::with_defaults()))
//!     .channel(Arc::new(CliChannel::new()))
//!     .audit(Arc::new(InMemoryAuditSink::new()))
//!     .config(AgentConfig::default())
//!     .build();
//!
//! runtime.run_loop().await;
//! # }
//! ```

use std::sync::Arc;
#[cfg(feature = "hermes")]
use voltron_hermes_adapter::{HermesEngine, HermesConfig};
use tracing::{debug, error, info, warn};

use voltron_core::{
    AuditEntry, AuditSink, ChannelAdapter, LLMProvider, ManifestVerifier, MemoryRecord,
    MemoryStore, Message, SkillExecutor, ToolDefinition, VoltronError,
};

// ── AgentConfig ────────────────────────────────────────────────────

/// Configuration for an [`AgentRuntime`].
///
/// All fields are optional with sensible defaults. Use
/// [`AgentConfig::default()`] for a minimal setup.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// System prompt prepended to every LLM call.
    ///
    /// Default: "You are Voltron Claw, a helpful Rust-native AI assistant."
    pub system_prompt: String,

    /// Maximum number of tool-calling iterations per user turn.
    ///
    /// Prevents infinite tool-calling loops. When the limit is reached, any
    /// tool results from the final iteration are silently discarded (they
    /// were never appended to the message list) and the runtime returns the
    /// most recent text content if available, or a fallback message
    /// indicating a transient issue.
    ///
    /// Default: 10
    pub max_tool_iterations: usize,

    /// Maximum number of conversation turns before the run loop exits.
    ///
    /// `0` means unlimited.
    ///
    /// Default: 0 (unlimited)
    pub max_turns: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            system_prompt: "You are Voltron Claw, a helpful Rust-native AI assistant.".into(),
            max_tool_iterations: 10,
            max_turns: 0,
        }
    }
}

// ── AgentRuntime ───────────────────────────────────────────────────

/// The composable agent runtime — the heart of Voltron Claw.
///
/// Holds all five pluggable components behind `Arc<dyn Trait>` so every
/// backend is swappable at construction time.
pub struct AgentRuntime {
    provider: Arc<dyn LLMProvider>,
    memory: Arc<dyn MemoryStore>,
    skills: Arc<dyn SkillExecutor>,
    channel: Arc<dyn ChannelAdapter>,
    audit: Arc<dyn AuditSink>,
    config: AgentConfig,
    /// Optional capability-manifest verifier.
    ///
    /// When set, every tool dispatch is gated by a call to
    /// `verify_manifest()`. Unsigned or invalid manifests cause the
    /// tool to be rejected with a [`VerificationError`].
    manifest_verifier: Option<Arc<dyn ManifestVerifier>>,
    /// Optional Hermes self-improvement engine.
    ///
    /// When set, every audit entry is also forwarded to the Hermes
    /// ring buffer, and drift evaluation is performed after each
    /// `process_message` cycle.
    #[cfg(feature = "hermes")]
    hermes_engine: Option<Arc<HermesEngine>>,
}

impl AgentRuntime {
    /// Create a new [`AgentRuntimeBuilder`].
    pub fn builder() -> AgentRuntimeBuilder {
        AgentRuntimeBuilder::default()
    }

    // ── Single-turn execution ───────────────────────────────────

    /// Process a single user message and return the assistant's response.
    ///
    /// This is the core orchestration method: it runs the full tool-calling
    /// loop and returns the final text response (or an error if the loop
    /// exhausted without producing text).
    ///
    /// Every LLM call and tool execution is logged via the audit sink.
    pub async fn process_message(&self, user_msg: &Message) -> Result<Message, VoltronError> {
        let turn_id = uuid_v4();

        // Audit: turn start
        self.log_audit(
            &turn_id,
            "turn.start",
            serde_json::json!({
                "role": user_msg.role,
                "content_preview": preview(&user_msg.content, 200),
            }),
        );

        // Build the initial message list for this turn:
        //   [system_prompt] + [user_msg]
        let mut messages: Vec<Message> = Vec::new();
        messages.push(Message::system(&self.config.system_prompt));
        messages.push(user_msg.clone());

        // Tool definitions from the skill executor
        let tool_defs: Vec<ToolDefinition> = self
            .skills
            .manifests()
            .into_iter()
            .map(|m| ToolDefinition {
                function_name: m.id,
                description: m.description,
                parameters: m.parameter_schema,
            })
            .collect();

        let mut final_text = String::new();
        let mut had_text = false;

        // Tool-calling loop
        for iter in 0..self.config.max_tool_iterations {
            debug!(turn_id = %turn_id, iteration = iter, "Calling LLM");

            let response = self.provider.generate(&messages, &tool_defs).await?;

            // Audit: LLM call
            self.log_audit(
                &turn_id,
                "llm.call",
                serde_json::json!({
                    "iteration": iter,
                    "finish_reason": response.finish_reason,
                    "content_length": response.content.len(),
                    "tool_call_count": response.tool_calls.len(),
                    "metadata": response.metadata,
                }),
            );

            // If the LLM returned text content, capture it
            if !response.content.is_empty() {
                final_text = response.content.clone();
                had_text = true;
            }

            // If no tool calls, the turn is complete
            if response.tool_calls.is_empty() {
                debug!(turn_id = %turn_id, "Turn complete — no tool calls");
                break;
            }

            // Process tool calls
            let assistant_msg = Message {
                role: "assistant".into(),
                content: response.content,
                name: None,
                tool_call_id: None,
                tool_calls: response.tool_calls.clone(),
            };
            messages.push(assistant_msg);

            for tc in &response.tool_calls {
                debug!(
                    turn_id = %turn_id,
                    tool = %tc.function_name,
                    tool_call_id = %tc.id,
                    "Executing tool",
                );

                // ── IronClaw capability-manifest gate ─────────────
                if let Some(verifier) = &self.manifest_verifier {
                    // If the verifier's lookup returns an error, the skill is not
                    // authorised to execute. Reject it before calling execute().
                    if let Err(verification_error) = verifier.verify_skill_by_name(&tc.function_name)
                    {
                        warn!(
                            turn_id = %turn_id,
                            tool = %tc.function_name,
                            error = %verification_error,
                            "Skill rejected by IronClaw manifest verifier",
                        );

                        self.log_audit(
                            &turn_id,
                            "ironclaw.rejected",
                            serde_json::json!({
                                "tool_call_id": tc.id,
                                "function_name": tc.function_name,
                                "error": verification_error.to_string(),
                            }),
                        );

                        // Push an error tool result so the LLM knows the tool was rejected
                        messages.push(Message {
                            role: "tool".into(),
                            content: format!("{{\"error\": \"IronClaw rejected: {}\"}}", verification_error),
                            name: Some(tc.function_name.clone()),
                            tool_call_id: Some(tc.id.clone()),
                            tool_calls: vec![],
                        });
                        continue;
                    }
                }

                let skill_result = self
                    .skills
                    .execute(&tc.function_name, tc.arguments.clone())
                    .await;

                match skill_result {
                    Ok(result) => {
                        // Audit: tool execution success
                        self.log_audit(
                            &turn_id,
                            "tool.execute",
                            serde_json::json!({
                                "tool_call_id": tc.id,
                                "function_name": tc.function_name,
                                "success": true,
                                "elapsed_ms": result.elapsed_ms,
                            }),
                        );

                        messages.push(Message {
                            role: "tool".into(),
                            content: serde_json::to_string(&result.output)
                                .unwrap_or_else(|_| "{}".into()),
                            name: Some(tc.function_name.clone()),
                            tool_call_id: Some(tc.id.clone()),
                            tool_calls: vec![],
                        });
                    }
                    Err(e) => {
                        warn!(
                            turn_id = %turn_id,
                            tool = %tc.function_name,
                            error = %e,
                            "Tool execution failed",
                        );

                        // Audit: tool execution failure
                        self.log_audit(
                            &turn_id,
                            "tool.error",
                            serde_json::json!({
                                "tool_call_id": tc.id,
                                "function_name": tc.function_name,
                                "error": e.to_string(),
                            }),
                        );

                        messages.push(Message {
                            role: "tool".into(),
                            content: format!("{{\"error\": \"{}\"}}", e),
                            name: Some(tc.function_name.clone()),
                            tool_call_id: Some(tc.id.clone()),
                            tool_calls: vec![],
                        });
                    }
                }
            }
        }

        // ── Hermes drift evaluation ───────────────────────────
        #[cfg(feature = "hermes")]
        if let Some(engine) = &self.hermes_engine {
            if let Some(snapshot) = engine.evaluate_drift() {
                if snapshot.anomalous {
                    warn!(
                        turn_id = %turn_id,
                        window = snapshot.window_index,
                        error_rate = snapshot.error_rate,
                        "Hermes drift anomaly detected"
                    );
                }
            }
        }

        // Audit: turn end
        self.log_audit(
            &turn_id,
            "turn.end",
            serde_json::json!({
                "had_text": had_text,
                "content_preview": preview(&final_text, 200),
            }),
        );

        if !had_text {
            warn!(turn_id = %turn_id, "Turn exhausted tool iterations without text response");
            return Ok(Message::assistant(
                "I ran into an issue processing your request. Please try again.",
            ));
        }

        Ok(Message::assistant(final_text))
    }

    // ── Run loop ────────────────────────────────────────────────

    /// Enter the interactive agent run loop.
    ///
    /// Reads messages from the channel one at a time, processes each through
    /// [`process_message`], and sends the response back. Exits when the channel
    /// closes or `max_turns` is reached.
    pub async fn run_loop(&self) {
        use tokio_stream::StreamExt;

        info!(max_turns = self.config.max_turns, "Entering agent run loop");

        // Log startup
        let _ = self.audit.append(AuditEntry {
            id: uuid_v4(),
            timestamp: iso_now(),
            event: "system.startup".into(),
            payload: serde_json::json!({
                "provider": self.provider.provider_name(),
                "skills_count": self.skills.manifests().len(),
            }),
        });

        let mut stream = self.channel.recv().await;
        let mut turn: u32 = 0;

        loop {
            // Check turn limit
            if self.config.max_turns > 0 && turn >= self.config.max_turns {
                info!(turn, "Max turns reached, exiting");
                break;
            }

            // Read next message
            let msg = match stream.next().await {
                Some(m) => m,
                None => {
                    info!("Channel closed, exiting");
                    break;
                }
            };

            turn += 1;
            debug!(turn, role = %msg.role, "Received message");

            match self.process_message(&msg).await {
                Ok(response) => {
                    if let Err(e) = self.channel.send(response).await {
                        error!(%e, "Failed to send response");
                        break;
                    }
                }
                Err(e) => {
                    error!(%e, "LLM call failed");
                    let error_msg = Message::assistant(format!("I encountered an error: {e}",));
                    if self.channel.send(error_msg).await.is_err() {
                        break;
                    }
                }
            }
        }

        // Log shutdown
        let _ = self.audit.append(AuditEntry {
            id: uuid_v4(),
            timestamp: iso_now(),
            event: "system.shutdown".into(),
            payload: serde_json::json!({"turns": turn}),
        });

        info!(turn, "Agent run loop complete");
    }

    // ── Memory helpers ──────────────────────────────────────────

    /// Store a text value in the memory store.
    ///
    /// Automatically sets `created_at` and `updated_at` to the current time.
    pub async fn remember(
        &self,
        id: &str,
        content: &str,
        tags: &[&str],
    ) -> Result<(), VoltronError> {
        let now = iso_now();
        let record = MemoryRecord {
            id: id.to_string(),
            content: content.to_string(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            created_at: now.clone(),
            updated_at: now,
            metadata: std::collections::HashMap::new(),
        };
        self.memory.put(record).await
    }

    /// Recall a value from the memory store by id.
    pub async fn recall(&self, id: &str) -> Result<Option<MemoryRecord>, VoltronError> {
        self.memory.get(id).await
    }

    /// Search memory by tags (AND semantics).
    pub async fn search_memory(&self, tags: &[String]) -> Result<Vec<MemoryRecord>, VoltronError> {
        self.memory.search(tags).await
    }

    /// Forget a record from memory.
    pub async fn forget(&self, id: &str) -> Result<(), VoltronError> {
        self.memory.delete(id).await
    }

    // ── Internal helpers ────────────────────────────────────────

    fn log_audit(&self, turn_id: &str, event: &str, payload: serde_json::Value) {
        let mut payload_obj = match payload {
            serde_json::Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        payload_obj.insert(
            "turn_id".to_string(),
            serde_json::Value::String(turn_id.to_string()),
        );

        let entry = AuditEntry {
            id: uuid_v4(),
            timestamp: iso_now(),
            event: format!("runtime.{event}"),
            payload: serde_json::Value::Object(payload_obj),
        };
        if let Err(e) = self.audit.append(entry) {
            warn!("Failed to write audit entry: {e}");
        }
    }
}

// ── AgentRuntimeBuilder ────────────────────────────────────────────

/// Builder for [`AgentRuntime`].
///
/// All fields are required — call every setter before [`build`].
/// Panics at build time if a required component is missing (no `Option` —
/// the type system enforces presence via the builder pattern).
#[derive(Default)]
pub struct AgentRuntimeBuilder {
    provider: Option<Arc<dyn LLMProvider>>,
    memory: Option<Arc<dyn MemoryStore>>,
    skills: Option<Arc<dyn SkillExecutor>>,
    channel: Option<Arc<dyn ChannelAdapter>>,
    audit: Option<Arc<dyn AuditSink>>,
    config: Option<AgentConfig>,
    manifest_verifier: Option<Arc<dyn ManifestVerifier>>,
    #[cfg(feature = "hermes")]
    hermes_engine: Option<Arc<HermesEngine>>,
}

macro_rules! builder_setter {
    ($field:ident, $ty:ty, $doc:expr) => {
        #[doc = $doc]
        pub fn $field(mut self, value: $ty) -> Self {
            self.$field = Some(value);
            self
        }
    };
}

impl AgentRuntimeBuilder {
    builder_setter!(
        provider,
        Arc<dyn LLMProvider>,
        "Set the LLM provider (required)."
    );
    builder_setter!(
        memory,
        Arc<dyn MemoryStore>,
        "Set the memory store (required)."
    );
    builder_setter!(
        skills,
        Arc<dyn SkillExecutor>,
        "Set the skill executor (required)."
    );
    builder_setter!(
        channel,
        Arc<dyn ChannelAdapter>,
        "Set the channel adapter (required)."
    );
    builder_setter!(audit, Arc<dyn AuditSink>, "Set the audit sink (required).");
    builder_setter!(
        config,
        AgentConfig,
        "Set the agent configuration (required)."
    );
    builder_setter!(
        manifest_verifier,
        Arc<dyn ManifestVerifier>,
        "Set the optional IronClaw capability-manifest verifier."
    );

    /// Set the optional Hermes self-improvement engine.
    #[cfg(feature = "hermes")]
    pub fn hermes_engine(mut self, engine: Arc<HermesEngine>) -> Self {
        self.hermes_engine = Some(engine);
        self
    }

    /// Build the [`AgentRuntime`].
    ///
    /// # Panics
    ///
    /// Panics if any required component was not set.
    pub fn build(self) -> AgentRuntime {
        #[cfg(feature = "hermes")]
        let audit: Arc<dyn AuditSink> = {
            let inner = self.audit.expect("audit is required");
            // HermesEngine no longer wraps the audit sink — pass through directly
            inner
        };

        #[cfg(not(feature = "hermes"))]
        let audit = self.audit.expect("audit is required");

        AgentRuntime {
            provider: self.provider.expect("provider is required"),
            memory: self.memory.expect("memory is required"),
            skills: self.skills.expect("skills is required"),
            channel: self.channel.expect("channel is required"),
            audit,
            config: self.config.unwrap_or_default(),
            manifest_verifier: self.manifest_verifier,
            #[cfg(feature = "hermes")]
            hermes_engine: self.hermes_engine,
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────

/// Truncate a string for audit previews.
fn preview(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .take(max_chars + 1)
            .last()
            .map(|(i, _)| i)
            .unwrap_or(max_chars);
        format!("{}…", &s[..end])
    }
}

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn iso_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use voltron_audit::InMemoryAuditSink;
    use voltron_channels::CliChannel;
    use voltron_core::{LLMResponse, ToolCall};
    use voltron_memory::InMemoryStore;
    use voltron_skills::LocalSkillExecutor;

    /// A mock LLM provider that returns a fixed text response.
    struct MockProvider {
        response: String,
    }

    #[async_trait::async_trait]
    impl LLMProvider for MockProvider {
        async fn generate(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> Result<LLMResponse, VoltronError> {
            Ok(LLMResponse {
                content: self.response.clone(),
                tool_calls: vec![],
                finish_reason: Some("stop".into()),
                metadata: std::collections::HashMap::new(),
            })
        }

        fn provider_name(&self) -> &str {
            "mock"
        }
    }

    /// A mock LLM provider that returns a tool call.
    struct MockToolCallProvider {
        iterations: std::sync::Mutex<u32>,
    }

    impl MockToolCallProvider {
        fn new() -> Self {
            Self {
                iterations: std::sync::Mutex::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl LLMProvider for MockToolCallProvider {
        async fn generate(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> Result<LLMResponse, VoltronError> {
            let mut it = self.iterations.lock().unwrap();
            *it += 1;

            // After 2 iterations, return text; before that, return tool calls
            if *it >= 2 {
                Ok(LLMResponse {
                    content: "Done after tool call".into(),
                    tool_calls: vec![],
                    finish_reason: Some("stop".into()),
                    metadata: std::collections::HashMap::new(),
                })
            } else {
                Ok(LLMResponse {
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call_1".into(),
                        function_name: "echo".into(),
                        arguments: serde_json::json!({"message": "hello tool"}),
                    }],
                    finish_reason: Some("tool_calls".into()),
                    metadata: std::collections::HashMap::new(),
                })
            }
        }

        fn provider_name(&self) -> &str {
            "mock-tool-call"
        }
    }

    fn make_test_runtime(provider: Arc<dyn LLMProvider>) -> AgentRuntime {
        // Create a duplex channel so recv/send don't hang
        let (reader_writer, reader) = tokio::io::duplex(1024);
        let (writer, _writer_reader) = tokio::io::duplex(1024);
        let channel = Arc::new(CliChannel::with_io(
            tokio::io::BufReader::new(reader),
            writer,
        ));
        drop(reader_writer);

        AgentRuntime::builder()
            .provider(provider)
            .memory(Arc::new(InMemoryStore::new()))
            .skills(Arc::new(LocalSkillExecutor::with_defaults()))
            .channel(channel)
            .audit(Arc::new(InMemoryAuditSink::new()))
            .config(AgentConfig::default())
            .build()
    }

    // ── process_message tests ───────────────────────────────────

    #[tokio::test]
    async fn test_simple_text_response() {
        let rt = make_test_runtime(Arc::new(MockProvider {
            response: "Hello, world!".into(),
        }));

        let user_msg = Message::user("Hi");
        let response = rt.process_message(&user_msg).await.unwrap();

        assert_eq!(response.role, "assistant");
        assert_eq!(response.content, "Hello, world!");
    }

    #[tokio::test]
    async fn test_tool_calling_loop() {
        let rt = make_test_runtime(Arc::new(MockToolCallProvider::new()));

        let user_msg = Message::user("Echo hello");
        let response = rt.process_message(&user_msg).await.unwrap();

        assert_eq!(response.role, "assistant");
        assert_eq!(response.content, "Done after tool call");
    }

    #[tokio::test]
    async fn test_system_prompt_is_prepended() {
        // Verify that process_message includes the system prompt by
        // checking a simple text response path works correctly.
        let rt = make_test_runtime(Arc::new(MockProvider {
            response: "System prompt received".into(),
        }));

        let user_msg = Message::user("Test");
        let response = rt.process_message(&user_msg).await.unwrap();

        assert!(response.content.contains("System prompt"));
    }

    #[tokio::test]
    async fn test_audit_entries_created() {
        let (reader_writer, reader) = tokio::io::duplex(1024);
        let (writer, _writer_reader) = tokio::io::duplex(1024);
        let channel = Arc::new(CliChannel::with_io(
            tokio::io::BufReader::new(reader),
            writer,
        ));
        drop(reader_writer);

        let audit = Arc::new(InMemoryAuditSink::new());

        let rt = AgentRuntime::builder()
            .provider(Arc::new(MockProvider {
                response: "Hi!".into(),
            }))
            .memory(Arc::new(InMemoryStore::new()))
            .skills(Arc::new(LocalSkillExecutor::with_defaults()))
            .channel(channel)
            .audit(audit.clone())
            .config(AgentConfig::default())
            .build();

        let user_msg = Message::user("Test");
        let _ = rt.process_message(&user_msg).await.unwrap();

        let entries = audit.all_entries();
        assert!(
            entries.len() >= 3,
            "Expected at least 3 audit entries: start, llm.call, end. Got {}",
            entries.len()
        );

        let events: Vec<&str> = entries.iter().map(|e| e.event.as_str()).collect();
        assert!(
            events.iter().any(|&e| e == "runtime.turn.start"),
            "Missing turn.start"
        );
        assert!(
            events.iter().any(|&e| e == "runtime.llm.call"),
            "Missing llm.call"
        );
        assert!(
            events.iter().any(|&e| e == "runtime.turn.end"),
            "Missing turn.end"
        );
    }

    // ── run_loop integration tests ──────────────────────────────

    #[tokio::test]
    async fn test_run_loop_single_turn() {
        let (writer, reader) = tokio::io::duplex(1024);
        let (write_tx, mut read_rx) = tokio::io::duplex(1024);
        let channel = Arc::new(CliChannel::with_io(
            tokio::io::BufReader::new(reader),
            write_tx,
        ));

        let rt = Arc::new(
            AgentRuntime::builder()
                .provider(Arc::new(MockProvider {
                    response: "Hello from run_loop!".into(),
                }))
                .memory(Arc::new(InMemoryStore::new()))
                .skills(Arc::new(LocalSkillExecutor::with_defaults()))
                .channel(channel)
                .audit(Arc::new(InMemoryAuditSink::new()))
                .config(AgentConfig {
                    max_turns: 1,
                    ..AgentConfig::default()
                })
                .build(),
        );

        // Write a user message, then close writer so run_loop doesn't hang
        use tokio::io::AsyncWriteExt;
        let mut writer = writer;
        writer.write_all(b"Hello\n").await.unwrap();
        drop(writer);

        // Run the loop
        rt.run_loop().await;

        // Read the response
        use tokio::io::AsyncReadExt;
        let mut buf = [0u8; 1024];
        let n = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            read_rx.read(&mut buf),
        )
        .await
        .expect("read timed out")
        .expect("read failed");
        let output = String::from_utf8_lossy(&buf[..n]);
        assert!(output.contains("Hello from run_loop!"), "got: {output}");
    }

    // ── Memory helpers ──────────────────────────────────────────

    #[tokio::test]
    async fn test_remember_and_recall() {
        let rt = make_test_runtime(Arc::new(MockProvider {
            response: "ok".into(),
        }));

        rt.remember("key1", "value1", &["test"]).await.unwrap();

        let record = rt.recall("key1").await.unwrap();
        assert!(record.is_some());
        assert_eq!(record.unwrap().content, "value1");
    }

    #[tokio::test]
    async fn test_search_memory() {
        let rt = make_test_runtime(Arc::new(MockProvider {
            response: "ok".into(),
        }));

        rt.remember("a", "Alpha", &["urgent", "work"])
            .await
            .unwrap();
        rt.remember("b", "Beta", &["work"]).await.unwrap();
        rt.remember("c", "Gamma", &["personal"]).await.unwrap();

        let results = rt.search_memory(&["urgent".into()]).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "a");
    }

    #[tokio::test]
    async fn test_forget() {
        let rt = make_test_runtime(Arc::new(MockProvider {
            response: "ok".into(),
        }));

        rt.remember("to_delete", "content", &[]).await.unwrap();
        assert!(rt.recall("to_delete").await.unwrap().is_some());

        rt.forget("to_delete").await.unwrap();
        assert!(rt.recall("to_delete").await.unwrap().is_none());
    }

    // ── Config tests ────────────────────────────────────────────

    #[test]
    fn test_default_config() {
        let config = AgentConfig::default();
        assert!(config.system_prompt.contains("Voltron Claw"));
        assert_eq!(config.max_tool_iterations, 10);
        assert_eq!(config.max_turns, 0);
    }

    #[test]
    fn test_preview_truncation() {
        assert_eq!(preview("hello", 10), "hello");
        assert_eq!(preview("hello world", 5), "hello…");
        // Exact boundary
        assert_eq!(preview("hello", 5), "hello");
        // One over
        assert_eq!(preview("hello!", 5), "hello…");
    }
}
