//! Bucket-seal cascade algorithm — the core summarization engine.
//!
//! ## Algorithm
//!
//! `append_leaf` pushes a chunk into the L0 buffer. After the append,
//! a cascade check runs bottom-up:
//!
//! 1. **L0 (leaves → L1)**: seal when `token_sum >= INPUT_TOKEN_BUDGET`.
//!    Token-only gating allows small items (e.g., ~20 token commit messages)
//!    to accumulate into large, meaningful batches.
//!
//! 2. **L≥1 (summaries → next level)**: seal when `item_ids.len() >= SUMMARY_FANOUT`.
//!    Counting siblings keeps the tree's fan-in stable regardless of
//!    summarizer quality.
//!
//! 3. **Cascade**: when a buffer seals, its items become `child_ids` of a
//!    new summary node at level+1. The buffer clears, the new summary id
//!    is queued at the next level, and the check repeats upward until
//!    a buffer fails its gate.
//!
//! 4. **Root update**: after each successful cascade, if the tree's root
//!    has changed (new highest-level node), `tree.root_id` is updated.
//!
//! ## Concurrency
//!
//! All writes in one seal step execute within a single atomic operation
//! per the store trait. Callers should serialize `append_leaf` per tree.
//!
//! ## Safety
//!
//! `MAX_CASCADE_DEPTH` prevents runaway recursion if token accounting
//! drifts. Each level reduces item count by at least `SUMMARY_FANOUT`×,
//! so the depth bound is effectively unreachable.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use chrono::Utc;

use crate::types::{
    AppendResult, Buffer, LabelStrategy, LeafChunk, SummaryNode, Tree, TreeStatus,
    INPUT_TOKEN_BUDGET, MAX_CASCADE_DEPTH, SUMMARY_FANOUT,
};
use crate::store::TreeStore;
use crate::summarize::Summarizer;

/// Append a leaf chunk to a tree and cascade-seal upward.
///
/// Returns an `AppendResult` describing the new summaries created and
/// whether any sealing occurred.
pub async fn append_leaf<S: TreeStore>(
    store: &mut S,
    summarizer: &dyn Summarizer,
    tree: &mut Tree,
    leaf: LeafChunk,
    label_strategy: &LabelStrategy,
) -> Result<AppendResult> {
    // Guard: tree must be active
    if tree.status != TreeStatus::Active {
        anyhow::bail!(
            "Cannot append to tree '{}' with status {:?}",
            tree.label,
            tree.status
        );
    }

    let leaf_id = leaf.id.clone();
    let leaf_tokens = leaf.token_count;
    let leaf_ts = leaf.timestamp;

    // Store the leaf chunk
    store
        .put_leaf(&leaf)
        .await
        .context("failed to store leaf chunk")?;

    let mut new_summaries = Vec::new();

    // Get or create L0 buffer
    let mut l0_buffer = store
        .get_buffer(&tree.id, 0)
        .await
        .context("failed to get L0 buffer")?
        .unwrap_or_else(|| Buffer {
            tree_id: tree.id.clone(),
            level: 0,
            parent_id: tree.root_id.clone(),
            item_ids: Vec::new(),
            token_sum: 0,
            oldest_timestamp: None,
        });

    // Update L0 buffer
    l0_buffer.item_ids.push(leaf_id.clone());
    l0_buffer.token_sum += leaf_tokens;
    if l0_buffer.oldest_timestamp.is_none()
        || leaf_ts < l0_buffer.oldest_timestamp.unwrap()
    {
        l0_buffer.oldest_timestamp = Some(leaf_ts);
    }

    let mut sealed = false;

    // L0 seal check: token-gated
    if l0_buffer.token_sum >= INPUT_TOKEN_BUDGET {
        let child_ids = std::mem::take(&mut l0_buffer.item_ids);
        let l0_parent = l0_buffer.parent_id.clone();

        let summary = seal_level(
            store,
            summarizer,
            tree,
            0,                    // level being sealed
            child_ids,
            l0_parent,
            l0_buffer.token_sum,  // pass token sum for content sizing
            label_strategy,
        )
        .await?;

        tracing::debug!(
            tree_id = %tree.id,
            level = 0,
            summary_id = %summary.id,
            child_count = summary.child_ids.len(),
            "L0 buffer sealed → L1 summary"
        );

        new_summaries.push(summary.clone());

        // Clear L0 buffer
        l0_buffer.token_sum = 0;
        l0_buffer.oldest_timestamp = None;
        sealed = true;

        // Cascade upward: queue the new summary at level 1
        sealed |= cascade_seal(
            store,
            summarizer,
            tree,
            1,                    // start checking at level 1
            summary.id,
            summary.token_count,
            label_strategy,
            &mut new_summaries,
        )
        .await?;
    }

    // Persist the (possibly modified) L0 buffer
    store
        .put_buffer(&l0_buffer)
        .await
        .context("failed to persist L0 buffer")?;

    // Update root if we created a new top-level node
    if let Some(last) = new_summaries.last() {
        match tree.root_id {
            None => {
                tree.root_id = Some(last.id.clone());
                store
                    .update_tree_root(&tree.id, &last.id)
                    .await
                    .context("failed to update tree root")?;
            }
            Some(ref root) if last.level > 0 && Some(last.id.clone()) != Some(root.clone()) => {
                // Check if this summary is higher than current root
                let current_root = store
                    .get_summary(root)
                    .await
                    .context("failed to get current root")?;
                if let Some(current_root) = current_root {
                    if last.level > current_root.level {
                        tree.root_id = Some(last.id.clone());
                        store
                            .update_tree_root(&tree.id, &last.id)
                            .await
                            .context("failed to update tree root")?;
                    }
                }
            }
            _ => {}
        }
    }

    Ok(AppendResult {
        leaf_id,
        new_summaries,
        sealed,
    })
}

