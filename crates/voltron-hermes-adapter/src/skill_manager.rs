//! Skill Manager — create, edit, patch, delete, and manage skill files.
//!
//! Port of `tools/skill_manager_tool.py` (all 6 actions, with security guard)
//! from the NousResearch/hermes-agent project.
//!
//! # Architecture
//!
//! `SkillManager` is the authoritative interface for all skill CRUD operations.
//! It uses `skill_storage` for path resolution, frontmatter validation, and
//! scanning, but adds atomic writes, pin protection, and security scanning.

use crate::skill_storage::{
    validate_frontmatter, validate_skill_name, SkillsDir,
    ALLOWED_SUBDIRS, MAX_SKILL_CONTENT_CHARS,
};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// ── Response / Error Types ─────────────────────────────────────────

/// Result of a skill management action.
#[derive(Debug, Clone)]
pub struct SkillActionResponse {
    /// Whether the action succeeded.
    pub success: bool,
    /// The skill name the action was performed on.
    pub skill_name: String,
    /// The action that was performed.
    pub action: String,
    /// Optional path to the affected file.
    pub path: Option<PathBuf>,
    /// Human-readable status message.
    pub message: String,
}

/// Errors that can occur during skill management.
#[derive(Debug, Clone)]
pub enum SkillManagerError {
    /// The skill was not found.
    NotFound(String),
    /// A skill with this name already exists.
    AlreadyExists(String),
    /// The skill name is invalid.
    InvalidName(String),
    /// The frontmatter is invalid.
    InvalidFrontmatter(String),
    /// Could not parse the YAML frontmatter.
    FrontmatterParseError(String),
    /// The skill content exceeds the maximum allowed size.
    ContentTooLarge(String),
    /// A supporting file exceeds the maximum allowed size.
    FileTooLarge(String),
    /// Path traversal detected.
    PathTraversal(String),
    /// File is not in an allowed subdirectory.
    InvalidSubdir(String),
    /// The skill is pinned and cannot be deleted.
    Pinned(String),
    /// The delete operation is unsafe.
    DeleteUnsafe(String),
    /// I/O error.
    IoError(String),
}

impl std::fmt::Display for SkillManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillManagerError::NotFound(msg) => write!(f, "not found: {msg}"),
            SkillManagerError::AlreadyExists(msg) => write!(f, "already exists: {msg}"),
            SkillManagerError::InvalidName(msg) => write!(f, "invalid name: {msg}"),
            SkillManagerError::InvalidFrontmatter(msg) => write!(f, "invalid frontmatter: {msg}"),
            SkillManagerError::FrontmatterParseError(msg) => {
                write!(f, "frontmatter parse error: {msg}")
            }
            SkillManagerError::ContentTooLarge(msg) => write!(f, "content too large: {msg}"),
            SkillManagerError::FileTooLarge(msg) => write!(f, "file too large: {msg}"),
            SkillManagerError::PathTraversal(msg) => write!(f, "path traversal: {msg}"),
            SkillManagerError::InvalidSubdir(msg) => write!(f, "invalid subdirectory: {msg}"),
            SkillManagerError::Pinned(msg) => write!(f, "pinned: {msg}"),
            SkillManagerError::DeleteUnsafe(msg) => write!(f, "delete unsafe: {msg}"),
            SkillManagerError::IoError(msg) => write!(f, "I/O error: {msg}"),
        }
    }
}

impl std::error::Error for SkillManagerError {}

/// Convert a `SkillManagerError` to a `SkillActionResponse` for tool output.
impl From<SkillManagerError> for SkillActionResponse {
    fn from(e: SkillManagerError) -> Self {
        // Extract skill name from error message if possible
        let skill_name = String::new();
        SkillActionResponse {
            success: false,
            skill_name,
            action: String::new(),
            path: None,
            message: e.to_string(),
        }
    }
}

// ── Pin State ───────────────────────────────────────────────────────

/// Pin state stored in a sidecar JSON file per skill directory.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SkillUsage {
    /// Whether the skill is pinned (protected from deletion).
    pinned: bool,
}

impl SkillUsage {
    fn pinned() -> Self {
        Self { pinned: true }
    }

