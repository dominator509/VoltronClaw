use crate::error::VoltronError;
use crate::types::{
    AuditEntry, LLMResponse, MemoryRecord, Message, SkillManifest, SkillResult, ToolDefinition,
};
use async_trait::async_trait;
use futures::stream::Stream;

// ── LLMProvider ──────────────────────────────────────────────────

/// Abstract LLM provider capable of generating responses from a
/// conversation history and optional tool definitions.
///
/// # Implementation Notes
///
/// * `messages` is the full conversation context (system + user +
///   assistant + tool messages).
/// * `tools` is the set of available tool definitions for the model.
/// * The returned `LLMResponse` may contain zero or more tool calls
///   as well as optional conversation text content.
/// * Implementors must handle provider-specific quirks (token limits,
///   retry backoff, streaming) internally; the trait surface is
///   intentionally narrow.
#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Generate a completion from the LLM.
    async fn generate(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LLMResponse, VoltronError>;

    /// Human-readable provider identifier (e.g., "deepseek", "openai").
    fn provider_name(&self) -> &str;
}

// ── MemoryStore ──────────────────────────────────────────────────

/// Abstract persistent or in-memory store for conversation records.
///
/// Implementations must be `Send + Sync` and safe to use from
/// multiple concurrent tasks. Backends include in-memory hash maps
/// (testing / development), SQLite, and (in a future phase) an
/// encrypted SQLCipher + AES-256-GCM composite.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// Store a new record or update an existing one by id.
    async fn put(&self, record: MemoryRecord) -> Result<(), VoltronError>;

    /// Retrieve a record by its unique id.
    async fn get(&self, id: &str) -> Result<Option<MemoryRecord>, VoltronError>;

    /// Search records whose tags contain ALL of the given tags
    /// (AND semantics). Returns matching records ordered by
    /// `updated_at` descending.
    async fn search(&self, tags: &[String]) -> Result<Vec<MemoryRecord>, VoltronError>;

    /// Delete a record by id. No-op if the record does not exist.
    async fn delete(&self, id: &str) -> Result<(), VoltronError>;
}

// ── SkillExecutor ────────────────────────────────────────────────

/// Registry-backed skill execution engine.
///
/// Skills are identified by a string `skill_id` and receive
/// `serde_json::Value` arguments. The executor is responsible for
/// looking up the skill, validating arguments against its manifest
/// schema, and measuring execution time.
#[async_trait]
pub trait SkillExecutor: Send + Sync {
    /// Execute a registered skill by id.
    async fn execute(
        &self,
        skill_id: &str,
        args: serde_json::Value,
    ) -> Result<SkillResult, VoltronError>;

    /// List all registered skill manifests.
    fn manifests(&self) -> Vec<SkillManifest>;

    /// Look up a specific manifest by id.
    fn manifest(&self, skill_id: &str) -> Option<SkillManifest>;
}

// ── ChannelAdapter ───────────────────────────────────────────────

/// Abstract communication channel for the agent.
///
/// Implementations back different interaction surfaces: CLI stdin/stdout,
/// WebSocket, Telegram bot, Slack RTM, etc. The agent calls `recv()` to
/// pull incoming messages and `send()` to push responses.
///
/// `recv()` returns a `Stream` so the agent loop can use `StreamExt`
/// combinators (`.next()`, `.take_while()`, etc.) without blocking.
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Stream of incoming messages from the channel.
    async fn recv(&self) -> Box<dyn Stream<Item = Message> + Unpin + Send>;

    /// Send a message out through the channel.
    async fn send(&self, message: Message) -> Result<(), VoltronError>;
}

// ── AuditSink ────────────────────────────────────────────────────

/// Append-only audit trail.
///
/// Every significant agent action (LLM call, skill execution, memory
/// mutation, channel I/O) should be recorded as an `AuditEntry`.
///
/// `append()` is **synchronous** — callers must not block the async
/// runtime. Implementations should use an internal buffered channel or
/// lock-free queue if the underlying write may block.
///
/// A future phase will add HMAC-chained immutability.
pub trait AuditSink: Send + Sync {
    /// Persist an audit entry.
    fn append(&self, entry: AuditEntry) -> Result<(), VoltronError>;
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that the five core traits are object-safe enough to be
    /// boxed (required by the runtime for dynamic dispatch).
    #[test]
    fn traits_are_object_safe() {
        // Compile-time check: if any trait is not object-safe, these
        // lines won't compile.
        fn _box_provider(p: Box<dyn LLMProvider>) -> Box<dyn LLMProvider> {
            p
        }
        fn _box_memory(m: Box<dyn MemoryStore>) -> Box<dyn MemoryStore> {
            m
        }
        fn _box_skills(s: Box<dyn SkillExecutor>) -> Box<dyn SkillExecutor> {
            s
        }
        fn _box_channel(c: Box<dyn ChannelAdapter>) -> Box<dyn ChannelAdapter> {
            c
        }
        fn _box_audit(a: Box<dyn AuditSink>) -> Box<dyn AuditSink> {
            a
        }
    }
}
