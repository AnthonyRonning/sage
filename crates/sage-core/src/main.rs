use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod agent_manager;
mod config;
mod memory;
mod sage_agent;
mod schema;
mod scheduler;
mod scheduler_tools;
mod shell_tool;
mod signal;
mod storage;

use agent_manager::{AgentManager, ContextType};
use sage_agent::{SageAgent, Tool, ToolResult};
use signal::{IncomingMessage, SignalClient, run_receive_loop, run_receive_loop_tcp};

/// Done tool - signals the agent is finished and doesn't need to send another message
pub struct DoneTool;

#[async_trait::async_trait]
impl Tool for DoneTool {
    fn name(&self) -> &str {
        "done"
    }

    fn description(&self) -> &str {
        "No-op signal. Use ONLY when messages is [] AND no other tools needed. Indicates nothing to do this turn."
    }

    fn args_schema(&self) -> &str {
        r#"{}"#
    }

    async fn execute(&self, _args: &std::collections::HashMap<String, String>) -> Result<ToolResult> {
        Ok(ToolResult::success("Done.".to_string()))
    }
}

/// Web search tool implementation using Brave Search API (Pro)
pub struct WebSearchTool {
    client: Arc<sage_tools::BraveClient>,
}

impl WebSearchTool {
    pub fn new(api_key: &str) -> Result<Self> {
        Ok(Self {
            client: Arc::new(sage_tools::BraveClient::new(api_key.to_string())?),
        })
    }
}

#[async_trait::async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web with AI summaries, real-time data (weather, stocks, sports), and rich results. \
         Use 'freshness' for time-sensitive queries, 'location' for local results."
    }

    fn args_schema(&self) -> &str {
        r#"{ "query": "search query", "count": "results (default 10)", "freshness": "pd=24h, pw=week, pm=month (optional)", "location": "city or 'city, state' for local results (optional)" }"#
    }

    async fn execute(&self, args: &std::collections::HashMap<String, String>) -> Result<ToolResult> {
        let query = args
            .get("query")
            .ok_or_else(|| anyhow::anyhow!("query argument required"))?;

        let options = sage_tools::SearchOptions {
            count: args.get("count").and_then(|c| c.parse().ok()),
            freshness: args.get("freshness").cloned(),
            location: args.get("location").cloned(),
            timezone: None,
        };

        match self.client.search(query, Some(options)).await {
            Ok(results) => {
                let formatted = results.format_results();
                Ok(ToolResult::success(formatted))
            }
            Err(e) => Ok(ToolResult::error(format!("Search failed: {}", e))),
        }
    }
}

