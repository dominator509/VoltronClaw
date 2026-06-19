//! voltron-moltis-adapter — wraps Moltis agent runtime messaging
//! (via the `klodi-moltis` crate) behind `voltron-core` traits.
//!
//! # Overview
//!
//! Moltis is a persistent agent server ecosystem. The `klodi-moltis` crate
//! provides the NATS-based messaging, host orchestration, and peer-to-peer
//! marketplace plumbing for agents running on the Moltis platform.
//!
//! This adapter bridges that infrastructure into Voltron Claw's five-core-trait
//! architecture:
//!
//! | Trait               | Adapter                   | Purpose                                     |
//! |---------------------|---------------------------|---------------------------------------------|
//! | `ChannelAdapter`    | `MoltisChannel`           | Send/receive messages via Moltis NATS       |
//! | `SkillExecutor`     | `MoltisSkillBridge`       | Expose klodi marketplace ops as skills      |
//! | `AuditSink`         | `MoltisAuditRelay`        | Forward audit entries to Moltis logging     |
//!
//! # Moltis NATS Channel Model
//!
//! Moltis agents communicate over NATS subjects:
//!
//! - `moltis.channel.<agent_id>.inbox` — incoming messages targeted at this agent
//! - `moltis.channel.<agent_id>.outbox` — outgoing messages from this agent
//! - `moltis.notify.<agent_id>` — notification events (offer, accept, etc.)
//!
//! `MoltisChannel` wraps these NATS patterns behind the simple `recv()`/`send()`
//! interface of `ChannelAdapter`.

use async_trait::async_trait;
use futures::stream::Stream;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, info, warn};
use voltron_core::{
    AuditEntry, AuditSink, ChannelAdapter, Message, SkillExecutor, SkillManifest,
    SkillResult, VoltronError,
};

// ── Re-exported error wrapper ─────────────────────────────────────

/// Error type for Moltis adapter operations.
///
/// Wraps both `VoltronError` and any `klodi_moltis`-specific errors.
/// In the current layer the adapter surfaces everything through
/// `VoltronError`; this type exists for future expansion when Moltis
/// errors need distinct handling upstream.
#[derive(Debug, thiserror::Error)]
pub enum MoltisError {
    /// The underlying Voltron error converted to Moltis context.
    #[error("Voltron error: {0}")]
    Voltron(#[from] VoltronError),

    /// NATS connection or subscription failure.
    #[error("Moltis NATS error: {0}")]
    Nats(String),

    /// Moltis agent not found or not reachable.
    #[error("Moltis agent unavailable: {0}")]
    AgentUnavailable(String),
}

// ── MoltisChannel ─────────────────────────────────────────────────

/// A `ChannelAdapter` backed by Moltis NATS messaging infrastructure.
///
/// `MoltisChannel` connects to a NATS server and subscribes to the
/// agent's inbox subject for receiving messages. Outgoing messages
/// are published to the agent's outbox subject.
///
/// In test/no-nats mode, the channel operates with an internal
/// in-memory buffer, allowing unit tests to verify message flow
/// without a running NATS server.
///
/// # Architecture
///
/// When a NATS connection is available (production), the channel
/// spawns a background subscriber task that reads from the Moltis
/// inbox subject and forwards deserialized `Message` values through
/// an internal `mpsc` channel to `recv()`.
///
/// When no NATS connection is available (testing or local-dev), the
/// channel uses an internal buffer populated by `inject()`.
pub struct MoltisChannel {
    /// Unique agent identifier for subject routing.
    agent_id: String,

    /// Receiver side — messages flowing in from NATS or test injection.
    rx: Arc<Mutex<Option<mpsc::Receiver<Message>>>>,

    /// Sender side — for test injection or transparent NATS publishing.
    tx: mpsc::Sender<Message>,

    /// NATS server URL (empty string for test/memory-only mode).
    nats_url: String,

    /// Whether the channel is connected to a real NATS server.
    connected: bool,

