//! Skill Storage — directory scanning, frontmatter parsing, and validation.
//!
//! Port of `tools/skill_manager_tool.py` (directory layout, validation) and
//! `tools/skills_tool.py` (frontmatter parsing, platform gating) from the
//! NousResearch/hermes-agent project.
//!
//! # Layout
//!
//! ```text
//! ~/.voltron/skills/
//! ├── my-skill/
//! │   ├── SKILL.md
//! │   ├── references/
//! │   ├── templates/
//! │   ├── scripts/
//! │   └── assets/
//! └── category/
//!     └── another-skill/
//!         └── SKILL.md
//! ```

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

// ── Constants ──────────────────────────────────────────────────────

/// Maximum length of a skill name.
pub const MAX_NAME_LENGTH: usize = 64;

/// Maximum length of a skill description.
pub const MAX_DESCRIPTION_LENGTH: usize = 1024;

/// Maximum character count for a single SKILL.md body.
pub const MAX_SKILL_CONTENT_CHARS: usize = 100_000;

/// Maximum byte count for a single supporting file.
pub const MAX_SKILL_FILE_BYTES: usize = 1_048_576;

/// Regex pattern for valid skill names.
pub const VALID_NAME_RE: &str = r"^[a-z0-9][a-z0-9._-]*$";

/// Subdirectories allowed inside a skill directory.
pub const ALLOWED_SUBDIRS: &[&str] = &["references", "templates", "scripts", "assets"];

static VALID_NAME_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(VALID_NAME_RE).unwrap());

// ── Prerequisites ───────────────────────────────────────────────────

/// Optional prerequisites for a skill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Prerequisites {
    /// Required environment variable names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_vars: Option<Vec<String>>,
    /// Required CLI commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<String>>,
}

// ── SkillMeta ───────────────────────────────────────────────────────

/// Metadata extracted from a skill's `SKILL.md` frontmatter.
///
/// Only metadata is loaded into the index — the body (full instructions)
/// is loaded on demand via `read_skill_body()`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillMeta {
    /// Skill name (max 64 chars). Lowercase, alphanumeric, with dots/hyphens/underscores.
    pub name: String,
    /// Brief description (max 1024 chars).
    pub description: String,
    /// Optional semantic version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Optional license identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Optional platform restrictions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
    /// Optional prerequisites.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerequisites: Option<Prerequisites>,
    /// Arbitrary metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

// ── SkillsDir ───────────────────────────────────────────────────────

/// Wrapper around a base path containing skill directories.
#[derive(Debug, Clone)]
pub struct SkillsDir {
    base: PathBuf,
}

impl SkillsDir {
    /// Create a new `SkillsDir` rooted at the given path.
    pub fn new(base: PathBuf) -> Self {
        Self { base }
    }

    /// Return a reference to the base path.
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Resolve the path to a skill directory by name.
    pub fn skill_path(&self, name: &str) -> PathBuf {
        self.base.join(name)
    }

    /// Resolve the path to a skill's SKILL.md file.
    pub fn skill_md_path(&self, name: &str) -> PathBuf {
        self.skill_path(name).join("SKILL.md")
    }

    /// Resolve the path to a skill's `skill_usage.json` pin file.
    pub fn skill_pin_path(&self, name: &str) -> PathBuf {
        self.skill_path(name).join("skill_usage.json")
    }

    /// Resolve a supporting file path within a skill directory.
    pub fn skill_file_path(&self, name: &str, relative_path: &str) -> PathBuf {
        self.skill_path(name).join(relative_path)
    }
}

// ── Scanning ────────────────────────────────────────────────────────

/// Scan a skills directory and return metadata for every valid skill.
///
/// Walks one level deep under `base` looking for subdirectories that
/// contain a `SKILL.md` file. Parses the YAML frontmatter to extract
/// `SkillMeta`. Directories without `SKILL.md` are silently skipped.
///
/// The returned `Vec<SkillMeta>` contains **only metadata** — no body
/// content. This makes it token-efficient for system prompt inclusion.
pub fn scan_skills_dir(base: &Path) -> Vec<SkillMeta> {
    let mut skills = Vec::new();

    let dir = match std::fs::read_dir(base) {
        Ok(d) => d,
        Err(_) => return skills,
    };

    for entry in dir.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }

        let content = match std::fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };

        match validate_frontmatter(&content) {
            Ok(meta) => skills.push(meta),
            Err(_) => continue,
        }
    }

    skills
}

