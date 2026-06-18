//! voltron-audit — AuditSink implementations.
//!
//! Provides two backends:
//! - `InMemoryAuditSink` — `Vec<AuditEntry>` behind a `Mutex`, suitable for testing
//! - `FileAuditSink` — append-only JSONL file, flushed on each write
//!
//! Both implementations are **synchronous** per the `AuditSink` trait contract,
//! so callers must not block the async runtime around `append()` calls.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use voltron_core::{AuditEntry, AuditSink, VoltronError};

// ── InMemoryAuditSink ─────────────────────────────────────────────

/// An append-only in-memory audit trail backed by a `Vec<AuditEntry>`.
///
/// All entries are kept in memory behind a `std::sync::Mutex`. Useful for
/// testing and low-resource environments where persistence is not required.
pub struct InMemoryAuditSink {
    entries: Mutex<Vec<AuditEntry>>,
}

impl InMemoryAuditSink {
    /// Create a new empty in-memory audit sink.
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    /// Return all entries recorded so far (for inspection in tests).
    pub fn all_entries(&self) -> Vec<AuditEntry> {
        self.entries.lock().unwrap().clone()
    }

    /// Return the number of entries recorded so far.
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    /// Returns `true` if no entries have been recorded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for InMemoryAuditSink {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditSink for InMemoryAuditSink {
    fn append(&self, entry: AuditEntry) -> Result<(), VoltronError> {
        self.entries
            .lock()
            .map_err(|e| VoltronError::Internal(format!("audit mutex poisoned: {e}")))?
            .push(entry);
        Ok(())
    }
}

// ── FileAuditSink ─────────────────────────────────────────────────

/// An append-only JSONL (JSON Lines) audit trail.
///
/// Each `AuditEntry` is serialised to a single line of JSON and appended
/// to the file. The file is opened in append mode and flushed after every
/// write to minimise data loss on crash.
///
/// # Locking
///
/// Internal `Mutex` guards the file handle, so this sink is `Send + Sync`.
/// The trait contract requires `append()` to be synchronous — the lock
/// is held only for the duration of the serialise + write + flush cycle.
pub struct FileAuditSink {
    file: Mutex<File>,
    _path: PathBuf,
}

impl FileAuditSink {
    /// Open (or create) a JSONL file at the given path for append-only audit.
    ///
    /// If the file already exists, new entries are appended after existing data.
    /// The parent directory is created if it does not exist.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, VoltronError> {
        let path: PathBuf = path.into();

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| VoltronError::AuditPersistence(format!("mkdir failed: {e}")))?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| VoltronError::AuditPersistence(format!("open failed: {e}")))?;

        Ok(Self {
            file: Mutex::new(file),
            _path: path,
        })
    }
}

impl AuditSink for FileAuditSink {
    fn append(&self, entry: AuditEntry) -> Result<(), VoltronError> {
        let mut guard = self
            .file
            .lock()
            .map_err(|e| VoltronError::Internal(format!("audit file mutex poisoned: {e}")))?;

        let line = serde_json::to_string(&entry)
            .map_err(|e| VoltronError::Serialization(e.to_string()))?;

        writeln!(guard, "{line}")
            .map_err(|e| VoltronError::AuditPersistence(format!("write failed: {e}")))?;

        guard
            .flush()
            .map_err(|e| VoltronError::AuditPersistence(format!("flush failed: {e}")))?;

        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn make_entry(id: &str, event: &str) -> AuditEntry {
        AuditEntry {
            id: id.to_string(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            event: event.to_string(),
            payload: json!({"key": "value"}),
        }
    }

    mod in_memory {
        use super::*;

        #[test]
        fn test_append_and_retrieve() {
            let sink = InMemoryAuditSink::new();
            assert!(sink.is_empty());

            sink.append(make_entry("e1", "llm.call")).unwrap();
            sink.append(make_entry("e2", "skill.execute")).unwrap();

            assert_eq!(sink.len(), 2);
            let all = sink.all_entries();
            assert_eq!(all[0].id, "e1");
            assert_eq!(all[1].event, "skill.execute");
        }

        #[test]
        fn test_empty_new() {
            let sink = InMemoryAuditSink::new();
            assert!(sink.is_empty());
            assert_eq!(sink.len(), 0);
            assert!(sink.all_entries().is_empty());
        }

        #[test]
        fn test_default_is_empty() {
            let sink: InMemoryAuditSink = Default::default();
            assert!(sink.is_empty());
        }
    }

    mod file {
        use super::*;
        use std::io::BufRead;

        #[test]
        fn test_append_and_read_back() {
            let dir = std::env::temp_dir().join("voltron-audit-test");
            let path = dir.join("audit.jsonl");

            // Remove any leftover from previous runs
            let _ = fs::remove_dir_all(&dir);

            let sink = FileAuditSink::new(&path).unwrap();

            sink.append(make_entry("e1", "llm.call")).unwrap();
            sink.append(make_entry("e2", "memory.put")).unwrap();

            // Drop the sink so the file handle is released
            drop(sink);

            // Read back and verify
            let file = fs::File::open(&path).unwrap();
            let reader = std::io::BufReader::new(file);
            let lines: Vec<String> = reader.lines().map(|l| l.unwrap()).collect();

            assert_eq!(lines.len(), 2);
            assert!(lines[0].contains("\"llm.call\""));
            assert!(lines[1].contains("\"memory.put\""));

            // Cleanup
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn test_append_is_json_line() {
            let dir = std::env::temp_dir().join("voltron-audit-test-json");
            let path = dir.join("audit.jsonl");
            let _ = fs::remove_dir_all(&dir);

            let sink = FileAuditSink::new(&path).unwrap();
            let entry = make_entry("e1", "test.event");
            sink.append(entry.clone()).unwrap();
            drop(sink);

            // Read and parse to verify valid JSON
            let content = fs::read_to_string(&path).unwrap();
            let parsed: AuditEntry = serde_json::from_str(content.trim()).unwrap();
            assert_eq!(parsed.id, "e1");
            assert_eq!(parsed.event, "test.event");

            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn test_parent_dir_creation() {
            let dir = std::env::temp_dir()
                .join("voltron-audit-nested")
                .join("deep");
            let path = dir.join("audit.jsonl");

            // Parent dir doesn't exist yet
            let sink = FileAuditSink::new(&path);
            assert!(sink.is_ok());

            // Cleanup
            let _ = fs::remove_dir_all(&std::env::temp_dir().join("voltron-audit-nested"));
        }
    }
}
