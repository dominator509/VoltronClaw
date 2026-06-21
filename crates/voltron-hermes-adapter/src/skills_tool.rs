//! Skills Tool — progressive disclosure tools for skill discovery and loading.
//!
//! Port of `tools/skills_tool.py` (skills_list, skill_view with tiered loading)
//! from the NousResearch/hermes-agent project.
//!
//! # Progressive Disclosure Architecture
//!
//! 1. **skills_list** — returns a compact index of all skills (name + description only).
//!    Token-efficient (~100 tokens for 10 skills), suitable for system prompt inclusion.
//! 2. **skill_view** — loads the full SKILL.md body (or a supporting file) on demand.
//! 3. **check_skill_requirements** — checks platform compatibility and prerequisites.

use crate::skill_storage::{
    platform_matches, read_skill_body, read_skill_file, scan_skills_dir, SkillMeta,
};
use std::path::Path;

// ── Result Types ────────────────────────────────────────────────────

/// Result of listing all skills — a compact index with no body content.
#[derive(Debug, Clone)]
pub struct SkillsListResult {
    /// Metadata entries (name + description only — never the body).
    pub skills: Vec<SkillMeta>,
    /// Total number of skills found.
    pub count: usize,
}

/// Result of viewing a skill's full content.
#[derive(Debug, Clone)]
pub struct SkillViewResult {
    /// The skill name.
    pub skill_name: String,
    /// The relative file path if a supporting file was requested, or `None` for SKILL.md.
    pub file: Option<String>,
    /// Full content of the requested file.
    pub content: String,
}

/// Result of checking a skill's requirements against the current environment.
#[derive(Debug, Clone)]
pub struct SkillRequirements {
    /// Whether the skill is fully compatible with the current environment.
    pub compatible: bool,
    /// Environment variables that are required but not set.
    pub missing_env_vars: Vec<String>,
    /// Whether the skill is restricted on the current platform.
    pub unsupported_platform: bool,
}

// ── SkillsTool Trait ────────────────────────────────────────────────

/// Progressive disclosure tools for skill discovery and loading.
pub trait SkillsTool: Send + Sync {
    /// List all skills, returning only metadata (name + description).
    ///
    /// This is a lightweight operation — it scans directories, parses frontmatter,
    /// but never loads full skill bodies. The result is token-efficient and
    /// suitable for inclusion in system prompts.
    fn skills_list(&self, base: &Path) -> SkillsListResult;

    /// View the full content of a skill's SKILL.md or a supporting file.
    ///
    /// If `file` is `None`, returns the full SKILL.md body (after stripping frontmatter).
    /// If `file` is `Some(path)`, returns the content of that supporting file.
    ///
    /// Validates path traversal attempts.
    fn skill_view(&self, name: &str, base: &Path, file: Option<&str>) -> SkillViewResult;

    /// Check whether a skill's requirements are satisfied in the current environment.
    ///
    /// Checks platform restrictions and prerequisite environment variables.
    fn check_skill_requirements(&self, name: &str, base: &Path) -> SkillRequirements;
}

// ── DiskSkillsTool ──────────────────────────────────────────────────

/// Concrete `SkillsTool` implementation backed by the local filesystem.
#[derive(Debug, Clone)]
pub struct DiskSkillsTool;

impl DiskSkillsTool {
    /// Create a new `DiskSkillsTool`.
    pub fn new() -> Self {
        Self
    }
}

impl Default for DiskSkillsTool {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillsTool for DiskSkillsTool {
    fn skills_list(&self, base: &Path) -> SkillsListResult {
        let skills = scan_skills_dir(base);
        let count = skills.len();
        SkillsListResult { skills, count }
    }

    fn skill_view(&self, name: &str, base: &Path, file: Option<&str>) -> SkillViewResult {
        match file {
            Some(relative_path) => {
                let content =
                    read_skill_file(name, base, relative_path).unwrap_or_else(|e| {
                        format!("Error reading file: {e}")
                    });
                SkillViewResult {
                    skill_name: name.to_string(),
                    file: Some(relative_path.to_string()),
                    content,
                }
            }
            None => {
                let content = read_skill_body(name, base).unwrap_or_else(|e| {
                    format!("Error reading skill: {e}")
                });
                SkillViewResult {
                    skill_name: name.to_string(),
                    file: None,
                    content,
                }
            }
        }
    }

