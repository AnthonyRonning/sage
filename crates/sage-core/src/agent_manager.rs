//! Agent Manager - Manages multiple SageAgents for multi-user support
//!
//! Each chat context (user or group) gets its own isolated agent with:
//! - Separate memory (blocks, recall, archival, summaries)
//! - Separate preferences
//! - Separate scheduled tasks
//! - Separate workspace directory

use anyhow::Result;
use chrono::Utc;
use diesel::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};
use uuid::Uuid;

use crate::config::Config;
use crate::memory::MemoryManager;
use crate::sage_agent::{SageAgent, ToolRegistry};
use crate::scheduler::SchedulerDb;
use crate::scheduler_tools;
use crate::schema::chat_contexts;
use crate::shell_tool::ShellTool;

/// Row from chat_contexts table
#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = chat_contexts)]
#[allow(dead_code)]
pub struct ChatContext {
    pub id: Uuid,
    pub signal_identifier: String,
    pub context_type: String,
    pub display_name: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
}

/// New chat context for insertion
#[derive(Insertable)]
#[diesel(table_name = chat_contexts)]
struct NewChatContext<'a> {
    pub id: Uuid,
    pub signal_identifier: &'a str,
    pub context_type: &'a str,
    pub display_name: Option<&'a str>,
}

/// Context type for chat
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum ContextType {
    Direct,
    Group,
}

impl ContextType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContextType::Direct => "direct",
            ContextType::Group => "group",
        }
    }
}

/// Cached agent with its tools and metadata
#[allow(dead_code)]
struct CachedAgent {
    agent: Arc<Mutex<SageAgent>>,
    context: ChatContext,
}

/// Manages multiple SageAgents for different chat contexts
pub struct AgentManager {
    /// Database URL for creating new memory managers
    database_url: String,
    /// Maple API configuration
    maple_api_url: String,
    maple_api_key: String,
    maple_model: String,
    maple_embedding_model: String,
    /// Brave API key for web search
    brave_api_key: Option<String>,
    /// Base workspace path
    workspace_base: PathBuf,
    /// Scheduler database (shared across all agents)
    scheduler_db: Arc<SchedulerDb>,
    /// Database connection for chat_contexts
    db_conn: Arc<std::sync::Mutex<diesel::PgConnection>>,
    /// Cached agents
    agents: Mutex<HashMap<Uuid, CachedAgent>>,
}

impl AgentManager {
    /// Create a new agent manager
    pub fn new(config: &Config, scheduler_db: Arc<SchedulerDb>) -> Result<Self> {
        let conn = diesel::PgConnection::establish(&config.database_url)?;

        // Ensure workspace base directory exists
        let workspace_base = PathBuf::from(&config.workspace_path);
        std::fs::create_dir_all(&workspace_base)?;

        let maple_api_key = config
            .maple_api_key
            .clone()
            .ok_or_else(|| anyhow::anyhow!("MAPLE_API_KEY not set"))?;

        Ok(Self {
            database_url: config.database_url.clone(),
            maple_api_url: config.maple_api_url.clone(),
            maple_api_key,
            maple_model: config.maple_model.clone(),
            maple_embedding_model: config.maple_embedding_model.clone(),
            brave_api_key: config.brave_api_key.clone(),
            workspace_base,
            scheduler_db,
            db_conn: Arc::new(std::sync::Mutex::new(conn)),
            agents: Mutex::new(HashMap::new()),
        })
    }

    /// Get or create an agent for a Signal identifier
    ///
    /// For direct messages, signal_identifier is the user's UUID
    /// For group messages, signal_identifier is the group ID
    pub async fn get_or_create_agent(
        &self,
        signal_identifier: &str,
        context_type: ContextType,
        display_name: Option<&str>,
    ) -> Result<(Uuid, Arc<Mutex<SageAgent>>)> {
        // First, look up or create the chat context
        let context = self.get_or_create_context(signal_identifier, context_type, display_name)?;
        let agent_id = context.id;

        // Check if we have a cached agent
        {
            let agents = self.agents.lock().await;
            if let Some(cached) = agents.get(&agent_id) {
                debug!("Using cached agent for {}", signal_identifier);
                return Ok((agent_id, cached.agent.clone()));
            }
        }

        // Create new agent
        info!(
            "Creating new agent for {} (id: {})",
            signal_identifier, agent_id
        );
        let agent = self.create_agent(agent_id).await?;
        let agent = Arc::new(Mutex::new(agent));

        // Cache it
        {
            let mut agents = self.agents.lock().await;
            agents.insert(
                agent_id,
                CachedAgent {
                    agent: agent.clone(),
                    context,
                },
            );
        }

        Ok((agent_id, agent))
    }

