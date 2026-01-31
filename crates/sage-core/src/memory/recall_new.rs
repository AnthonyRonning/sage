//! Recall Memory (Conversation History with Embeddings)
//!
//! Full conversation history stored in PostgreSQL with embeddings.
//! Supports both keyword and semantic search via pgvector.

use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::db::{MemoryDb, MessageRow};
use super::embedding::EmbeddingService;

/// A message in recall memory
#[derive(Debug, Clone)]
pub struct RecallMessage {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub user_id: String,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub sequence_id: i64,
}

impl From<MessageRow> for RecallMessage {
    fn from(row: MessageRow) -> Self {
        Self {
            id: row.id,
            agent_id: row.agent_id,
            user_id: row.user_id,
            role: row.role,
            content: row.content,
            created_at: row.created_at,
            sequence_id: row.sequence_id,
        }
    }
}

/// Search result from recall memory
#[derive(Debug, Clone)]
pub struct RecallSearchResult {
    pub message: RecallMessage,
    pub relevance_score: Option<f32>,
    pub match_type: MatchType,
}

/// How the result was matched
#[derive(Debug, Clone, Copy)]
pub enum MatchType {
    Keyword,
    Semantic,
    Hybrid,
}

impl RecallSearchResult {
    /// Format the search result for display to the agent
    pub fn format(&self) -> String {
        let timestamp = self.message.created_at.format("%Y-%m-%d %H:%M:%S UTC");
        let time_ago = format_time_ago(self.message.created_at, Utc::now());
        let role = &self.message.role;
        let content = &self.message.content;

        let score_str = self
            .relevance_score
            .map(|s| format!(" (score: {:.2})", s))
            .unwrap_or_default();

        let mut result = format!("[{}] ({}, {}){}\n", timestamp, time_ago, role, score_str);

        // Truncate long content (handle UTF-8 boundaries safely)
        if content.len() > 500 {
            let mut end = 500;
            while !content.is_char_boundary(end) && end > 0 {
                end -= 1;
            }
            result.push_str(&content[..end]);
            result.push_str("...[truncated]");
        } else {
            result.push_str(content);
        }

        result
    }
}

/// Manages recall memory (conversation history with embeddings)
#[derive(Clone)]
pub struct RecallManager {
    agent_id: Uuid,
    db: MemoryDb,
    embedding: EmbeddingService,
}

impl RecallManager {
    /// Create a new recall manager for an agent
    pub fn new(agent_id: Uuid, db: MemoryDb, embedding: EmbeddingService) -> Self {
        Self {
            agent_id,
            db,
            embedding,
        }
    }

    /// Get the agent ID
    pub fn agent_id(&self) -> Uuid {
        self.agent_id
    }

    /// Get a reference to the database
    pub fn db(&self) -> MemoryDb {
        self.db.clone()
    }

    /// Get a reference to the embedding service
    pub fn embedding_service(&self) -> EmbeddingService {
        self.embedding.clone()
    }

    /// Get the total number of messages in recall memory
    pub fn message_count(&self) -> usize {
        self.db
            .messages()
            .count_messages(self.agent_id)
            .unwrap_or(0) as usize
    }

    /// Add a message to recall memory with embedding
    pub async fn add_message(&self, user_id: &str, role: &str, content: &str) -> Result<Uuid> {
        // Generate embedding for the message
        let embedding = self.embedding.embed(content).await?;

        // Store in database with embedding
        let id = self.db.messages().insert_message(
            self.agent_id,
            user_id,
            role,
            content,
            &embedding,
            None, // tool_calls
            None, // tool_results
        )?;

        tracing::debug!("Stored message {} with embedding", id);
        Ok(id)
    }

    /// Add a message WITHOUT embedding (for fast insertion)
    /// Use update_embedding() later to add the embedding in background
    pub fn add_message_sync(&self, user_id: &str, role: &str, content: &str) -> Result<Uuid> {
        // Store with zero embedding - will be updated later
        let zero_embedding = vec![0.0f32; super::embedding::EMBEDDING_DIM];

        let id = self.db.messages().insert_message(
            self.agent_id,
            user_id,
            role,
            content,
            &zero_embedding,
            None,
            None,
        )?;

        tracing::debug!("Stored message {} (embedding pending)", id);
        Ok(id)
    }

