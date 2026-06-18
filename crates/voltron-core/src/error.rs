use thiserror::Error;

/// Unified error type for all Voltron Claw operations.
///
/// Every crate returns `Result<T, VoltronError>`. Variants are designed
/// to be narrow enough for callers to match on, but broad enough to
/// avoid an explosion of one-off variants.
#[derive(Error, Debug)]
pub enum VoltronError {
    // ── Provider Errors ──────────────────────────────────────────
    /// LLM provider returned an error or timed out.
    #[error("LLM provider error: {0}")]
    Provider(String),

    /// Provider authentication failed (bad API key, expired token).
    #[error("LLM authentication failed: {0}")]
    Auth(String),

    /// Provider returned a rate-limit response.
    #[error("LLM rate limited: {0}")]
    RateLimit(String),

    // ── Memory Errors ────────────────────────────────────────────
    /// Record not found by id or search criteria.
    #[error("memory record not found: {0}")]
    MemoryNotFound(String),

    /// Storage backend error (disk full, SQLite corruption, etc.).
    #[error("memory storage error: {0}")]
    MemoryStorage(String),

    // ── Skill Errors ─────────────────────────────────────────────
    /// The requested skill is not registered.
    #[error("skill not found: {0}")]
    SkillNotFound(String),

    /// The skill executed but returned an error.
    #[error("skill execution failed: skill={skill}, error={error}")]
    SkillExecution { skill: String, error: String },

    // ── Channel Errors ───────────────────────────────────────────
    /// Channel read/write error (broken pipe, disconnected client).
    #[error("channel I/O error: {0}")]
    ChannelIO(String),

    // ── Audit Errors ─────────────────────────────────────────────
    /// Audit sink failed to persist an entry.
    #[error("audit persistence error: {0}")]
    AuditPersistence(String),

    // ── Serialization / Deserialization ──────────────────────────
    /// JSON or message serialization failure.
    #[error("serialization error: {0}")]
    Serialization(String),

    // ── Configuration Errors ─────────────────────────────────────
    /// Missing or invalid configuration.
    #[error("configuration error: {0}")]
    Config(String),

    // ── Internal / Catch-all ─────────────────────────────────────
    /// Unexpected internal error that should not happen.
    #[error("internal error: {0}")]
    Internal(String),
}

impl VoltronError {
    /// True if retrying the operation might succeed (rate limits, transient I/O).
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            VoltronError::RateLimit(_) | VoltronError::ChannelIO(_)
        )
    }

    /// True if the error is permanent and retrying won't help.
    pub fn is_permanent(&self) -> bool {
        !self.is_retryable()
    }
}

// ── Convenience conversions from common external error types ─────

impl From<serde_json::Error> for VoltronError {
    fn from(e: serde_json::Error) -> Self {
        VoltronError::Serialization(e.to_string())
    }
}
