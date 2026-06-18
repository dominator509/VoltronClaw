# SPEC_ANCHOR

> Version: 0.1.0  
> Scope: Phases 0–2  
> Status: DRAFT — awaiting Phase 1 trait definitions

## Project Mission

Voltron Claw is a greenfield Rust-native composite agent that combines proven
algorithms from existing open-source claws with novel security and architecture
layers no current project ships.

## Phase 0 — Foundation

- Workspace scaffold with 7 crates
- License enforcement (cargo-deny)
- CI/CD pipeline (github-actions)
- Documentation framework

## Phase 1 — Core Traits + Minimum Viable Loop

### 1.1 VoltronError (thiserror)

Central error enum. All crate operations return `Result<T, VoltronError>`.

| Variant | Meaning | Retryable |
|---------|---------|-----------|
| `Provider(String)` | LLM provider error or timeout | No |
| `Auth(String)` | Bad API key or expired token | No |
| `RateLimit(String)` | Provider rate-limit response | Yes |
| `MemoryNotFound(String)` | Record not found by id or search | No |
| `MemoryStorage(String)` | Storage backend error | No |
| `SkillNotFound(String)` | Skill id not registered | No |
| `SkillExecution { skill, error }` | Skill ran but returned error | No |
| `ChannelIO(String)` | Channel read/write failure | Yes |
| `AuditPersistence(String)` | Audit sink write failure | No |
| `Serialization(String)` | JSON/message serialization error | No |
| `Config(String)` | Missing or invalid configuration | No |
| `Internal(String)` | Unexpected invariant violation | No |

### 1.2 Data Types

All types in `crates/voltron-core/src/types.rs` implement `Serialize + Deserialize + Debug + Clone + PartialEq`.

- **Message** — `{ role: String, content: String, name: Option<String>, tool_calls: Vec<ToolCall> }`. Constructors: `.system()`, `.user()`, `.assistant()`, `.tool()`.
- **ToolCall** — `{ id: String, function_name: String, arguments: Value }`. LLM-requested function invocation.
- **ToolDefinition** — `{ function_name: String, description: String, parameters: Value }`. Schema sent to LLM.
- **LLMResponse** — `{ content: String, tool_calls: Vec<ToolCall>, finish_reason: Option<String>, metadata: HashMap<String,String> }`. Provider generate() return type.
- **MemoryRecord** — `{ id: String, content: String, tags: Vec<String>, created_at: String, updated_at: String, metadata: HashMap<String,String> }`. Timestamps are ISO-8601.
- **SkillManifest** — `{ id: String, name: String, description: String, parameter_schema: Value }`. Registered skill metadata.
- **SkillResult** — `{ output: Value, elapsed_ms: u64, success: bool, error: Option<String> }`. Execution outcome.
- **AuditEntry** — `{ id: String, timestamp: String, event: String, payload: Value }`. Immutable audit trail record.

### 1.3 Core Traits

All five traits are `#[async_trait]` with `Send + Sync` bounds. Defined in `crates/voltron-core/src/traits.rs`.

#### LLMProvider
```rust
#[async_trait]
pub trait LLMProvider: Send + Sync {
    async fn generate(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LLMResponse, VoltronError>;

    fn provider_name(&self) -> &str;
}
```
- `messages`: Full conversation context (system, user, assistant, tool roles).
- `tools`: Available tool definitions for the model.
- `LLMResponse` may contain zero or more tool calls plus optional text content.
- Implementors handle provider-specific retry backoff internally.

#### MemoryStore
```rust
#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn put(&self, record: MemoryRecord) -> Result<(), VoltronError>;
    async fn get(&self, id: &str) -> Result<Option<MemoryRecord>, VoltronError>;
    async fn search(&self, tags: &[String]) -> Result<Vec<MemoryRecord>, VoltronError>;
    async fn delete(&self, id: &str) -> Result<(), VoltronError>;
}
```
- `put()`: Insert or replace by id.
- `search()`: AND-semantics tag match, ordered by `updated_at` descending.
- `delete()`: No-op if record does not exist.

#### SkillExecutor
```rust
#[async_trait]
pub trait SkillExecutor: Send + Sync {
    async fn execute(
        &self,
        skill_id: &str,
        args: Value,
    ) -> Result<SkillResult, VoltronError>;

    fn manifests(&self) -> Vec<SkillManifest>;
    fn manifest(&self, skill_id: &str) -> Option<SkillManifest>;
}
```
- `execute()`: Looks up skill, validates args against parameter_schema, measures elapsed_ms.
- `manifests()`: Returns all registered skill manifests.

#### ChannelAdapter
```rust
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    async fn recv(&self) -> Box<dyn Stream<Item = Message> + Unpin + Send>;
    async fn send(&self, message: Message) -> Result<(), VoltronError>;
}
```
- `recv()`: Non-blocking stream of incoming messages. Agent loop uses `StreamExt::next()`.
- `send()`: Push a message out to the channel.

#### AuditSink
```rust
pub trait AuditSink: Send + Sync {
    fn append(&self, entry: AuditEntry) -> Result<(), VoltronError>;
}
```
- Synchronous interface. Implementors must use internal buffering or lock-free queues.
- Every significant agent action (LLM call, skill exec, memory mutation) must be recorded.

### 1.4 Trait Implementations (Phase 1)

| Trait | Implementation | Crate | Assignee |
|-------|---------------|-------|----------|
| LLMProvider | DeepSeek (primary), OpenAI (fallback) via rig-core | voltron-providers | Ip Man |
| MemoryStore | InMemoryStore (HashMap), SqliteStore (sqlx) | voltron-memory | Ip Man |
| SkillExecutor | LocalSkillExecutor (echo, time_now) | voltron-skills | Ip Man |
| ChannelAdapter | CliChannel (stdin/stdout) | voltron-channels | Ip Man |
| AuditSink | InMemoryAuditSink (Vec), FileAuditSink (JSONL) | voltron-audit | Ip Man |

### 1.5 Runtime

`voltron-runtime` — Agent struct + run_loop, assembled from all five trait objects, implemented by Alfred.

### 1.6 SPEC_ANCHOR_REVIEWED

- [x] traits.rs — 5 async traits locked (commit: 3e6b405)
- [x] error.rs — 12 VoltronError variants (commit: 3e6b405)
- [x] types.rs — 8 data types with unit tests (commit: 3e6b405)

## Phase 2 — First Borrowed Component Intake

Operator selects a component from §3.4 of the handoff prompt. Full license
audit (§3 of LICENSE_STRATEGY.md), fenced placement in `/third_party/`,
adapter crate behind `voltron-core` traits.

## Out of Scope (this prompt)

- Trinity-anchored congruence layer
- Encrypted memory composite (SQLCipher + AES-256-GCM + Argon2id)
- Two-role internal split
- Tokenization layer (PII/PHI redaction)
- Append-only HMAC-chained audit log
- TEE deployment target
- Additional borrowed components beyond Phase 2

## Spec Lock

No Rust code shall be written until the implementing trait or module
is specified in this document and marked `SPEC_ANCHOR_REVIEWED.flag`
in the `/docs/` directory.