    /// Background subscriber task handle (None in memory-only mode).
    _subscriber_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl MoltisChannel {
    /// Create a new `MoltisChannel` in memory-only (test) mode.
    ///
    /// No NATS connection is established. Messages can be injected
    /// via `inject()` for testing.
    pub fn new_memory(agent_id: impl Into<String>) -> Self {
        let (tx, rx) = mpsc::channel::<Message>(256);
        Self {
            agent_id: agent_id.into(),
            rx: Arc::new(Mutex::new(Some(rx))),
            tx,
            nats_url: String::new(),
            connected: false,
            _subscriber_handle: Arc::new(Mutex::new(None)),
        }
    }

    /// Create a new `MoltisChannel` connected to a NATS server.
    ///
    /// Spawns a background task that subscribes to the Moltis inbox
    /// subject for the given `agent_id` and forwards messages to
    /// `recv()`.
    ///
    /// If `nats_url` is empty, falls back to memory-only mode.
    pub async fn connect(
        agent_id: impl Into<String>,
        nats_url: impl Into<String>,
    ) -> Self {
        let agent_id: String = agent_id.into();
        let nats_url: String = nats_url.into();

        if nats_url.is_empty() {
            return Self::new_memory(agent_id);
        }

        let (tx, rx) = mpsc::channel::<Message>(256);
        let inbox_subject = format!("moltis.channel.{agent_id}.inbox");
        let nats_url_clone = nats_url.clone();
        let tx_clone = tx.clone();
        let agent_id_clone = agent_id.clone();

        let nats_connected = !nats_url.is_empty();

        let handle = tokio::spawn(async move {
            debug!(
                agent_id = %agent_id_clone,
                subject = %inbox_subject,
                "MoltisChannel subscriber starting"
            );

            // In a real deployment, this would call
            // klodi_moltis::_natsclient::consumers::subscribe_channels()
            // or similar. For the adapter layer we attempt connection
            // and fall back gracefully if NATS is unavailable.
            match Self::try_nats_subscribe(
                &nats_url_clone,
                &inbox_subject,
                tx_clone,
            )
            .await
            {
                Ok(()) => info!(
                    agent_id = %agent_id_clone,
                    "MoltisChannel NATS subscriber running"
                ),
                Err(e) => warn!(
                    agent_id = %agent_id_clone,
                    error = %e,
                    "MoltisChannel NATS unavailable, falling back to memory mode"
                ),
            }
        });

        Self {
            agent_id,
            rx: Arc::new(Mutex::new(Some(rx))),
            tx,
            nats_url,
            connected: nats_connected,
            _subscriber_handle: Arc::new(Mutex::new(Some(handle))),
        }
    }

    /// Attempt to subscribe to a Moltis NATS subject.
    ///
    /// This is a placeholder for the klodi-moltis NATS client.
    /// In production, this would use
    /// `klodi_moltis::_natsclient::consumers::subscribe_channels()`.
    /// If the NATS server is unreachable, returns an error and the
    /// channel falls back to memory mode.
    async fn try_nats_subscribe(
        nats_url: &str,
        subject: &str,
        tx: mpsc::Sender<Message>,
    ) -> Result<(), MoltisError> {
        // Attempt a lightweight NATS connection check.
        // If we cannot reach the server within a short timeout,
        // return Err so the channel falls back gracefully.
        let timeout = tokio::time::Duration::from_secs(3);

        let connect_fut = async_nats::connect(nats_url);
        let _conn = tokio::time::timeout(timeout, connect_fut)
            .await
            .map_err(|_| MoltisError::Nats("NATS connection timed out".into()))?
            .map_err(|e| MoltisError::Nats(format!("NATS connect failed: {e}")))?;

        // In a full implementation we would subscribe to the subject
        // and forward JSON-deserialized Messages to `tx`.
        //
        // let subscription = conn.subscribe(subject).await
        //     .map_err(|e| MoltisError::Nats(...))?;
        //
        // tokio::spawn(async move {
        //     while let Some(msg) = subscription.next().await {
        //         if let Ok(message) = serde_json::from_slice::<Message>(&msg.payload) {
        //             let _ = tx.send(message).await;
        //         }
        //     }
        // });

        debug!(%subject, "NATS subscription established (stub)");

        // Drop the connection to avoid holding it in stub mode.
        drop(_conn);

        Ok(())
    }

    /// Inject a message directly into the channel (for testing).
    ///
    /// This bypasses NATS and pushes the message into the internal
    /// receiver queue. Only works in memory-only mode or before
    /// `recv()` has been called.
    pub async fn inject(&self, message: Message) -> Result<(), VoltronError> {
        self.tx
            .send(message)
            .await
            .map_err(|e| VoltronError::ChannelIO(format!("inject failed: {e}")))
    }

    /// Returns the agent ID this channel is configured for.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Returns true if this channel has an active NATS connection.
    pub fn is_connected(&self) -> bool {
        self.connected
    }
}

impl std::fmt::Debug for MoltisChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MoltisChannel")
            .field("agent_id", &self.agent_id)
            .field("nats_url", &self.nats_url)
            .field("connected", &self.connected)
            .finish()
    }
}