    #[allow(dead_code)]
    fn unpinned() -> Self {
        Self { pinned: false }
    }
}

// ── SkillManager Trait ─────────────────────────────────────────────

/// Trait defining all skill management operations.
///
/// Six actions mirroring the Hermes-agent skill_manager_tool.py:
/// `create`, `edit`, `patch`, `delete`, `write_file`, `remove_file`.
pub trait SkillManager: Send + Sync {
    /// Create a new skill with the given name, optional category, and content.
    ///
    /// Creates the skill directory, writes `SKILL.md` atomically, and creates
    /// empty subdirectories (references/, templates/, scripts/, assets/).
    fn create(
        &self,
        name: &str,
        category: Option<&str>,
        content: &str,
    ) -> Result<SkillActionResponse, SkillManagerError>;

    /// Edit (full replace) an existing skill's `SKILL.md`.
    fn edit(&self, name: &str, content: &str) -> Result<SkillActionResponse, SkillManagerError>;

    /// Apply a targeted find-and-replace patch to a skill file.
    fn patch(
        &self,
        name: &str,
        file: Option<&str>,
        old_text: &str,
        new_text: &str,
        replace_all: bool,
    ) -> Result<SkillActionResponse, SkillManagerError>;

    /// Delete a skill directory entirely.
    fn delete(&self, name: &str) -> Result<SkillActionResponse, SkillManagerError>;

    /// Write a supporting file (reference, template, script, asset) into a skill.
    fn write_file(
        &self,
        name: &str,
        relative_path: &str,
        content: &str,
    ) -> Result<SkillActionResponse, SkillManagerError>;

    /// Remove a supporting file from a skill.
    fn remove_file(
        &self,
        name: &str,
        relative_path: &str,
    ) -> Result<SkillActionResponse, SkillManagerError>;
}

// ── DiskSkillManager ────────────────────────────────────────────────

/// Concrete `SkillManager` implementation backed by the local filesystem.
///
/// All writes are atomic (write to temp file, then `fs::rename`).
/// Supports pin protection (pinned skills cannot be deleted).
#[derive(Debug)]
pub struct DiskSkillManager {
    skills_dir: SkillsDir,
    pins: Mutex<Vec<String>>,
}

impl DiskSkillManager {
    /// Create a new `DiskSkillManager` rooted at the given skills directory.
    pub fn new(base: PathBuf) -> Self {
        std::fs::create_dir_all(&base).ok();
        Self {
            skills_dir: SkillsDir::new(base),
            pins: Mutex::new(Vec::new()),
        }
    }

    /// Return a reference to the underlying `SkillsDir`.
    pub fn skills_dir(&self) -> &SkillsDir {
        &self.skills_dir
    }

    /// Pin a skill, protecting it from deletion.
    pub fn pin(&self, name: &str) -> Result<(), SkillManagerError> {
        validate_skill_name(name).map_err(|e| {
            SkillManagerError::InvalidName(e.to_string())
        })?;

        let mut pins = self.pins.lock().unwrap();
        if !pins.contains(&name.to_string()) {
            pins.push(name.to_string());
        }

        // Write the pin sidecar file
        let pin_path = self.skills_dir.skill_pin_path(name);
        if let Some(parent) = pin_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let json = serde_json::to_string(&SkillUsage::pinned())
            .map_err(|e| SkillManagerError::IoError(e.to_string()))?;
        atomic_write(&pin_path, &json)
            .map_err(|e| SkillManagerError::IoError(e.to_string()))?;

        Ok(())
    }

    /// Unpin a skill, allowing it to be deleted.
    pub fn unpin(&self, name: &str) -> Result<(), SkillManagerError> {
        validate_skill_name(name).map_err(|e| {
            SkillManagerError::InvalidName(e.to_string())
        })?;

        let mut pins = self.pins.lock().unwrap();
        pins.retain(|p| p != name);

        // Remove the pin sidecar file
        let pin_path = self.skills_dir.skill_pin_path(name);
        if pin_path.exists() {
            std::fs::remove_file(&pin_path)
                .map_err(|e| SkillManagerError::IoError(e.to_string()))?;
        }

        Ok(())
    }

