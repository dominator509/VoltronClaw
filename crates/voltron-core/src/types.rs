use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── LLM Provider Types ──────────────────────────────────────────

/// A single message in an LLM conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    /// "system", "user", "assistant", or "tool"
    pub role: String,
    /// Message text content (may be empty when tool_calls are present)
    pub content: String,
    /// Optional display name for the sender
    pub name: Option<String>,
    /// Tool calls embedded in an assistant message
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
            name: None,
            tool_calls: vec![],
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
            name: None,
            tool_calls: vec![],
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            name: None,
            tool_calls: vec![],
        }
    }

    pub fn tool(_tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            name: None,
            tool_calls: vec![],
        }
        // Note: tool_call_id is tracked via ToolCall, not on Message itself
        // To associate a tool response, include the id in content or use
        // provider-specific metadata. This is intentionally minimal.
    }
}

/// An LLM-requested tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    /// Provider-assigned call identifier
    pub id: String,
    /// Function name to dispatch
    pub function_name: String,
    /// JSON arguments for the function
    pub arguments: serde_json::Value,
}

/// A tool definition sent to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolDefinition {
    pub function_name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Response from an LLM generate call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LLMResponse {
    /// The assistant's text content
    pub content: String,
    /// Tool calls requested by the model (if any)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Provider finish reason (e.g., "stop", "tool_calls", "length")
    pub finish_reason: Option<String>,
    /// Provider-specific metadata (model name, token counts, etc.)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

// ── Memory Types ─────────────────────────────────────────────────

/// A record stored in a MemoryStore backend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRecord {
    /// Unique identifier
    pub id: String,
    /// Raw content
    pub content: String,
    /// Searchable tags
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// ISO-8601 creation timestamp
    pub created_at: String,
    /// ISO-8601 last-updated timestamp
    pub updated_at: String,
    /// Arbitrary metadata
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

// ── Skill Types ──────────────────────────────────────────────────

/// Metadata describing a registered skill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillManifest {
    /// Unique skill identifier (e.g., "echo", "time_now")
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// What the skill does
    pub description: String,
    /// JSON Schema for input parameters
    pub parameter_schema: serde_json::Value,
}

/// Result of executing a skill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillResult {
    /// The skill's output (may be any JSON value)
    pub output: serde_json::Value,
    /// Wall-clock execution time in milliseconds
    pub elapsed_ms: u64,
    /// Whether execution succeeded
    pub success: bool,
    /// Optional error message if success is false
    pub error: Option<String>,
}

// ── Audit Types ──────────────────────────────────────────────────

/// An immutable audit trail entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEntry {
    /// Unique identifier
    pub id: String,
    /// ISO-8601 timestamp of the event
    pub timestamp: String,
    /// Event category (e.g., "llm.call", "skill.execute", "memory.put")
    pub event: String,
    /// Serialized event payload
    pub payload: serde_json::Value,
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_constructors() {
        let sys = Message::system("You are helpful.");
        assert_eq!(sys.role, "system");
        assert_eq!(sys.content, "You are helpful.");

        let user = Message::user("Hello");
        assert_eq!(user.role, "user");

        let asst = Message::assistant("Hi there");
        assert_eq!(asst.role, "assistant");
    }

    #[test]
    fn llm_response_serialization() {
        let resp = LLMResponse {
            content: "OK".into(),
            tool_calls: vec![],
            finish_reason: Some("stop".into()),
            metadata: HashMap::new(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("stop"));
    }

    #[test]
    fn skill_result_success_and_failure() {
        let ok = SkillResult {
            output: serde_json::json!("done"),
            elapsed_ms: 42,
            success: true,
            error: None,
        };
        assert!(ok.success);

        let fail = SkillResult {
            output: serde_json::Value::Null,
            elapsed_ms: 1,
            success: false,
            error: Some("timeout".into()),
        };
        assert!(!fail.success);
    }

    #[test]
    fn audit_entry_roundtrip() {
        let entry = AuditEntry {
            id: "a1".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            event: "llm.call".into(),
            payload: serde_json::json!({"model": "deepseek-v4"}),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: AuditEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.event, "llm.call");
    }

    #[test]
    fn memory_record_tags() {
        let rec = MemoryRecord {
            id: "m1".into(),
            content: "important note".into(),
            tags: vec!["urgent".into(), "todo".into()],
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            metadata: HashMap::new(),
        };
        assert!(rec.tags.contains(&"urgent".to_string()));
    }

    #[test]
    fn tool_definition_schema() {
        let td = ToolDefinition {
            function_name: "search".into(),
            description: "Search docs".into(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        };
        assert_eq!(td.function_name, "search");
    }
}
