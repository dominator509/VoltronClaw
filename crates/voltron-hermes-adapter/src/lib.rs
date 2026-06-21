//! voltron-hermes-adapter — Self-improving skill loop.
//!
//! Port of the NousResearch/hermes-agent self-improving skill loop
//! (Python → Rust).
//!
//! # Architecture
//!
//! Four core components:
//!
//! 1. **Skill Storage** — directory scanning, YAML frontmatter parsing,
//!    validation, and platform gating.
//!
//! 2. **Skill Manager** — create, edit, patch, delete, and manage skill
//!    supporting files with atomic writes and pin protection.
//!
//! 3. **Skills Tool** — progressive disclosure: compact index (name +
//!    description only) for system prompt inclusion, full body loaded
//!    on demand via `skill_view`.
//!
//! 4. **Security Guard** (optional, `skill-guard` feature) — scans
//!    agent-created skills for dangerous patterns (shell execution,
//!    network exfiltration, prompt injection).
//!
//! 5. **HermesEngine** — top-level integration wrapping SkillManager +
//!    SkillsTool for plugging into AgentRuntime.

pub mod engine;
pub mod skill_guard;
pub mod skill_manager;
pub mod skill_storage;
pub mod skills_tool;

// Re-export key types at crate root
pub use engine::{HermesConfig, HermesEngine};
pub use skill_manager::{
    DiskSkillManager, SkillActionResponse, SkillManager, SkillManagerError,
};
pub use skill_storage::{
    platform_matches, read_skill_body, read_skill_file, scan_skills_dir,
    validate_frontmatter, validate_skill_name, Prerequisites, SkillMeta, SkillsDir,
    SkillStorageError, ALLOWED_SUBDIRS, MAX_DESCRIPTION_LENGTH, MAX_NAME_LENGTH,
    MAX_SKILL_CONTENT_CHARS, MAX_SKILL_FILE_BYTES, VALID_NAME_RE,
};
pub use skills_tool::{
    DiskSkillsTool, SkillRequirements, SkillsListResult, SkillsTool, SkillViewResult,
};

#[cfg(feature = "skill-guard")]
pub use skill_guard::{
    DefaultSkillGuard, Finding, ScanResult, Severity, SkillGuard,
};

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that all expected types are re-exported correctly.
    #[test]
    fn test_reexports_skill_storage() {
        let _ = MAX_NAME_LENGTH;
        let _ = MAX_DESCRIPTION_LENGTH;
        let _ = MAX_SKILL_CONTENT_CHARS;
        let _ = MAX_SKILL_FILE_BYTES;
        let _ = VALID_NAME_RE;
        let _ = ALLOWED_SUBDIRS;
    }

    #[test]
    fn test_reexports_skill_manager() {
        let _ = std::mem::discriminant(&SkillManagerError::NotFound(String::new()));
    }

    #[test]
    fn test_reexports_skills_tool() {
        let result = SkillsListResult {
            skills: vec![],
            count: 0,
        };
        assert_eq!(result.count, 0);
    }

    #[test]
    fn test_reexports_engine() {
        let config = HermesConfig::default();
        // Default skills dir should be set
        assert!(config.skills_dir.as_os_str().len() > 0);
    }
}
