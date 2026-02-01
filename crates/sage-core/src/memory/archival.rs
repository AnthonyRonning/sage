//! Archival Memory (Long-term Semantic Storage)
//!
//! Agent-created long-term memories stored with embeddings for semantic search.
//! Uses pgvector for efficient similarity queries.
//!
//! Note: This is an in-memory implementation kept for reference.
//! The production implementation is in archival_new.rs using PostgreSQL.

#![allow(dead_code)]

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

/// A passage in archival memory
#[derive(Debug, Clone)]
pub struct Passage {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl Passage {
    /// Create a new passage
    pub fn new(agent_id: Uuid, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            agent_id,
            content: content.into(),
            embedding: None,
            tags: Vec::new(),
            created_at: Utc::now(),
        }
    }

    /// Add tags to the passage
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Set the embedding
    pub fn with_embedding(mut self, embedding: Vec<f32>) -> Self {
        self.embedding = Some(embedding);
        self
    }
}

/// Search result from archival memory
#[derive(Debug, Clone)]
pub struct ArchivalSearchResult {
    pub passage: Passage,
    pub relevance_score: Option<f32>,
    pub time_ago: String,
}

impl ArchivalSearchResult {
    /// Format the search result for display to the agent
    pub fn format(&self) -> String {
        let timestamp = self.passage.created_at.format("%Y-%m-%d %H:%M:%S UTC");
        let tags = if self.passage.tags.is_empty() {
            String::new()
        } else {
            format!(" [tags: {}]", self.passage.tags.join(", "))
        };

        format!(
            "[{}] ({}){}\n{}",
            timestamp, self.time_ago, tags, self.passage.content
        )
    }
}

/// Manages archival memory (long-term semantic storage)
#[derive(Clone)]
pub struct ArchivalManager {
    agent_id: Uuid,
    passages: Arc<RwLock<Vec<Passage>>>,
    embedding_api_url: String,
    embedding_api_key: String,
    // TODO: Database connection for persistence
}

impl ArchivalManager {
    /// Create a new archival manager for an agent
    pub async fn new(
        agent_id: Uuid,
        _db_url: &str,
        embedding_api_url: &str,
        embedding_api_key: &str,
    ) -> Result<Self> {
        // TODO: Load passages from database

        Ok(Self {
            agent_id,
            passages: Arc::new(RwLock::new(Vec::new())),
            embedding_api_url: embedding_api_url.to_string(),
            embedding_api_key: embedding_api_key.to_string(),
        })
    }

    /// Get the total number of passages
    pub fn passage_count(&self) -> usize {
        self.passages.read().ok().map(|p| p.len()).unwrap_or(0)
    }

    /// Get all unique tags across all passages
    #[allow(dead_code)]
    pub fn all_tags(&self) -> Vec<String> {
        self.passages
            .read()
            .ok()
            .map(|passages| {
                let mut tags: Vec<String> = passages
                    .iter()
                    .flat_map(|p| p.tags.iter().cloned())
                    .collect();
                tags.sort();
                tags.dedup();
                tags
            })
            .unwrap_or_default()
    }

    /// Insert a new passage into archival memory
    pub async fn insert(&self, content: &str, tags: Option<Vec<String>>) -> Result<Uuid> {
        let mut passage = Passage::new(self.agent_id, content);

        if let Some(t) = tags {
            passage = passage.with_tags(t);
        }

        // Generate embedding
        let embedding = self.generate_embedding(content).await?;
        passage = passage.with_embedding(embedding);

        let id = passage.id;

        // Store in memory
        let mut passages = self
            .passages
            .write()
            .map_err(|_| anyhow::anyhow!("Failed to acquire write lock"))?;
        passages.push(passage);

        // TODO: Persist to database with pgvector

        Ok(id)
    }

    /// Search archival memory by semantic similarity
    pub async fn search(
        &self,
        query: &str,
        top_k: usize,
        tags: Option<Vec<String>>,
    ) -> Result<Vec<ArchivalSearchResult>> {
        // Generate query embedding
        let query_embedding = self.generate_embedding(query).await?;

        let passages = self
            .passages
            .read()
            .map_err(|_| anyhow::anyhow!("Failed to acquire read lock"))?;

        let now = Utc::now();

        // Score all passages by cosine similarity
        let mut scored: Vec<(f32, &Passage)> = passages
            .iter()
            .filter(|p| {
                // Filter by tags if specified
                if let Some(ref filter_tags) = tags {
                    filter_tags.iter().any(|t| p.tags.contains(t))
                } else {
                    true
                }
            })
            .filter_map(|p| {
                p.embedding.as_ref().map(|emb| {
                    let score = cosine_similarity(&query_embedding, emb);
                    (score, p)
                })
            })
            .collect();

        // Sort by score (highest first)
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // Take top_k results
        let results: Vec<ArchivalSearchResult> = scored
            .into_iter()
            .take(top_k)
            .map(|(score, p)| {
                let time_ago = format_time_ago(p.created_at, now);
                ArchivalSearchResult {
                    passage: p.clone(),
                    relevance_score: Some(score),
                    time_ago,
                }
            })
            .collect();

        Ok(results)
    }

    /// Generate an embedding for text
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        // TODO: Call Maple embedding API (maple/nomic-embed-text)
        // For now, return a placeholder embedding

        let client = reqwest::Client::new();

        let response = client
            .post(format!("{}/embeddings", self.embedding_api_url))
            .header(
                "Authorization",
                format!("Bearer {}", self.embedding_api_key),
            )
            .json(&serde_json::json!({
                "model": "nomic-embed-text",
                "input": text
            }))
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let json: serde_json::Value = resp.json().await?;
                    if let Some(embedding) = json["data"][0]["embedding"].as_array() {
                        let vec: Vec<f32> = embedding
                            .iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect();
                        return Ok(vec);
                    }
                }
                // Fall back to placeholder if API call fails
                tracing::warn!("Embedding API call failed, using placeholder");
                Ok(vec![0.0; 768]) // nomic-embed-text dimension
            }
            Err(e) => {
                tracing::warn!("Failed to generate embedding: {}, using placeholder", e);
                Ok(vec![0.0; 768])
            }
        }
    }
}

/// Calculate cosine similarity between two vectors
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 0.001);

        let c = vec![0.0, 1.0, 0.0];
        assert!((cosine_similarity(&a, &c)).abs() < 0.001);

        let d = vec![-1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &d) + 1.0).abs() < 0.001);
    }

    #[test]
    fn test_passage_creation() {
        let agent_id = Uuid::new_v4();
        let passage = Passage::new(agent_id, "Test content")
            .with_tags(vec!["tag1".to_string(), "tag2".to_string()]);

        assert_eq!(passage.content, "Test content");
        assert_eq!(passage.tags, vec!["tag1", "tag2"]);
    }
}
