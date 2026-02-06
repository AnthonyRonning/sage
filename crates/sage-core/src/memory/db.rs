//! Database persistence layer for the memory system
//!
//! Provides Diesel-based CRUD operations for blocks, passages, and agents.

#![allow(dead_code)]

use anyhow::Result;
use chrono::{DateTime, Utc};
use diesel::pg::PgConnection;
use diesel::prelude::*;
use diesel::sql_types::{Array, Double, Text, Timestamptz, Uuid as DieselUuid};

use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::schema::{agents, blocks, passages, summaries, user_preferences};
// ============================================================================
// Block Database Operations
// ============================================================================

/// Block row from the database
#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = blocks)]
pub struct BlockRow {
    pub id: Uuid,
    pub agent_id: String,
    pub label: String,
    pub description: Option<String>,
    pub value: String,
    pub char_limit: i32,
    pub read_only: bool,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// New block to insert
#[derive(Insertable)]
#[diesel(table_name = blocks)]
pub struct NewBlock<'a> {
    pub id: Uuid,
    pub agent_id: &'a str,
    pub label: &'a str,
    pub description: Option<&'a str>,
    pub value: &'a str,
    pub char_limit: i32,
    pub read_only: bool,
}

/// Block update changeset
#[derive(AsChangeset)]
#[diesel(table_name = blocks)]
pub struct BlockUpdate<'a> {
    pub value: Option<&'a str>,
    pub description: Option<Option<&'a str>>,
}

/// Database operations for blocks
pub struct BlockDb {
    conn: Arc<Mutex<PgConnection>>,
}

impl BlockDb {
    pub fn new(conn: Arc<Mutex<PgConnection>>) -> Self {
        Self { conn }
    }

    /// Load all blocks for an agent
    pub fn load_blocks(&self, agent_id: &str) -> Result<Vec<BlockRow>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let results = blocks::table
            .filter(blocks::agent_id.eq(agent_id))
            .select(BlockRow::as_select())
            .load(&mut *conn)?;

        Ok(results)
    }

    /// Get a single block by agent and label
    pub fn get_block(&self, agent_id: &str, label: &str) -> Result<Option<BlockRow>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let result = blocks::table
            .filter(blocks::agent_id.eq(agent_id))
            .filter(blocks::label.eq(label))
            .select(BlockRow::as_select())
            .first(&mut *conn)
            .optional()?;

        Ok(result)
    }

    /// Insert a new block
    pub fn insert_block(&self, block: NewBlock) -> Result<BlockRow> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let result = diesel::insert_into(blocks::table)
            .values(&block)
            .get_result(&mut *conn)?;

        Ok(result)
    }

    /// Update a block's value
    pub fn update_block_value(&self, agent_id: &str, label: &str, value: &str) -> Result<BlockRow> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let result = diesel::update(blocks::table)
            .filter(blocks::agent_id.eq(agent_id))
            .filter(blocks::label.eq(label))
            .set(blocks::value.eq(value))
            .get_result(&mut *conn)?;

        Ok(result)
    }

    /// Upsert a block (insert or update)
    pub fn upsert_block(&self, block: NewBlock) -> Result<BlockRow> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let result = diesel::insert_into(blocks::table)
            .values(&block)
            .on_conflict((blocks::agent_id, blocks::label))
            .do_update()
            .set((
                blocks::value.eq(&block.value),
                blocks::description.eq(&block.description),
                blocks::char_limit.eq(&block.char_limit),
                blocks::read_only.eq(&block.read_only),
            ))
            .get_result(&mut *conn)?;

        Ok(result)
    }
}

// ============================================================================
// Passage Database Operations
// ============================================================================

/// Passage row from the database (without embedding for simpler queries)
/// Note: We manually handle this due to pgvector complexity
#[derive(Debug, Clone)]
pub struct PassageRow {
    pub id: Uuid,
    pub agent_id: String,
    pub content: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// Database operations for passages
pub struct PassageDb {
    conn: Arc<Mutex<PgConnection>>,
}

impl PassageDb {
    pub fn new(conn: Arc<Mutex<PgConnection>>) -> Self {
        Self { conn }
    }

    /// Count passages for an agent
    pub fn count_passages(&self, agent_id: &str) -> Result<i64> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let count: i64 = passages::table
            .filter(passages::agent_id.eq(agent_id))
            .count()
            .get_result(&mut *conn)?;