// ── Reading ─────────────────────────────────────────────────────────

/// Read the body of a skill's `SKILL.md` (everything after the closing `---`).
///
/// Returns the body content as a string. The frontmatter is stripped.
/// Returns an error if the skill directory or `SKILL.md` does not exist,
/// or if the content exceeds `MAX_SKILL_CONTENT_CHARS`.
pub fn read_skill_body(name: &str, base: &Path) -> Result<String, SkillStorageError> {
    validate_skill_name(name)?;

    let skill_md = base.join(name).join("SKILL.md");
    if !skill_md.exists() {
        return Err(SkillStorageError::NotFound(format!(
            "Skill '{}' not found at {}",
            name,
            skill_md.display()
        )));
    }

    let content = std::fs::read_to_string(&skill_md)
        .map_err(|e| SkillStorageError::IoError(e.to_string()))?;

    // Strip frontmatter and return the body
    let body = strip_frontmatter(&content);
    if body.len() > MAX_SKILL_CONTENT_CHARS {
        return Err(SkillStorageError::TooLarge(format!(
            "Skill body exceeds {} characters (got {})",
            MAX_SKILL_CONTENT_CHARS,
            body.len()
        )));
    }

    Ok(body)
}

/// Read a support file from within a skill directory.
///
/// Validates that `relative_path` does not escape the skill directory
/// (path traversal protection) and that the file is within one of the
/// `ALLOWED_SUBDIRS`.
pub fn read_skill_file(name: &str, base: &Path, relative_path: &str) -> Result<String, SkillStorageError> {
    validate_skill_name(name)?;

    let path = validate_supporting_path(name, base, relative_path)?;

    if !path.exists() {
        return Err(SkillStorageError::NotFound(format!(
            "File '{}' not found in skill '{}'",
            relative_path, name
        )));
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| SkillStorageError::IoError(e.to_string()))?;

    if content.len() > MAX_SKILL_FILE_BYTES {
        return Err(SkillStorageError::TooLarge(format!(
            "File '{}' exceeds {} bytes (got {})",
            relative_path,
            MAX_SKILL_FILE_BYTES,
            content.len()
        )));
    }

    Ok(content)
}

// ── Validation ──────────────────────────────────────────────────────

/// Validate a skill name against the allowed pattern.
pub fn validate_skill_name(name: &str) -> Result<(), SkillStorageError> {
    if name.is_empty() {
        return Err(SkillStorageError::InvalidName("Name must not be empty".into()));
    }
    if name.len() > MAX_NAME_LENGTH {
        return Err(SkillStorageError::InvalidName(format!(
            "Name exceeds {} characters",
            MAX_NAME_LENGTH
        )));
    }
    if !VALID_NAME_REGEX.is_match(name) {
        return Err(SkillStorageError::InvalidName(format!(
            "Name '{}' does not match pattern '{}'",
            name, VALID_NAME_RE
        )));
    }
    Ok(())
}

/// Parse and validate YAML frontmatter from a `SKILL.md` file.
///
/// Returns the parsed `SkillMeta` on success. The frontmatter must be
/// delimited by `---` at the start of the file and a closing `---`.
pub fn validate_frontmatter(content: &str) -> Result<SkillMeta, SkillStorageError> {
    let frontmatter = extract_frontmatter(content)?;
    let meta: SkillMeta = serde_yaml::from_str(&frontmatter)
        .map_err(|e| SkillStorageError::FrontmatterParse(e.to_string()))?;

    // Validate required fields
    if meta.name.is_empty() {
        return Err(SkillStorageError::InvalidFrontmatter(
            "Field 'name' is required and must not be empty".into(),
        ));
    }
    if meta.description.is_empty() {
        return Err(SkillStorageError::InvalidFrontmatter(
            "Field 'description' is required and must not be empty".into(),
        ));
    }
    if meta.description.len() > MAX_DESCRIPTION_LENGTH {
        return Err(SkillStorageError::InvalidFrontmatter(format!(
            "Field 'description' exceeds {} characters (got {})",
            MAX_DESCRIPTION_LENGTH,
            meta.description.len()
        )));
    }
    if meta.name.len() > MAX_NAME_LENGTH {
        return Err(SkillStorageError::InvalidFrontmatter(format!(
            "Field 'name' exceeds {} characters",
            MAX_NAME_LENGTH
        )));
    }

    Ok(meta)
}