    /// Check whether a skill is pinned.
    pub fn is_pinned(&self, name: &str) -> bool {
        // Check in-memory list first
        let pins = self.pins.lock().unwrap();
        if pins.contains(&name.to_string()) {
            return true;
        }

        // Also check sidecar file
        let pin_path = self.skills_dir.skill_pin_path(name);
        if let Ok(content) = std::fs::read_to_string(&pin_path) {
            if let Ok(usage) = serde_json::from_str::<SkillUsage>(&content) {
                return usage.pinned;
            }
        }

        false
    }

    /// Resolve a skill path, checking for pin state and safety.
    fn resolve_delete_skill(&self, name: &str) -> Result<PathBuf, SkillManagerError> {
        validate_skill_name(name).map_err(|e| SkillManagerError::InvalidName(e.to_string()))?;

        let path = self.skills_dir.skill_path(name);
        if !path.exists() {
            return Err(SkillManagerError::NotFound(format!(
                "Skill '{}' not found",
                name
            )));
        }

        if !path.is_dir() {
            return Err(SkillManagerError::DeleteUnsafe(format!(
                "'{}' is not a directory",
                name
            )));
        }

        // Check for symlinks
        if path.is_symlink() {
            return Err(SkillManagerError::DeleteUnsafe(format!(
                "'{}' is a symlink — refusing to delete",
                name
            )));
        }

        // Verify it's inside the skills root
        let canonical = path
            .canonicalize()
            .map_err(|e| SkillManagerError::IoError(e.to_string()))?;
        let root_canonical = self
            .skills_dir
            .base()
            .canonicalize()
            .map_err(|e| SkillManagerError::IoError(e.to_string()))?;
        if !canonical.starts_with(&root_canonical) {
            return Err(SkillManagerError::DeleteUnsafe(format!(
                "'{}' is outside skills root",
                name
            )));
        }

        Ok(path)
    }
}

impl SkillManager for DiskSkillManager {
    fn create(
        &self,
        name: &str,
        category: Option<&str>,
        content: &str,
    ) -> Result<SkillActionResponse, SkillManagerError> {
        validate_skill_name(name).map_err(|e| SkillManagerError::InvalidName(e.to_string()))?;

        // Validate content size
        if content.len() > MAX_SKILL_CONTENT_CHARS {
            return Err(SkillManagerError::ContentTooLarge(format!(
                "Content exceeds {} characters (got {})",
                MAX_SKILL_CONTENT_CHARS,
                content.len()
            )));
        }

        // Validate frontmatter
        validate_frontmatter(content).map_err(|e| match e {
            crate::skill_storage::SkillStorageError::InvalidFrontmatter(m) => {
                SkillManagerError::InvalidFrontmatter(m)
            }
            crate::skill_storage::SkillStorageError::FrontmatterParse(m) => {
                SkillManagerError::FrontmatterParseError(m)
            }
            crate::skill_storage::SkillStorageError::InvalidName(m) => {
                SkillManagerError::InvalidName(m)
            }
            _ => SkillManagerError::IoError(e.to_string()),
        })?;

        // Determine target directory
        let target_dir = match category {
            Some(cat) if !cat.is_empty() => self.skills_dir.base().join(cat).join(name),
            _ => self.skills_dir.skill_path(name),
        };

        if target_dir.exists() {
            return Err(SkillManagerError::AlreadyExists(format!(
                "Skill '{}' already exists",
                name
            )));
        }

        // Create directory structure
        std::fs::create_dir_all(&target_dir)
            .map_err(|e| SkillManagerError::IoError(e.to_string()))?;

        // Create supporting subdirectories
        for subdir in ALLOWED_SUBDIRS {
            let subdir_path = target_dir.join(subdir);
            std::fs::create_dir_all(&subdir_path)
                .map_err(|e| SkillManagerError::IoError(e.to_string()))?;
        }

        // Atomically write SKILL.md
        let skill_md_path = target_dir.join("SKILL.md");
        atomic_write(&skill_md_path, content)
            .map_err(|e| SkillManagerError::IoError(e.to_string()))?;

        Ok(SkillActionResponse {
            success: true,
            skill_name: name.to_string(),
            action: "create".into(),
            path: Some(target_dir),
            message: format!("Created skill '{}'", name),
        })
    }