        Ok(count)
    }

    /// Insert a passage with embedding using raw SQL
    pub fn insert_passage_with_embedding(
        &self,
        agent_id: &str,
        content: &str,
        embedding: &[f32],
        tags: &[String],
    ) -> Result<Uuid> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let id = Uuid::new_v4();
        let embedding_str = format!(
            "[{}]",
            embedding
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        let tags_array = tags
            .iter()
            .map(|t| format!("'{}'", t.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");

        diesel::sql_query(format!(
            "INSERT INTO passages (id, agent_id, content, embedding, tags) \
             VALUES ('{}', '{}', '{}', '{}', ARRAY[{}]::text[])",
            id,
            agent_id.replace('\'', "''"),
            content.replace('\'', "''"),
            embedding_str,
            tags_array
        ))
        .execute(&mut *conn)?;

        Ok(id)
    }

    /// Search passages by vector similarity using raw SQL
    pub fn search_passages_by_embedding(
        &self,
        agent_id: &str,
        query_embedding: &[f32],
        limit: i64,
        tags_filter: Option<&[String]>,
    ) -> Result<Vec<(PassageRow, f64)>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let embedding_str = format!(
            "[{}]",
            query_embedding
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );

        let tags_clause = if let Some(tags) = tags_filter {
            if tags.is_empty() {
                String::new()
            } else {
                let tags_array = tags
                    .iter()
                    .map(|t| format!("'{}'", t.replace('\'', "''")))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(" AND tags && ARRAY[{}]::text[]", tags_array)
            }
        } else {
            String::new()
        };

        // Use cosine distance (smaller is better, 0 = identical)
        let query = format!(
            "SELECT id, agent_id, content, tags, created_at, \
                    (embedding <=> '{}') as distance \
             FROM passages \
             WHERE agent_id = '{}'{} \
             ORDER BY distance \
             LIMIT {}",
            embedding_str,
            agent_id.replace('\'', "''"),
            tags_clause,
            limit
        );

        // Execute raw query and parse results
        #[allow(clippy::type_complexity)]
        let results: Vec<(Uuid, String, String, Vec<String>, DateTime<Utc>, f64)> =
            diesel::sql_query(&query)
                .load::<PassageSearchRow>(&mut *conn)?
                .into_iter()
                .map(|row| {
                    (
                        row.id,
                        row.agent_id,
                        row.content,
                        row.tags,
                        row.created_at,
                        row.distance,
                    )
                })
                .collect();

        Ok(results
            .into_iter()
            .map(|(id, agent_id, content, tags, created_at, distance)| {
                (
                    PassageRow {
                        id,
                        agent_id,
                        content,
                        tags,
                        created_at,
                    },
                    distance,
                )
            })
            .collect())
    }
}

/// Helper struct for passage search results with distance
#[derive(QueryableByName, Debug)]
struct PassageSearchRow {
    #[diesel(sql_type = DieselUuid)]
    id: Uuid,
    #[diesel(sql_type = Text)]
    agent_id: String,
    #[diesel(sql_type = Text)]
    content: String,
    #[diesel(sql_type = Array<Text>)]
    tags: Vec<String>,
    #[diesel(sql_type = Timestamptz)]
    created_at: DateTime<Utc>,
    #[diesel(sql_type = Double)]
    distance: f64,
}

// ============================================================================
// Agent Database Operations
// ============================================================================

/// Agent data (manually managed due to Array<Uuid> complexity)
#[derive(Debug, Clone)]
pub struct AgentRow {
    pub id: Uuid,
    pub name: String,
    pub system_prompt: String,
    pub message_ids: Vec<Uuid>,
    pub llm_config: serde_json::Value,
    pub last_memory_update: Option<DateTime<Utc>>,
    pub max_context_tokens: i32,
    pub compaction_threshold: f32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Database operations for agents
pub struct AgentDb {
    conn: Arc<Mutex<PgConnection>>,
}

impl AgentDb {
    pub fn new(conn: Arc<Mutex<PgConnection>>) -> Self {
        Self { conn }
    }

