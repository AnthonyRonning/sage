//! Recall Memory (Conversation History Search)
//!
//! Provides searchable access to the full conversation history.
//! Supports both keyword matching and semantic search via embeddings.

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

/// A message in recall memory
#[derive(Debug, Clone)]
pub struct RecallMessage {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub sequence_id: i64,
}

/// Search result from recall memory
#[derive(Debug, Clone)]
pub struct RecallSearchResult {
    pub message: RecallMessage,
    pub relevance_score: Option<f32>,
    pub time_ago: String,
}

impl RecallSearchResult {
    /// Format the search result for display to the agent
    pub fn format(&self) -> String {
        let timestamp = self.message.created_at.format("%Y-%m-%d %H:%M:%S UTC");
        let role = &self.message.role;
        let content = &self.message.content;
        
        let mut result = format!("[{}] ({}, {})\n", timestamp, self.time_ago, role);
        
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

/// Manages recall memory (conversation history search)
#[derive(Clone)]
pub struct RecallManager {
    agent_id: Uuid,
    // In-memory cache of recent messages
    messages: Arc<RwLock<Vec<RecallMessage>>>,
    // Embedding API configuration
    #[allow(dead_code)]
    embedding_api_url: String,
    #[allow(dead_code)]
    embedding_api_key: String,
    // TODO: Database connection for persistence
}

impl RecallManager {
    /// Create a new recall manager for an agent
    pub async fn new(
        agent_id: Uuid,
        _db_url: &str,
        embedding_api_url: &str,
        embedding_api_key: &str,
    ) -> Result<Self> {
        // TODO: Load messages from database
        
        Ok(Self {
            agent_id,
            messages: Arc::new(RwLock::new(Vec::new())),
            embedding_api_url: embedding_api_url.to_string(),
            embedding_api_key: embedding_api_key.to_string(),
        })
    }
    
    /// Get the total number of messages in recall memory
    pub fn message_count(&self) -> usize {
        self.messages.read().ok()
            .map(|m| m.len())
            .unwrap_or(0)
    }
    
    /// Add a message to recall memory
    pub fn add_message(&self, role: &str, content: &str) -> Result<Uuid> {
        let mut messages = self.messages.write()
            .map_err(|_| anyhow::anyhow!("Failed to acquire write lock"))?;
        
        let sequence_id = messages.len() as i64;
        let message = RecallMessage {
            id: Uuid::new_v4(),
            agent_id: self.agent_id,
            role: role.to_string(),
            content: content.to_string(),
            created_at: Utc::now(),
            sequence_id,
        };
        
        let id = message.id;
        messages.push(message);
        
        // TODO: Persist to database
        // TODO: Generate and store embedding
        
        Ok(id)
    }
    
    /// Search recall memory by keyword
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<RecallSearchResult>> {
        let messages = self.messages.read()
            .map_err(|_| anyhow::anyhow!("Failed to acquire read lock"))?;
        
        let query_lower = query.to_lowercase();
        let now = Utc::now();
        
        let mut results: Vec<RecallSearchResult> = messages
            .iter()
            .filter(|m| {
                // Filter out tool messages and conversation_search calls
                if m.role == "tool" {
                    return false;
                }
                if m.content.contains("conversation_search") {
                    return false;
                }
                // Keyword match
                m.content.to_lowercase().contains(&query_lower)
            })
            .map(|m| {
                let time_ago = format_time_ago(m.created_at, now);
                RecallSearchResult {
                    message: m.clone(),
                    relevance_score: None, // TODO: Add semantic similarity score
                    time_ago,
                }
            })
            .collect();
        
        // Sort by recency (most recent first)
        results.sort_by(|a, b| b.message.created_at.cmp(&a.message.created_at));
        
        // Limit results
        results.truncate(limit);
        
        Ok(results)
    }
    
    /// Search recall memory with semantic similarity
    #[allow(dead_code)]
    pub async fn search_semantic(&self, query: &str, limit: usize) -> Result<Vec<RecallSearchResult>> {
        // TODO: Implement semantic search using embeddings
        // 1. Generate embedding for query
        // 2. Search pgvector for similar messages
        // 3. Combine with keyword search (hybrid)
        
        // For now, fall back to keyword search
        self.search(query, limit)
    }
    
    /// Get recent messages (for context building)
    pub fn get_recent(&self, limit: usize) -> Vec<RecallMessage> {
        self.messages.read().ok()
            .map(|m| {
                m.iter()
                    .rev()
                    .take(limit)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect()
            })
            .unwrap_or_default()
    }
    
    /// Get messages by IDs (for loading in-context messages)
    #[allow(dead_code)]
    pub fn get_by_ids(&self, ids: &[Uuid]) -> Vec<RecallMessage> {
        self.messages.read().ok()
            .map(|messages| {
                ids.iter()
                    .filter_map(|id| messages.iter().find(|m| m.id == *id).cloned())
                    .collect()
            })
            .unwrap_or_default()
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

    #[tokio::test]
    async fn test_recall_manager() {
        let agent_id = Uuid::new_v4();
        let manager = RecallManager::new(
            agent_id, 
            "", 
            "http://localhost:8080/v1",
            "test-key"
        ).await.unwrap();
        
        assert_eq!(manager.message_count(), 0);
        
        manager.add_message("user", "Hello, how are you?").unwrap();
        manager.add_message("assistant", "I'm doing well, thank you!").unwrap();
        
        assert_eq!(manager.message_count(), 2);
        
        let results = manager.search("hello", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].message.role, "user");
    }
    
    #[test]
    fn test_format_time_ago() {
        let now = Utc::now();
        let five_min_ago = now - chrono::Duration::minutes(5);
        let two_hours_ago = now - chrono::Duration::hours(2);
        let three_days_ago = now - chrono::Duration::days(3);
        
        assert_eq!(format_time_ago(five_min_ago, now), "5m ago");
        assert_eq!(format_time_ago(two_hours_ago, now), "2h ago");
        assert_eq!(format_time_ago(three_days_ago, now), "3d ago");
    }
}
