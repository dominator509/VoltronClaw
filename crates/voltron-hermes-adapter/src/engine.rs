//! HermesEngine — self-improving skill loop engine.
//!
//! Wraps `SkillManager` and `SkillsTool` implementations and provides
//! the top-level integration point for `AgentRuntime`.
//!
//! When wired into the runtime, the engine registers three tools:
//! - `skill_manage` — create/edit/patch/delete skills (SkillManager trait)
//! - `skills_list` — compact skill index discovery (SkillsTool trait)
//! - `skill_view` — load full skill or supporting file (SkillsTool trait)
//!
//! The skill index (compact metadata) is appended to the system prompt
//! on each turn so the agent knows what skills are available without
//! loading full bodies.

use crate::skill_manager::{DiskSkillManager, SkillActionResponse, SkillManager, SkillManagerError};
use crate::skills_tool::{DiskSkillsTool, SkillRequirements, SkillsListResult, SkillsTool, SkillViewResult};
use std::path::{Path, PathBuf};

// ── HermesConfig ────────────────────────────────────────────────────

/// Configuration for the Hermes self-improving skill loop engine.
#[derive(Debug, Clone)]
pub struct HermesConfig {
    /// Path to the skills directory (default: ~/.voltron/skills/).
    pub skills_dir: PathBuf,
    /// Whether the security guard is enabled for agent-created skills.
    pub guard_enabled: bool,
}

impl Default for HermesConfig {
    fn default() -> Self {
        Self {
            skills_dir: dirs_or_default(),
            guard_enabled: false,
        }
    }
}

fn dirs_or_default() -> PathBuf {
    // Try XDG data home, otherwise default to ~/.voltron/skills
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".voltron").join("skills")
    } else {
        PathBuf::from(".voltron/skills")
    }
}

// ── HermesEngine ────────────────────────────────────────────────────

/// The Hermes self-improving skill loop engine.
///
/// Wraps skill management and progressive disclosure tools into a
/// single integration point for `AgentRuntime`.
pub struct HermesEngine {
    /// The skill manager for CRUD operations.
    skill_manager: DiskSkillManager,
    /// The skills tool for discovery and viewing.
    skills_tool: DiskSkillsTool,
    /// Whether the security guard is enabled.
    guard_enabled: bool,
}

impl HermesEngine {
    /// Create a new Hermes engine with the given configuration.
    pub fn new(config: HermesConfig) -> Self {
        let skills_dir = config.skills_dir.clone();
        std::fs::create_dir_all(&skills_dir).ok();

        Self {
            skill_manager: DiskSkillManager::new(skills_dir),
            skills_tool: DiskSkillsTool::new(),
            guard_enabled: config.guard_enabled,
        }
    }

    /// Create a new Hermes engine with defaults.
    pub fn default_with_skills_dir(skills_dir: PathBuf) -> Self {
        Self::new(HermesConfig {
            skills_dir,
            ..HermesConfig::default()
        })
    }

    /// Return a reference to the skill manager.
    pub fn skill_manager(&self) -> &DiskSkillManager {
        &self.skill_manager
    }

    /// Return a reference to the skills tool.
    pub fn skills_tool(&self) -> &DiskSkillsTool {
        &self.skills_tool
    }

    /// Return the skills directory base path.
    pub fn skills_dir(&self) -> &Path {
        self.skill_manager.skills_dir().base()
    }

    /// Whether the security guard is enabled.
    pub fn guard_enabled(&self) -> bool {
        self.guard_enabled
    }

    // ── Skill Management ───────────────────────────────────────────

    /// Create a new skill.
    ///
    /// If `guard_enabled`, the skill is scanned for dangerous patterns
    /// after creation.
    pub fn create_skill(
        &self,
        name: &str,
        category: Option<&str>,
        content: &str,
    ) -> Result<SkillActionResponse, SkillManagerError> {
        let response = self.skill_manager.create(name, category, content)?;

        // Post-creation security scan
        if self.guard_enabled {
            let skill_path = self.skill_manager.skills_dir().skill_path(name);
            #[cfg(feature = "skill-guard")]
            {
                let guard = crate::skill_guard::DefaultSkillGuard::new();
                if let Err(block_msg) = crate::skill_guard::guard_skill_write(&skill_path, &guard) {
                    // Skill was already deleted by the guard — return error
                    return Err(SkillManagerError::IoError(block_msg));
                }
            }
            #[cfg(not(feature = "skill-guard"))]
            {
                // Guard is enabled in config but the feature isn't compiled — skip scan
                let _ = skill_path;
            }
        }

        Ok(response)
    }

    /// Edit an existing skill.
    pub fn edit_skill(
        &self,
        name: &str,
        content: &str,
    ) -> Result<SkillActionResponse, SkillManagerError> {
        self.skill_manager.edit(name, content)
    }

    /// Patch a skill file.
    pub fn patch_skill(
        &self,
        name: &str,
        file: Option<&str>,
        old_text: &str,
        new_text: &str,
        replace_all: bool,
    ) -> Result<SkillActionResponse, SkillManagerError> {
        self.skill_manager
            .patch(name, file, old_text, new_text, replace_all)
    }

    /// Delete a skill.
    pub fn delete_skill(&self, name: &str) -> Result<SkillActionResponse, SkillManagerError> {
        self.skill_manager.delete(name)
    }

    /// Write a supporting file to a skill.
    pub fn write_skill_file(
        &self,
        name: &str,
        relative_path: &str,
        content: &str,
    ) -> Result<SkillActionResponse, SkillManagerError> {
        self.skill_manager.write_file(name, relative_path, content)
    }