    /// Get an agent by ID using raw SQL
    #[allow(dead_code)]
    pub fn get_agent(&self, agent_id: Uuid) -> Result<Option<AgentRow>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        // Use raw SQL to avoid Array<Uuid> type issues
        let exists: bool = diesel::dsl::select(diesel::dsl::exists(
            agents::table.filter(agents::id.eq(agent_id)),
        ))
        .get_result(&mut *conn)?;

        if !exists {
            return Ok(None);
        }

        // TODO: Full implementation with raw SQL to parse message_ids array
        // For now, return a basic agent with empty message_ids
        Ok(None)
    }

    /// Create a new agent using raw SQL
    pub fn create_agent(&self, id: Uuid, name: &str, system_prompt: &str) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        diesel::sql_query(format!(
            "INSERT INTO agents (id, name, system_prompt, llm_config) \
             VALUES ('{}', '{}', '{}', '{{}}')",
            id,
            name.replace('\'', "''"),
            system_prompt.replace('\'', "''"),
        ))
        .execute(&mut *conn)?;

        Ok(())
    }

    /// Ensure an agent exists in the database, creating it if necessary
    pub fn ensure_agent_exists(&self, id: Uuid, name: &str) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        // Check if agent exists
        let exists: bool =
            diesel::dsl::select(diesel::dsl::exists(agents::table.filter(agents::id.eq(id))))
                .get_result(&mut *conn)?;

        if !exists {
            // Create the agent with minimal data
            diesel::sql_query(format!(
                "INSERT INTO agents (id, name, system_prompt, llm_config) \
                 VALUES ('{}', '{}', '', '{{}}')",
                id,
                name.replace('\'', "''"),
            ))
            .execute(&mut *conn)?;
            tracing::info!("Created agent {} in database", id);
        }

        Ok(())
    }

    /// Update agent's message_ids using raw SQL
    pub fn update_message_ids(&self, agent_id: Uuid, message_ids: &[Uuid]) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let ids_str = message_ids
            .iter()
            .map(|id| format!("'{}'", id))
            .collect::<Vec<_>>()
            .join(",");

        diesel::sql_query(format!(
            "UPDATE agents SET message_ids = ARRAY[{}]::uuid[] WHERE id = '{}'",
            ids_str, agent_id
        ))
        .execute(&mut *conn)?;

        Ok(())
    }

    /// Update agent's last memory update timestamp
    pub fn update_last_memory_update(&self, agent_id: Uuid) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        diesel::update(agents::table)
            .filter(agents::id.eq(agent_id))
            .set(agents::last_memory_update.eq(Some(Utc::now())))
            .execute(&mut *conn)?;

        Ok(())
    }
}

// ============================================================================
// Message Database Operations (for Recall Memory)
// ============================================================================

/// Message data with embedding support
#[derive(Debug, Clone)]
pub struct MessageRow {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub user_id: String,
    pub role: String,
    pub content: String,
    pub sequence_id: i64,
    pub tool_calls: Option<serde_json::Value>,
    pub tool_results: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub attachment_text: Option<String>,
}

/// Message search result with similarity score
#[derive(Debug, Clone)]
pub struct MessageSearchResult {
    pub message: MessageRow,
    pub distance: f64, // Cosine distance (smaller = more similar)
}

/// Database operations for messages (recall memory)
pub struct MessageDb {
    conn: Arc<Mutex<PgConnection>>,
}

impl MessageDb {
    pub fn new(conn: Arc<Mutex<PgConnection>>) -> Self {
        Self { conn }
    }

    /// Insert a message with embedding
    #[allow(clippy::too_many_arguments)]
    pub fn insert_message(
        &self,
        agent_id: Uuid,
        user_id: &str,
        role: &str,
        content: &str,
        embedding: &[f32],
        tool_calls: Option<&serde_json::Value>,
        tool_results: Option<&serde_json::Value>,
        attachment_text: Option<&str>,
    ) -> Result<Uuid> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let id = Uuid::new_v4();
        let embedding_str = format!(
            "[{}]",
            embedding
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );

