//! Time-based buffer flushing.
//!
//! For low-volume sources that never hit the token or sibling-count
//! thresholds, time-based sealing ensures summaries are still produced.
//!
//! `flush_stale_buffers` iterates all buffers across all trees and
//! force-seals any L0 buffer whose oldest item is older than `max_age`.

use anyhow::{Context, Result};
use chrono::{Duration, Utc};

use crate::seal::append_leaf;
use crate::store::TreeStore;
use crate::summarize::Summarizer;
use crate::types::{LabelStrategy, LeafChunk, SummaryNode, TreeStatus, DEFAULT_FLUSH_AGE_SECS};

/// Force-seal any L0 buffer whose oldest item is older than `max_age`.
///
/// Returns the list of newly-created summary nodes from all seals.
///
/// This function does NOT cascade — it only targets stale L0 buffers.
/// Sources that accumulate items slowly will produce L1 summaries
/// via this mechanism; cascading from L1 upward is handled by the
/// normal append_leaf cascade logic when sibling counts reach SUMMARY_FANOUT.
pub async fn flush_stale_buffers<S: TreeStore>(
    store: &mut S,
    summarizer: &dyn Summarizer,
    max_age: Duration,
) -> Result<Vec<SummaryNode>> {
    let buffers = store
        .list_buffers()
        .await
        .context("failed to list buffers")?;

    let now = Utc::now();
    let threshold = now - max_age;
    let mut new_summaries = Vec::new();

    for buffer in &buffers {
        // Only flush L0 buffers
        if buffer.level != 0 {
            continue;
        }

        // Check staleness
        let is_stale = match buffer.oldest_timestamp {
            Some(ts) => ts < threshold,
            None => false,
        };

        if !is_stale || buffer.item_ids.is_empty() {
            continue;
        }

        // Get the tree
        let mut tree = match store.get_tree(&buffer.tree_id).await? {
            Some(t) => t,
            None => {
                tracing::warn!(
                    tree_id = %buffer.tree_id,
                    "Buffer exists but tree not found — skipping"
                );
                continue;
            }
        };

        if tree.status != TreeStatus::Active {
            continue;
        }

        tracing::info!(
            tree_id = %tree.id,
            tree_label = %tree.label,
            item_count = buffer.item_ids.len(),
            token_sum = buffer.token_sum,
            age_secs = buffer.oldest_timestamp
                .map(|ts| (now - ts).num_seconds())
                .unwrap_or(-1),
            "Flushing stale L0 buffer"
        );

        let strategy = LabelStrategy::for_kind(tree.kind);

        // Force-seal by creating a dummy leaf that triggers the L0 gate
        // We append a sentinel chunk with enough tokens to cross INPUT_TOKEN_BUDGET
        let sentinel_id = uuid::Uuid::new_v4().to_string();
        let sentinel = LeafChunk {
            id: sentinel_id,
            content: "(time-based flush sentinel)".to_string(),
            // Use remaining token budget to trigger seal
            token_count: crate::types::INPUT_TOKEN_BUDGET
                .saturating_sub(buffer.token_sum),
            timestamp: now,
        };

        let result = append_leaf(
            store,
            summarizer,
            &mut tree,
            sentinel,
            &strategy,
        )
        .await
        .context("flush append_leaf failed")?;

        new_summaries.extend(result.new_summaries);

        // Persist updated tree state
        store
            .put_tree(&tree)
            .await
            .context("failed to persist tree after flush")?;
    }

    Ok(new_summaries)
}

/// Convenience wrapper that uses the default flush age (7 days).
pub async fn flush_stale_buffers_default<S: TreeStore>(
    store: &mut S,
    summarizer: &dyn Summarizer,
) -> Result<Vec<SummaryNode>> {
    flush_stale_buffers(store, summarizer, Duration::seconds(DEFAULT_FLUSH_AGE_SECS)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::InMemoryTreeStore;
    use crate::types::{LeafChunk, Tree, TreeKind, TreeStatus};
    use crate::Buffer;

    struct MockSummarizer;
    #[async_trait::async_trait]
    impl Summarizer for MockSummarizer {
        async fn summarize(&self, contents: &[String], _input_tokens: usize) -> Result<String> {
            Ok(contents.join(" | "))
        }
    }

    #[tokio::test]
    async fn test_flush_stale_buffer_triggers_seal() {
        let mut store = InMemoryTreeStore::new();
        let summarizer = MockSummarizer;
        let tree = Tree {
            id: "stale-tree".to_string(),
            kind: TreeKind::Source,
            status: TreeStatus::Active,
            root_id: None,
            source_id: None,
            label: "stale-test".to_string(),
            created_at: Utc::now(),
        };
        store.put_tree(&tree).await.unwrap();

        // Add a leaf with an old timestamp directly to the buffer
        let old_ts = Utc::now() - Duration::seconds(DEFAULT_FLUSH_AGE_SECS + 1);
        let leaf = LeafChunk {
            id: "old-leaf".to_string(),
            content: "old content".to_string(),
            token_count: 100,
            timestamp: old_ts,
        };
        store.put_leaf(&leaf).await.unwrap();

        let buffer = Buffer {
            tree_id: "stale-tree".to_string(),
            level: 0,
            parent_id: None,
            item_ids: vec!["old-leaf".to_string()],
            token_sum: 100,
            oldest_timestamp: Some(old_ts),
        };
        store.put_buffer(&buffer).await.unwrap();

        // Flush with a very short max age to ensure it triggers
        let results = flush_stale_buffers(
            &mut store,
            &summarizer,
            Duration::seconds(1), // 1 second → everything is stale
        )
        .await
        .unwrap();

        assert!(!results.is_empty(), "should have created a summary from stale buffer");
    }

    #[tokio::test]
    async fn test_flush_ignores_non_stale_buffers() {
        let mut store = InMemoryTreeStore::new();
        let summarizer = MockSummarizer;

        let tree = Tree {
            id: "fresh-tree".to_string(),
            kind: TreeKind::Source,
            status: TreeStatus::Active,
            root_id: None,
            source_id: None,
            label: "fresh-test".to_string(),
            created_at: Utc::now(),
        };
        store.put_tree(&tree).await.unwrap();

        let buffer = Buffer {
            tree_id: "fresh-tree".to_string(),
            level: 0,
            parent_id: None,
            item_ids: vec!["recent-leaf".to_string()],
            token_sum: 50,
            oldest_timestamp: Some(Utc::now()),
        };
        store.put_buffer(&buffer).await.unwrap();

        // Flush with max_age = 7 days — should NOT seal
        let results = flush_stale_buffers_default(&mut store, &summarizer)
            .await
            .unwrap();

        assert!(results.is_empty(), "should not seal non-stale buffer");
    }
}