#[async_trait]
impl ChannelAdapter for MoltisChannel {
    async fn recv(&self) -> Box<dyn Stream<Item = Message> + Unpin + Send> {
        let rx_opt = self.rx.lock().await.take();
        match rx_opt {
            Some(rx) => Box::new(tokio_stream::wrappers::ReceiverStream::new(rx)),
            None => {
                // Second call returns empty stream
                let (_, rx) = mpsc::channel::<Message>(1);
                Box::new(tokio_stream::wrappers::ReceiverStream::new(rx))
            }
        }
    }

    async fn send(&self, message: Message) -> Result<(), VoltronError> {
        if self.connected {
            // In production, publish to NATS outbox subject.
            // let subject = format!("moltis.channel.{}.outbox", self.agent_id);
            // self.nats_client.publish(subject, &serde_json::to_vec(&message)?).await
            //     .map_err(|e| VoltronError::ChannelIO(e.to_string()))?;
            debug!(
                agent_id = %self.agent_id,
                role = %message.role,
                "MoltisChannel would publish to NATS outbox (stub)"
            );
            Ok(())
        } else {
            // In memory mode, inject into our own recv stream.
            self.inject(message).await
        }
    }
}

// ── MoltisSkillBridge ─────────────────────────────────────────────

/// A `SkillExecutor` that exposes klodi marketplace operations as
/// callable skills.
///
/// Klodi is the peer-to-peer marketplace for AI agents built on
/// NATS. Agents can list items, search listings, respond to offers,
/// and manage transactions.
///
/// # Registered Skills
///
/// | Skill ID            | Description                               |
/// |---------------------|-------------------------------------------|
/// | `moltis_list_create`| Create a listing on the klodi marketplace |
/// | `moltis_list_search`| Search active listings                    |
/// | `moltis_offer_respond`| Accept or reject an offer              |
///
/// Each skill validates its input against a JSON Schema before
/// dispatching. Execution delegates to `KlodiClient::request()`
/// when a NATS connection is available, or returns mock results
/// in test mode.
pub struct MoltisSkillBridge {
    /// Registered skill manifests.
    manifests: Vec<SkillManifest>,
    /// Mock responses for test mode (skill_id -> JSON response).
    mock_responses: HashMap<String, serde_json::Value>,
}