        let tool_calls_str = tool_calls
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string());
        let tool_results_str = tool_results
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string());

        let attachment_text_str = attachment_text
            .map(|t| format!("'{}'", t.replace('\'', "''")))
            .unwrap_or_else(|| "NULL".to_string());

        diesel::sql_query(format!(
            "INSERT INTO messages (id, agent_id, user_id, role, content, embedding, tool_calls, tool_results, attachment_text) \
             VALUES ('{}', '{}', '{}', '{}', '{}', '{}', '{}', '{}', {})",
            id,
            agent_id,
            user_id.replace('\'', "''"),
            role.replace('\'', "''"),
            content.replace('\'', "''"),
            embedding_str,
            tool_calls_str.replace('\'', "''"),
            tool_results_str.replace('\'', "''"),
            attachment_text_str,
        ))
        .execute(&mut *conn)?;

        Ok(id)
    }

    /// Get messages by IDs (for loading context window)
    pub fn get_by_ids(&self, ids: &[Uuid]) -> Result<Vec<MessageRow>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        use crate::schema::messages;

        #[derive(Queryable)]
        struct RawMessage {
            id: Uuid,
            agent_id: Uuid,
            user_id: String,
            role: String,
            content: String,
            sequence_id: i64,
            tool_calls: Option<serde_json::Value>,
            tool_results: Option<serde_json::Value>,
            created_at: DateTime<Utc>,
            attachment_text: Option<String>,
        }

        let results: Vec<RawMessage> = messages::table
            .filter(messages::id.eq_any(ids))
            .order(messages::sequence_id.asc())
            .select((
                messages::id,
                messages::agent_id,
                messages::user_id,
                messages::role,
                messages::content,
                messages::sequence_id,
                messages::tool_calls,
                messages::tool_results,
                messages::created_at,
                messages::attachment_text,
            ))
            .load(&mut *conn)?;

        Ok(results
            .into_iter()
            .map(|r| MessageRow {
                id: r.id,
                agent_id: r.agent_id,
                user_id: r.user_id,
                role: r.role,
                content: r.content,
                sequence_id: r.sequence_id,
                tool_calls: r.tool_calls,
                tool_results: r.tool_results,
                created_at: r.created_at,
                attachment_text: r.attachment_text,
            })
            .collect())
    }

    /// Search messages by vector similarity
    pub fn search_by_embedding(
        &self,
        agent_id: Uuid,
        query_embedding: &[f32],
        limit: i64,
    ) -> Result<Vec<MessageSearchResult>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let embedding_str = format!(
            "[{}]",
            query_embedding
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );

        // Raw SQL for pgvector cosine distance search
        let query = format!(
            "SELECT id, agent_id, user_id, role, content, sequence_id, \
                    tool_calls, tool_results, created_at, \
                    (embedding <=> '{}') as distance \
             FROM messages \
             WHERE agent_id = '{}' AND embedding IS NOT NULL \
             ORDER BY distance \
             LIMIT {}",
            embedding_str, agent_id, limit
        );

        // TODO: Execute raw query and parse results
        // For now, return empty - need custom result parsing for pgvector
        let _ = query;
        let _ = &mut *conn;
        Ok(Vec::new())
    }

    /// Count messages for an agent
    pub fn count_messages(&self, agent_id: Uuid) -> Result<i64> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        use crate::schema::messages;

        let count: i64 = messages::table
            .filter(messages::agent_id.eq(agent_id))
            .count()
            .get_result(&mut *conn)?;

        Ok(count)
    }

    /// Get recent messages for an agent
    pub fn get_recent(&self, agent_id: Uuid, limit: i64) -> Result<Vec<MessageRow>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        use crate::schema::messages;

        #[derive(Queryable)]
        struct RawMessage {
            id: Uuid,
            agent_id: Uuid,
            user_id: String,
            role: String,
            content: String,
            sequence_id: i64,
            tool_calls: Option<serde_json::Value>,
            tool_results: Option<serde_json::Value>,
            created_at: DateTime<Utc>,
            attachment_text: Option<String>,
        }

        let mut results: Vec<RawMessage> = messages::table
            .filter(messages::agent_id.eq(agent_id))
            .order(messages::sequence_id.desc())
            .limit(limit)
            .select((
                messages::id,
                messages::agent_id,
                messages::user_id,
                messages::role,
                messages::content,
                messages::sequence_id,
                messages::tool_calls,
                messages::tool_results,
                messages::created_at,
                messages::attachment_text,
            ))
            .load(&mut *conn)?;

        results.reverse(); // Chronological order

        Ok(results
            .into_iter()
            .map(|r| MessageRow {
                id: r.id,
                agent_id: r.agent_id,
                user_id: r.user_id,
                role: r.role,
                content: r.content,
                sequence_id: r.sequence_id,
                tool_calls: r.tool_calls,
                tool_results: r.tool_results,
                created_at: r.created_at,
                attachment_text: r.attachment_text,
            })
            .collect())
    }

    /// Update embedding for an existing message (for background processing)
    pub fn update_embedding(&self, message_id: Uuid, embedding: &[f32]) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let embedding_str = format!(
            "[{}]",
            embedding
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );

        diesel::sql_query(format!(
            "UPDATE messages SET embedding = '{}' WHERE id = '{}'",
            embedding_str, message_id,
        ))
        .execute(&mut *conn)?;

        Ok(())
    }
}

