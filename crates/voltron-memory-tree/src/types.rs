//! Core types for the memory tree system.
//!
//! The memory tree is a multi-level summarization structure:
//! - **L0**: raw input buffer (chunks, messages, events)
//! - **L1+**: summarized nodes, each summarizing its children
//!
//! Sealing gates:
//! - L0 → L1: triggered when `token_sum >= INPUT_TOKEN_BUDGET`
//! - L≥1 → L+1: triggered when sibling count reaches `SUMMARY_FANOUT`
//!
//! This two-tier gating prevents the tree from collapsing to 1:1 chains
//! when the summarizer produces inconsistent token output sizes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Token budget that triggers L0 → L1 seal (50k tokens).
/// A conservative estimate — one token ≈ 4 chars of English text.
pub const INPUT_TOKEN_BUDGET: u32 = 50_000;

/// Target output token count for summary generation (5k tokens).
/// Summarizers should aim for this when condensing INPUT_TOKEN_BUDGET.
pub const OUTPUT_TOKEN_BUDGET: u32 = 5_000;

/// Number of siblings at level ≥1 that trigger a cascade seal upward.
/// Using count (not tokens) guarantees stable fan-in regardless of
/// summarizer quality — a weak summarizer producing small summaries
/// won't cause the tree to degenerate.
pub const SUMMARY_FANOUT: u32 = 10;

/// Hard cap on cascade recursion depth. Prevents infinite loops if
/// token accounting drifts. At 10x fan-in per level, 32 levels covers
/// 10^32 raw items — far beyond any realistic source.
pub const MAX_CASCADE_DEPTH: u32 = 32;

/// Default age (in seconds) after which a stale L0 buffer is force-sealed.
/// Sources that receive low-volume input may never hit token or count
/// thresholds; time-based flushing ensures they still produce summaries.
pub const DEFAULT_FLUSH_AGE_SECS: i64 = 604_800; // 7 days

/// The kind of tree, determining its label strategy and sealing behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeKind {
    /// Source-anchored tree: one tree per conversation source (chat, email, docs).
    /// Labels extracted from freshly-summarized content via entity extraction.
    Source,
    /// Global digest tree: merges summaries from all source trees.
    /// Labels unioned from children (no LLM extraction).
    Global,
    /// Topic-specific tree: scoped to a single designated theme.
    /// Labels inherited as empty — scope already pins the dominant theme.
    Topic,
}

impl TreeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TreeKind::Source => "source",
            TreeKind::Global => "global",
            TreeKind::Topic => "topic",
        }
    }
}

/// Lifecycle status of a tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeStatus {
    /// Tree is actively accepting new leaves.
    Active,
    /// Tree has been frozen (no new input accepted).
    Frozen,
    /// Tree has been marked for archival.
    Archived,
}

/// A single node in the memory tree hierarchy.
///
/// L0 nodes are raw chunks stored externally (referenced by id).
/// L1+ nodes are summaries with LLM-generated content and extracted labels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryNode {
    /// Unique node identifier (UUID v4).
    pub id: String,
    /// The tree this node belongs to.
    pub tree_id: String,
    /// Depth level in the tree (0 = raw leaf, 1 = first summary, etc.).
    pub level: u32,
    /// Parent summary node that contains this node in its child_ids.
    pub parent_id: Option<String>,
    /// Child node ids (either raw chunks at L0, or lower-level summaries at L1+).
    pub child_ids: Vec<String>,
    /// The summarized content for L1+ nodes; empty for L0 references.
    pub content: String,
    /// Canonical entity IDs extracted from this node's content.
    pub entities: Vec<String>,
    /// Topic labels associated with this node.
    pub topics: Vec<String>,
    /// Estimated token count of this node's content.
    pub token_count: u32,
    /// When this node was created.
    pub created_at: DateTime<Utc>,
}

/// Accumulator buffer for a single level within a tree.
///
/// Buffers collect items until their seal gate is met. When a buffer
/// seals, its items move into a new SummaryNode's child_ids, the buffer
/// clears, and the new summary's id is queued at the next level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Buffer {
    /// The tree this buffer belongs to.
    pub tree_id: String,
    /// Level of this buffer (same as the items it collects).
    pub level: u32,
    /// Parent summary node id (if this buffer sits below a summary).
    pub parent_id: Option<String>,
    /// IDs of items accumulated in this buffer (raw chunks or summary node ids).
    pub item_ids: Vec<String>,
    /// Running sum of token counts for all items in this buffer.
    pub token_sum: u32,
    /// Timestamp of the oldest item in this buffer (for time-based flush).
    pub oldest_timestamp: Option<DateTime<Utc>>,
}

/// Represents a complete memory tree with its root and buffers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tree {
    /// Unique tree identifier.
    pub id: String,
    /// The kind of tree (source, global, topic).
    pub kind: TreeKind,
    /// Current lifecycle status.
    pub status: TreeStatus,
    /// The root summary node id (top of the hierarchy).
    pub root_id: Option<String>,
    /// For source trees, the external source identifier.
    pub source_id: Option<String>,
    /// Human-readable label for this tree.
    pub label: String,
    /// When this tree was created.
    pub created_at: DateTime<Utc>,
}

/// A leaf item to be ingested into a tree (raw chunk at L0).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeafChunk {
    /// Unique chunk identifier.
    pub id: String,
    /// The raw text content.
    pub content: String,
    /// Estimated token count.
    pub token_count: u32,
    /// When this chunk was created.
    pub timestamp: DateTime<Utc>,
}

/// Result of an append_leaf operation — describes what happened.
#[derive(Debug, Clone)]
pub struct AppendResult {
    /// The leaf chunk id that was appended.
    pub leaf_id: String,
    /// Any new summary nodes created during cascade sealing.
    pub new_summaries: Vec<SummaryNode>,
    /// Whether a cascade seal occurred.
    pub sealed: bool,
}

/// Label strategy determines how entities and topics are populated
/// on newly-created summary nodes.
#[derive(Debug, Clone)]
pub enum LabelStrategy {
    /// Run entity extraction on the summary content (used for Source trees).
    /// Captures emergent themes that no individual leaf expressed.
    ExtractFromContent,
    /// Union deduplicated entities/topics from child nodes (used for Global trees).
    /// Preserves labels for time-based retrieval without an LLM call.
    UnionFromChildren,
    /// Leave both entities and topics empty (used for Topic trees).
    /// The topic scope already pins the dominant theme; cross-pollination
    /// would noise the entity index.
    Empty,
}

impl LabelStrategy {
    /// Select the appropriate strategy for a given tree kind.
    pub fn for_kind(kind: TreeKind) -> Self {
        match kind {
            TreeKind::Source => LabelStrategy::ExtractFromContent,
            TreeKind::Global => LabelStrategy::UnionFromChildren,
            TreeKind::Topic => LabelStrategy::Empty,
        }
    }
}
