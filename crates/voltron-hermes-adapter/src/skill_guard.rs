//! Skill Guard — security scanner for agent-created skills.
//!
//! Port of `tools/skills_guard.py` (scan_skill, should_allow_install) from
//! the NousResearch/hermes-agent project.
//!
//! Off by default. Enabled via `skills.guard_agent_created` config or
//! the `skill-guard` feature flag.
//!
//! When enabled, scans agent-created skills for dangerous patterns before
//! write. On any dangerous finding, blocks creation and returns a scan report.

use std::path::Path;

// ── Types ───────────────────────────────────────────────────────────

/// Severity of a security finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    /// Informational — not dangerous but worth noting.
    Low,
    /// Potentially suspicious.
    Medium,
    /// Likely dangerous.
    High,
    /// Definitely dangerous — shell execution, data exfiltration, etc.
    Critical,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Low => write!(f, "low"),
            Severity::Medium => write!(f, "medium"),
            Severity::High => write!(f, "high"),
            Severity::Critical => write!(f, "critical"),
        }
    }
}

/// A single security finding.
#[derive(Debug, Clone)]
pub struct Finding {
    /// The file where the pattern was found.
    pub file: String,
    /// The line number (1-indexed).
    pub line: usize,
    /// The matched pattern description.
    pub pattern: String,
    /// How dangerous this finding is.
    pub severity: Severity,
}

/// Result of scanning a skill directory.
#[derive(Debug, Clone)]
pub struct ScanResult {
    /// All findings discovered during the scan.
    pub findings: Vec<Finding>,
    /// Number of files scanned.
    pub file_count: usize,
    /// Source of the skill (e.g., "agent-created").
    pub source: String,
}

// ── Scan Patterns ───────────────────────────────────────────────────

/// Dangerous shell execution patterns (simple substring matching).
const SHELL_PATTERNS: &[(&str, Severity)] = &[
    ("rm -rf", Severity::Critical),
    ("rm -fr", Severity::Critical),
    ("rm -r -f", Severity::Critical),
    ("curl | sh", Severity::Critical),
    ("curl | bash", Severity::Critical),
    ("wget -O - | sh", Severity::Critical),
    ("sudo ", Severity::High),
    ("eval(", Severity::Critical),
    ("exec(", Severity::Critical),
    ("exec \"", Severity::Critical),
    ("system(", Severity::Critical),
    ("passthru(", Severity::Critical),
    ("shell_exec(", Severity::Critical),
    ("`", Severity::Critical), // backtick execution
];

/// Network exfiltration patterns.
const NETWORK_PATTERNS: &[(&str, Severity)] = &[
    ("nc -", Severity::Critical),
    ("ncat", Severity::Critical),
    ("telnet", Severity::Medium),
];

/// Filesystem escape patterns.
const FS_PATTERNS: &[(&str, Severity)] = &[
    ("/etc/passwd", Severity::High),
    ("/etc/shadow", Severity::Critical),
    ("/etc/sudoers", Severity::Critical),
    ("/root/.ssh", Severity::Critical),
    ("~/.ssh", Severity::High),
];

/// Prompt injection patterns in skill content.
const INJECTION_PATTERNS: &[(&str, Severity)] = &[
    ("ignore previous instructions", Severity::Critical),
    ("ignore all instructions", Severity::Critical),
    ("system prompt:", Severity::High),
    ("<system>", Severity::Medium),
    ("<|system|>", Severity::High),
    ("you are now", Severity::Low),
    ("from now on, you are", Severity::Low),
];

/// All patterns grouped for scanning.
const ALL_PATTERNS: &[(&[(&str, Severity)], &str)] = &[
    (SHELL_PATTERNS, "shell_execution"),
    (NETWORK_PATTERNS, "network_exfiltration"),
    (FS_PATTERNS, "filesystem_escape"),
    (INJECTION_PATTERNS, "prompt_injection"),
];

// ── SkillGuard Trait ────────────────────────────────────────────────

/// Security scanner for agent-created skills.
pub trait SkillGuard: Send + Sync {
    /// Scan a skill directory for dangerous patterns.
    fn scan_skill(&self, skill_dir: &Path, source: &str) -> ScanResult;

    /// Determine whether the scan result should be allowed.
    fn should_allow(&self, result: &ScanResult) -> (Option<bool>, String);

    /// Format a scan report for human-readable output.
    fn format_scan_report(&self, result: &ScanResult) -> String;
}

// ── DefaultSkillGuard ───────────────────────────────────────────────