/// Cascade-seal upward from `level` until a buffer fails its gate.
///
/// At each level:
/// 1. Get or create the level buffer
/// 2. Append the `new_item_id` to it
/// 3. Check the seal gate (SUMMARY_FANOUT for L≥1)
/// 4. If gate is met: seal → create summary at level+1 → recurse
/// 5. If gate is NOT met: persist buffer and stop
async fn cascade_seal<S: TreeStore>(
    store: &mut S,
    summarizer: &dyn Summarizer,
    tree: &mut Tree,
    level: u32,
    new_item_id: String,
    item_token_count: u32,
    label_strategy: &LabelStrategy,
    new_summaries: &mut Vec<SummaryNode>,
) -> Result<bool> {
    if level >= MAX_CASCADE_DEPTH {
        tracing::warn!(
            tree_id = %tree.id,
            level,
            "Hit MAX_CASCADE_DEPTH — stopping cascade"
        );
        return Ok(false);
    }

    let mut any_sealed = false;

    // Get or create the buffer at this level
    let mut buffer = store
        .get_buffer(&tree.id, level)
        .await
        .context("failed to get buffer")?
        .unwrap_or_else(|| Buffer {
            tree_id: tree.id.clone(),
            level,
            parent_id: tree.root_id.clone(),
            item_ids: Vec::new(),
            token_sum: 0,
            oldest_timestamp: None,
        });

    buffer.item_ids.push(new_item_id);
    buffer.token_sum += item_token_count;

    // L≥1 seal check: sibling-count-gated
    if (buffer.item_ids.len() as u32) >= SUMMARY_FANOUT {
        let child_ids = std::mem::take(&mut buffer.item_ids);
        let parent_id = buffer.parent_id.clone();

        let summary = seal_level(
            store,
            summarizer,
            tree,
            level,
            child_ids,
            parent_id,
            buffer.token_sum,
            label_strategy,
        )
        .await?;

        tracing::debug!(
            tree_id = %tree.id,
            level,
            summary_id = %summary.id,
            child_count = summary.child_ids.len(),
            "L{} buffer sealed → L{} summary",
            level,
            level + 1,
        );

        new_summaries.push(summary.clone());

        // Clear buffer
        buffer.token_sum = 0;
        buffer.oldest_timestamp = None;
        any_sealed = true;

        // Recurse upward
        any_sealed |= Box::pin(cascade_seal(
            store,
            summarizer,
            tree,
            level + 1,
            summary.id,
            summary.token_count,
            label_strategy,
            new_summaries,
        ))
        .await?;
    }

    // Persist buffer state
    store
        .put_buffer(&buffer)
        .await
        .context("failed to persist buffer")?;

    Ok(any_sealed)
}