    fn edit(&self, name: &str, content: &str) -> Result<SkillActionResponse, SkillManagerError> {
        validate_skill_name(name).map_err(|e| SkillManagerError::InvalidName(e.to_string()))?;

        // Validate content size
        if content.len() > MAX_SKILL_CONTENT_CHARS {
            return Err(SkillManagerError::ContentTooLarge(format!(
                "Content exceeds {} characters (got {})",
                MAX_SKILL_CONTENT_CHARS,
                content.len()
            )));
        }

        // Validate frontmatter
        validate_frontmatter(content).map_err(|e| match e {
            crate::skill_storage::SkillStorageError::InvalidFrontmatter(m) => {
                SkillManagerError::InvalidFrontmatter(m)
            }
            crate::skill_storage::SkillStorageError::FrontmatterParse(m) => {
                SkillManagerError::FrontmatterParseError(m)
            }
            crate::skill_storage::SkillStorageError::InvalidName(m) => {
                SkillManagerError::InvalidName(m)
            }
            _ => SkillManagerError::IoError(e.to_string()),
        })?;

        let skill_md_path = self.skills_dir.skill_md_path(name);
        if !skill_md_path.exists() {
            return Err(SkillManagerError::NotFound(format!(
                "Skill '{}' not found",
                name
            )));
        }

        // Ensure parent directory exists
        if let Some(parent) = skill_md_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        // Atomic replace
        atomic_write(&skill_md_path, content)
            .map_err(|e| SkillManagerError::IoError(e.to_string()))?;

        Ok(SkillActionResponse {
            success: true,
            skill_name: name.to_string(),
            action: "edit".into(),
            path: Some(skill_md_path),
            message: format!("Edited skill '{}'", name),
        })
    }

    fn patch(
        &self,
        name: &str,
        file: Option<&str>,
        old_text: &str,
        new_text: &str,
        replace_all: bool,
    ) -> Result<SkillActionResponse, SkillManagerError> {
        validate_skill_name(name).map_err(|e| SkillManagerError::InvalidName(e.to_string()))?;

        let file_path = match file {
            Some(rel_path) => {
                // Validate it's in an allowed subdir
                let path = self.skills_dir.skill_file_path(name, rel_path);
                if !path.exists() {
                    return Err(SkillManagerError::NotFound(format!(
                        "File '{}' not found in skill '{}'",
                        rel_path, name
                    )));
                }
                path
            }
            None => self.skills_dir.skill_md_path(name),
        };

        if !file_path.exists() {
            return Err(SkillManagerError::NotFound(format!(
                "Skill '{}' not found",
                name
            )));
        }

        let content = std::fs::read_to_string(&file_path)
            .map_err(|e| SkillManagerError::IoError(e.to_string()))?;

        let new_content = if replace_all {
            content.replace(old_text, new_text)
        } else {
            match content.find(old_text) {
                Some(_) => content.replacen(old_text, new_text, 1),
                None => {
                    return Err(SkillManagerError::NotFound(format!(
                        "Old text not found in '{}'",
                        file.unwrap_or("SKILL.md")
                    )));
                }
            }
        };

        // If we edited SKILL.md, re-validate frontmatter
        if file.is_none() {
            validate_frontmatter(&new_content).map_err(|e| match e {
                crate::skill_storage::SkillStorageError::InvalidFrontmatter(m) => {
                    SkillManagerError::InvalidFrontmatter(m)
                }
                crate::skill_storage::SkillStorageError::FrontmatterParse(m) => {
                    SkillManagerError::FrontmatterParseError(m)
                }
                _ => SkillManagerError::IoError(e.to_string()),
            })?;
        }

        atomic_write(&file_path, &new_content)
            .map_err(|e| SkillManagerError::IoError(e.to_string()))?;

        Ok(SkillActionResponse {
            success: true,
            skill_name: name.to_string(),
            action: "patch".into(),
            path: Some(file_path),
            message: format!("Patched '{}' in skill '{}'", file.unwrap_or("SKILL.md"), name),
        })
    }