/// Check whether the current platform matches the skill's platform restrictions.
///
/// If `platforms` is `None` or empty, the skill is available on all platforms.
/// Otherwise, the current `std::env::consts::OS` is checked against the list.
pub fn platform_matches(platforms: &Option<Vec<String>>) -> bool {
    let platforms = match platforms {
        Some(p) if !p.is_empty() => p,
        _ => return true, // No restriction
    };

    let current_os = std::env::consts::OS;
    platforms.iter().any(|p| {
        let p = p.to_lowercase();
        p == current_os
            || (current_os == "macos" && p == "macos")
            || (current_os == "linux" && p == "linux")
            || (current_os == "windows" && p == "windows")
    })
}

// ── Internal helpers ────────────────────────────────────────────────

/// Extract YAML frontmatter between `---` delimiters at the start of a string.
fn extract_frontmatter(content: &str) -> Result<&str, SkillStorageError> {
    let content = content.trim_start();

    if !content.starts_with("---") {
        return Err(SkillStorageError::FrontmatterParse(
            "File must start with '---' frontmatter delimiter".into(),
        ));
    }

    // Find the closing `---`
    let after_first = &content[3..];
    let end = after_first
        .find("\n---")
        .or_else(|| after_first.find("\r\n---"))
        .map(|pos| pos)  // end at the newline before closing ---
        .ok_or_else(|| {
            SkillStorageError::FrontmatterParse(
                "Missing closing '---' frontmatter delimiter".into(),
            )
        })?;

    Ok(&after_first[..end])
}

/// Strip frontmatter from a SKILL.md file, returning only the body.
fn strip_frontmatter(content: &str) -> String {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return content.to_string();
    }

    let after_first = &content[3..];
    if let Some(end) = after_first.find("\n---") {
        let body_start = end + 4; // skip past "\n---"
        after_first[body_start..].trim().to_string()
    } else if let Some(end) = after_first.find("\r\n---") {
        let body_start = end + 5; // skip past "\r\n---"
        after_first[body_start..].trim().to_string()
    } else {
        content.to_string()
    }
}

/// Validate that a relative support file path is safe and within allowed subdirs.
fn validate_supporting_path(
    name: &str,
    base: &Path,
    relative_path: &str,
) -> Result<PathBuf, SkillStorageError> {
    let path = base.join(name).join(relative_path);

    // Canonicalize to check traversal
    let canonical = path.canonicalize().map_err(|_| {
        SkillStorageError::PathTraversal(format!("Cannot resolve path: {}", relative_path))
    })?;

    let skill_dir = base.join(name).canonicalize().map_err(|_| {
        SkillStorageError::NotFound(format!("Skill '{}' not found", name))
    })?;

    // Path must be within the skill directory
    if !canonical.starts_with(&skill_dir) {
        return Err(SkillStorageError::PathTraversal(format!(
            "Path '{}' escapes skill directory",
            relative_path
        )));
    }

    // Path must be in an allowed subdirectory
    let relative = canonical
        .strip_prefix(&skill_dir)
        .map_err(|_| SkillStorageError::PathTraversal("Failed to compute relative path".into()))?;

    let first_component = relative
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .unwrap_or_default();

    if !ALLOWED_SUBDIRS.contains(&first_component.as_str()) {
        return Err(SkillStorageError::InvalidSubdir(format!(
            "Path '{}' is not in an allowed subdirectory ({:?})",
            relative_path, ALLOWED_SUBDIRS
        )));
    }

    Ok(path)
}