/// Default security guard implementation.
#[derive(Debug, Clone)]
pub struct DefaultSkillGuard;

impl DefaultSkillGuard {
    /// Create a new `DefaultSkillGuard`.
    pub fn new() -> Self {
        Self
    }
}

impl Default for DefaultSkillGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillGuard for DefaultSkillGuard {
    fn scan_skill(&self, skill_dir: &Path, source: &str) -> ScanResult {
        let mut findings = Vec::new();
        let mut file_count = 0;

        // Recursively collect files
        let mut dirs = vec![skill_dir.to_path_buf()];
        let mut files = Vec::new();

        while let Some(dir) = dirs.pop() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        dirs.push(path);
                    } else if path.is_file() {
                        files.push(path);
                    }
                }
            }
        }

        for path in &files {
            file_count += 1;
            if let Ok(content) = std::fs::read_to_string(path) {
                let file_name = path
                    .strip_prefix(skill_dir)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();

                for (line_num, line) in content.lines().enumerate() {
                    let line_lower = line.to_lowercase();

                    // Check for curl/wget with URLs (special handling for regex-like patterns)
                    if line_lower.contains("curl")
                        && (line_lower.contains("http://") || line_lower.contains("https://"))
                    {
                        findings.push(Finding {
                            file: file_name.clone(),
                            line: line_num + 1,
                            pattern: "curl.*https://".into(),
                            severity: Severity::High,
                        });
                    }
                    if line_lower.contains("wget")
                        && (line_lower.contains("http://") || line_lower.contains("https://"))
                    {
                        findings.push(Finding {
                            file: file_name.clone(),
                            line: line_num + 1,
                            pattern: "wget.*https://".into(),
                            severity: Severity::High,
                        });
                    }

                    // Standard substring patterns
                    for (patterns, _category) in ALL_PATTERNS {
                        for (pattern, severity) in *patterns {
                            // Skip network URL patterns handled above
                            if *pattern == "curl.*http://"
                                || *pattern == "curl.*https://"
                                || *pattern == "wget.*http://"
                                || *pattern == "wget.*https://"
                            {
                                continue;
                            }
                            if line_lower.contains(pattern) {
                                findings.push(Finding {
                                    file: file_name.clone(),
                                    line: line_num + 1,
                                    pattern: pattern.to_string(),
                                    severity: severity.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }

        ScanResult {
            findings,
            file_count,
            source: source.to_string(),
        }
    }

    fn should_allow(&self, result: &ScanResult) -> (Option<bool>, String) {
        if result.findings.is_empty() {
            return (Some(true), "No dangerous patterns found".into());
        }

        let has_critical = result
            .findings
            .iter()
            .any(|f| f.severity == Severity::Critical);
        let has_high = result
            .findings
            .iter()
            .any(|f| f.severity == Severity::High);

        if has_critical {
            return (
                Some(false),
                format!(
                    "Blocked: {} critical-severity finding(s) detected",
                    result
                        .findings
                        .iter()
                        .filter(|f| f.severity == Severity::Critical)
                        .count()
                ),
            );
        }

        if has_high {
            return (
                None,
                format!(
                    "Dangerous patterns detected ({} high-severity). Review required.",
                    result
                        .findings
                        .iter()
                        .filter(|f| f.severity == Severity::High)
                        .count()
                ),
            );
        }

        (None, "Low-to-medium severity patterns detected. Review required.".into())
    }

    fn format_scan_report(&self, result: &ScanResult) -> String {
        if result.findings.is_empty() {
            return format!(
                "Scan passed: {} files scanned, no dangerous patterns found.",
                result.file_count
            );
        }

        let mut report = format!("Security Scan Report (source: {})\n", result.source);
        report.push_str(&format!("   Files scanned: {}\n", result.file_count));
        report.push_str(&format!("   Findings: {}\n\n", result.findings.len()));

        let mut by_severity: Vec<(usize, &Finding)> =
            result.findings.iter().enumerate().collect();
        by_severity.sort_by_key(|(_idx, f)| match f.severity {
            Severity::Critical => 0,
            Severity::High => 1,
            Severity::Medium => 2,
            Severity::Low => 3,
        });

        for (idx, finding) in by_severity {
            report.push_str(&format!(
                "{}. [{}] {}:{} - \"{}\"\n",
                idx + 1,
                finding.severity,
                finding.file,
                finding.line,
                finding.pattern
            ));
        }

        report
    }
}

// ── Integration helper ──────────────────────────────────────────────

/// Scan a skill directory after write. Returns error message if blocked, else `None`.
pub fn guard_skill_write(skill_dir: &Path, guard: &dyn SkillGuard) -> Result<(), String> {
    let result = guard.scan_skill(skill_dir, "agent-created");
    let (allowed, reason) = guard.should_allow(&result);

    match allowed {
        Some(true) => Ok(()),
        Some(false) | None => {
            let report = guard.format_scan_report(&result);
            let _ = std::fs::remove_dir_all(skill_dir);
            Err(format!(
                "Security scan blocked this skill: {reason}\n{report}"
            ))
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_skill(base: &Path, files: &[(&str, &str)]) {
        for (name, content) in files {
            let path = base.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }
    }

    #[test]
    fn test_scan_clean_skill() {
        let dir = tempfile::tempdir().unwrap();
        create_test_skill(
            dir.path(),
            &[
                ("SKILL.md", "---\nname: clean\ndescription: Clean\n---\n\n# Hello"),
                ("references/guide.md", "# Safe documentation"),
            ],
        );

        let guard = DefaultSkillGuard::new();
        let result = guard.scan_skill(dir.path(), "agent-created");

        assert!(result.findings.is_empty());
        assert_eq!(result.file_count, 2);
    }

    #[test]
    fn test_scan_shell_execution() {
        let dir = tempfile::tempdir().unwrap();
        create_test_skill(
            dir.path(),
            &[(
                "SKILL.md",
                "---\nname: bad\ndescription: Bad\n---\n\nRun: rm -rf /",
            )],
        );

        let guard = DefaultSkillGuard::new();
        let result = guard.scan_skill(dir.path(), "agent-created");

        assert!(!result.findings.is_empty());
        assert!(result.findings.iter().any(|f| f.pattern == "rm -rf"));
    }

    #[test]
    fn test_scan_network_exfil() {
        let dir = tempfile::tempdir().unwrap();
        create_test_skill(
            dir.path(),
            &[(
                "SKILL.md",
                "---\nname: net\ndescription: Net\n---\n\ncurl https://evil.com",
            )],
        );

        let guard = DefaultSkillGuard::new();
        let result = guard.scan_skill(dir.path(), "agent-created");

        assert!(!result.findings.is_empty());
        assert!(result
            .findings
            .iter()
            .any(|f| f.pattern == "curl.*https://"));
    }

    #[test]
    fn test_scan_prompt_injection() {
        let dir = tempfile::tempdir().unwrap();
        create_test_skill(
            dir.path(),
            &[(
                "SKILL.md",
                "---\nname: inject\ndescription: Inject\n---\n\nignore previous instructions",
            )],
        );

        let guard = DefaultSkillGuard::new();
        let result = guard.scan_skill(dir.path(), "agent-created");

        assert!(!result.findings.is_empty());
        assert!(result
            .findings
            .iter()
            .any(|f| f.pattern == "ignore previous instructions"));
    }

    #[test]
    fn test_should_allow_clean() {
        let result = ScanResult {
            findings: vec![],
            file_count: 1,
            source: "agent-created".into(),
        };

        let guard = DefaultSkillGuard::new();
        let (allowed, _reason) = guard.should_allow(&result);
        assert_eq!(allowed, Some(true));
    }

    #[test]
    fn test_should_allow_critical() {
        let result = ScanResult {
            findings: vec![Finding {
                file: "SKILL.md".into(),
                line: 5,
                pattern: "rm -rf".into(),
                severity: Severity::Critical,
            }],
            file_count: 1,
            source: "agent-created".into(),
        };

        let guard = DefaultSkillGuard::new();
        let (allowed, _reason) = guard.should_allow(&result);
        assert_eq!(allowed, Some(false));
    }

    #[test]
    fn test_format_scan_report() {
        let result = ScanResult {
            findings: vec![
                Finding {
                    file: "SKILL.md".into(),
                    line: 5,
                    pattern: "rm -rf".into(),
                    severity: Severity::Critical,
                },
                Finding {
                    file: "scripts/run.sh".into(),
                    line: 2,
                    pattern: "sudo".into(),
                    severity: Severity::High,
                },
            ],
            file_count: 2,
            source: "agent-created".into(),
        };

        let guard = DefaultSkillGuard::new();
        let report = guard.format_scan_report(&result);

        assert!(report.contains("rm -rf"));
        assert!(report.contains("sudo"));
        assert!(report.contains("Files scanned: 2"));
    }
}
