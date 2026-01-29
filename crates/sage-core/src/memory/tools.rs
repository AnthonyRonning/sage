//! Memory Tools
//!
//! Tools that allow the agent to manipulate its memory:
//! - memory_replace, memory_append, memory_insert (core memory)
//! - conversation_search (recall memory + summaries)
//! - archival_insert, archival_search (archival memory)

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use uuid::Uuid;

use crate::sage_agent::{Tool, ToolResult};
use super::block::BlockManager;
use super::recall_new::RecallManager;
use super::archival_new::ArchivalManager;
use super::db::MemoryDb;
use super::EmbeddingService;

// ============================================================================
// Core Memory Tools
// ============================================================================

/// Replace text in a memory block
pub struct MemoryReplaceTool {
    blocks: BlockManager,
}

impl MemoryReplaceTool {
    pub fn new(blocks: BlockManager) -> Self {
        Self { blocks }
    }
}

#[async_trait]
impl Tool for MemoryReplaceTool {
    fn name(&self) -> &str {
        "memory_replace"
    }
    
    fn description(&self) -> &str {
        "Replace text in a memory block. Requires exact match of old text."
    }
    
    fn args_schema(&self) -> &str {
        r#"{"block": "block label (e.g., 'persona', 'human')", "old": "exact text to find", "new": "replacement text"}"#
    }
    
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let block = args.get("block")
            .ok_or_else(|| anyhow::anyhow!("'block' argument required"))?;
        let old = args.get("old")
            .ok_or_else(|| anyhow::anyhow!("'old' argument required"))?;
        let new = args.get("new")
            .ok_or_else(|| anyhow::anyhow!("'new' argument required"))?;
        
        match self.blocks.replace(block, old, new) {
            Ok(()) => Ok(ToolResult::success(format!(
                "Successfully replaced text in '{}' block.",
                block
            ))),
            Err(e) => Ok(ToolResult::error(e.to_string())),
        }
    }
}

/// Append text to a memory block
pub struct MemoryAppendTool {
    blocks: BlockManager,
}

impl MemoryAppendTool {
    pub fn new(blocks: BlockManager) -> Self {
        Self { blocks }
    }
}

#[async_trait]
impl Tool for MemoryAppendTool {
    fn name(&self) -> &str {
        "memory_append"
    }
    
    fn description(&self) -> &str {
        "Append text to the end of a memory block."
    }
    
    fn args_schema(&self) -> &str {
        r#"{"block": "block label (e.g., 'persona', 'human')", "content": "text to append"}"#
    }
    
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let block = args.get("block")
            .ok_or_else(|| anyhow::anyhow!("'block' argument required"))?;
        let content = args.get("content")
            .ok_or_else(|| anyhow::anyhow!("'content' argument required"))?;
        
        match self.blocks.append(block, content) {
            Ok(()) => Ok(ToolResult::success(format!(
                "Successfully appended to '{}' block.",
                block
            ))),
            Err(e) => Ok(ToolResult::error(e.to_string())),
        }
    }
}

/// Insert text at a specific line in a memory block
pub struct MemoryInsertTool {
    blocks: BlockManager,
}

impl MemoryInsertTool {
    pub fn new(blocks: BlockManager) -> Self {
        Self { blocks }
    }
}

#[async_trait]
impl Tool for MemoryInsertTool {
    fn name(&self) -> &str {
        "memory_insert"
    }
    
    fn description(&self) -> &str {
        "Insert text at a specific line in a memory block. Use line=-1 for end."
    }
    
    fn args_schema(&self) -> &str {
        r#"{"block": "block label", "content": "text to insert", "line": "line number (0-indexed, -1 for end)"}"#
    }
    
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let block = args.get("block")
            .ok_or_else(|| anyhow::anyhow!("'block' argument required"))?;
        let content = args.get("content")
            .ok_or_else(|| anyhow::anyhow!("'content' argument required"))?;
        let line: i32 = args.get("line")
            .and_then(|l| l.parse().ok())
            .unwrap_or(-1);
        
        match self.blocks.insert_at_line(block, content, line) {
            Ok(()) => Ok(ToolResult::success(format!(
                "Successfully inserted text into '{}' block at line {}.",
                block,
                if line < 0 { "end".to_string() } else { line.to_string() }
            ))),
            Err(e) => Ok(ToolResult::error(e.to_string())),
        }
    }
}

// ============================================================================
// Recall Memory Tools
// ============================================================================

/// Search conversation history (including messages AND summaries)
pub struct ConversationSearchTool {
    recall: RecallManager,
    agent_id: Uuid,
    db: MemoryDb,
    embedding: EmbeddingService,
}

impl ConversationSearchTool {
    pub fn new(recall: RecallManager) -> Self {
        Self { 
            recall: recall.clone(),
            agent_id: recall.agent_id(),
            db: recall.db(),
            embedding: recall.embedding_service(),
        }
    }
    
    /// Search summaries by semantic similarity
    async fn search_summaries(&self, query: &str, limit: usize) -> Result<Vec<super::db::SummarySearchResult>> {
        let embedding = self.embedding.embed(query).await?;
        self.db.summaries().search_by_embedding(self.agent_id, &embedding, limit as i64)
    }
}

#[async_trait]
impl Tool for ConversationSearchTool {
    fn name(&self) -> &str {
        "conversation_search"
    }
    
    fn description(&self) -> &str {
        "Search through past conversation history, including older summarized conversations. Returns matching messages and summaries with relevance scores."
    }
    
    fn args_schema(&self) -> &str {
        r#"{"query": "search query", "limit": "max results (default 5)"}"#
    }
    
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let query = args.get("query")
            .ok_or_else(|| anyhow::anyhow!("'query' argument required"))?;
        let limit: usize = args.get("limit")
            .and_then(|l| l.parse().ok())
            .unwrap_or(5);
        