    /// Update embedding for a message (call in background after add_message_sync)
    pub async fn update_embedding(&self, message_id: Uuid, content: &str) -> Result<()> {
        let embedding = self.embedding.embed(content).await?;
        self.db
            .messages()
            .update_embedding(message_id, &embedding)?;
        tracing::debug!("Updated embedding for message {}", message_id);
        Ok(())
    }

    /// Add a message with tool call information
    pub async fn add_tool_message(
        &self,
        user_id: &str,
        role: &str,
        content: &str,
        tool_calls: Option<&serde_json::Value>,
        tool_results: Option<&serde_json::Value>,
    ) -> Result<Uuid> {
        let embedding = self.embedding.embed(content).await?;

        let id = self.db.messages().insert_message(
            self.agent_id,
            user_id,
            role,
            content,
            &embedding,
            tool_calls,
            tool_results,
        )?;

        Ok(id)
    }

    /// Search recall memory by keyword
    pub fn search_keyword(&self, query: &str, limit: usize) -> Result<Vec<RecallSearchResult>> {
        let messages = self.db.messages().get_recent(self.agent_id, 1000)?;
        let query_lower = query.to_lowercase();

        let mut results: Vec<RecallSearchResult> = messages
            .into_iter()
            .filter(|m| {
                // Skip tool messages and meta-queries
                if m.role == "tool" {
                    return false;
                }
                m.content.to_lowercase().contains(&query_lower)
            })
            .map(|m| RecallSearchResult {
                message: m.into(),
                relevance_score: None,
                match_type: MatchType::Keyword,
            })
            .collect();

        // Sort by recency
        results.sort_by(|a, b| b.message.sequence_id.cmp(&a.message.sequence_id));
        results.truncate(limit);

        Ok(results)
    }

    /// Search recall memory by semantic similarity
    pub async fn search_semantic(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RecallSearchResult>> {
        // Generate query embedding
        let query_embedding = self.embedding.embed(query).await?;

        // Search database with pgvector
        let results = self.db.messages().search_by_embedding(
            self.agent_id,
            &query_embedding,
            limit as i64,
        )?;

        Ok(results
            .into_iter()
            .map(|r| RecallSearchResult {
                message: r.message.into(),
                relevance_score: Some(1.0 - r.distance as f32), // Convert distance to similarity
                match_type: MatchType::Semantic,
            })
            .collect())
    }

    /// Hybrid search combining keyword and semantic
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<RecallSearchResult>> {
        // Get keyword results
        let keyword_results = self.search_keyword(query, limit)?;

        // Get semantic results
        let semantic_results = self.search_semantic(query, limit).await?;

        // Merge and deduplicate by message ID
        let mut seen = std::collections::HashSet::new();
        let mut combined: Vec<RecallSearchResult> = Vec::new();

        // Add semantic results first (they have scores)
        for result in semantic_results {
            if seen.insert(result.message.id) {
                combined.push(result);
            }
        }

        // Add keyword results that weren't in semantic
        for mut result in keyword_results {
            if seen.insert(result.message.id) {
                result.match_type = MatchType::Keyword;
                combined.push(result);
            }
        }

        // Sort by relevance score (semantic first), then by recency
        combined.sort_by(|a, b| match (a.relevance_score, b.relevance_score) {
            (Some(sa), Some(sb)) => sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => b.message.sequence_id.cmp(&a.message.sequence_id),
        });

        combined.truncate(limit);
        Ok(combined)
    }

    /// Get messages by IDs (for loading context window)
    pub fn get_by_ids(&self, ids: &[Uuid]) -> Result<Vec<RecallMessage>> {
        let messages = self.db.messages().get_by_ids(ids)?;
        Ok(messages.into_iter().map(|m| m.into()).collect())
    }

    /// Get recent messages
    pub fn get_recent(&self, limit: usize) -> Result<Vec<RecallMessage>> {
        let messages = self.db.messages().get_recent(self.agent_id, limit as i64)?;
        Ok(messages.into_iter().map(|m| m.into()).collect())
    }
}

/// Format a duration as human-readable "time ago"
fn format_time_ago(then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let duration = now.signed_duration_since(then);

    if duration.num_days() > 0 {
        format!("{}d ago", duration.num_days())
    } else if duration.num_hours() > 0 {
        format!("{}h ago", duration.num_hours())
    } else if duration.num_minutes() > 0 {
        format!("{}m ago", duration.num_minutes())
    } else {
        "just now".to_string()
    }
}
