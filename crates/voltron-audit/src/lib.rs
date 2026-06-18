//! voltron-audit — AuditSink implementations.
//!
//! Provides two backends:
//! - `InMemoryAuditSink` — Vec-backed, suitable for testing
//! - `FileAuditSink` — JSONL file-backed for persistent audit trails
//!
//! # TODO: HMAC chain
//! Future work will add append-only HMAC-chained immutability for production audit.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use voltron_core::{AuditEntry, AuditSink, VoltronError};

// ── InMemoryAuditSink ─────────────────────────────────────────────

/// Thread-safe in-memory audit sink backed by a `Vec<AuditEntry>`.
///
/// All entries are stored in memory. Useful for testing and environments
/// where audit persistence is not required.
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

    /// Return all entries recorded so far.
    pub fn all(&self) -> Vec<AuditEntry> {
        self.entries.lock().unwrap().clone()
    }

    /// Clear all entries.
    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }

    /// Return the number of entries recorded.
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    /// Returns true if no entries have been recorded.
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
            .map_err(|e| VoltronError::AuditPersistence(e.to_string()))?
            .push(entry);
        Ok(())
    }
}

// ── FileAuditSink ─────────────────────────────────────────────────

/// An `AuditSink` that appends JSON lines to a file.
///
/// Each entry is serialized as a single JSON line and appended to the
/// configured file path. Creates the file if it does not exist.
///
/// # TODO: HMAC chain
/// Future work will add an HMAC chain to detect tampering.
pub struct FileAuditSink {
    path: String,
    /// Mutex guards the file handle for thread-safe appending.
    file: Mutex<std::fs::File>,
}

impl FileAuditSink {
    /// Open or create a JSONL audit file at the given path.
    pub fn new(path: impl Into<String>) -> Result<Self, VoltronError> {
        let path = path.into();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| VoltronError::AuditPersistence(format!("Failed to open {path}: {e}")))?;

        Ok(Self {
            path,
            file: Mutex::new(file),
        })
    }

    /// The file path this sink writes to.
    pub fn path(&self) -> &str {
        &self.path
    }
}

impl AuditSink for FileAuditSink {
    fn append(&self, entry: AuditEntry) -> Result<(), VoltronError> {
        let mut file = self
            .file
            .lock()
            .map_err(|e| VoltronError::AuditPersistence(e.to_string()))?;

        let line = serde_json::to_string(&entry)
            .map_err(|e| VoltronError::Serialization(e.to_string()))?;

        writeln!(file, "{line}")
            .map_err(|e| VoltronError::AuditPersistence(format!("Write failed: {e}")))?;

        file.flush()
            .map_err(|e| VoltronError::AuditPersistence(format!("Flush failed: {e}")))?;

        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(id: &str, event: &str) -> AuditEntry {
        AuditEntry {
            id: id.to_string(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            event: event.to_string(),
            payload: serde_json::json!({"detail": "test"}),
        }
    }

    mod in_memory {
        use super::*;

        #[test]
        fn test_append_and_read_back() {
            let sink = InMemoryAuditSink::new();
            let entry = make_entry("e1", "test.event");
            sink.append(entry).unwrap();

            let all = sink.all();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].id, "e1");
            assert_eq!(all[0].event, "test.event");
        }

        #[test]
        fn test_multiple_entries() {
            let sink = InMemoryAuditSink::new();
            sink.append(make_entry("e1", "first")).unwrap();
            sink.append(make_entry("e2", "second")).unwrap();
            sink.append(make_entry("e3", "third")).unwrap();

            assert_eq!(sink.len(), 3);
            let all = sink.all();
            assert_eq!(all[0].id, "e1");
            assert_eq!(all[2].id, "e3");
        }

        #[test]
        fn test_clear() {
            let sink = InMemoryAuditSink::new();
            sink.append(make_entry("e1", "test")).unwrap();
            assert!(!sink.is_empty());
            sink.clear();
            assert!(sink.is_empty());
        }
    }

    mod file {
        use super::*;
        use std::io::BufRead;

        #[test]
        fn test_append_to_file() {
            let dir = std::env::temp_dir();
            let path = dir.join(format!("voltron-audit-test-{}.jsonl", std::process::id()));
            let _ = std::fs::remove_file(&path); // clean up from previous runs

            let sink = FileAuditSink::new(path.to_str().unwrap()).unwrap();
            sink.append(make_entry("e1", "test.event")).unwrap();
            sink.append(make_entry("e2", "another.event")).unwrap();

            // Read back and verify
            let file = std::fs::File::open(&path).unwrap();
            let reader = std::io::BufReader::new(file);
            let lines: Vec<String> = reader.lines().map(|l| l.unwrap()).collect();
            assert_eq!(lines.len(), 2);

            let parsed: AuditEntry = serde_json::from_str(&lines[0]).unwrap();
            assert_eq!(parsed.id, "e1");
            assert_eq!(parsed.event, "test.event");

            let parsed: AuditEntry = serde_json::from_str(&lines[1]).unwrap();
            assert_eq!(parsed.id, "e2");

            // Clean up
            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_file_created_automatically() {
            let dir = std::env::temp_dir();
            let path = dir.join(format!("voltron-audit-auto-{}.jsonl", std::process::id()));
            let _ = std::fs::remove_file(&path);

            assert!(!path.exists());
            let sink = FileAuditSink::new(path.to_str().unwrap()).unwrap();
            sink.append(make_entry("e1", "event")).unwrap();
            assert!(path.exists());

            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_append_preserves_existing_content() {
            let dir = std::env::temp_dir();
            let path = dir.join(format!("voltron-audit-append-{}.jsonl", std::process::id()));
            let _ = std::fs::remove_file(&path);

            // First sink writes one entry
            {
                let sink = FileAuditSink::new(path.to_str().unwrap()).unwrap();
                sink.append(make_entry("e1", "first")).unwrap();
            }
            // Drop the first sink, second sink appends
            let sink = FileAuditSink::new(path.to_str().unwrap()).unwrap();
            sink.append(make_entry("e2", "second")).unwrap();

            let file = std::fs::File::open(&path).unwrap();
            let reader = std::io::BufReader::new(file);
            let lines: Vec<String> = reader.lines().map(|l| l.unwrap()).collect();
            assert_eq!(lines.len(), 2);

            let _ = std::fs::remove_file(&path);
        }
    }
}