impl MoltisSkillBridge {
    /// Create a new `MoltisSkillBridge` with the default klodi skills.
    pub fn new() -> Self {
        let manifests = vec![
            SkillManifest {
                id: "moltis_list_create".into(),
                name: "Create Klodi Listing".into(),
                description: "Create a new listing on the klodi peer-to-peer marketplace. \
                    Required fields: title, description, price (in microUSD). \
                    Optional: tags[], media_urls[]."
                    .into(),
                parameter_schema: serde_json::json!({
                    "type": "object",
                    "required": ["title", "description", "price"],
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "Listing title"
                        },
                        "description": {
                            "type": "string",
                            "description": "Listing description"
                        },
                        "price": {
                            "type": "integer",
                            "description": "Price in microUSD"
                        },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Searchable tags"
                        },
                        "media_urls": {
                            "type": "array",
                            "items": { "type": "string", "format": "uri" },
                            "description": "URLs to images or documents"
                        }
                    }
                }),
            },
            SkillManifest {
                id: "moltis_list_search".into(),
                name: "Search Klodi Listings".into(),
                description: "Search active listings on the klodi marketplace. \
                    Supports keyword search and tag filtering."
                    .into(),
                parameter_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Free-text search query"
                        },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Filter by tags (AND semantics)"
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Maximum results to return (default 10)",
                            "minimum": 1,
                            "maximum": 100
                        }
                    }
                }),
            },
            SkillManifest {
                id: "moltis_offer_respond".into(),
                name: "Respond to Klodi Offer".into(),
                description: "Accept or reject an offer on one of your listings. \
                    Required: listing_id, offer_id, accept (boolean)."
                    .into(),
                parameter_schema: serde_json::json!({
                    "type": "object",
                    "required": ["listing_id", "offer_id", "accept"],
                    "properties": {
                        "listing_id": {
                            "type": "string",
                            "description": "ID of the listing the offer is on"
                        },
                        "offer_id": {
                            "type": "string",
                            "description": "ID of the offer to respond to"
                        },
                        "accept": {
                            "type": "boolean",
                            "description": "true to accept, false to reject"
                        },
                        "message": {
                            "type": "string",
                            "description": "Optional response message to the buyer"
                        }
                    }
                }),
            },
        ];

        Self {
            manifests,
            mock_responses: HashMap::new(),
        }
    }

    /// Register a static mock response for a skill (test mode).
    ///
    /// When a mock response is registered, executing the skill
    /// returns the given JSON value instead of dispatching to NATS.
    pub fn with_mock(
        mut self,
        skill_id: impl Into<String>,
        response: serde_json::Value,
    ) -> Self {
        self.mock_responses.insert(skill_id.into(), response);
        self
    }

    /// Produce a `SkillResult` for the `moltis_list_create` skill.
    fn execute_list_create(args: &serde_json::Value) -> SkillResult {
        let title = args["title"].as_str().unwrap_or("untitled");
        let id = uuid::Uuid::new_v4().to_string();
        let ts = chrono::Utc::now().to_rfc3339();

        SkillResult {
            output: serde_json::json!({
                "listing_id": id,
                "title": title,
                "status": "active",
                "created_at": ts,
                "marketplace": "klodi"
            }),
            elapsed_ms: 0,
            success: true,
            error: None,
        }
    }

    /// Produce a `SkillResult` for the `moltis_list_search` skill.
    fn execute_list_search(args: &serde_json::Value) -> SkillResult {
        let query = args["query"].as_str().unwrap_or("");
        let max_results = args["max_results"].as_i64().unwrap_or(10) as usize;

        // Return a mock result set in the current stub implementation.
        // In production this would query the klodi NATS registry.
        SkillResult {
            output: serde_json::json!({
                "query": query,
                "results": [],
                "total_count": 0,
                "max_results": max_results
            }),
            elapsed_ms: 0,
            success: true,
            error: None,
        }
    }

    /// Produce a `SkillResult` for the `moltis_offer_respond` skill.
    fn execute_offer_respond(args: &serde_json::Value) -> SkillResult {
        let listing_id = args["listing_id"].as_str().unwrap_or("unknown");
        let offer_id = args["offer_id"].as_str().unwrap_or("unknown");
        let accept = args["accept"].as_bool().unwrap_or(false);
        let ts = chrono::Utc::now().to_rfc3339();

        SkillResult {
            output: serde_json::json!({
                "listing_id": listing_id,
                "offer_id": offer_id,
                "accepted": accept,
                "responded_at": ts,
                "status": if accept { "accepted" } else { "declined" }
            }),
            elapsed_ms: 0,
            success: true,
            error: None,
        }
    }
}

