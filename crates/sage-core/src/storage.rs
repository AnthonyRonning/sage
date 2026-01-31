//! PostgreSQL storage for message history using Diesel
//!
//! This module provides basic message storage. For the full memory system
//! with embeddings and semantic search, see the `memory` module.

use anyhow::Result;
use chrono::{DateTime, Utc};
use diesel::pg::PgConnection;
use diesel::prelude::*;
use std::sync::Mutex;
use uuid::Uuid;

use crate::schema::messages;

/// Message row from the database (basic fields, embedding handled separately)
#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = messages)]
#[allow(dead_code)]
pub struct Message {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub user_id: String,
    pub role: String,
    pub content: String,
    pub sequence_id: i64,
    pub tool_calls: Option<serde_json::Value>,
    pub tool_results: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

/// New message to insert (without embedding - that's done via raw SQL)
#[derive(Insertable)]
#[diesel(table_name = messages)]
struct NewMessage<'a> {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub user_id: &'a str,
    pub role: &'a str,
    pub content: &'a str,
}

/// Message store for basic CRUD operations
/// For full memory features (embeddings, search), use MemoryManager
pub struct MessageStore {
    conn: Mutex<PgConnection>,
}

impl MessageStore {
    pub fn new(database_url: &str) -> Result<Self> {
        let conn = PgConnection::establish(database_url)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Store a message (basic, no embedding)
    /// For messages with embeddings, use MemoryManager.recall()
    pub fn store_message(
        &self,
        agent_id: Uuid,
        user_id: &str,
        role: &str,
        content: &str,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let new_message = NewMessage {
            id,
            agent_id,
            user_id,
            role,
            content,
        };

        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        diesel::insert_into(messages::table)
            .values(&new_message)
            .execute(&mut *conn)?;

        Ok(id)
    }

    #[allow(dead_code)]
    pub fn get_recent_messages(&self, agent_id: Uuid, limit: i64) -> Result<Vec<Message>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let mut results: Vec<Message> = messages::table
            .filter(messages::agent_id.eq(agent_id))
            .order(messages::sequence_id.desc())
            .limit(limit)
            .select(Message::as_select())
            .load(&mut *conn)?;

        // Reverse to get chronological order
        results.reverse();
        Ok(results)
    }

    /// Get messages by their IDs (for loading context window)
    #[allow(dead_code)]
    pub fn get_by_ids(&self, ids: &[Uuid]) -> Result<Vec<Message>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let results: Vec<Message> = messages::table
            .filter(messages::id.eq_any(ids))
            .order(messages::sequence_id.asc())
            .select(Message::as_select())
            .load(&mut *conn)?;

        Ok(results)
    }
}