/// Seal a single buffer level: collect child contents, summarize, create node.
async fn seal_level<S: TreeStore>(
    store: &mut S,
    summarizer: &dyn Summarizer,
    tree: &Tree,
    level: u32,
    child_ids: Vec<String>,
    parent_id: Option<String>,
    total_tokens: u32,
    label_strategy: &LabelStrategy,
) -> Result<SummaryNode> {
    // Collect child content for summarization
    let mut child_contents = Vec::new();
    for child_id in &child_ids {
        // Try as summary first, then as leaf
        if let Some(summary) = store.get_summary(child_id).await? {
            child_contents.push(summary.content);
        } else if let Some(leaf) = store.get_leaf(child_id).await? {
            child_contents.push(leaf.content);
        }
    }

    // Summarize (or use fallback)
    let content = if !child_contents.is_empty() {
        summarizer
            .summarize(&child_contents, total_tokens as usize)
            .await
            .context("summarization failed")?
    } else {
        "(empty summary — no child content)".to_string()
    };

    // Estimate token count for the summary
    let token_count = (content.len() as u32 / 4).max(1);

    // Resolve labels according to strategy
    let (entities, topics) = resolve_labels(label_strategy, &child_ids, store, &content).await?;

    // Generate a unique summary id
    let summary_id = uuid::Uuid::new_v4().to_string();

    let summary = SummaryNode {
        id: summary_id,
        tree_id: tree.id.clone(),
        level: level + 1, // summary lives one level above its children
        parent_id,
        child_ids,
        content,
        entities,
        topics,
        token_count,
        created_at: Utc::now(),
    };

    // Persist the summary
    store
        .put_summary(&summary)
        .await
        .context("failed to store summary")?;

    Ok(summary)
}

/// Resolve entity and topic labels for a newly-created summary node.
async fn resolve_labels<S: TreeStore>(
    strategy: &LabelStrategy,
    child_ids: &[String],
    store: &mut S,
    summary_content: &str,
) -> Result<(Vec<String>, Vec<String>)> {
    match strategy {
        LabelStrategy::ExtractFromContent => {
            // Simple keyword-based extraction (placeholder for full entity extraction)
            let entities = extract_keyword_entities(summary_content);
            let topics = extract_basic_topics(summary_content);
            Ok((entities, topics))
        }
        LabelStrategy::UnionFromChildren => {
            let mut all_entities = BTreeSet::new();
            let mut all_topics = BTreeSet::new();

            for child_id in child_ids {
                if let Some(summary) = store.get_summary(child_id).await? {
                    for e in &summary.entities {
                        all_entities.insert(e.clone());
                    }
                    for t in &summary.topics {
                        all_topics.insert(t.clone());
                    }
                }
            }

            Ok((
                all_entities.into_iter().collect(),
                all_topics.into_iter().collect(),
            ))
        }
        LabelStrategy::Empty => Ok((Vec::new(), Vec::new())),
    }
}

/// Simple keyword-based entity extraction from summary text.
/// In production, this would use a dedicated entity extractor (LLM or NER model).
fn extract_keyword_entities(text: &str) -> Vec<String> {
    let mut entities = BTreeSet::new();
    let lower = text.to_lowercase();

    // Common entity indicators — capitalised nouns, proper names
    for word in text.split_whitespace() {
        let trimmed = word.trim_matches(|c: char| !c.is_alphanumeric());
        if trimmed.len() > 1
            && trimmed.chars().next().map_or(false, |c| c.is_uppercase())
            && !trimmed.chars().all(|c| c.is_uppercase())
        {
            entities.insert(trimmed.to_string());
        }
    }

    // Domain-specific patterns (extensible)
    let domain_patterns = [
        "api", "database", "migration", "deployment", "security",
        "authentication", "authorization", "cache", "queue", "worker",
        "webhook", "schema", "mutation", "query", "endpoint",
    ];

    for pattern in &domain_patterns {
        if lower.contains(pattern) {
            entities.insert(pattern.to_string());
        }
    }

    entities.into_iter().take(20).collect()
}