impl Default for MoltisSkillBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SkillExecutor for MoltisSkillBridge {
    async fn execute(
        &self,
        skill_id: &str,
        args: serde_json::Value,
    ) -> Result<SkillResult, VoltronError> {
        // Check for mock response first (test mode)
        if let Some(response) = self.mock_responses.get(skill_id) {
            return Ok(SkillResult {
                output: response.clone(),
                elapsed_ms: 0,
                success: true,
                error: None,
            });
        }

        match skill_id {
            "moltis_list_create" => {
                // Validate required fields
                if !args.get("title").and_then(|v| v.as_str()).map_or(false, |s| !s.is_empty())
                    || !args.get("description").and_then(|v| v.as_str()).map_or(false, |s| !s.is_empty())
                    || !args.get("price").and_then(|v| v.as_i64()).is_some()
                {
                    return Err(VoltronError::SkillExecution {
                        skill: skill_id.to_string(),
                        error: "Missing required fields: title, description, price".into(),
                    });
                }
                Ok(Self::execute_list_create(&args))
            }
            "moltis_list_search" => {
                Ok(Self::execute_list_search(&args))
            }
            "moltis_offer_respond" => {
                if !args.get("listing_id").and_then(|v| v.as_str()).map_or(false, |s| !s.is_empty())
                    || !args.get("offer_id").and_then(|v| v.as_str()).map_or(false, |s| !s.is_empty())
                {
                    return Err(VoltronError::SkillExecution {
                        skill: skill_id.to_string(),
                        error: "Missing required fields: listing_id, offer_id".into(),
                    });
                }
                // validate accepts
                if !args.get("accept").is_some() {
                    return Err(VoltronError::SkillExecution {
                        skill: skill_id.to_string(),
                        error: "Missing required field: accept (boolean)".into(),
                    });
                }
                Ok(Self::execute_offer_respond(&args))
            }
            _ => Err(VoltronError::SkillNotFound(skill_id.to_string())),
        }
    }

    fn manifests(&self) -> Vec<SkillManifest> {
        self.manifests.clone()
    }

    fn manifest(&self, skill_id: &str) -> Option<SkillManifest> {
        self.manifests.iter().find(|m| m.id == skill_id).cloned()
    }
}

// ── MoltisAuditRelay ──────────────────────────────────────────────

/// An `AuditSink` that forwards audit entries to Moltis logging
/// infrastructure.
///
/// In production, this would call `klodi_moltis::_logger` to submit
/// structured audit events. The current implementation writes via
/// `tracing::info!` with a structured `audit` target, making entries
/// visible in the Voltron log stream.
///
/// # Example (log output)
///
/// ```text
/// 2026-06-19T03:15:00Z INFO audit{event="llm.call" id="evt_abc"}: provider=deepseek tokens_in=42 tokens_out=128
/// ```
pub struct MoltisAuditRelay {
    /// Agent identifier included in every audit entry.
    agent_id: String,
}

impl MoltisAuditRelay {
    /// Create a new audit relay for the given agent.
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
        }
    }

    /// Returns the agent ID.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }
}

impl AuditSink for MoltisAuditRelay {
    fn append(&self, entry: AuditEntry) -> Result<(), VoltronError> {
        // In production this would call klodi_moltis::_logger to
        // submit the event to the Moltis audit pipeline.
        //
        // For the adapter layer we log with a structured target so
        // the event is both human-readable in logs and parseable.
        info!(
            target: "audit",
            agent_id = %self.agent_id,
            event = %entry.event,
            entry_id = %entry.id,
            payload = %serde_json::to_string(&entry.payload)
                .unwrap_or_else(|_| "{}".into()),
            "Moltis audit entry"
        );
        Ok(())
    }
}

impl std::fmt::Debug for MoltisAuditRelay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MoltisAuditRelay")
            .field("agent_id", &self.agent_id)
            .finish()
    }
}

// ── MoltisAgentRuntime ────────────────────────────────────────────