    /// Look up or create a chat context in the database
    fn get_or_create_context(
        &self,
        signal_identifier: &str,
        context_type: ContextType,
        display_name: Option<&str>,
    ) -> Result<ChatContext> {
        let mut conn = self
            .db_conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        // Try to find existing context
        let existing: Option<ChatContext> = chat_contexts::table
            .filter(chat_contexts::signal_identifier.eq(signal_identifier))
            .select(ChatContext::as_select())
            .first(&mut *conn)
            .optional()?;

        if let Some(ctx) = existing {
            debug!(
                "Found existing context for {}: {}",
                signal_identifier, ctx.id
            );
            return Ok(ctx);
        }

        // Create new context
        let new_id = Uuid::new_v4();
        info!(
            "Creating new chat context for {} -> {}",
            signal_identifier, new_id
        );

        let new_context = NewChatContext {
            id: new_id,
            signal_identifier,
            context_type: context_type.as_str(),
            display_name,
        };

        diesel::insert_into(chat_contexts::table)
            .values(&new_context)
            .execute(&mut *conn)?;

        // Return the created context
        Ok(ChatContext {
            id: new_id,
            signal_identifier: signal_identifier.to_string(),
            context_type: context_type.as_str().to_string(),
            display_name: display_name.map(|s| s.to_string()),
            created_at: Utc::now(),
        })
    }

    /// Create a new SageAgent for the given agent_id
    async fn create_agent(&self, agent_id: Uuid) -> Result<SageAgent> {
        // Create workspace directory for this agent
        let workspace = self.workspace_base.join(agent_id.to_string());
        std::fs::create_dir_all(&workspace)?;
        info!("Agent workspace: {}", workspace.display());

        // Initialize memory manager for this agent
        let memory_manager = MemoryManager::new(
            agent_id,
            &self.database_url,
            &self.maple_api_url,
            &self.maple_api_key,
            &self.maple_embedding_model,
        )
        .await?;

        // Get default timezone from preferences (or UTC)
        let default_timezone = memory_manager
            .get_preference("timezone")
            .ok()
            .flatten()
            .unwrap_or_else(|| "UTC".to_string());

        // Create tool registry
        let mut tools = ToolRegistry::new();

        // Register memory tools
        for tool in memory_manager.tools() {
            tools.register(tool);
        }

        // Register scheduler tools (with this agent's ID)
        tools.register(Arc::new(scheduler_tools::ScheduleTaskTool::new(
            self.scheduler_db.clone(),
            agent_id,
            default_timezone.clone(),
        )));
        tools.register(Arc::new(scheduler_tools::ListSchedulesTool::new(
            self.scheduler_db.clone(),
            agent_id,
        )));
        tools.register(Arc::new(scheduler_tools::CancelScheduleTool::new(
            self.scheduler_db.clone(),
        )));

        // Register shell tool with agent-specific workspace
        tools.register(Arc::new(ShellTool::new(workspace.to_string_lossy())));
        info!("Shell tool registered (workspace: {})", workspace.display());

        // Register web search if configured
        if let Some(ref api_key) = self.brave_api_key {
            tools.register(Arc::new(crate::WebSearchTool::new(api_key)?));
            debug!("Web search tool registered");
        }

        // Register done tool
        tools.register(Arc::new(crate::DoneTool));

        // Configure LLM
        SageAgent::configure_lm(&self.maple_api_url, &self.maple_api_key, &self.maple_model)
            .await?;

        // Create agent
        let agent = SageAgent::new(tools, memory_manager);

        Ok(agent)
    }

    /// Get agent_id for a signal identifier (if exists)
    #[allow(dead_code)]
    pub fn get_agent_id(&self, signal_identifier: &str) -> Result<Option<Uuid>> {
        let mut conn = self
            .db_conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let result: Option<Uuid> = chat_contexts::table
            .filter(chat_contexts::signal_identifier.eq(signal_identifier))
            .select(chat_contexts::id)
            .first(&mut *conn)
            .optional()?;

        Ok(result)
    }

    /// Get signal_identifier for an agent_id (reverse lookup for scheduled tasks)
    pub fn get_signal_identifier(&self, agent_id: Uuid) -> Result<Option<String>> {
        let mut conn = self
            .db_conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let result: Option<String> = chat_contexts::table
            .filter(chat_contexts::id.eq(agent_id))
            .select(chat_contexts::signal_identifier)
            .first(&mut *conn)
            .optional()?;

        Ok(result)
    }

    /// Get all chat contexts
    #[allow(dead_code)]
    pub fn list_contexts(&self) -> Result<Vec<ChatContext>> {
        let mut conn = self
            .db_conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire database lock"))?;

        let results = chat_contexts::table
            .select(ChatContext::as_select())
            .load(&mut *conn)?;

        Ok(results)
    }
}
