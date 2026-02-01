//! Archival Memory (Long-term Semantic Storage) - Database Backed
//!
//! Agent-created long-term memories stored with embeddings for semantic search.
//! Uses PostgreSQL with pgvector for persistence and efficient similarity queries.

#![allow(dead_code)]

use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::db::MemoryDb;
use super::embedding::EmbeddingService;

/// A passage in archival memory
#[derive(Debug, Clone)]
pub struct Passage {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub content: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// Search result from archival memory
#[derive(Debug, Clone)]
pub struct ArchivalSearchResult {
    pub passage: Passage,
    pub relevance_score: f32,
}

impl ArchivalSearchResult {
    /// Format the search result for display to the agent
    pub fn format(&self) -> String {
        let timestamp = self.passage.created_at.format("%Y-%m-%d %H:%M:%S UTC");
        let time_ago = format_time_ago(self.passage.created_at, Utc::now());
        let tags = if self.passage.tags.is_empty() {
            String::new()
        } else {
            format!(" [tags: {}]", self.passage.tags.join(", "))
        };

        format!(
            "[{}] ({}, score: {:.2}){}\n{}",
            timestamp, time_ago, self.relevance_score, tags, self.passage.content
        )
    }
}

/// Manages archival memory with database persistence
#[derive(Clone)]
pub struct ArchivalManager {
    agent_id: Uuid,
    db: MemoryDb,
    embedding: EmbeddingService,
}

impl ArchivalManager {
    /// Create a new archival manager for an agent
    pub fn new(agent_id: Uuid, db: MemoryDb, embedding: EmbeddingService) -> Self {
        Self {
            agent_id,
            db,
            embedding,
        }
    }

    /// Get the total number of passages
    pub fn passage_count(&self) -> usize {
        self.db
            .passages()
            .count_passages(&self.agent_id.to_string())
            .unwrap_or(0) as usize
    }

    /// Insert a new passage into archival memory with embedding
    pub async fn insert(&self, content: &str, tags: Option<Vec<String>>) -> Result<Uuid> {
        // Generate embedding
        let embedding = self.embedding.embed(content).await?;

        let tags = tags.unwrap_or_default();

        // Store in database with embedding
        let id = self.db.passages().insert_passage_with_embedding(
            &self.agent_id.to_string(),
            content,
            &embedding,
            &tags,
        )?;

        tracing::debug!("Stored passage {} with embedding in archival memory", id);
        Ok(id)
    }

    /// Search archival memory by semantic similarity
    pub async fn search(
        &self,
        query: &str,
        top_k: usize,
        tags_filter: Option<Vec<String>>,
    ) -> Result<Vec<ArchivalSearchResult>> {
        // Generate query embedding
        let query_embedding = self.embedding.embed(query).await?;

        // Search database with pgvector
        let results = self.db.passages().search_passages_by_embedding(
            &self.agent_id.to_string(),
            &query_embedding,
            top_k as i64,
            tags_filter.as_deref(),
        )?;

        // Convert to ArchivalSearchResult
        Ok(results
            .into_iter()
            .map(|(row, distance)| {
                ArchivalSearchResult {
                    passage: Passage {
                        id: row.id,
                        agent_id: self.agent_id,
                        content: row.content,
                        tags: row.tags,
                        created_at: row.created_at,
                    },
                    relevance_score: 1.0 - distance as f32, // Convert distance to similarity
                }
            })
            .collect())
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
