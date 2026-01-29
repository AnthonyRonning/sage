//! Core Memory Blocks
//!
//! Editable memory blocks that are always present in the system prompt.
//! Default blocks: `persona` (who the agent is) and `human` (info about user).
//!
//! Blocks are persisted to PostgreSQL and loaded on startup.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, info};
use uuid::Uuid;

use super::db::{BlockDb, MemoryDb, NewBlock};
use super::{DEFAULT_HUMAN_DESCRIPTION, DEFAULT_PERSONA_DESCRIPTION};

/// Default character limit per block (from Letta)
pub const DEFAULT_BLOCK_CHAR_LIMIT: usize = 20_000;

/// A memory block that can be edited by the agent
#[derive(Debug, Clone)]
pub struct Block {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub label: String,
    pub description: Option<String>,
    pub value: String,
    pub char_limit: usize,
    pub read_only: bool,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Block {
    /// Create a new block with default settings
    pub fn new(agent_id: Uuid, label: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            agent_id,
            label: label.into(),
            description: None,
            value: String::new(),
            char_limit: DEFAULT_BLOCK_CHAR_LIMIT,
            read_only: false,
            version: 1,
            created_at: now,
            updated_at: now,
        }
    }
    
    /// Create a new block with a description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
    
    /// Create a new block with initial value
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }
    
    /// Create a new block with a character limit
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.char_limit = limit;
        self
    }
    
    /// Create a new read-only block
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }
    
    /// Check if a new value would exceed the character limit
    pub fn would_exceed_limit(&self, new_value: &str) -> bool {
        new_value.len() > self.char_limit
    }
    
    /// Update the block's value, returning error if limit exceeded
    pub fn set_value(&mut self, new_value: impl Into<String>) -> Result<()> {
        let new_value = new_value.into();
        if new_value.len() > self.char_limit {
            return Err(anyhow!(
                "Edit failed: Exceeds {} character limit (requested {})",
                self.char_limit,
                new_value.len()
            ));
        }
        self.value = new_value;
        self.updated_at = Utc::now();
        self.version += 1;
        Ok(())
    }
    
    /// Append content to the block
    pub fn append(&mut self, content: &str) -> Result<()> {
        let new_value = if self.value.is_empty() {
            content.to_string()
        } else {
            format!("{}\n{}", self.value, content)
        };
        self.set_value(new_value)
    }
    
    /// Replace text in the block
    pub fn replace(&mut self, old: &str, new: &str) -> Result<()> {
        if !self.value.contains(old) {
            return Err(anyhow!(
                "Old content '{}' not found in memory block '{}'",
                old,
                self.label
            ));
        }
        let new_value = self.value.replace(old, new);
        self.set_value(new_value)
    }
    
    /// Insert content at a specific line (-1 for end)
    pub fn insert_at_line(&mut self, content: &str, line: i32) -> Result<()> {
        let lines: Vec<&str> = self.value.lines().collect();
        let mut new_lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        
        let insert_idx = if line < 0 {
            new_lines.len()
        } else {
            (line as usize).min(new_lines.len())
        };
        
        new_lines.insert(insert_idx, content.to_string());
        let new_value = new_lines.join("\n");
        self.set_value(new_value)
    }
    
    /// Compile this block to XML format
    pub fn compile(&self) -> String {
        let label = &self.label;
        let desc = self.description.as_deref().unwrap_or("");
        let chars_current = self.value.len();
        let chars_limit = self.char_limit;
        
        let mut s = format!("<{}>\n", label);
        s.push_str("<description>\n");
        s.push_str(desc);
        s.push_str("\n</description>\n");
        s.push_str("<metadata>");
        if self.read_only {
            s.push_str("\n- read_only=true");
        }
        s.push_str(&format!("\n- chars_current={}", chars_current));
        s.push_str(&format!("\n- chars_limit={}\n", chars_limit));
        s.push_str("</metadata>\n");
        s.push_str("<value>\n");
        s.push_str(&self.value);
        s.push_str("\n</value>\n");
        s.push_str(&format!("</{}>\n", label));
        
        s
    }
}

/// Manages memory blocks for an agent with database persistence
#[derive(Clone)]
pub struct BlockManager {
    agent_id: Uuid,
    blocks: Arc<RwLock<HashMap<String, Block>>>,
    last_modified: Arc<RwLock<Option<DateTime<Utc>>>>,
    db: MemoryDb,
}

