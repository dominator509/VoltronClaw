//! High-level memory tree operations.
//!
//! The `MemoryTreeEngine` is the primary public API for creating trees,
//! appending leaves, and triggering time-based flushes.

use anyhow::{Context, Result};
use chrono::Utc;

use crate::flush;
use crate::seal::append_leaf;
use crate::store::TreeStore;
use crate::summarize::Summarizer;
use crate::types::{
    AppendResult, LabelStrategy, LeafChunk, SummaryNode, Tree, TreeKind, TreeStatus,
};

/// The main engine for operating on memory trees.
///
/// Wraps a `TreeStore` and `Summarizer` to provide a simplified API
/// for tree lifecycle management.
pub struct MemoryTreeEngine<S: TreeStore> {
    store: S,
    summarizer: Box<dyn Summarizer>,
}

impl<S: TreeStore> MemoryTreeEngine<S> {
    /// Create a new engine with the given store and summarizer.
    pub fn new(store: S, summarizer: Box<dyn Summarizer>) -> Self {
        Self { store, summarizer }
    }

    /// Create a new tree and persist it.
    pub async fn create_tree(
        &mut self,
        label: &str,
        kind: TreeKind,
        source_id: Option<String>,
    ) -> Result<Tree> {
        let tree = Tree {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            status: TreeStatus::Active,
            root_id: None,
            source_id,
            label: label.to_string(),
            created_at: Utc::now(),
        };

        self.store
            .put_tree(&tree)
            .await
            .context("failed to create tree")?;

        tracing::info!(
            tree_id = %tree.id,
            label = %tree.label,
            kind = %tree.kind.as_str(),
            "Created memory tree"
        );

        Ok(tree)
    }