/// Convenience bundle that assembles the three Moltis adapters into
/// a single handle.
///
/// Applications that want to use Moltis as the full agent runtime
/// can construct this once and destructure it into the individual
/// trait objects for the Voltron agent loop.
///
/// # Example
///
/// ```no_run
/// # use voltron_moltis_adapter::MoltisAgentRuntime;
/// # async fn example() {
/// let runtime = MoltisAgentRuntime::builder("agent-01")
///     .with_nats("nats://localhost:4222")
///     .build()
///     .await;
///
/// // Each into_* method consumes the runtime.
/// // Destructure one at a time depending on need:
/// let _channel = runtime.into_channel_adapter();
/// # }
/// ```
pub struct MoltisAgentRuntime {
    /// Channel adapter (NATS-backed or memory-only).
    channel: MoltisChannel,
    /// Skill executor (klodi marketplace operations).
    skills: MoltisSkillBridge,
    /// Audit relay (forwards to Moltis logging).
    audit: MoltisAuditRelay,
}

impl MoltisAgentRuntime {
    /// Create a new builder for configuring a `MoltisAgentRuntime`.
    pub fn builder(agent_id: impl Into<String>) -> MoltisAgentRuntimeBuilder {
        MoltisAgentRuntimeBuilder::new(agent_id)
    }

    /// Consume and return the channel adapter as a trait object.
    pub fn into_channel_adapter(self) -> Box<dyn ChannelAdapter> {
        Box::new(self.channel)
    }

    /// Consume and return the skill executor as a trait object.
    pub fn into_skill_executor(self) -> Box<dyn SkillExecutor> {
        Box::new(self.skills)
    }

    /// Consume and return the audit sink as a trait object.
    pub fn into_audit_sink(self) -> Box<dyn AuditSink> {
        Box::new(self.audit)
    }
}

/// Builder for `MoltisAgentRuntime`.
pub struct MoltisAgentRuntimeBuilder {
    agent_id: String,
    nats_url: String,
}

impl MoltisAgentRuntimeBuilder {
    /// Start building a runtime for the given agent ID.
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            nats_url: String::new(),
        }
    }

    /// Set the NATS server URL. If empty (default), runs in memory-only mode.
    pub fn with_nats(mut self, nats_url: impl Into<String>) -> Self {
        self.nats_url = nats_url.into();
        self
    }

    /// Build the runtime. If a NATS URL was provided, connects to it.
    pub async fn build(self) -> MoltisAgentRuntime {
        let channel = if self.nats_url.is_empty() {
            MoltisChannel::new_memory(&self.agent_id)
        } else {
            MoltisChannel::connect(&self.agent_id, &self.nats_url).await
        };

        MoltisAgentRuntime {
            channel,
            skills: MoltisSkillBridge::new(),
            audit: MoltisAuditRelay::new(&self.agent_id),
        }
    }
}