impl BlockManager {
    /// Create a new block manager for an agent, loading from database
    pub fn new(agent_id: Uuid, db: MemoryDb) -> Result<Self> {
        let mut blocks = HashMap::new();
        let block_db = db.blocks();
        let agent_id_str = agent_id.to_string();
        
        // Load existing blocks from database
        let db_blocks = block_db.load_blocks(&agent_id_str)?;
        
        if db_blocks.is_empty() {
            info!("No existing blocks found, creating defaults for agent {}", agent_id);
            
            // Create default blocks and persist them
            let persona = Block::new(agent_id, "persona")
                .with_description(DEFAULT_PERSONA_DESCRIPTION)
                .with_value("I am Sage, a helpful AI assistant communicating via Signal. I maintain long-term memory across our conversations and strive to be friendly, concise, and genuinely helpful.");
            
            let human = Block::new(agent_id, "human")
                .with_description(DEFAULT_HUMAN_DESCRIPTION);
            
            // Persist default blocks
            Self::persist_block_to_db(&block_db, &agent_id_str, &persona)?;
            Self::persist_block_to_db(&block_db, &agent_id_str, &human)?;
            
            blocks.insert("persona".to_string(), persona);
            blocks.insert("human".to_string(), human);
        } else {
            info!("Loaded {} blocks from database for agent {}", db_blocks.len(), agent_id);
            
            // Convert DB rows to Block structs
            for row in db_blocks {
                let block = Block {
                    id: row.id,
                    agent_id,
                    label: row.label.clone(),
                    description: row.description,
                    value: row.value,
                    char_limit: row.char_limit as usize,
                    read_only: row.read_only,
                    version: row.version,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                };
                debug!("  Block '{}': {} chars", row.label, block.value.len());
                blocks.insert(row.label, block);
            }
        }
        
        Ok(Self {
            agent_id,
            blocks: Arc::new(RwLock::new(blocks)),
            last_modified: Arc::new(RwLock::new(None)),
            db,
        })
    }
    
    /// Persist a block to the database (used during initialization)
    fn persist_block_to_db(db: &BlockDb, agent_id: &str, block: &Block) -> Result<()> {
        db.upsert_block(NewBlock {
            id: block.id,
            agent_id,
            label: &block.label,
            description: block.description.as_deref(),
            value: &block.value,
            char_limit: block.char_limit as i32,
            read_only: block.read_only,
        })?;
        Ok(())
    }
    
    /// Persist block value to database after modification
    fn persist_block(&self, label: &str, value: &str) -> Result<()> {
        let agent_id_str = self.agent_id.to_string();
        self.db.blocks().update_block_value(&agent_id_str, label, value)?;
        debug!("Persisted block '{}' to database ({} chars)", label, value.len());
        Ok(())
    }
    
    /// Get a block by label
    pub fn get(&self, label: &str) -> Option<Block> {
        self.blocks.read().ok()?.get(label).cloned()
    }
    
    /// Get all blocks
    pub fn all(&self) -> Vec<Block> {
        self.blocks.read().ok()
            .map(|b| b.values().cloned().collect())
            .unwrap_or_default()
    }
    
    /// Check if a block exists
    pub fn has(&self, label: &str) -> bool {
        self.blocks.read().ok()
            .map(|b| b.contains_key(label))
            .unwrap_or(false)
    }
    
    /// Update a block's value
    pub fn update(&self, label: &str, value: impl Into<String>) -> Result<()> {
        let value = value.into();
        
        let mut blocks = self.blocks.write()
            .map_err(|_| anyhow!("Failed to acquire write lock"))?;
        
        let block = blocks.get_mut(label)
            .ok_or_else(|| anyhow!("Block '{}' not found", label))?;
        
        if block.read_only {
            return Err(anyhow!("Block '{}' is read-only", label));
        }
        
        block.set_value(&value)?;
        
        // Update last modified timestamp
        if let Ok(mut last_mod) = self.last_modified.write() {
            *last_mod = Some(Utc::now());
        }
        
        // Persist to database
        drop(blocks); // Release lock before DB operation
        self.persist_block(label, &value)?;
        
        Ok(())
    }
    
    /// Replace text in a block
    pub fn replace(&self, label: &str, old: &str, new: &str) -> Result<()> {
        let new_value = {
            let mut blocks = self.blocks.write()
                .map_err(|_| anyhow!("Failed to acquire write lock"))?;
            
            let block = blocks.get_mut(label)
                .ok_or_else(|| anyhow!("Block '{}' not found", label))?;
            
            if block.read_only {
                return Err(anyhow!("Block '{}' is read-only", label));
            }
            
            block.replace(old, new)?;
            
            if let Ok(mut last_mod) = self.last_modified.write() {
                *last_mod = Some(Utc::now());
            }
            
            block.value.clone()
        };
        
        // Persist to database (lock already released)
        self.persist_block(label, &new_value)?;
        
        Ok(())
    }
    
    /// Append to a block
    pub fn append(&self, label: &str, content: &str) -> Result<()> {
        let new_value = {
            let mut blocks = self.blocks.write()
                .map_err(|_| anyhow!("Failed to acquire write lock"))?;
            
            let block = blocks.get_mut(label)
                .ok_or_else(|| anyhow!("Block '{}' not found", label))?;
            
            if block.read_only {
                return Err(anyhow!("Block '{}' is read-only", label));
            }
            
            block.append(content)?;
            
            if let Ok(mut last_mod) = self.last_modified.write() {
                *last_mod = Some(Utc::now());
            }
            
            block.value.clone()
        };
        
        // Persist to database (lock already released)
        self.persist_block(label, &new_value)?;
        
        Ok(())
    }
    