    /// Append a chunk of text to a tree. Handles cascade sealing automatically.
    pub async fn ingest(
        &mut self,
        tree_id: &str,
        content: &str,
        token_count: u32,
    ) -> Result<AppendResult> {
        let mut tree = self
            .store
            .get_tree(tree_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Tree not found: {}", tree_id))?;

        let strategy = LabelStrategy::for_kind(tree.kind);

        let leaf = LeafChunk {
            id: uuid::Uuid::new_v4().to_string(),
            content: content.to_string(),
            token_count,
            timestamp: Utc::now(),
        };

        let result = append_leaf(
            &mut self.store,
            self.summarizer.as_ref(),
            &mut tree,
            leaf,
            &strategy,
        )
        .await?;

        // Persist tree state (root may have changed)
        self.store.put_tree(&tree).await?;

        Ok(result)
    }

    /// Run time-based flush for stale buffers across all trees.
    pub async fn flush_stale(&mut self, max_age_secs: Option<i64>) -> Result<Vec<SummaryNode>> {
        let age = chrono::Duration::seconds(
            max_age_secs.unwrap_or(crate::types::DEFAULT_FLUSH_AGE_SECS),
        );
        flush::flush_stale_buffers(&mut self.store, self.summarizer.as_ref(), age).await
    }

    /// Get a tree by id.
    pub async fn get_tree(&self, tree_id: &str) -> Result<Option<Tree>> {
        self.store.get_tree(tree_id).await
    }

    /// Get a summary node by id.
    pub async fn get_summary(&self, node_id: &str) -> Result<Option<SummaryNode>> {
        self.store.get_summary(node_id).await
    }

    /// List all tree ids.
    pub async fn list_trees(&self) -> Result<Vec<String>> {
        self.store.list_trees().await
    }

    /// Freeze a tree (no new input accepted).
    pub async fn freeze_tree(&mut self, tree_id: &str) -> Result<()> {
        let mut tree = self
            .store
            .get_tree(tree_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Tree not found: {}", tree_id))?;
        tree.status = TreeStatus::Frozen;
        self.store.put_tree(&tree).await?;
        Ok(())
    }

    /// Archive a tree (kept for reference, no new input).
    pub async fn archive_tree(&mut self, tree_id: &str) -> Result<()> {
        let mut tree = self
            .store
            .get_tree(tree_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Tree not found: {}", tree_id))?;
        tree.status = TreeStatus::Archived;
        self.store.put_tree(&tree).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::InMemoryTreeStore;
    use crate::summarize::ConcatSummarizer;

    #[tokio::test]
    async fn test_create_and_ingest() {
        let store = InMemoryTreeStore::new();
        let summarizer = Box::new(ConcatSummarizer::new(" | "));
        let mut engine = MemoryTreeEngine::new(store, summarizer);

        let tree = engine
            .create_tree("test-source", TreeKind::Source, Some("chat-1".into()))
            .await
            .unwrap();

        assert_eq!(tree.label, "test-source");
        assert_eq!(tree.kind, TreeKind::Source);

        // Ingest a small chunk
        let result = engine.ingest(&tree.id, "hello world", 10).await.unwrap();
        assert!(!result.sealed);

        // Verify tree exists
        let loaded = engine.get_tree(&tree.id).await.unwrap().unwrap();
        assert_eq!(loaded.label, "test-source");
    }

    #[tokio::test]
    async fn test_archive_tree_rejects_ingest() {
        let store = InMemoryTreeStore::new();
        let summarizer = Box::new(ConcatSummarizer::new(" | "));
        let mut engine = MemoryTreeEngine::new(store, summarizer);

        let tree = engine
            .create_tree("to-archive", TreeKind::Source, None)
            .await
            .unwrap();

        engine.archive_tree(&tree.id).await.unwrap();

        let archived = engine.get_tree(&tree.id).await.unwrap().unwrap();
        assert_eq!(archived.status, TreeStatus::Archived);

        // Archived trees reject new input
        let result = engine.ingest(&tree.id, "late arrival", 5).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_trees() {
        let store = InMemoryTreeStore::new();
        let summarizer = Box::new(ConcatSummarizer::new(" | "));
        let mut engine = MemoryTreeEngine::new(store, summarizer);

        engine.create_tree("alpha", TreeKind::Source, None).await.unwrap();
        engine.create_tree("beta", TreeKind::Global, None).await.unwrap();

        let ids = engine.list_trees().await.unwrap();
        assert_eq!(ids.len(), 2);
    }

    #[tokio::test]
    async fn test_get_nonexistent_tree() {
        let store = InMemoryTreeStore::new();
        let summarizer = Box::new(ConcatSummarizer::new(" | "));
        let engine = MemoryTreeEngine::new(store, summarizer);

        let result = engine.get_tree("bogus").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_global_tree_empty_labels() {
        let store = InMemoryTreeStore::new();
        let summarizer = Box::new(ConcatSummarizer::new(" | "));
        let mut engine = MemoryTreeEngine::new(store, summarizer);

        let tree = engine
            .create_tree("global-root", TreeKind::Global, None)
            .await
            .unwrap();

        // Global trees use UnionFromChildren. With no children, summary should have empty labels.
        let content = &"x".repeat(crate::INPUT_TOKEN_BUDGET as usize * 4 + 10);
        let result = engine.ingest(&tree.id, content, crate::INPUT_TOKEN_BUDGET + 1)
            .await
            .unwrap();

        assert!(result.sealed);
        if let Some(summary) = &result.new_summaries.first() {
            // Global label strategy unions children; no pre-existing children = empty
            assert!(summary.entities.is_empty());
            assert!(summary.topics.is_empty());
        }
    }

    #[tokio::test]
    async fn test_topic_tree_empty_labels() {
        let store = InMemoryTreeStore::new();
        let summarizer = Box::new(ConcatSummarizer::new(" | "));
        let mut engine = MemoryTreeEngine::new(store, summarizer);

        let tree = engine
            .create_tree("topic-security", TreeKind::Topic, None)
            .await
            .unwrap();

        let content = &"x".repeat(crate::INPUT_TOKEN_BUDGET as usize * 4 + 10);
        let result = engine.ingest(&tree.id, content, crate::INPUT_TOKEN_BUDGET + 1)
            .await
            .unwrap();

        assert!(result.sealed);
        if let Some(summary) = &result.new_summaries.first() {
            // Topic trees use Empty strategy — always empty labels
            assert!(summary.entities.is_empty());
            assert!(summary.topics.is_empty());
        }
    }

    #[tokio::test]
    async fn test_ingest_updates_root_id() {
        let store = InMemoryTreeStore::new();
        let summarizer = Box::new(ConcatSummarizer::new(" | "));
        let mut engine = MemoryTreeEngine::new(store, summarizer);

        let tree = engine
            .create_tree("root-test", TreeKind::Source, None)
            .await
            .unwrap();

        assert!(tree.root_id.is_none());

        // Ingest enough to seal
        let content = &"x".repeat(crate::INPUT_TOKEN_BUDGET as usize * 4 + 10);
        engine.ingest(&tree.id, content, crate::INPUT_TOKEN_BUDGET + 1)
            .await
            .unwrap();

        // After seal, root_id should be set to the new L1 summary
        let updated = engine.get_tree(&tree.id).await.unwrap().unwrap();
        assert!(updated.root_id.is_some(), "root_id should be set after first seal");
    }
}