/// Basic topic extraction — identifies broad thematic categories.
fn extract_basic_topics(text: &str) -> Vec<String> {
    let mut topics = BTreeSet::new();
    let lower = text.to_lowercase();

    let topic_keywords: &[(&str, &[&str])] = &[
        ("security", &["vulnerability", "exploit", "csrf", "xss", "injection", "auth"]),
        ("performance", &["slow", "latency", "bottleneck", "optimize", "cache"]),
        ("testing", &["test", "assert", "mock", "fixture", "coverage"]),
        ("infrastructure", &["deploy", "server", "docker", "kubernetes", "config"]),
        ("data", &["migration", "schema", "model", "database", "query"]),
        ("api", &["endpoint", "route", "handler", "request", "response"]),
    ];

    for (topic, keywords) in topic_keywords {
        for kw in *keywords {
            if lower.contains(kw) {
                topics.insert(topic.to_string());
                break;
            }
        }
    }

    topics.into_iter().take(10).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::InMemoryTreeStore;
    use crate::TreeKind;
    use crate::types::{INPUT_TOKEN_BUDGET, SUMMARY_FANOUT};

    struct ConcatMock;
    #[async_trait::async_trait]
    impl Summarizer for ConcatMock {
        async fn summarize(&self, contents: &[String], _input_tokens: usize) -> Result<String> {
            Ok(contents.join(" | "))
        }
    }

    fn make_leaf(id: &str, content: &str, tokens: u32) -> LeafChunk {
        LeafChunk {
            id: id.to_string(),
            content: content.to_string(),
            token_count: tokens,
            timestamp: Utc::now(),
        }
    }

    fn make_tree(label: &str, kind: TreeKind) -> Tree {
        Tree {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            status: TreeStatus::Active,
            root_id: None,
            source_id: None,
            label: label.to_string(),
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_append_single_leaf_no_seal() {
        let mut store = InMemoryTreeStore::new();
        let summarizer = ConcatMock;
        let mut tree = make_tree("test-source", TreeKind::Source);
        let strategy = LabelStrategy::for_kind(tree.kind);

        // Save tree first
        store.put_tree(&tree).await.unwrap();

        let leaf = make_leaf("leaf-1", "hello world", 10);
        let result = append_leaf(&mut store, &summarizer, &mut tree, leaf, &strategy)
            .await
            .unwrap();

        assert!(!result.sealed, "should not seal with only 10 tokens");
        assert_eq!(result.leaf_id, "leaf-1");
        assert!(result.new_summaries.is_empty());

        // Verify L0 buffer
        let buf = store.get_buffer(&tree.id, 0).await.unwrap().unwrap();
        assert_eq!(buf.item_ids, vec!["leaf-1"]);
        assert_eq!(buf.token_sum, 10);
    }

    #[tokio::test]
    async fn test_seal_l0_when_token_budget_exceeded() {
        let mut store = InMemoryTreeStore::new();
        let summarizer = ConcatMock;
        let mut tree = make_tree("test-source", TreeKind::Source);
        let strategy = LabelStrategy::for_kind(tree.kind);

        store.put_tree(&tree).await.unwrap();

        // Add a leaf that exceeds the budget
        let leaf = make_leaf("big-leaf", &"x".repeat(INPUT_TOKEN_BUDGET as usize * 4), INPUT_TOKEN_BUDGET + 1);
        let result = append_leaf(&mut store, &summarizer, &mut tree, leaf, &strategy)
            .await
            .unwrap();

        assert!(result.sealed, "should seal when token budget exceeded");
        assert_eq!(result.new_summaries.len(), 1);

        let summary = &result.new_summaries[0];
        assert_eq!(summary.level, 1);
        assert_eq!(summary.child_ids, vec!["big-leaf"]);

        // L0 buffer should be empty now
        let buf = store.get_buffer(&tree.id, 0).await.unwrap().unwrap();
        assert_eq!(buf.token_sum, 0);
        assert!(buf.item_ids.is_empty());
    }

    #[tokio::test]
    async fn test_cascade_seal_at_l1() {
        let mut store = InMemoryTreeStore::new();
        let summarizer = ConcatMock;
        let mut tree = make_tree("test-source", TreeKind::Source);
        let strategy = LabelStrategy::for_kind(tree.kind);

        store.put_tree(&tree).await.unwrap();

        // Each leaf must individually exceed INPUT_TOKEN_BUDGET to seal L0 → L1.
        // After SUMMARY_FANOUT seals, L1 buffer fills and cascades to L2.
        let tokens_per = INPUT_TOKEN_BUDGET + 1; // 50,001 — seals immediately per leaf

        let mut total_summaries = 0;
        for i in 0..(SUMMARY_FANOUT * 2) {
            let leaf = make_leaf(
                &format!("leaf-{}", i),
                &"x".repeat(tokens_per as usize * 4),
                tokens_per,
            );
            let result = append_leaf(&mut store, &summarizer, &mut tree, leaf, &strategy)
                .await
                .unwrap();
            total_summaries += result.new_summaries.len();
        }

        // Each leaf seals L0 → L1 immediately (50,001 tokens > 50,000 budget).
        // After SUMMARY_FANOUT(10) L1 nodes accumulate, L1 buffer seals → L2.
        // After 2*SUMMARY_FANOUT(20) leaves: 2 L1→L2 seals = 2 L2 nodes + 20 L1 nodes.
        // 2 L2 nodes do not trigger another cascade (need 10 siblings).
        assert!(total_summaries > SUMMARY_FANOUT as usize,
            "should have created L1 + L2 summaries: got {}", total_summaries);
    }

    #[tokio::test]
    async fn test_label_strategy_union_from_children() {
        let mut store = InMemoryTreeStore::new();
        let summarizer = ConcatMock;
        let tree = make_tree("global-digest", TreeKind::Global);
        let strategy = LabelStrategy::for_kind(tree.kind);

        store.put_tree(&tree).await.unwrap();

        // Create a few L1 summaries with different entities
        // (simulating pre-existing source tree summaries)
        let summary_ids: Vec<_> = (0..SUMMARY_FANOUT)
            .map(|i| {
                let id = format!("src-summary-{}", i);
                let s = SummaryNode {
                    id: id.clone(),
                    tree_id: tree.id.clone(),
                    level: 1,
                    parent_id: None,
                    child_ids: vec![format!("src-leaf-{}", i)],
                    content: format!("source content {}", i),
                    entities: vec![format!("entity-{}", i), "shared-entity".to_string()],
                    topics: vec![format!("topic-{}", i % 3)],
                    token_count: 100,
                    created_at: Utc::now(),
                };
                (id, s)
            })
            .collect();

        // Store the summaries
        for (_, summary) in &summary_ids {
            store.put_summary(summary).await.unwrap();
        }

        // Now append each summary id to L1 buffer of the global tree
        let mut buffer = store.get_buffer(&tree.id, 1).await.unwrap()
            .unwrap_or_else(|| Buffer {
                tree_id: tree.id.clone(),
                level: 1,
                parent_id: tree.root_id.clone(),
                item_ids: Vec::new(),
                token_sum: 0,
                oldest_timestamp: None,
            });

        for (id, _) in &summary_ids {
            buffer.item_ids.push(id.clone());
            buffer.token_sum += 100;
        }

        // This should trigger a seal since we have SUMMARY_FANOUT items
        let child_ids = std::mem::take(&mut buffer.item_ids);
        let summary = seal_level(
            &mut store,
            &summarizer,
            &tree,
            1,
            child_ids,
            None,
            buffer.token_sum,
            &strategy,
        )
        .await
        .unwrap();

        // UnionFromChildren should have "shared-entity" once (deduplicated)
        assert!(summary.entities.contains(&"shared-entity".to_string()));
        // And each entity-i should be present
        for i in 0..SUMMARY_FANOUT as usize {
            assert!(summary.entities.contains(&format!("entity-{}", i)));
        }
    }
}
