//! `voltron-memory-tree` — Obsidian-compatible memory tree with bucket-seal cascade.
//!
//! ## Architecture
//!
//! The memory tree is a multi-level summarization structure inspired by
//! OpenHuman's memory architecture. It organizes raw inputs into a
//! hierarchy of increasingly abstract summaries:
//!
//! ```text
//! L3:                [Root Summary]
//!                        │
//! L2:        [S1] [S2] [S3] ... [S10]   ← SUMMARY_FANOUT siblings
//!              │    │    │         │
//! L1:     [s1..s10] [...]                ← 10x L0 seals each
//!              │
//! L0:     [chunk₁ ... chunkₙ]            ← raw input buffer
//! ```
//!
//! ## Sealing Gates
//!
//! - **L0 → L1**: token-gated (`INPUT_TOKEN_BUDGET = 50k`)
//! - **L≥1 → L+1**: sibling-count-gated (`SUMMARY_FANOUT = 10`)
//!
//! This two-tier gating ensures stable fan-in regardless of summarizer
//! quality — a weak summarizer producing tiny summaries won't cause the
//! tree to degenerate into a 1:1 chain.
//!
//! ## Features
//!
//! - Three tree kinds: Source (per-conversation), Global (cross-source digest),
//!   Topic (themed scope)
//! - Pluggable storage backend via `TreeStore` trait
//! - Pluggable summarization via `Summarizer` trait
//! - Time-based flushing for low-volume sources
//! - Label strategies: extract, union, or empty per tree kind
//! - Thread-safe cascade with depth safety cap

pub mod types;
pub mod store;
pub mod summarize;
pub mod seal;
pub mod flush;
pub mod tree;

// Re-export core types
pub use types::{
    AppendResult, Buffer, LabelStrategy, LeafChunk, SummaryNode, Tree, TreeKind, TreeStatus,
    DEFAULT_FLUSH_AGE_SECS, INPUT_TOKEN_BUDGET, MAX_CASCADE_DEPTH, OUTPUT_TOKEN_BUDGET,
    SUMMARY_FANOUT,
};

// Re-export store
pub use store::{InMemoryTreeStore, TreeStore};

// Re-export summarizer
pub use summarize::{ConcatSummarizer, Summarizer, TruncatingSummarizer};

// Re-export engine
pub use tree::MemoryTreeEngine;

// Re-export seal
pub use seal::append_leaf;

// Re-export flush
pub use flush::{flush_stale_buffers, flush_stale_buffers_default};