    fn check_skill_requirements(&self, name: &str, base: &Path) -> SkillRequirements {
        // Scan all skills to find the requested one
        let skills = scan_skills_dir(base);
        let skill = match skills.into_iter().find(|s| s.name == name) {
            Some(s) => s,
            None => {
                return SkillRequirements {
                    compatible: false,
                    missing_env_vars: vec![],
                    unsupported_platform: false,
                };
            }
        };

        // Check platform
        let platform_ok = platform_matches(&skill.platforms);
        if !platform_ok {
            return SkillRequirements {
                compatible: false,
                missing_env_vars: vec![],
                unsupported_platform: true,
            };
        }

        // Check prerequisites
        let mut missing = Vec::new();
        if let Some(prereqs) = &skill.prerequisites {
            if let Some(env_vars) = &prereqs.env_vars {
                for var in env_vars {
                    if std::env::var(var).is_err() {
                        missing.push(var.clone());
                    }
                }
            }
        }

        SkillRequirements {
            compatible: missing.is_empty(),
            missing_env_vars: missing,
            unsupported_platform: !platform_ok,
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_skills_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path();

        // skill-a
        let a_dir = skills_dir.join("skill-a");
        std::fs::create_dir_all(&a_dir).unwrap();
        std::fs::write(
            a_dir.join("SKILL.md"),
            "---\nname: skill-a\ndescription: First skill\n---\n\n# Skill A\nBody for A",
        )
        .unwrap();

        // skill-b (with references)
        let b_dir = skills_dir.join("skill-b");
        std::fs::create_dir_all(b_dir.join("references")).unwrap();
        std::fs::write(
            b_dir.join("SKILL.md"),
            "---\nname: skill-b\ndescription: Second skill\n---\n\n# Skill B\nBody for B",
        )
        .unwrap();
        std::fs::write(b_dir.join("references").join("guide.md"), "# Reference Guide").unwrap();

        // No-SKILL.md dir (should be skipped)
        let empty_dir = skills_dir.join("no-skill");
        std::fs::create_dir_all(&empty_dir).unwrap();

        dir
    }

    #[test]
    fn test_skills_list_returns_metadata() {
        let dir = setup_skills_dir();
        let tool = DiskSkillsTool::new();
        let result = tool.skills_list(dir.path());

        assert_eq!(result.count, 2);
        assert_eq!(result.skills.len(), 2);

        // Verify only metadata (no body content)
        let names: Vec<&str> = result.skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"skill-a"));
        assert!(names.contains(&"skill-b"));

        // Verify no body leaked into description
        for skill in &result.skills {
            assert!(!skill.description.contains("Body"));
        }
    }

    #[test]
    fn test_skills_list_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let tool = DiskSkillsTool::new();
        let result = tool.skills_list(dir.path());
        assert_eq!(result.count, 0);
    }

    #[test]
    fn test_skill_view_returns_body() {
        let dir = setup_skills_dir();
        let tool = DiskSkillsTool::new();
        let result = tool.skill_view("skill-a", dir.path(), None);

        assert_eq!(result.skill_name, "skill-a");
        assert!(result.file.is_none());
        assert!(result.content.contains("Skill A"));
        assert!(result.content.contains("Body for A"));
    }

    #[test]
    fn test_skill_view_with_file() {
        let dir = setup_skills_dir();
        let tool = DiskSkillsTool::new();
        let result = tool.skill_view("skill-b", dir.path(), Some("references/guide.md"));

        assert_eq!(result.skill_name, "skill-b");
        assert_eq!(result.file, Some("references/guide.md".into()));
        assert_eq!(result.content, "# Reference Guide");
    }

    #[test]
    fn test_skill_view_not_found() {
        let tool = DiskSkillsTool::new();
        let dir = tempfile::tempdir().unwrap();
        let result = tool.skill_view("nonexistent", dir.path(), None);

        // Should return error message in content (not panic)
        assert!(result.content.contains("Error"));
    }

    #[test]
    fn test_check_skill_requirements_no_restrictions() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path();
        let s_dir = skills_dir.join("simple");
        std::fs::create_dir_all(&s_dir).unwrap();
        std::fs::write(
            s_dir.join("SKILL.md"),
            "---\nname: simple\ndescription: Simple skill\n---\n\nBody",
        )
        .unwrap();

        let tool = DiskSkillsTool::new();
        let reqs = tool.check_skill_requirements("simple", skills_dir);
        assert!(reqs.compatible);
        assert!(!reqs.unsupported_platform);
        assert!(reqs.missing_env_vars.is_empty());
    }

    #[test]
    fn test_check_skill_requirements_not_found() {
        let tool = DiskSkillsTool::new();
        let dir = tempfile::tempdir().unwrap();
        let reqs = tool.check_skill_requirements("missing", dir.path());
        assert!(!reqs.compatible);
    }

    #[test]
    fn test_skills_list_skips_no_skill_md() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path();

        // Dir with SKILL.md
        let valid_dir = skills_dir.join("valid-skill");
        std::fs::create_dir_all(&valid_dir).unwrap();
        std::fs::write(
            valid_dir.join("SKILL.md"),
            "---\nname: valid-skill\ndescription: Valid\n---\n\nBody",
        )
        .unwrap();

        // Dir without SKILL.md
        std::fs::create_dir_all(skills_dir.join("empty-dir")).unwrap();

        // File (not a dir)
        std::fs::write(skills_dir.join("not-a-dir"), "content").unwrap();

        let tool = DiskSkillsTool::new();
        let result = tool.skills_list(skills_dir);
        assert_eq!(result.count, 1);
        assert_eq!(result.skills[0].name, "valid-skill");
    }
}