// ============================================================================
// Summary Database Operations (for Compaction)
// ============================================================================

/// Summary row from the database
#[derive(Debug, Clone)]
pub struct SummaryRow {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub from_sequence_id: i64,
    pub to_sequence_id: i64,
    pub content: String,
    pub previous_summary_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// Summary search result with similarity score
#[derive(Debug, Clone)]
pub struct SummarySearchResult {
    pub summary: SummaryRow,
    pub distance: f64,
}

/// Helper struct for summary search results
#[derive(QueryableByName, Debug)]
struct SummarySearchRow {
    #[diesel(sql_type = DieselUuid)]
    id: Uuid,
    #[diesel(sql_type = DieselUuid)]
    agent_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Int8)]
    from_sequence_id: i64,
    #[diesel(sql_type = diesel::sql_types::Int8)]
    to_sequence_id: i64,
    #[diesel(sql_type = Text)]
    content: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<DieselUuid>)]
    previous_summary_id: Option<Uuid>,
    #[diesel(sql_type = Timestamptz)]
    created_at: DateTime<Utc>,
    #[diesel(sql_type = Double)]
    distance: f64,
}

/// Database operations for summaries
pub struct SummaryDb {
    conn: Arc<Mutex<PgConnection>>,
}

impl SummaryDb {
    pub fn new(conn: Arc<Mutex<PgConnection>>) -> Self {
        Self { conn }
    }

    /// Insert a new summary with embedding
    pub fn insert_summary(
        &self,
        agent_id: Uuid,
        from_sequence_id: i64,
        to_sequence_id: i64,
        content: &str,
        embedding: &[f32],
        previous_summary_id: Option<Uuid>,
    ) -> Result<Uuid> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let id = Uuid::new_v4();
        let embedding_str = format!(
            "[{}]",
            embedding
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );

        let prev_id_str = previous_summary_id
            .map(|id| format!("'{}'", id))
            .unwrap_or_else(|| "NULL".to_string());

        diesel::sql_query(format!(
            "INSERT INTO summaries (id, agent_id, from_sequence_id, to_sequence_id, content, embedding, previous_summary_id) \
             VALUES ('{}', '{}', {}, {}, '{}', '{}', {})",
            id,
            agent_id,
            from_sequence_id,
            to_sequence_id,
            content.replace('\'', "''"),
            embedding_str,
            prev_id_str,
        ))
        .execute(&mut *conn)?;