    /// Insert at a specific line in a block
    pub fn insert_at_line(&self, label: &str, content: &str, line: i32) -> Result<()> {
        let new_value = {
            let mut blocks = self.blocks.write()
                .map_err(|_| anyhow!("Failed to acquire write lock"))?;
            
            let block = blocks.get_mut(label)
                .ok_or_else(|| anyhow!("Block '{}' not found", label))?;
            
            if block.read_only {
                return Err(anyhow!("Block '{}' is read-only", label));
            }
            
            block.insert_at_line(content, line)?;
            
            if let Ok(mut last_mod) = self.last_modified.write() {
                *last_mod = Some(Utc::now());
            }
            
            block.value.clone()
        };
        
        // Persist to database (lock already released)
        self.persist_block(label, &new_value)?;
        
        Ok(())
    }
    
    /// Add a new block
    pub fn add(&self, block: Block) -> Result<()> {
        {
            let mut blocks = self.blocks.write()
                .map_err(|_| anyhow!("Failed to acquire write lock"))?;
            
            if blocks.contains_key(&block.label) {
                return Err(anyhow!("Block '{}' already exists", block.label));
            }
            
            blocks.insert(block.label.clone(), block.clone());
        }
        
        // Persist to database (lock released)
        let agent_id_str = self.agent_id.to_string();
        Self::persist_block_to_db(&self.db.blocks(), &agent_id_str, &block)?;
        
        Ok(())
    }
    
    /// Get the last modified timestamp
    pub fn last_modified(&self) -> Option<DateTime<Utc>> {
        self.last_modified.read().ok().and_then(|lm| *lm)
    }
    
    /// Compile all blocks to XML format for system prompt
    pub fn compile(&self) -> String {
        let blocks = match self.blocks.read() {
            Ok(b) => b,
            Err(_) => return String::new(),
        };
        
        if blocks.is_empty() {
            return String::new();
        }
        
        let mut s = String::from("<memory_blocks>\nThe following memory blocks are currently engaged in your core memory unit:\n\n");
        
        // Sort by label for consistent ordering (persona first, then human, then others)
        let mut labels: Vec<_> = blocks.keys().collect();
        labels.sort_by(|a, b| {
            match (a.as_str(), b.as_str()) {
                ("persona", _) => std::cmp::Ordering::Less,
                (_, "persona") => std::cmp::Ordering::Greater,
                ("human", _) => std::cmp::Ordering::Less,
                (_, "human") => std::cmp::Ordering::Greater,
                _ => a.cmp(b),
            }
        });
        
        for (idx, label) in labels.iter().enumerate() {
            if let Some(block) = blocks.get(*label) {
                s.push_str(&block.compile());
                if idx < labels.len() - 1 {
                    s.push('\n');
                }
            }
        }
        
        s.push_str("\n</memory_blocks>");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_creation() {
        let agent_id = Uuid::new_v4();
        let block = Block::new(agent_id, "test")
            .with_description("A test block")
            .with_value("Hello, world!");
        
        assert_eq!(block.label, "test");
        assert_eq!(block.description, Some("A test block".to_string()));
        assert_eq!(block.value, "Hello, world!");
        assert_eq!(block.char_limit, DEFAULT_BLOCK_CHAR_LIMIT);
        assert!(!block.read_only);
    }
    
    #[test]
    fn test_block_char_limit() {
        let agent_id = Uuid::new_v4();
        let mut block = Block::new(agent_id, "test").with_limit(10);
        
        assert!(block.set_value("12345").is_ok());
        assert!(block.set_value("12345678901").is_err()); // 11 chars > 10 limit
    }
    
    #[test]
    fn test_block_replace() {
        let agent_id = Uuid::new_v4();
        let mut block = Block::new(agent_id, "test")
            .with_value("Hello, world!");
        
        assert!(block.replace("world", "Sage").is_ok());
        assert_eq!(block.value, "Hello, Sage!");
        
        assert!(block.replace("notfound", "test").is_err());
    }
    
    #[test]
    fn test_block_append() {
        let agent_id = Uuid::new_v4();
        let mut block = Block::new(agent_id, "test")
            .with_value("Line 1");
        
        assert!(block.append("Line 2").is_ok());
        assert_eq!(block.value, "Line 1\nLine 2");
    }
    
    #[test]
    fn test_block_insert_at_line() {
        let agent_id = Uuid::new_v4();
        let mut block = Block::new(agent_id, "test")
            .with_value("Line 1\nLine 3");
        
        assert!(block.insert_at_line("Line 2", 1).is_ok());
        assert_eq!(block.value, "Line 1\nLine 2\nLine 3");
    }
    
    #[test]
    fn test_block_compile() {
        let agent_id = Uuid::new_v4();
        let block = Block::new(agent_id, "test")
            .with_description("A test block")
            .with_value("Test value");
        
        let compiled = block.compile();
        assert!(compiled.contains("<test>"));
        assert!(compiled.contains("</test>"));
        assert!(compiled.contains("<description>"));
        assert!(compiled.contains("A test block"));
        assert!(compiled.contains("<value>"));
        assert!(compiled.contains("Test value"));
    }
}