/// Check if a user is allowed to interact with Sage
fn is_user_allowed(user_id: &str, allowed_users: &[String]) -> bool {
    // "*" means allow all users
    if allowed_users.iter().any(|u| u == "*") {
        return true;
    }
    // Empty list also means allow all (legacy behavior)
    if allowed_users.is_empty() {
        return true;
    }
    // Check if user is in allowed list
    allowed_users.iter().any(|u| u == user_id)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "sage=debug,info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("🌿 Sage starting up...");

    // Load configuration
    dotenvy::dotenv().ok();
    let config = config::Config::from_env()?;

    info!("Configuration loaded");
    info!("  Maple API: {}", config.maple_api_url);
    info!("  Model: {}", config.maple_model);

    // Run database migrations first
    {
        use diesel::prelude::*;
        use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
        pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");
        
        let mut conn = diesel::PgConnection::establish(&config.database_url)?;
        conn.run_pending_migrations(MIGRATIONS)
            .map_err(|e| anyhow::anyhow!("Migration failed: {}", e))?;
        info!("Database migrations applied");
    }

    let api_key = config
        .maple_api_key
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("MAPLE_API_KEY not set"))?;

    // Configure DSRs LM globally (required before creating agents)
    SageAgent::configure_lm(&config.maple_api_url, api_key, &config.maple_model).await?;
    info!("DSRs LM configured");

    // Check for Brave Search
    if config.brave_api_key.is_some() {
        info!("Brave Search enabled");
    } else {
        warn!("BRAVE_API_KEY not set - web search disabled");
    }

    // Initialize scheduler (shared across all agents)
    let scheduler_db = Arc::new(scheduler::SchedulerDb::connect(&config.database_url)?);
    
    // Create agent manager
    let agent_manager = Arc::new(AgentManager::new(&config, scheduler_db.clone())?);
    info!("Agent manager initialized (workspace: {})", config.workspace_path);

    // Check if Signal is configured
    let signal_phone = match &config.signal_phone_number {
        Some(phone) => phone.clone(),
        None => {
            warn!("SIGNAL_PHONE_NUMBER not set - cannot start Signal interface");
            info!("Set SIGNAL_PHONE_NUMBER in .env to enable messaging.");
            tokio::signal::ctrl_c().await?;
            return Ok(());
        }
    };

    // Create channel for incoming messages
    let (tx, mut rx) = mpsc::channel::<IncomingMessage>(100);

    // Start Signal client
    let (signal_client, receive_handle) = if let Some(ref host) = config.signal_cli_host {
        info!(
            "Starting Signal interface (TCP mode: {}:{})...",
            host, config.signal_cli_port
        );

        let signal_client = SignalClient::connect_tcp(&signal_phone, host, config.signal_cli_port)?;
        let signal_client = Arc::new(Mutex::new(signal_client));

        let host = host.clone();
        let port = config.signal_cli_port;
        let account = signal_phone.clone();
        let receive_handle =
            tokio::spawn(async move { run_receive_loop_tcp(&host, port, &account, tx).await });

        (signal_client, receive_handle)
    } else {
        info!("Starting Signal interface (subprocess mode)...");

        let signal_client = SignalClient::spawn_subprocess(&signal_phone)?;
        let reader = signal_client.take_reader()?;
        let signal_client = Arc::new(Mutex::new(signal_client));

        let receive_handle = tokio::spawn(async move { run_receive_loop(reader, tx).await });

        (signal_client, receive_handle)
    };

    // Log allowed users configuration
    if config.signal_allowed_users.iter().any(|u| u == "*") {
        info!("Allowed users: * (all users)");
    } else if config.signal_allowed_users.is_empty() {
        warn!("⚠️  No SIGNAL_ALLOWED_USERS configured - Sage will respond to ANYONE!");
    } else {
        info!("Allowed users: {:?}", config.signal_allowed_users);
    }

    info!("🌿 Sage is awake and listening on Signal!");

    // Start background scheduler
    let mut scheduler_rx = scheduler::spawn_scheduler(scheduler_db.clone(), 30);
    info!("Background scheduler started (polling every 30s)");

    // Main event loop
    loop {
        tokio::select! {
            // Handle scheduled task events
            Some(event) = scheduler_rx.recv() => {
                let task = event.task;
                info!("⏰ Processing scheduled task: {} ({})", task.description, task.task_type.as_str());
                
                // For scheduled tasks, we need to find the right agent
                // The task has an agent_id, so we look up which signal_identifier that maps to
                // For now, we'll handle this by looking at which context the task belongs to
                
                // TODO: Store signal_identifier in task payload or maintain reverse lookup
                // For now, scheduled tasks still need the old behavior
                warn!("Scheduled task processing not yet updated for multi-agent - skipping");
            }
            
            // Handle incoming messages
            Some(msg) = rx.recv() => {
                // Check if sender is allowed
                if !is_user_allowed(&msg.source, &config.signal_allowed_users) {
                    warn!("🚫 Ignoring message from unauthorized user: {}", msg.source);
                    continue;
                }

                let user_name = msg.source_name.as_deref().unwrap_or(&msg.source);
                info!("Processing message from {}...", user_name);

                // Get or create agent for this user
                let (agent_id, agent) = match agent_manager.get_or_create_agent(
                    &msg.source,
                    ContextType::Direct,
                    msg.source_name.as_deref(),
                ).await {
                    Ok(result) => result,
                    Err(e) => {
                        error!("Failed to get/create agent for {}: {}", msg.source, e);
                        continue;
                    }
                };
                
                info!("Using agent {} for user {}", agent_id, user_name);

                // Store incoming message (sync, no embedding) and update embedding in background
                let user_msg_id = {
                    let agent_guard = agent.lock().await;
                    match agent_guard.store_message_sync(&msg.source, "user", &msg.message) {
                        Ok(msg_id) => {
                            tracing::debug!("Stored user message {}", msg_id);
                            Some(msg_id)
                        }
                        Err(e) => {
                            error!("Failed to store message: {}", e);
                            None
                        }
                    }
                };
                
                // Update embedding in background
                if let Some(msg_id) = user_msg_id {
                    let agent_clone = agent.clone();
                    let content = msg.message.clone();
                    tokio::spawn(async move {
                        let agent_guard = agent_clone.lock().await;
                        if let Err(e) = agent_guard.update_message_embedding(msg_id, &content).await {
                            tracing::warn!("Failed to update embedding for user message: {}", e);
                        }
                    });
                }

                // Send typing indicator
                {
                    let client = signal_client.lock().await;
                    let _ = client.send_typing(&msg.source, false);
                }

                // Process message with agent
                let recipient = msg.source.clone();
                let user_message = msg.message.clone();
                
                let mut had_error = false;
                let max_steps = 10;
                
                for step_num in 0..max_steps {
                    let step_result = {
                        let mut agent_guard = agent.lock().await;
                        agent_guard.step(&user_message, step_num == 0).await
                    };
                    
                    match step_result {
                        Ok(result) => {
                            // Send messages from this step IMMEDIATELY (don't wait for storage)
                            let msg_count = result.messages.len();
                            let mut messages_to_store: Vec<String> = Vec::new();
                            
                            for (i, response) in result.messages.iter().enumerate() {
                                let log_preview: String = response.chars().take(50).collect();
                                info!("📤 Sending response ({}/{}): {}...", i + 1, msg_count, log_preview);

                                // Send reply FIRST (don't block on storage)
                                {
                                    let client = signal_client.lock().await;
                                    if let Err(e) = client.send_message(&recipient, response) {
                                        error!("Failed to send reply: {}", e);
                                    }
                                }
                                
                                // Queue for background storage
                                messages_to_store.push(response.clone());

                                // Delay between multiple messages for natural feel
                                if i < msg_count - 1 {
                                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                                    {
                                        let client = signal_client.lock().await;
                                        let _ = client.send_typing(&recipient, false);
                                    }
                                    tokio::time::sleep(tokio::time::Duration::from_millis(1450)).await;
                                }
                            }
                            
                            // Stop typing indicator
                            if msg_count > 0 {
                                let client = signal_client.lock().await;
                                let _ = client.send_typing(&recipient, true);
                            }
                            
                            // Store messages: sync DB write, then async embedding update
                            if !messages_to_store.is_empty() {
                                let agent_clone = agent.clone();
                                let recipient_clone = recipient.clone();
                                tokio::spawn(async move {
                                    for response in messages_to_store {
                                        // Sync store (fast, no embedding)
                                        let msg_id = {
                                            let agent_guard = agent_clone.lock().await;
                                            agent_guard.store_message_sync(&recipient_clone, "assistant", &response)
                                        };
                                        
                                        // Async embedding update
                                        if let Ok(msg_id) = msg_id {
                                            let agent_guard = agent_clone.lock().await;
                                            if let Err(e) = agent_guard.update_message_embedding(msg_id, &response).await {
                                                tracing::warn!("Failed to update embedding: {}", e);
                                            }
                                        }
                                    }
                                });
                            }
                            
                            // Store executed tool calls in background (still uses async store_tool_message)
                            if !result.executed_tools.is_empty() {
                                let agent_clone = agent.clone();
                                let recipient_clone = recipient.clone();
                                let executed_tools = result.executed_tools.clone();
                                tokio::spawn(async move {
                                    let agent_guard = agent_clone.lock().await;
                                    for executed in &executed_tools {
                                        if let Err(e) = agent_guard.store_tool_message(&recipient_clone, &executed.tool_call, &executed.result).await {
                                            error!("Failed to store tool message: {}", e);
                                        }
                                    }
                                });
                                info!("🔧 Queued {} tool calls for storage", result.executed_tools.len());
                            }
                            
                            // If done, break
                            if result.done {
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Agent error at step {}: {}", step_num, e);
                            had_error = true;
                            break;
                        }
                    }
                }
                
                if had_error {
                    let client = signal_client.lock().await;
                    let _ = client.send_message(
                        &recipient,
                        "Sorry, I encountered an error processing your message."
                    );
                }
            }

            // Handle shutdown
            _ = tokio::signal::ctrl_c() => {
                info!("Shutting down...");
                break;
            }
        }
    }

    // Cleanup
    receive_handle.abort();
    info!("🌿 Sage has shut down.");

    Ok(())
}
