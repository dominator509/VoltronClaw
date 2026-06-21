//! Abstract storage backend for the memory tree.
//!
//! The `TreeStore` trait allows swapping between in-memory (for testing),
//! SQLite, or distributed backends without changing the core algorithm.

use anyhow::Result;
use async_trait::async_trait;

use crate::types::{Buffer, LeafChunk, SummaryNode, Tree};

/// Backend-agnostic storage operations for memory trees.
///
/// All operations are async to support networked/distributed backends.
/// Implementations must be internally consistent — a `put_*` must be
/// visible to a subsequent `get_*` within the same store instance.
#[async_trait]
pub trait TreeStore: Send {
    /// --- Tree operations ---

    /// Persist a tree.
    async fn put_tree(&mut self, tree: &Tree) -> Result<()>;

    /// Load a tree by id.
    async fn get_tree(&self, tree_id: &str) -> Result<Option<Tree>>;

    /// Update the root_id of a tree.
    async fn update_tree_root(&mut self, tree_id: &str, root_id: &str) -> Result<()>;

    /// List all tree ids.
    async fn list_trees(&self) -> Result<Vec<String>>;

    /// --- Summary node operations ---

    /// Persist a summary node.
    async fn put_summary(&mut self, node: &SummaryNode) -> Result<()>;

    /// Load a summary node by id.
    async fn get_summary(&self, node_id: &str) -> Result<Option<SummaryNode>>;

    /// --- Leaf chunk operations ---

    /// Persist a raw leaf chunk.
    async fn put_leaf(&mut self, leaf: &LeafChunk) -> Result<()>;

    /// Load a leaf chunk by id.
    async fn get_leaf(&self, leaf_id: &str) -> Result<Option<LeafChunk>>;

    /// --- Buffer operations ---

    /// Persist a buffer for a given tree and level.
    async fn put_buffer(&mut self, buffer: &Buffer) -> Result<()>;

    /// Load a buffer for a given tree and level.
    async fn get_buffer(&self, tree_id: &str, level: u32) -> Result<Option<Buffer>>;

    /// List all non-empty buffers across all trees (for time-based flush).
    async fn list_buffers(&self) -> Result<Vec<Buffer>>;
}

/// In-memory store for testing and development.
///
/// Uses `HashMap` and `Vec` for O(1) lookups. Not suitable for production
/// (no persistence, unbounded growth).
pub struct InMemoryTreeStore {
    trees: std::collections::HashMap<String, Tree>,
    summaries: std::collections::HashMap<String, SummaryNode>,
    leaves: std::collections::HashMap<String, LeafChunk>,
    buffers: Vec<Buffer>,
}

impl InMemoryTreeStore {
    pub fn new() -> Self {
        Self {
            trees: std::collections::HashMap::new(),
            summaries: std::collections::HashMap::new(),
            leaves: std::collections::HashMap::new(),
            buffers: Vec::new(),
        }
    }
}

impl Default for InMemoryTreeStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TreeStore for InMemoryTreeStore {
    async fn put_tree(&mut self, tree: &Tree) -> Result<()> {
        self.trees.insert(tree.id.clone(), tree.clone());
        Ok(())
    }

    async fn get_tree(&self, tree_id: &str) -> Result<Option<Tree>> {
        Ok(self.trees.get(tree_id).cloned())
    }

    async fn update_tree_root(&mut self, tree_id: &str, root_id: &str) -> Result<()> {
        if let Some(tree) = self.trees.get_mut(tree_id) {
            tree.root_id = Some(root_id.to_string());
        }
        Ok(())
    }

    async fn list_trees(&self) -> Result<Vec<String>> {
        Ok(self.trees.keys().cloned().collect())
    }

    async fn put_summary(&mut self, node: &SummaryNode) -> Result<()> {
        self.summaries.insert(node.id.clone(), node.clone());
        Ok(())
    }

    async fn get_summary(&self, node_id: &str) -> Result<Option<SummaryNode>> {
        Ok(self.summaries.get(node_id).cloned())
    }

    async fn put_leaf(&mut self, leaf: &LeafChunk) -> Result<()> {
        self.leaves.insert(leaf.id.clone(), leaf.clone());
        Ok(())
    }

    async fn get_leaf(&self, leaf_id: &str) -> Result<Option<LeafChunk>> {
        Ok(self.leaves.get(leaf_id).cloned())
    }

    async fn put_buffer(&mut self, buffer: &Buffer) -> Result<()> {
        // Replace existing buffer for same tree+level, or insert new
        self.buffers.retain(|b| !(b.tree_id == buffer.tree_id && b.level == buffer.level));
        self.buffers.push(buffer.clone());
        Ok(())
    }

    async fn get_buffer(&self, tree_id: &str, level: u32) -> Result<Option<Buffer>> {
        Ok(self
            .buffers
            .iter()
            .find(|b| b.tree_id == tree_id && b.level == level)
            .cloned())
    }

    async fn list_buffers(&self) -> Result<Vec<Buffer>> {
        Ok(self.buffers.clone())
    }
}