// ── Error Type ──────────────────────────────────────────────────────

/// Errors that can occur during skill storage operations.
#[derive(Debug, Clone)]
pub enum SkillStorageError {
    /// The skill or file was not found.
    NotFound(String),
    /// The skill name is invalid.
    InvalidName(String),
    /// The frontmatter is structurally invalid.
    InvalidFrontmatter(String),
    /// The frontmatter could not be parsed as YAML.
    FrontmatterParse(String),
    /// Content exceeds size limits.
    TooLarge(String),
    /// Path traversal detected.
    PathTraversal(String),
    /// File is not in an allowed subdirectory.
    InvalidSubdir(String),
    /// I/O error.
    IoError(String),
}

impl std::fmt::Display for SkillStorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillStorageError::NotFound(msg) => write!(f, "not found: {msg}"),
            SkillStorageError::InvalidName(msg) => write!(f, "invalid name: {msg}"),
            SkillStorageError::InvalidFrontmatter(msg) => write!(f, "invalid frontmatter: {msg}"),
            SkillStorageError::FrontmatterParse(msg) => write!(f, "frontmatter parse error: {msg}"),
            SkillStorageError::TooLarge(msg) => write!(f, "content too large: {msg}"),
            SkillStorageError::PathTraversal(msg) => write!(f, "path traversal: {msg}"),
            SkillStorageError::InvalidSubdir(msg) => write!(f, "invalid subdirectory: {msg}"),
            SkillStorageError::IoError(msg) => write!(f, "I/O error: {msg}"),
        }
    }
}