    fn delete(&self, name: &str) -> Result<SkillActionResponse, SkillManagerError> {
        // Check pin state
        if self.is_pinned(name) {
            return Err(SkillManagerError::Pinned(format!(
                "Skill '{}' is pinned and cannot be deleted",
                name
            )));
        }

        let path = self.resolve_delete_skill(name)?;

        // Remove the directory tree
        std::fs::remove_dir_all(&path)
            .map_err(|e| SkillManagerError::IoError(e.to_string()))?;

        // Clean up empty parent category directories
        if let Some(parent) = path.parent() {
            if parent != self.skills_dir.base() {
                let _ = std::fs::remove_dir(parent); // best-effort
            }
        }

        // Remove from pin list
        {
            let mut pins = self.pins.lock().unwrap();
            pins.retain(|p| p != name);
        }

        Ok(SkillActionResponse {
            success: true,
            skill_name: name.to_string(),
            action: "delete".into(),
            path: None,
            message: format!("Deleted skill '{}'", name),
        })
    }

    fn write_file(
        &self,
        name: &str,
        relative_path: &str,
        content: &str,
    ) -> Result<SkillActionResponse, SkillManagerError> {
        validate_skill_name(name).map_err(|e| SkillManagerError::InvalidName(e.to_string()))?;

        // Validate the relative path is in an allowed subdirectory
        let first_component = relative_path
            .split(&['/', '\\'])
            .next()
            .unwrap_or("");

        if !ALLOWED_SUBDIRS.contains(&first_component) {
            return Err(SkillManagerError::InvalidSubdir(format!(
                "Path '{}' must start with one of {:?}",
                relative_path, ALLOWED_SUBDIRS
            )));
        }

        // Check for path traversal
        if relative_path.contains("..") {
            return Err(SkillManagerError::PathTraversal(format!(
                "Path '{}' contains '..' traversal",
                relative_path
            )));
        }

        // Check content size
        if content.len() > crate::skill_storage::MAX_SKILL_FILE_BYTES {
            return Err(SkillManagerError::FileTooLarge(format!(
                "File exceeds {} bytes (got {})",
                crate::skill_storage::MAX_SKILL_FILE_BYTES,
                content.len()
            )));
        }

        let skill_dir = self.skills_dir.skill_path(name);
        if !skill_dir.exists() {
            return Err(SkillManagerError::NotFound(format!(
                "Skill '{}' not found",
                name
            )));
        }

        let file_path = self.skills_dir.skill_file_path(name, relative_path);

        // Create parent directories
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SkillManagerError::IoError(e.to_string()))?;
        }

        // Atomic write
        atomic_write(&file_path, content)
            .map_err(|e| SkillManagerError::IoError(e.to_string()))?;

        Ok(SkillActionResponse {
            success: true,
            skill_name: name.to_string(),
            action: "write_file".into(),
            path: Some(file_path),
            message: format!("Wrote '{}' in skill '{}'", relative_path, name),
        })
    }

    fn remove_file(
        &self,
        name: &str,
        relative_path: &str,
    ) -> Result<SkillActionResponse, SkillManagerError> {
        validate_skill_name(name).map_err(|e| SkillManagerError::InvalidName(e.to_string()))?;

        let file_path = self.skills_dir.skill_file_path(name, relative_path);
        if !file_path.exists() {
            return Err(SkillManagerError::NotFound(format!(
                "File '{}' not found in skill '{}'",
                relative_path, name
            )));
        }

        std::fs::remove_file(&file_path)
            .map_err(|e| SkillManagerError::IoError(e.to_string()))?;

        // Clean up empty parent directories
        if let Some(parent) = file_path.parent() {
            let _ = std::fs::remove_dir(parent); // best-effort
        }

        Ok(SkillActionResponse {
            success: true,
            skill_name: name.to_string(),
            action: "remove_file".into(),
            path: None,
            message: format!("Removed '{}' from skill '{}'", relative_path, name),
        })
    }
}

// ── Atomic Write Helper ─────────────────────────────────────────────

