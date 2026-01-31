//! Context Window Management
//!
//! Manages the in-context message buffer and token counting.
//! The `message_ids` list represents which messages are visible to the LLM.

use uuid::Uuid;

/// Default context window size for Kimi K2
pub const DEFAULT_CONTEXT_WINDOW: usize = 256_000;

/// Compaction threshold (80% of context window)
pub const COMPACTION_THRESHOLD: f32 = 0.80;

/// Manages the context window state
pub struct ContextManager {
    /// Maximum tokens in context window
    max_tokens: usize,
    /// Currently in-context message IDs (ordered)
    message_ids: Vec<Uuid>,
    /// Compaction threshold ratio
    threshold: f32,
}

impl ContextManager {
    /// Create a new context manager with the given max token limit
    pub fn new(max_tokens: usize) -> Self {
        Self {
            max_tokens,
            message_ids: Vec::new(),
            threshold: COMPACTION_THRESHOLD,
        }
    }

    /// Create with custom threshold
    #[allow(dead_code)]
    pub fn with_threshold(max_tokens: usize, threshold: f32) -> Self {
        Self {
            max_tokens,
            message_ids: Vec::new(),
            threshold: threshold.clamp(0.5, 0.95),
        }
    }

    /// Get the maximum tokens allowed
    pub fn max_tokens(&self) -> usize {
        self.max_tokens
    }

    /// Get the compaction threshold in tokens
    pub fn threshold_tokens(&self) -> usize {
        (self.max_tokens as f32 * self.threshold) as usize
    }

    /// Check if compaction is needed
    pub fn needs_compaction(&self, current_tokens: usize) -> bool {
        current_tokens > self.threshold_tokens()
    }

    /// Get in-context message IDs
    pub fn message_ids(&self) -> &[Uuid] {
        &self.message_ids
    }

    /// Set in-context message IDs
    pub fn set_message_ids(&mut self, ids: Vec<Uuid>) {
        self.message_ids = ids;
    }

    /// Add a message ID to the context
    pub fn add_message(&mut self, id: Uuid) {
        self.message_ids.push(id);
    }

    /// Remove message IDs that were compacted
    pub fn remove_messages(&mut self, ids_to_remove: &[Uuid]) {
        self.message_ids.retain(|id| !ids_to_remove.contains(id));
    }

    /// Get the number of in-context messages
    pub fn message_count(&self) -> usize {
        self.message_ids.len()
    }

    /// Clear all message IDs
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.message_ids.clear();
    }
}

/// Token counter using tiktoken (cl100k_base for GPT-4 compatible models)
pub struct TokenCounter {
    // We'll use tiktoken-rs for actual counting
    // For now, use a simple approximation
}

impl TokenCounter {
    /// Create a new token counter
    pub fn new() -> Self {
        Self {}
    }

    /// Count tokens in a string (approximate)
    /// Uses ~4 chars per token as a rough estimate
    /// TODO: Use tiktoken-rs for accurate counting
    pub fn count(&self, text: &str) -> usize {
        // Rough approximation: ~4 chars per token
        // This is conservative and works reasonably well for English
        text.len() / 4
    }

    /// Count tokens in multiple strings
    pub fn count_many(&self, texts: &[&str]) -> usize {
        texts.iter().map(|t| self.count(t)).sum()
    }
}

impl Default for TokenCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_manager() {
        let mut ctx = ContextManager::new(100_000);

        assert_eq!(ctx.max_tokens(), 100_000);
        assert_eq!(ctx.threshold_tokens(), 80_000);
        assert!(!ctx.needs_compaction(50_000));
        assert!(ctx.needs_compaction(85_000));
    }

    #[test]
    fn test_message_management() {
        let mut ctx = ContextManager::new(100_000);

        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        ctx.add_message(id1);
        ctx.add_message(id2);
        ctx.add_message(id3);

        assert_eq!(ctx.message_count(), 3);

        ctx.remove_messages(&[id2]);
        assert_eq!(ctx.message_count(), 2);
        assert!(!ctx.message_ids().contains(&id2));
    }

    #[test]
    fn test_token_counter() {
        let counter = TokenCounter::new();

        // ~4 chars per token
        assert!(counter.count("Hello, world!") >= 2); // 13 chars -> ~3 tokens
        assert!(counter.count("") == 0);
    }
}