        Ok(id)
    }

    /// Get the latest summary for an agent (highest to_sequence_id)
    pub fn get_latest(&self, agent_id: Uuid) -> Result<Option<SummaryRow>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        #[derive(Queryable)]
        struct RawSummary {
            id: Uuid,
            agent_id: Uuid,
            from_sequence_id: i64,
            to_sequence_id: i64,
            content: String,
            previous_summary_id: Option<Uuid>,
            created_at: DateTime<Utc>,
        }

        let result: Option<RawSummary> = summaries::table
            .filter(summaries::agent_id.eq(agent_id))
            .order(summaries::to_sequence_id.desc())
            .select((
                summaries::id,
                summaries::agent_id,
                summaries::from_sequence_id,
                summaries::to_sequence_id,
                summaries::content,
                summaries::previous_summary_id,
                summaries::created_at,
            ))
            .first(&mut *conn)
            .optional()?;

        Ok(result.map(|r| SummaryRow {
            id: r.id,
            agent_id: r.agent_id,
            from_sequence_id: r.from_sequence_id,
            to_sequence_id: r.to_sequence_id,
            content: r.content,
            previous_summary_id: r.previous_summary_id,
            created_at: r.created_at,
        }))
    }

    /// Search summaries by vector similarity
    pub fn search_by_embedding(
        &self,
        agent_id: Uuid,
        query_embedding: &[f32],
        limit: i64,
    ) -> Result<Vec<SummarySearchResult>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let embedding_str = format!(
            "[{}]",
            query_embedding
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );

        let query = format!(
            "SELECT id, agent_id, from_sequence_id, to_sequence_id, content, \
                    previous_summary_id, created_at, \
                    (embedding <=> '{}') as distance \
             FROM summaries \
             WHERE agent_id = '{}' AND embedding IS NOT NULL \
             ORDER BY distance \
             LIMIT {}",
            embedding_str, agent_id, limit
        );

        let results: Vec<SummarySearchRow> = diesel::sql_query(&query).load(&mut *conn)?;

        Ok(results
            .into_iter()
            .map(|row| SummarySearchResult {
                summary: SummaryRow {
                    id: row.id,
                    agent_id: row.agent_id,
                    from_sequence_id: row.from_sequence_id,
                    to_sequence_id: row.to_sequence_id,
                    content: row.content,
                    previous_summary_id: row.previous_summary_id,
                    created_at: row.created_at,
                },
                distance: row.distance,
            })
            .collect())
    }

    /// Get messages after a specific sequence ID (for loading context after summary)
    pub fn get_messages_after_sequence(
        &self,
        agent_id: Uuid,
        after_sequence_id: i64,
        limit: i64,
    ) -> Result<Vec<MessageRow>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        use crate::schema::messages;

        #[derive(Queryable)]
        struct RawMessage {
            id: Uuid,
            agent_id: Uuid,
            user_id: String,
            role: String,
            content: String,
            sequence_id: i64,
            tool_calls: Option<serde_json::Value>,
            tool_results: Option<serde_json::Value>,
            created_at: DateTime<Utc>,
            attachment_text: Option<String>,
        }

        let results: Vec<RawMessage> = messages::table
            .filter(messages::agent_id.eq(agent_id))
            .filter(messages::sequence_id.gt(after_sequence_id))
            .order(messages::sequence_id.asc())
            .limit(limit)
            .select((
                messages::id,
                messages::agent_id,
                messages::user_id,
                messages::role,
                messages::content,
                messages::sequence_id,
                messages::tool_calls,
                messages::tool_results,
                messages::created_at,
                messages::attachment_text,
            ))
            .load(&mut *conn)?;

        Ok(results
            .into_iter()
            .map(|r| MessageRow {
                id: r.id,
                agent_id: r.agent_id,
                user_id: r.user_id,
                role: r.role,
                content: r.content,
                sequence_id: r.sequence_id,
                tool_calls: r.tool_calls,
                tool_results: r.tool_results,
                created_at: r.created_at,
                attachment_text: r.attachment_text,
            })
            .collect())
    }

    /// Get the maximum sequence_id for an agent's messages
    pub fn get_max_sequence_id(&self, agent_id: Uuid) -> Result<Option<i64>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        use crate::schema::messages;
        use diesel::dsl::max;

        let result: Option<i64> = messages::table
            .filter(messages::agent_id.eq(agent_id))
            .select(max(messages::sequence_id))
            .first(&mut *conn)?;

        Ok(result)
    }
}

// ============================================================================
// User Preferences Database Operations
// ============================================================================

/// Known preference keys (validated at code level)
pub mod preference_keys {
    /// User's timezone (IANA format, e.g., "America/Chicago")
    pub const TIMEZONE: &str = "timezone";
    /// User's preferred language (ISO 639-1, e.g., "en", "es")
    pub const LANGUAGE: &str = "language";
    /// User's preferred name/nickname
    pub const DISPLAY_NAME: &str = "display_name";
}

/// Preference row from the database
#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = user_preferences)]
pub struct PreferenceRow {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub key: String,
    pub value: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// New preference to insert
#[derive(Insertable)]
#[diesel(table_name = user_preferences)]
pub struct NewPreference<'a> {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub key: &'a str,
    pub value: &'a str,
}

/// Database operations for user preferences
pub struct PreferenceDb {
    conn: Arc<Mutex<PgConnection>>,
}

impl PreferenceDb {
    pub fn new(conn: Arc<Mutex<PgConnection>>) -> Self {
        Self { conn }
    }