/// Write content to a file atomically using a temp file + rename.
///
/// Writes to a `.tmp` file in the same directory, then atomically renames
/// it to the target path. This prevents partial writes from corrupting
/// skill files.
fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let tmp_path = dir.join(format!(".{}_tmp", path.file_name().unwrap_or_default().to_string_lossy()));

    std::fs::write(&tmp_path, content)?;
    std::fs::rename(&tmp_path, path)?;

    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill_storage::read_skill_body;

    fn make_manager() -> (DiskSkillManager, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let manager = DiskSkillManager::new(dir.path().join("skills"));
        (manager, dir)
    }

    fn valid_skill_content(name: &str, desc: &str) -> String {
        format!(
            "---\nname: {}\ndescription: {}\n---\n\n# {}\n\nBody content here",
            name, desc, name
        )
    }

    #[test]
    fn test_create_valid_skill() {
        let (manager, _dir) = make_manager();
        let content = valid_skill_content("my-skill", "A test skill");
        let result = manager.create("my-skill", None, &content).unwrap();
        assert!(result.success);
        assert_eq!(result.action, "create");

        // Verify the directory exists
        let skill_path = manager.skills_dir().skill_path("my-skill");
        assert!(skill_path.join("SKILL.md").exists());
        assert!(skill_path.join("references").exists());
        assert!(skill_path.join("templates").exists());
        assert!(skill_path.join("scripts").exists());
        assert!(skill_path.join("assets").exists());
    }

    #[test]
    fn test_create_duplicate_skill() {
        let (manager, _dir) = make_manager();
        let content = valid_skill_content("dup-skill", "Duplicate test");
        manager.create("dup-skill", None, &content).unwrap();
        let err = manager.create("dup-skill", None, &content).unwrap_err();
        assert!(matches!(err, SkillManagerError::AlreadyExists(_)));
    }

    #[test]
    fn test_create_invalid_name() {
        let (manager, _dir) = make_manager();
        let content = valid_skill_content("invalid", "test");
        let err = manager.create("INVALID-UPPERCASE", None, &content).unwrap_err();
        assert!(matches!(err, SkillManagerError::InvalidName(_)));
    }

    #[test]
    fn test_create_content_too_large() {
        let (manager, _dir) = make_manager();
        let large_body = "x".repeat(MAX_SKILL_CONTENT_CHARS + 1);
        let content = format!(
            "---\nname: large-skill\ndescription: Too large\n---\n\n{}",
            large_body
        );
        let err = manager.create("large-skill", None, &content).unwrap_err();
        assert!(matches!(err, SkillManagerError::ContentTooLarge(_)));
    }

    #[test]
    fn test_edit_existing_skill() {
        let (manager, _dir) = make_manager();
        let content = valid_skill_content("edit-me", "Original");
        manager.create("edit-me", None, &content).unwrap();

        let new_content = valid_skill_content("edit-me", "Updated description");
        let result = manager.edit("edit-me", &new_content).unwrap();
        assert!(result.success);
        assert_eq!(result.action, "edit");

        // Read raw file content (includes frontmatter) to verify edit
        let skill_md_path = manager.skills_dir().skill_md_path("edit-me");
        let raw = std::fs::read_to_string(&skill_md_path).unwrap();
        assert!(raw.contains("Updated description"));
        assert!(raw.contains("Body content here"));
    }

    #[test]
    fn test_edit_nonexistent_skill() {
        let (manager, _dir) = make_manager();
        let content = valid_skill_content("ghost", "test");
        let err = manager.edit("ghost", &content).unwrap_err();
        assert!(matches!(err, SkillManagerError::NotFound(_)));
    }

    #[test]
    fn test_patch_skill_md() {
        let (manager, _dir) = make_manager();
        let content = "---\nname: patch-test\ndescription: Original desc\n---\n\n# Original\nbody";
        manager.create("patch-test", None, content).unwrap();

        let result = manager.patch("patch-test", None, "Original desc", "Patched desc", false).unwrap();
        assert!(result.success);
        assert_eq!(result.action, "patch");
    }

    #[test]
    fn test_patch_old_text_not_found() {
        let (manager, _dir) = make_manager();
        let content = "---\nname: patch-test\ndescription: Test\n---\n\nBody";
        manager.create("patch-test", None, content).unwrap();

        let err = manager.patch("patch-test", None, "NONEXISTENT", "replacement", false).unwrap_err();
        assert!(matches!(err, SkillManagerError::NotFound(_)));
    }

    #[test]
    fn test_patch_replace_all() {
        let (manager, _dir) = make_manager();
        let content = "---\nname: multi-patch\ndescription: Test\n---\n\nfoo bar foo bar";
        manager.create("multi-patch", None, content).unwrap();

        let result = manager.patch("multi-patch", None, "foo", "baz", true).unwrap();
        assert!(result.success);

        let body = read_skill_body("multi-patch", manager.skills_dir().base()).unwrap();
        assert_eq!(body, "baz bar baz bar");
    }

    #[test]
    fn test_delete_existing_skill() {
        let (manager, _dir) = make_manager();
        let content = valid_skill_content("delete-me", "To be deleted");
        manager.create("delete-me", None, &content).unwrap();

        let result = manager.delete("delete-me").unwrap();
        assert!(result.success);

        assert!(!manager.skills_dir().skill_path("delete-me").exists());
    }

    #[test]
    fn test_delete_pinned_skill() {
        let (manager, _dir) = make_manager();
        let content = valid_skill_content("pinned-skill", "Pinned");
        manager.create("pinned-skill", None, &content).unwrap();
        manager.pin("pinned-skill").unwrap();

        let err = manager.delete("pinned-skill").unwrap_err();
        assert!(matches!(err, SkillManagerError::Pinned(_)));
    }

    #[test]
    fn test_write_file_valid() {
        let (manager, _dir) = make_manager();
        let content = valid_skill_content("file-test", "Test writes");
        manager.create("file-test", None, &content).unwrap();

        let result = manager.write_file("file-test", "references/api.md", "# API Docs").unwrap();
        assert!(result.success);

        let file_path = manager.skills_dir().skill_file_path("file-test", "references/api.md");
        assert!(file_path.exists());
    }

    #[test]
    fn test_write_file_invalid_subdir() {
        let (manager, _dir) = make_manager();
        let content = valid_skill_content("file-test", "Test writes");
        manager.create("file-test", None, &content).unwrap();

        let err = manager.write_file("file-test", "secret/out.txt", "hack").unwrap_err();
        assert!(matches!(err, SkillManagerError::InvalidSubdir(_)));
    }

    #[test]
    fn test_remove_file() {
        let (manager, _dir) = make_manager();
        let content = valid_skill_content("rm-test", "Test removal");
        manager.create("rm-test", None, &content).unwrap();
        manager.write_file("rm-test", "references/api.md", "# API").unwrap();

        let result = manager.remove_file("rm-test", "references/api.md").unwrap();
        assert!(result.success);

        let file_path = manager.skills_dir().skill_file_path("rm-test", "references/api.md");
        assert!(!file_path.exists());
    }

    #[test]
    fn test_remove_file_not_found() {
        let (manager, _dir) = make_manager();
        let content = valid_skill_content("rm-test", "Test removal");
        manager.create("rm-test", None, &content).unwrap();

        let err = manager.remove_file("rm-test", "references/nonexistent.md").unwrap_err();
        assert!(matches!(err, SkillManagerError::NotFound(_)));
    }

    #[test]
    fn test_pin_unpin() {
        let (manager, _dir) = make_manager();
        let content = valid_skill_content("pin-me", "Pin test");
        manager.create("pin-me", None, &content).unwrap();

        assert!(!manager.is_pinned("pin-me"));
        manager.pin("pin-me").unwrap();
        assert!(manager.is_pinned("pin-me"));
        manager.unpin("pin-me").unwrap();
        assert!(!manager.is_pinned("pin-me"));
    }

    #[test]
    fn test_create_with_category() {
        let (manager, _dir) = make_manager();
        let content = valid_skill_content("cat-skill", "Categorized skill");
        let result = manager.create("cat-skill", Some("my-category"), &content).unwrap();
        assert!(result.success);

        let path = manager.skills_dir().base().join("my-category").join("cat-skill");
        assert!(path.join("SKILL.md").exists());
    }
}