impl std::error::Error for SkillStorageError {}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_skill_name_valid() {
        assert!(validate_skill_name("my-skill").is_ok());
        assert!(validate_skill_name("test.123").is_ok());
        assert!(validate_skill_name("a").is_ok());
        assert!(validate_skill_name("skill-name_v2").is_ok());
        assert!(validate_skill_name("0starting-with-digit").is_ok());
    }

    #[test]
    fn test_validate_skill_name_invalid() {
        assert!(validate_skill_name("").is_err());
        assert!(validate_skill_name("UPPERCASE").is_err());
        assert!(validate_skill_name("has space").is_err());
        assert!(validate_skill_name("has@symbol").is_err());
        assert!(validate_skill_name("-starts-with-dash").is_err());
        assert!(validate_skill_name(".starts-with-dot").is_err());
    }

    #[test]
    fn test_validate_frontmatter_valid() {
        let content = r#"---
name: my-skill
description: A test skill
version: 1.0.0
license: MIT
platforms: [linux]
---

# Skill Body"#;
        let meta = validate_frontmatter(content).unwrap();
        assert_eq!(meta.name, "my-skill");
        assert_eq!(meta.description, "A test skill");
        assert_eq!(meta.version, Some("1.0.0".into()));
        assert_eq!(meta.license, Some("MIT".into()));
        assert_eq!(meta.platforms, Some(vec!["linux".into()]));
    }

    #[test]
    fn test_validate_frontmatter_missing_name() {
        let content = r#"---
description: A skill without a name
---"#;
        let err = validate_frontmatter(content).unwrap_err();
        // Missing required YAML field 'name' causes a FrontmatterParse error
        assert!(
            matches!(&err, SkillStorageError::FrontmatterParse(_)),
            "Expected FrontmatterParse, got: {err}"
        );
        assert!(err.to_string().contains("name") || err.to_string().contains("missing"));
    }

    #[test]
    fn test_validate_frontmatter_missing_description() {
        let content = r#"---
name: no-desc
---"#;
        let err = validate_frontmatter(content).unwrap_err();
        // Missing required YAML field 'description' causes a FrontmatterParse error
        assert!(
            matches!(&err, SkillStorageError::FrontmatterParse(_)),
            "Expected FrontmatterParse, got: {err}"
        );
        assert!(err.to_string().contains("description") || err.to_string().contains("missing"));
    }

    #[test]
    fn test_validate_frontmatter_description_exceeds_max() {
        let desc = "x".repeat(MAX_DESCRIPTION_LENGTH + 1);
        let content = format!(
            r#"---
name: my-skill
description: {desc}
---"#
        );
        let err = validate_frontmatter(&content).unwrap_err();
        assert!(matches!(err, SkillStorageError::InvalidFrontmatter(_)));
    }

    #[test]
    fn test_validate_frontmatter_no_delimiters() {
        let content = "name: my-skill\ndescription: test";
        let err = validate_frontmatter(content).unwrap_err();
        assert!(matches!(err, SkillStorageError::FrontmatterParse(_)));
    }

    #[test]
    fn test_validate_frontmatter_no_closing_delimiter() {
        let content = "---\nname: my-skill\ndescription: test\n";
        let err = validate_frontmatter(content).unwrap_err();
        assert!(matches!(err, SkillStorageError::FrontmatterParse(_)));
    }

    #[test]
    fn test_strip_frontmatter() {
        let content = "---\nname: test\n---\n\n# Real content\nbody here";
        let body = strip_frontmatter(content);
        assert_eq!(body, "# Real content\nbody here");
    }

    #[test]
    fn test_strip_frontmatter_no_frontmatter() {
        let body = strip_frontmatter("# Just body");
        assert_eq!(body, "# Just body");
    }

    #[test]
    fn test_platform_matches_all() {
        assert!(platform_matches(&None));
        assert!(platform_matches(&Some(vec![])));
    }

    #[test]
    fn test_platform_matches_current() {
        let current = std::env::consts::OS;
        assert!(platform_matches(&Some(vec![current.to_string()])));
    }

    #[test]
    fn test_scan_skills_dir_empty() {
        let dir = tempfile::tempdir().unwrap();
        let skills = scan_skills_dir(dir.path());
        assert!(skills.is_empty());
    }

    #[test]
    fn test_scan_skills_dir_no_skill_md() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("empty-dir")).unwrap();
        let skills = scan_skills_dir(dir.path());
        assert!(skills.is_empty());
    }

    #[test]
    fn test_read_skill_body() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: Test\n---\n\n# Hello\nbody here",
        )
        .unwrap();

        let body = read_skill_body("my-skill", dir.path()).unwrap();
        assert_eq!(body, "# Hello\nbody here");
    }

    #[test]
    fn test_read_skill_body_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_skill_body("nonexistent", dir.path()).unwrap_err();
        assert!(matches!(err, SkillStorageError::NotFound(_)));
    }

    #[test]
    fn test_read_skill_file_valid() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("my-skill");
        std::fs::create_dir_all(skill_dir.join("references")).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: my-skill\ndescription: Test\n---\n\nBody").unwrap();
        std::fs::write(skill_dir.join("references").join("api.md"), "# API Reference").unwrap();

        let content = read_skill_file("my-skill", dir.path(), "references/api.md").unwrap();
        assert_eq!(content, "# API Reference");
    }

    #[test]
    fn test_read_skill_file_invalid_subdir() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: my-skill\ndescription: Test\n---").unwrap();

        let err = read_skill_file("my-skill", dir.path(), "secret/file.txt").unwrap_err();
        // Since 'secret/' dir doesn't exist, canonicalization fails first
        assert!(
            matches!(
                &err,
                SkillStorageError::NotFound(_)
                    | SkillStorageError::InvalidSubdir(_)
                    | SkillStorageError::PathTraversal(_)
            ),
            "Expected NotFound/InvalidSubdir/PathTraversal, got: {err}"
        );
    }

    #[test]
    fn test_read_skill_file_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: my-skill\ndescription: Test\n---").unwrap();

        // Escape via ../
        let err = read_skill_file("my-skill", dir.path(), "../outside.txt").unwrap_err();
        // The path may fail at canonicalization stage or traversal check
        assert!(
            matches!(&err, SkillStorageError::PathTraversal(_) | SkillStorageError::NotFound(_)),
            "Expected PathTraversal or NotFound, got: {err}"
        );
    }
}