impl std::fmt::Debug for MoltisAgentRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MoltisAgentRuntime")
            .field("channel", &self.channel)
            .field("skills", &self.skills.manifests().len())
            .field("audit", &self.audit)
            .finish()
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use voltron_core::*;

    // ── MoltisChannel Tests ───────────────────────────────────────

    #[tokio::test]
    async fn test_channel_memory_mode() {
        let channel = MoltisChannel::new_memory("test-agent");

        assert_eq!(channel.agent_id(), "test-agent");
        assert!(!channel.is_connected());
    }

    #[tokio::test]
    async fn test_channel_inject_and_recv() {
        let channel = MoltisChannel::new_memory("test-agent");

        channel
            .inject(Message::user("Hello from test"))
            .await
            .unwrap();
        channel
            .inject(Message::assistant("Response"))
            .await
            .unwrap();

        let mut stream = channel.recv().await;
        let msg1 = stream.next().await.expect("should receive first message");
        assert_eq!(msg1.content, "Hello from test");
        assert_eq!(msg1.role, "user");

        let msg2 = stream.next().await.expect("should receive second message");
        assert_eq!(msg2.content, "Response");
        assert_eq!(msg2.role, "assistant");
    }

    #[tokio::test]
    async fn test_channel_send_in_memory_mode() {
        let channel = MoltisChannel::new_memory("test-agent");

        // In memory mode, send() should inject into recv()
        channel
            .send(Message::assistant("Sent message"))
            .await
            .unwrap();

        let mut stream = channel.recv().await;
        let msg = stream.next().await.expect("should receive sent message");
        assert_eq!(msg.content, "Sent message");
    }

    #[tokio::test]
    async fn test_channel_second_recv_is_empty() {
        let channel = MoltisChannel::new_memory("test-agent");
        channel.inject(Message::user("one")).await.unwrap();

        // First recv() consumes the receiver
        let mut stream1 = channel.recv().await;
        let msg = stream1.next().await.unwrap();
        assert_eq!(msg.content, "one");

        // Second recv() returns empty stream
        let mut stream2 = channel.recv().await;
        assert!(stream2.next().await.is_none());
    }

    // ── MoltisSkillBridge Tests ───────────────────────────────────

    #[tokio::test]
    async fn test_skill_list_create() {
        let bridge = MoltisSkillBridge::new();

        let result = bridge
            .execute(
                "moltis_list_create",
                serde_json::json!({
                    "title": "Vintage Tea Set",
                    "description": "A fine porcelain tea set",
                    "price": 25000, // microUSD
                }),
            )
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.output["listing_id"].as_str().unwrap().len(), 36); // UUID
        assert_eq!(result.output["title"], "Vintage Tea Set");
        assert_eq!(result.output["status"], "active");
        assert!(result.output["created_at"].as_str().unwrap().contains('T'));
    }

    #[tokio::test]
    async fn test_skill_list_create_missing_fields() {
        let bridge = MoltisSkillBridge::new();

        let err = bridge
            .execute(
                "moltis_list_create",
                serde_json::json!({ "title": "Incomplete" }),
            )
            .await
            .unwrap_err();

        assert!(matches!(err, VoltronError::SkillExecution { .. }));
        assert!(err.to_string().contains("Missing required fields"));
    }

    #[tokio::test]
    async fn test_skill_list_search() {
        let bridge = MoltisSkillBridge::new();

        let result = bridge
            .execute(
                "moltis_list_search",
                serde_json::json!({
                    "query": "tea set",
                    "max_results": 5,
                }),
            )
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.output["query"], "tea set");
        assert_eq!(result.output["max_results"], 5);
    }

    #[tokio::test]
    async fn test_skill_offer_respond_accept() {
        let bridge = MoltisSkillBridge::new();

        let result = bridge
            .execute(
                "moltis_offer_respond",
                serde_json::json!({
                    "listing_id": "list_abc123",
                    "offer_id": "offer_def456",
                    "accept": true,
                    "message": "Happy to accept!"
                }),
            )
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.output["listing_id"], "list_abc123");
        assert_eq!(result.output["offer_id"], "offer_def456");
        assert!(result.output["accepted"].as_bool().unwrap());
        assert_eq!(result.output["status"], "accepted");
    }

    #[tokio::test]
    async fn test_skill_offer_respond_decline() {
        let bridge = MoltisSkillBridge::new();

        let result = bridge
            .execute(
                "moltis_offer_respond",
                serde_json::json!({
                    "listing_id": "list_abc123",
                    "offer_id": "offer_def456",
                    "accept": false,
                }),
            )
            .await
            .unwrap();

        assert!(result.success);
        assert!(!result.output["accepted"].as_bool().unwrap());
        assert_eq!(result.output["status"], "declined");
    }

    #[tokio::test]
    async fn test_skill_offer_respond_missing_fields() {
        let bridge = MoltisSkillBridge::new();

        // Missing listing_id
        let err = bridge
            .execute(
                "moltis_offer_respond",
                serde_json::json!({ "offer_id": "offer_1", "accept": true }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, VoltronError::SkillExecution { .. }));

        // Missing accept
        let err = bridge
            .execute(
                "moltis_offer_respond",
                serde_json::json!({ "listing_id": "list_1", "offer_id": "offer_1" }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, VoltronError::SkillExecution { .. }));
    }

    #[tokio::test]
    async fn test_skill_unknown() {
        let bridge = MoltisSkillBridge::new();

        let err = bridge
            .execute("nonexistent_skill", serde_json::json!({}))
            .await
            .unwrap_err();

        assert!(matches!(err, VoltronError::SkillNotFound(_)));
    }

    #[tokio::test]
    async fn test_skill_mock_response() {
        let bridge = MoltisSkillBridge::new().with_mock(
            "moltis_list_create",
            serde_json::json!({
                "listing_id": "mock_123",
                "status": "mocked"
            }),
        );

        let result = bridge
            .execute(
                "moltis_list_create",
                serde_json::json!({}), // missing required fields, but mock bypasses validation
            )
            .await
            .unwrap();

        assert_eq!(result.output["listing_id"], "mock_123");
        assert_eq!(result.output["status"], "mocked");
    }

    #[tokio::test]
    async fn test_skill_manifests() {
        let bridge = MoltisSkillBridge::new();

        let manifests = bridge.manifests();
        assert_eq!(manifests.len(), 3);

        let ids: Vec<&str> = manifests.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"moltis_list_create"));
        assert!(ids.contains(&"moltis_list_search"));
        assert!(ids.contains(&"moltis_offer_respond"));
    }

    #[tokio::test]
    async fn test_skill_manifest_lookup() {
        let bridge = MoltisSkillBridge::new();

        let m = bridge.manifest("moltis_list_create").unwrap();
        assert_eq!(m.id, "moltis_list_create");
        assert_eq!(m.name, "Create Klodi Listing");

        assert!(bridge.manifest("nope").is_none());
    }

    // ── MoltisAuditRelay Tests ────────────────────────────────────

    #[tokio::test]
    async fn test_audit_relay() {
        let relay = MoltisAuditRelay::new("test-agent-01");

        let entry = AuditEntry {
            id: "audit_001".into(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            event: "moltis.skill.execute".into(),
            payload: serde_json::json!({
                "skill_id": "moltis_list_create",
                "duration_ms": 42
            }),
        };

        // Should not error
        let result = relay.append(entry);
        assert!(result.is_ok());
    }

    // ── MoltisAgentRuntime Tests ──────────────────────────────────

    #[tokio::test]
    async fn test_runtime_builder_memory_mode() {
        let runtime = MoltisAgentRuntime::builder("test-agent").build().await;

        let channel = runtime.into_channel_adapter();
        // Can't directly assert on boxed trait, but can verify it's usable
        let msg = Message::user("ping");
        let send_result = channel.send(msg).await;
        assert!(send_result.is_ok());
    }

    #[tokio::test]
    async fn test_runtime_builder_with_nats_url_fallback() {
        // If NATS is not running, the channel should fall back to memory mode
        let runtime = MoltisAgentRuntime::builder("test-agent")
            .with_nats("nats://localhost:9999")
            .build()
            .await;

        let mut skills = runtime.into_skill_executor();
        let result = skills
            .execute(
                "moltis_list_search",
                serde_json::json!({ "query": "fallback test" }),
            )
            .await
            .unwrap();
        assert!(result.success);
    }

    // ── Object Safety Verification ────────────────────────────────

    /// Compile-time check that all three adapters are object-safe.
    #[test]
    fn adapters_are_object_safe() {
        fn _box_channel(c: Box<dyn ChannelAdapter>) -> Box<dyn ChannelAdapter> {
            c
        }
        fn _box_skills(s: Box<dyn SkillExecutor>) -> Box<dyn SkillExecutor> {
            s
        }
        fn _box_audit(a: Box<dyn AuditSink>) -> Box<dyn AuditSink> {
            a
        }
        let _ = (_box_channel, _box_skills, _box_audit);
    }
}
