//! voltron-skills — SkillExecutor implementation.
//!
//! `LocalSkillExecutor` dispatches registered Rust function pointers
//! by `skill_id`. Example skills: `echo` (returns input) and `time_now`
//! (ISO-8601 in America/Los_Angeles).

use async_trait::async_trait;
use std::collections::HashMap;
use voltron_core::{SkillExecutor, SkillManifest, SkillResult, VoltronError};

/// Type alias for a skill implementation.
///
/// Takes `serde_json::Value` arguments and returns a `SkillResult`.
type SkillFn = Arc<dyn Fn(serde_json::Value) -> SkillResult + Send + Sync>;

use std::sync::Arc;

/// A `SkillExecutor` that dispatches registered Rust closures by id.
///
/// Skills are registered at construction time via `register()`.
///
/// Example:
/// ```no_run
/// # use voltron_skills::LocalSkillExecutor;
/// # use voltron_core::{SkillManifest, SkillResult};
/// let mut executor = LocalSkillExecutor::new();
/// executor.register(
///     SkillManifest {
///         id: "echo".into(),
///         name: "Echo".into(),
///         description: "Returns input".into(),
///         parameter_schema: serde_json::json!({"type": "object"}),
///     },
///     |args| SkillResult {
///         output: args,
///         elapsed_ms: 0,
///         success: true,
///         error: None,
///     },
/// );
/// ```
pub struct LocalSkillExecutor {
    skills: HashMap<String, SkillEntry>,
}

struct SkillEntry {
    f: SkillFn,
    manifest: SkillManifest,
}

impl LocalSkillExecutor {
    /// Create an empty executor.
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    /// Register a skill function with its manifest.
    pub fn register<F>(&mut self, manifest: SkillManifest, f: F)
    where
        F: Fn(serde_json::Value) -> SkillResult + Send + Sync + 'static,
    {
        self.skills.insert(
            manifest.id.clone(),
            SkillEntry {
                f: Arc::new(f),
                manifest,
            },
        );
    }

    /// Create a default executor with the two built-in skills (`echo`, `time_now`).
    pub fn with_defaults() -> Self {
        let mut ex = Self::new();

        // ── echo skill ─────────────────────────────────────────────
        ex.register(
            SkillManifest {
                id: "echo".into(),
                name: "Echo".into(),
                description: "Returns the input arguments as-is.".into(),
                parameter_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "message": {
                            "type": "string",
                            "description": "The message to echo back"
                        }
                    }
                }),
            },
            |args| {
                let output = args
                    .get("message")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                SkillResult {
                    output,
                    elapsed_ms: 0,
                    success: true,
                    error: None,
                }
            },
        );

        // ── time_now skill ─────────────────────────────────────────
        ex.register(
            SkillManifest {
                id: "time_now".into(),
                name: "Current Time".into(),
                description: "Returns the current time in ISO-8601 format for America/Los_Angeles."
                    .into(),
                parameter_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            |_args| {
                let now = chrono::Utc::now();
                let la_time = now.with_timezone(&chrono_tz::America::Los_Angeles);
                let iso = la_time.to_rfc3339();
                SkillResult {
                    output: serde_json::json!({"iso_8601": iso}),
                    elapsed_ms: 0,
                    success: true,
                    error: None,
                }
            },
        );

        ex
    }
}

impl Default for LocalSkillExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SkillExecutor for LocalSkillExecutor {
    async fn execute(
        &self,
        skill_id: &str,
        args: serde_json::Value,
    ) -> Result<SkillResult, VoltronError> {
        let entry = self
            .skills
            .get(skill_id)
            .ok_or_else(|| VoltronError::SkillNotFound(skill_id.to_string()))?;

        let args_obj = match args {
            serde_json::Value::Object(map) => map,
            other => {
                // Wrap non-object args into a single "input" key
                let mut m = serde_json::Map::new();
                m.insert("input".into(), other);
                m
            }
        };

        let result = (entry.f)(serde_json::Value::Object(args_obj));
        if !result.success {
            return Err(VoltronError::SkillExecution {
                skill: skill_id.to_string(),
                error: result.error.clone().unwrap_or_else(|| "unknown".into()),
            });
        }
        Ok(result)
    }

    fn manifests(&self) -> Vec<SkillManifest> {
        self.skills.values().map(|e| e.manifest.clone()).collect()
    }

    fn manifest(&self, skill_id: &str) -> Option<SkillManifest> {
        self.skills.get(skill_id).map(|e| e.manifest.clone())
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_echo_skill() {
        let ex = LocalSkillExecutor::with_defaults();

        let args = serde_json::json!({"message": "Hello, Voltron!"});
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(ex.execute("echo", args))
            .unwrap();

        assert!(result.success);
        assert_eq!(result.output, "Hello, Voltron!");
    }

    #[test]
    fn test_echo_skill_no_message() {
        let ex = LocalSkillExecutor::with_defaults();

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(ex.execute("echo", serde_json::json!({})))
            .unwrap();

        assert!(result.success);
        assert_eq!(result.output, serde_json::Value::Null);
    }

    #[test]
    fn test_time_now_skill() {
        let ex = LocalSkillExecutor::with_defaults();

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(ex.execute("time_now", serde_json::json!({})))
            .unwrap();

        assert!(result.success);
        let output = result.output.as_object().unwrap();
        let iso = output.get("iso_8601").unwrap().as_str().unwrap();
        // Should be ISO-8601 format with LA offset (starts with date like 2026-)
        assert!(iso.starts_with("2026"), "Expected ISO-8601, got {iso}");
        assert!(iso.contains('T'), "Expected ISO-8601 time, got {iso}");
    }

    #[test]
    fn test_unknown_skill() {
        let ex = LocalSkillExecutor::with_defaults();

        let err = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(ex.execute("nonexistent", serde_json::json!({})))
            .unwrap_err();

        assert!(matches!(err, VoltronError::SkillNotFound(_)));
    }

    #[test]
    fn test_manifests() {
        let ex = LocalSkillExecutor::with_defaults();

        let manifests = ex.manifests();
        assert_eq!(manifests.len(), 2);

        let ids: Vec<&str> = manifests.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"echo"));
        assert!(ids.contains(&"time_now"));
    }

    #[test]
    fn test_manifest_lookup() {
        let ex = LocalSkillExecutor::with_defaults();

        let m = ex.manifest("echo").unwrap();
        assert_eq!(m.id, "echo");
        assert_eq!(m.name, "Echo");

        assert!(ex.manifest("nope").is_none());
    }

    #[test]
    fn test_custom_skill_registration() {
        let mut ex = LocalSkillExecutor::new();
        ex.register(
            SkillManifest {
                id: "greet".into(),
                name: "Greet".into(),
                description: "Say hello".into(),
                parameter_schema: serde_json::json!({"type": "object"}),
            },
            |args| {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("world");
                SkillResult {
                    output: serde_json::json!({"greeting": format!("Hello, {}!", name)}),
                    elapsed_ms: 0,
                    success: true,
                    error: None,
                }
            },
        );

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(ex.execute("greet", serde_json::json!({"name": "Ip Man"})))
            .unwrap();

        assert_eq!(result.output["greeting"], "Hello, Ip Man!");
    }
}