    /// Validate a preference value for known keys
    pub fn validate(key: &str, value: &str) -> Result<()> {
        match key {
            preference_keys::TIMEZONE => {
                // Validate IANA timezone
                value.parse::<chrono_tz::Tz>()
                    .map_err(|_| anyhow::anyhow!(
                        "Invalid timezone '{}'. Use IANA format like 'America/Chicago' or 'Europe/London'", 
                        value
                    ))?;
                Ok(())
            }
            preference_keys::LANGUAGE => {
                // Basic validation: 2-3 letter language code
                if value.len() >= 2
                    && value.len() <= 3
                    && value.chars().all(|c| c.is_ascii_lowercase())
                {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(
                        "Invalid language code '{}'. Use ISO 639-1 format like 'en' or 'es'",
                        value
                    ))
                }
            }
            preference_keys::DISPLAY_NAME => {
                // Basic validation: not empty, reasonable length
                if value.is_empty() {
                    Err(anyhow::anyhow!("Display name cannot be empty"))
                } else if value.len() > 100 {
                    Err(anyhow::anyhow!(
                        "Display name too long (max 100 characters)"
                    ))
                } else {
                    Ok(())
                }
            }
            _ => Ok(()), // Unknown keys pass through (forward compatible)
        }
    }

    /// Set a preference (insert or update)
    pub fn set(&self, agent_id: Uuid, key: &str, value: &str) -> Result<PreferenceRow> {
        // Validate known keys
        Self::validate(key, value)?;

        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let now = Utc::now();

        // Upsert: insert or update on conflict
        let result = diesel::insert_into(user_preferences::table)
            .values(NewPreference {
                id: Uuid::new_v4(),
                agent_id,
                key,
                value,
            })
            .on_conflict((user_preferences::agent_id, user_preferences::key))
            .do_update()
            .set((
                user_preferences::value.eq(value),
                user_preferences::updated_at.eq(now),
            ))
            .get_result(&mut *conn)?;

        Ok(result)
    }

    /// Get a single preference by key
    pub fn get(&self, agent_id: Uuid, key: &str) -> Result<Option<PreferenceRow>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let result = user_preferences::table
            .filter(user_preferences::agent_id.eq(agent_id))
            .filter(user_preferences::key.eq(key))
            .select(PreferenceRow::as_select())
            .first(&mut *conn)
            .optional()?;

        Ok(result)
    }

    /// Get all preferences for an agent
    pub fn get_all(&self, agent_id: Uuid) -> Result<Vec<PreferenceRow>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let results = user_preferences::table
            .filter(user_preferences::agent_id.eq(agent_id))
            .select(PreferenceRow::as_select())
            .load(&mut *conn)?;

        Ok(results)
    }

    /// Delete a preference
    pub fn delete(&self, agent_id: Uuid, key: &str) -> Result<bool> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let deleted = diesel::delete(
            user_preferences::table
                .filter(user_preferences::agent_id.eq(agent_id))
                .filter(user_preferences::key.eq(key)),
        )
        .execute(&mut *conn)?;

        Ok(deleted > 0)
    }
}

// ============================================================================
// Shared Database Connection
// ============================================================================

/// Shared database connection for the memory system
#[derive(Clone)]
pub struct MemoryDb {
    conn: Arc<Mutex<PgConnection>>,
}

impl MemoryDb {
    /// Create a new memory database connection
    pub fn new(database_url: &str) -> Result<Self> {
        let conn = PgConnection::establish(database_url)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Get block database operations
    pub fn blocks(&self) -> BlockDb {
        BlockDb::new(Arc::clone(&self.conn))
    }

    /// Get passage database operations
    pub fn passages(&self) -> PassageDb {
        PassageDb::new(Arc::clone(&self.conn))
    }

    /// Get agent database operations
    pub fn agents(&self) -> AgentDb {
        AgentDb::new(Arc::clone(&self.conn))
    }

    /// Get message database operations
    pub fn messages(&self) -> MessageDb {
        MessageDb::new(Arc::clone(&self.conn))
    }

    /// Get summary database operations
    pub fn summaries(&self) -> SummaryDb {
        SummaryDb::new(Arc::clone(&self.conn))
    }

    /// Get preference database operations
    pub fn preferences(&self) -> PreferenceDb {
        PreferenceDb::new(Arc::clone(&self.conn))
    }
}
