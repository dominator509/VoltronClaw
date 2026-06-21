//! Summarization trait and built-in fallbacks.
//!
//! The `Summarizer` trait decouples the bucket-seal algorithm from any
//! specific LLM provider. Implementations can range from a simple
//! concatenation (for testing) to full LLM summarization via rig-core.

use anyhow::Result;
use async_trait::async_trait;

/// Summarizes a collection of content strings into a single condensed output.
///
/// The `input_tokens` parameter is a hint about the total token count of
/// `contents`, helping the summarizer allocate appropriate output budget.
#[async_trait]
pub trait Summarizer: Send + Sync {
    /// Produce a summary from child contents.
    ///
    /// `contents` — the text of each child node to summarize.
    /// `input_tokens` — estimated token count of all contents combined.
    async fn summarize(&self, contents: &[String], input_tokens: usize) -> Result<String>;
}

/// A no-op summarizer that concatenates input with a separator.
/// Useful for testing the cascade algorithm without an LLM dependency.
pub struct ConcatSummarizer {
    pub separator: String,
}

impl ConcatSummarizer {
    pub fn new(separator: impl Into<String>) -> Self {
        Self {
            separator: separator.into(),
        }
    }
}

#[async_trait]
impl Summarizer for ConcatSummarizer {
    async fn summarize(&self, contents: &[String], _input_tokens: usize) -> Result<String> {
        Ok(contents.join(&self.separator))
    }
}

/// Truncating summarizer: takes the first N chars of each input and joins them.
/// Simulates a lossy summarizer to test fan-in behavior.
pub struct TruncatingSummarizer {
    pub max_chars_per_item: usize,
    pub separator: String,
}

impl TruncatingSummarizer {
    pub fn new(max_chars_per_item: usize) -> Self {
        Self {
            max_chars_per_item,
            separator: " | ".to_string(),
        }
    }
}

#[async_trait]
impl Summarizer for TruncatingSummarizer {
    async fn summarize(&self, contents: &[String], _input_tokens: usize) -> Result<String> {
        let truncated: Vec<String> = contents
            .iter()
            .map(|c| {
                if c.len() > self.max_chars_per_item {
                    format!("{}...", &c[..self.max_chars_per_item])
                } else {
                    c.clone()
                }
            })
            .collect();
        Ok(truncated.join(&self.separator))
    }
}