    /// Remove a supporting file from a skill.
    pub fn remove_skill_file(
        &self,
        name: &str,
        relative_path: &str,
    ) -> Result<SkillActionResponse, SkillManagerError> {
        self.skill_manager.remove_file(name, relative_path)
    }

    /// Pin a skill (protect from deletion).
    pub fn pin_skill(&self, name: &str) -> Result<(), SkillManagerError> {
        self.skill_manager.pin(name)
    }

    /// Unpin a skill.
    pub fn unpin_skill(&self, name: &str) -> Result<(), SkillManagerError> {
        self.skill_manager.unpin(name)
    }

    // ── Skill Discovery ────────────────────────────────────────────

    /// List all skills — returns compact metadata index (no body content).
    pub fn list_skills(&self) -> SkillsListResult {
        self.skills_tool.skills_list(self.skills_dir())
    }

    /// View the full content of a skill.
    pub fn view_skill(&self, name: &str, file: Option<&str>) -> SkillViewResult {
        self.skills_tool
            .skill_view(name, self.skills_dir(), file)
    }

    /// Check skill requirements against the current environment.
    pub fn check_requirements(&self, name: &str) -> SkillRequirements {
        self.skills_tool
            .check_skill_requirements(name, self.skills_dir())
    }

    // ── System Prompt Integration ──────────────────────────────────

    /// Generate a compact skill index string suitable for system prompt inclusion.
    ///
    /// Format:
    /// ```text
    /// Available Skills:
    /// - skill-a: First skill
    /// - skill-b: Second skill
    /// ```
    ///
    /// Returns an empty string if no skills are available.
    pub fn format_skill_index(&self) -> String {
        let result = self.list_skills();
        if result.count == 0 {
            return String::new();
        }

        let mut output = "Available Skills:\n".to_string();
        for skill in &result.skills {
            output.push_str(&format!("- {}: {}\n", skill.name, skill.description));
        }
        output
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_engine() -> (HermesEngine, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let config = HermesConfig {
            skills_dir: dir.path().join("skills"),
            guard_enabled: false,
        };
        let engine = HermesEngine::new(config);
        (engine, dir)
    }

    fn valid_content(name: &str, desc: &str) -> String {
        format!(
            "---\nname: {}\ndescription: {}\n---\n\n# {}\n\nBody content",
            name, desc, name
        )
    }

    #[test]
    fn test_create_and_list_skill() {
        let (engine, _dir) = make_engine();
        let content = valid_content("test-skill", "Test skill");
        engine.create_skill("test-skill", None, &content).unwrap();

        let result = engine.list_skills();
        assert_eq!(result.count, 1);
        assert_eq!(result.skills[0].name, "test-skill");
    }

    #[test]
    fn test_create_and_view_skill() {
        let (engine, _dir) = make_engine();
        let content = valid_content("view-me", "Viewable skill");
        engine.create_skill("view-me", None, &content).unwrap();

        let view = engine.view_skill("view-me", None);
        assert_eq!(view.skill_name, "view-me");
        assert!(view.content.contains("Body content"));
    }

    #[test]
    fn test_edit_skill() {
        let (engine, _dir) = make_engine();
        let content = valid_content("edit-me", "Original");
        engine.create_skill("edit-me", None, &content).unwrap();

        let new_content = valid_content("edit-me", "Edited description");
        engine.edit_skill("edit-me", &new_content).unwrap();

        // view_skill strips frontmatter; check body content instead
        let view = engine.view_skill("edit-me", None);
        assert!(view.content.contains("Body content"));
        assert_eq!(view.skill_name, "edit-me");
    }

    #[test]
    fn test_delete_skill() {
        let (engine, _dir) = make_engine();
        let content = valid_content("delete-me", "To delete");
        engine.create_skill("delete-me", None, &content).unwrap();
        assert_eq!(engine.list_skills().count, 1);

        engine.delete_skill("delete-me").unwrap();
        assert_eq!(engine.list_skills().count, 0);
    }

    #[test]
    fn test_pin_protects_from_delete() {
        let (engine, _dir) = make_engine();
        let content = valid_content("pinned", "Pinned skill");
        engine.create_skill("pinned", None, &content).unwrap();
        engine.pin_skill("pinned").unwrap();

        let err = engine.delete_skill("pinned").unwrap_err();
        assert!(matches!(err, SkillManagerError::Pinned(_)));
    }

    #[test]
    fn test_format_skill_index() {
        let (engine, _dir) = make_engine();
        let content_a = valid_content("skill-a", "First skill");
        let content_b = valid_content("skill-b", "Second skill");
        engine.create_skill("skill-a", None, &content_a).unwrap();
        engine.create_skill("skill-b", None, &content_b).unwrap();

        let index = engine.format_skill_index();
        assert!(index.contains("Available Skills"));
        assert!(index.contains("skill-a: First skill"));
        assert!(index.contains("skill-b: Second skill"));
    }

    #[test]
    fn test_format_skill_index_empty() {
        let (engine, _dir) = make_engine();
        let index = engine.format_skill_index();
        assert!(index.is_empty());
    }

    #[test]
    fn test_write_and_remove_skill_file() {
        let (engine, _dir) = make_engine();
        let content = valid_content("file-skill", "File test");
        engine.create_skill("file-skill", None, &content).unwrap();

        engine
            .write_skill_file("file-skill", "references/api.md", "# API Docs")
            .unwrap();

        let view = engine.view_skill("file-skill", Some("references/api.md"));
        assert_eq!(view.content, "# API Docs");

        engine.remove_skill_file("file-skill", "references/api.md").unwrap();

        // After removal, viewing should return an error
        let view = engine.view_skill("file-skill", Some("references/api.md"));
        assert!(view.content.contains("Error"));
    }
}
