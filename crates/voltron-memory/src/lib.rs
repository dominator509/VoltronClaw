//! voltron-memory — MemoryStore implementations.
//!
//! Provides two backends:
//! - `InMemoryStore` — HashMap-backed, suitable for testing and development
//! - `SqliteStore` — persistent SQLite backend via sqlx
//!
//! # TODO: encrypted-memory phase
//! Future work will add an encrypted composite using SQLCipher + AES-256-GCM + Argon2id.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use voltron_core::{MemoryRecord, MemoryStore, VoltronError};

// ── InMemoryStore ─────────────────────────────────────────────────

/// Thread-safe HashMap-backed in-memory memory store.
///
/// All records are stored in a single `HashMap<String, MemoryRecord>`.
/// Intended for testing and low-stakes development.
pub struct InMemoryStore {
    data: Arc<RwLock<HashMap<String, MemoryRecord>>>,
}

impl InMemoryStore {
    /// Create a new empty in-memory store.
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MemoryStore for InMemoryStore {
    async fn put(&self, record: MemoryRecord) -> Result<(), VoltronError> {
        let mut data = self.data.write().await;
        data.insert(record.id.clone(), record);
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<MemoryRecord>, VoltronError> {
        let data = self.data.read().await;
        Ok(data.get(id).cloned())
    }

    async fn search(&self, tags: &[String]) -> Result<Vec<MemoryRecord>, VoltronError> {
        let data = self.data.read().await;
        let mut results: Vec<MemoryRecord> = data
            .values()
            .filter(|rec| {
                // AND semantics: record must contain ALL requested tags
                tags.iter().all(|tag| rec.tags.contains(tag))
            })
            .cloned()
            .collect();
        // Order by updated_at descending
        results.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(results)
    }

    async fn delete(&self, id: &str) -> Result<(), VoltronError> {
        let mut data = self.data.write().await;
        data.remove(id);
        Ok(())
    }
}

// ── SqliteStore ──────────────────────────────────────────────────

const SQLITE_INIT_SQL: &str = "
CREATE TABLE IF NOT EXISTS memory_records (
    id          TEXT PRIMARY KEY,
    content     TEXT NOT NULL,
    tags_json   TEXT NOT NULL DEFAULT '[]',
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_memory_updated_at ON memory_records(updated_at DESC);
";

/// A persistent MemoryStore backed by SQLite (via sqlx).
///
/// Requires a connection pool to an existing SQLite database.
/// Schemas are automatically created on first use.
///
/// # TODO: encrypted-memory phase
/// Future work will add transparent encryption of stored records.
pub struct SqliteStore {
    pool: sqlx::SqlitePool,
}

impl SqliteStore {
    /// Open or create a SQLite database at the given path and run migrations.
    pub async fn connect(path: &str) -> Result<Self, VoltronError> {
        let pool = sqlx::SqlitePool::connect(path).await.map_err(|e| {
            VoltronError::MemoryStorage(format!("Failed to connect to SQLite: {e}"))
        })?;

        // Run the schema init
        sqlx::query(SQLITE_INIT_SQL)
            .execute(&pool)
            .await
            .map_err(|e| VoltronError::MemoryStorage(format!("Failed to init schema: {e}")))?;

        Ok(Self { pool })
    }

    /// Create an in-memory SQLite store (useful for testing).
    pub async fn in_memory() -> Result<Self, VoltronError> {
        Self::connect(":memory:").await
    }
}

#[async_trait]
impl MemoryStore for SqliteStore {
    async fn put(&self, record: MemoryRecord) -> Result<(), VoltronError> {
        let tags_json = serde_json::to_string(&record.tags)
            .map_err(|e| VoltronError::Serialization(e.to_string()))?;
        let metadata_json = serde_json::to_string(&record.metadata)
            .map_err(|e| VoltronError::Serialization(e.to_string()))?;

        sqlx::query(
            "INSERT INTO memory_records (id, content, tags_json, created_at, updated_at, metadata_json)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 content      = excluded.content,
                 tags_json    = excluded.tags_json,
                 updated_at   = excluded.updated_at,
                 metadata_json = excluded.metadata_json",
        )
        .bind(&record.id)
        .bind(&record.content)
        .bind(&tags_json)
        .bind(&record.created_at)
        .bind(&record.updated_at)
        .bind(&metadata_json)
        .execute(&self.pool)
        .await
        .map_err(|e| VoltronError::MemoryStorage(format!("sqlite put failed: {e}")))?;

        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<MemoryRecord>, VoltronError> {
        let row = sqlx::query_as::<_, SqliteRow>("SELECT id, content, tags_json, created_at, updated_at, metadata_json FROM memory_records WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| VoltronError::MemoryStorage(format!("sqlite get failed: {e}")))?;

        match row {
            Some(r) => r.try_into().map(Some),
            None => Ok(None),
        }
    }

    async fn search(&self, tags: &[String]) -> Result<Vec<MemoryRecord>, VoltronError> {
        // For SQLite, we fetch all records and filter in Rust for tag AND semantics.
        // In production this would use a proper tag index; for Phase 1 this is sufficient.
        let rows = sqlx::query_as::<_, SqliteRow>(
            "SELECT id, content, tags_json, created_at, updated_at, metadata_json FROM memory_records ORDER BY updated_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| VoltronError::MemoryStorage(format!("sqlite search failed: {e}")))?;

        let mut results: Vec<MemoryRecord> = Vec::new();
        for row in rows {
            let rec: MemoryRecord = row.try_into()?;
            if tags.is_empty() || tags.iter().all(|tag| rec.tags.contains(tag)) {
                results.push(rec);
            }
        }
        Ok(results)
    }

    async fn delete(&self, id: &str) -> Result<(), VoltronError> {
        sqlx::query("DELETE FROM memory_records WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| VoltronError::MemoryStorage(format!("sqlite delete failed: {e}")))?;
        Ok(())
    }
}

// ── SQLite row type ───────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct SqliteRow {
    id: String,
    content: String,
    tags_json: String,
    created_at: String,
    updated_at: String,
    metadata_json: String,
}

impl TryFrom<SqliteRow> for MemoryRecord {
    type Error = VoltronError;

    fn try_from(row: SqliteRow) -> Result<Self, Self::Error> {
        let tags: Vec<String> = serde_json::from_str(&row.tags_json)
            .map_err(|e| VoltronError::Serialization(e.to_string()))?;
        let metadata: std::collections::HashMap<String, String> =
            serde_json::from_str(&row.metadata_json)
                .map_err(|e| VoltronError::Serialization(e.to_string()))?;

        Ok(MemoryRecord {
            id: row.id,
            content: row.content,
            tags,
            created_at: row.created_at,
            updated_at: row.updated_at,
            metadata,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_record(id: &str, content: &str, tags: Vec<&str>) -> MemoryRecord {
        MemoryRecord {
            id: id.to_string(),
            content: content.to_string(),
            tags: tags.into_iter().map(String::from).collect(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            metadata: HashMap::new(),
        }
    }

    mod in_memory {
        use super::*;

        #[tokio::test]
        async fn test_put_get_roundtrip() {
            let store = InMemoryStore::new();
            let rec = make_record("r1", "Hello, world!", vec!["greeting"]);
            store.put(rec.clone()).await.unwrap();

            let got = store.get("r1").await.unwrap();
            assert!(got.is_some());
            assert_eq!(got.unwrap().content, "Hello, world!");
        }

        #[tokio::test]
        async fn test_get_nonexistent() {
            let store = InMemoryStore::new();
            let got = store.get("missing").await.unwrap();
            assert!(got.is_none());
        }

        #[tokio::test]
        async fn test_search_by_tag() {
            let store = InMemoryStore::new();
            store
                .put(make_record("a", "Alpha", vec!["urgent", "work"]))
                .await
                .unwrap();
            store
                .put(make_record("b", "Beta", vec!["work"]))
                .await
                .unwrap();
            store
                .put(make_record("c", "Gamma", vec!["personal"]))
                .await
                .unwrap();

            // Search for "urgent" → should return only "a"
            let results = store.search(&["urgent".into()]).await.unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].id, "a");

            // Search for "work" → should return a, b
            let results = store.search(&["work".into()]).await.unwrap();
            assert_eq!(results.len(), 2);
        }

        #[tokio::test]
        async fn test_search_and_semantics() {
            let store = InMemoryStore::new();
            store
                .put(make_record("a", "Both tags", vec!["urgent", "work"]))
                .await
                .unwrap();
            store
                .put(make_record("b", "Only urgent", vec!["urgent"]))
                .await
                .unwrap();

            // AND semantics: both "urgent" AND "work"
            let results = store
                .search(&["urgent".into(), "work".into()])
                .await
                .unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].id, "a");
        }

        #[tokio::test]
        async fn test_delete() {
            let store = InMemoryStore::new();
            store
                .put(make_record("r1", "To delete", vec![]))
                .await
                .unwrap();
            assert!(store.get("r1").await.unwrap().is_some());

            store.delete("r1").await.unwrap();
            assert!(store.get("r1").await.unwrap().is_none());
        }

        #[tokio::test]
        async fn test_delete_nonexistent_is_noop() {
            let store = InMemoryStore::new();
            // Should not error
            store.delete("does-not-exist").await.unwrap();
        }
    }

    mod sqlite {
        use super::*;

        #[tokio::test]
        async fn test_put_get_roundtrip() {
            let store = SqliteStore::in_memory().await.unwrap();
            let rec = make_record("r1", "SQLite test", vec!["db"]);
            store.put(rec.clone()).await.unwrap();

            let got = store.get("r1").await.unwrap();
            assert!(got.is_some());
            assert_eq!(got.unwrap().content, "SQLite test");
        }

        #[tokio::test]
        async fn test_get_nonexistent() {
            let store = SqliteStore::in_memory().await.unwrap();
            let got = store.get("missing").await.unwrap();
            assert!(got.is_none());
        }

        #[tokio::test]
        async fn test_search_by_tag() {
            let store = SqliteStore::in_memory().await.unwrap();
            store
                .put(make_record("a", "Alpha", vec!["urgent", "work"]))
                .await
                .unwrap();
            store
                .put(make_record("b", "Beta", vec!["work"]))
                .await
                .unwrap();
            store
                .put(make_record("c", "Gamma", vec!["personal"]))
                .await
                .unwrap();

            let results = store.search(&["urgent".into()]).await.unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].id, "a");
        }

        #[tokio::test]
        async fn test_search_and_semantics() {
            let store = SqliteStore::in_memory().await.unwrap();
            store
                .put(make_record("a", "Both", vec!["urgent", "work"]))
                .await
                .unwrap();
            store
                .put(make_record("b", "Only urgent", vec!["urgent"]))
                .await
                .unwrap();

            let results = store
                .search(&["urgent".into(), "work".into()])
                .await
                .unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].id, "a");
        }

        #[tokio::test]
        async fn test_delete() {
            let store = SqliteStore::in_memory().await.unwrap();
            store
                .put(make_record("r1", "To delete", vec![]))
                .await
                .unwrap();
            assert!(store.get("r1").await.unwrap().is_some());

            store.delete("r1").await.unwrap();
            assert!(store.get("r1").await.unwrap().is_none());
        }

        #[tokio::test]
        async fn test_update_existing_record() {
            let store = SqliteStore::in_memory().await.unwrap();
            store
                .put(make_record("r1", "Original", vec![]))
                .await
                .unwrap();
            store
                .put(MemoryRecord {
                    content: "Updated".into(),
                    ..make_record("r1", "Updated", vec!["changed"])
                })
                .await
                .unwrap();

            let got = store.get("r1").await.unwrap().unwrap();
            assert_eq!(got.content, "Updated");
            assert!(got.tags.contains(&"changed".to_string()));
        }
    }
}