        let mut output = String::new();
        let mut total_results = 0;
        
        // Search messages
        match self.recall.search(query, limit).await {
            Ok(results) => {
                if !results.is_empty() {
                    total_results += results.len();
                    output.push_str(&format!("=== Messages ({}) ===\n\n", results.len()));
                    for (i, result) in results.iter().enumerate() {
                        output.push_str(&format!("{}. {}\n\n", i + 1, result.format()));
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Message search failed: {}", e);
            }
        }
        
        // Search summaries (older compacted history)
        match self.search_summaries(query, limit).await {
            Ok(results) => {
                if !results.is_empty() {
                    total_results += results.len();
                    output.push_str(&format!("=== Conversation Summaries ({}) ===\n\n", results.len()));
                    for (i, result) in results.iter().enumerate() {
                        output.push_str(&format!(
                            "{}. [Summary of messages {}-{}] (relevance: {:.2})\n{}\n\n",
                            i + 1,
                            result.summary.from_sequence_id,
                            result.summary.to_sequence_id,
                            1.0 - result.distance, // Convert distance to similarity
                            result.summary.content
                        ));
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Summary search failed: {}", e);
            }
        }
        
        if total_results == 0 {
            return Ok(ToolResult::success("No matching messages or summaries found.".to_string()));
        }
        
        Ok(ToolResult::success(output))
    }
}

// ============================================================================
// Archival Memory Tools
// ============================================================================

/// Insert content into archival memory
pub struct ArchivalInsertTool {
    archival: ArchivalManager,
}

impl ArchivalInsertTool {
    pub fn new(archival: ArchivalManager) -> Self {
        Self { archival }
    }
}

#[async_trait]
impl Tool for ArchivalInsertTool {
    fn name(&self) -> &str {
        "archival_insert"
    }
    
    fn description(&self) -> &str {
        "Store information in long-term archival memory for future recall. Good for important facts, preferences, and details you want to remember."
    }
    
    fn args_schema(&self) -> &str {
        r#"{"content": "text to store", "tags": "optional comma-separated tags"}"#
    }
    
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let content = args.get("content")
            .ok_or_else(|| anyhow::anyhow!("'content' argument required"))?;
        
        let tags = args.get("tags")
            .map(|t| t.split(',').map(|s| s.trim().to_string()).collect());
        
        match self.archival.insert(content, tags).await {
            Ok(id) => Ok(ToolResult::success(format!(
                "Successfully stored in archival memory (id: {}).",
                id
            ))),
            Err(e) => Ok(ToolResult::error(e.to_string())),
        }
    }
}

/// Search archival memory
pub struct ArchivalSearchTool {
    archival: ArchivalManager,
}

impl ArchivalSearchTool {
    pub fn new(archival: ArchivalManager) -> Self {
        Self { archival }
    }
}

#[async_trait]
impl Tool for ArchivalSearchTool {
    fn name(&self) -> &str {
        "archival_search"
    }
    
    fn description(&self) -> &str {
        "Search long-term archival memory using semantic similarity. Returns most relevant stored memories."
    }
    
    fn args_schema(&self) -> &str {
        r#"{"query": "search query", "top_k": "max results (default 5)", "tags": "optional comma-separated tags to filter by"}"#
    }
    
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let query = args.get("query")
            .ok_or_else(|| anyhow::anyhow!("'query' argument required"))?;
        let top_k: usize = args.get("top_k")
            .and_then(|k| k.parse().ok())
            .unwrap_or(5);
        let tags = args.get("tags")
            .map(|t| t.split(',').map(|s| s.trim().to_string()).collect());
        
        match self.archival.search(query, top_k, tags).await {
            Ok(results) => {
                if results.is_empty() {
                    return Ok(ToolResult::success("No matching memories found.".to_string()));
                }
                
                let mut output = format!("Found {} matching memories:\n\n", results.len());
                for (i, result) in results.iter().enumerate() {
                    output.push_str(&format!("{}. {}\n\n", i + 1, result.format()));
                }
                Ok(ToolResult::success(output))
            }
            Err(e) => Ok(ToolResult::error(e.to_string())),
        }
    }
}

// ============================================================================
// User Preference Tools
// ============================================================================

/// Set a user preference
pub struct SetPreferenceTool {
    db: MemoryDb,
    agent_id: Uuid,
}

impl SetPreferenceTool {
    pub fn new(db: MemoryDb, agent_id: Uuid) -> Self {
        Self { db, agent_id }
    }
}

#[async_trait]
impl Tool for SetPreferenceTool {
    fn name(&self) -> &str {
        "set_preference"
    }
    
    fn description(&self) -> &str {
        "Set a user preference. Known keys: 'timezone' (IANA format like 'America/Chicago'), 'language' (ISO code like 'en'), 'display_name'. Other keys are also allowed."
    }
    
    fn args_schema(&self) -> &str {
        r#"{"key": "preference key (e.g., 'timezone', 'language', 'display_name')", "value": "preference value"}"#
    }
    
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let key = args.get("key")
            .ok_or_else(|| anyhow::anyhow!("'key' argument required"))?;
        let value = args.get("value")
            .ok_or_else(|| anyhow::anyhow!("'value' argument required"))?;
        
        match self.db.preferences().set(self.agent_id, key, value) {
            Ok(pref) => Ok(ToolResult::success(format!(
                "Preference '{}' set to '{}' (updated: {})",
                pref.key, pref.value, pref.updated_at.format("%Y-%m-%d %H:%M UTC")
            ))),
            Err(e) => Ok(ToolResult::error(e.to_string())),
        }
    }
}

// Tests require a real database connection
// Integration tests should be in tests/ directory
