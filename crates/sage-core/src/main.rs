use anyhow::Result;
use axum::{routing::get, Json, Router};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

mod agent_manager;
mod config;
mod memory;
mod sage_agent;
mod scheduler;
mod scheduler_tools;
mod schema;
mod shell_tool;
mod signal;
mod storage;

use agent_manager::{AgentManager, ContextType};
use sage_agent::{SageAgent, Tool, ToolResult};
use signal::{run_receive_loop, run_receive_loop_tcp, IncomingMessage, SignalClient};

/// Health check response
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

/// Health check endpoint - returns 200 OK when the service is running
async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy",
        version: env!("CARGO_PKG_VERSION"),
    })
}

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

    async fn execute(
        &self,
        _args: &std::collections::HashMap<String, String>,
    ) -> Result<ToolResult> {
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

    async fn execute(
        &self,
        args: &std::collections::HashMap<String, String>,
    ) -> Result<ToolResult> {
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
    info!(
        "Agent manager initialized (workspace: {})",
        config.workspace_path
    );

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
        // Supervise the receive loop: if the TCP subscription drops, reconnect + resubscribe.
        let receive_handle = tokio::spawn(async move {
            let mut backoff = std::time::Duration::from_millis(250);
            let backoff_max = std::time::Duration::from_secs(60);

            loop {
                match run_receive_loop_tcp(&host, port, &account, tx.clone()).await {
                    Ok(()) => {
                        warn!(
                            "Signal TCP receive loop exited unexpectedly; restarting in {:?}",
                            backoff
                        );
                    }
                    Err(e) => {
                        warn!(
                            "Signal TCP receive loop error; restarting in {:?}: {}",
                            backoff, e
                        );
                    }
                }

                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(backoff_max);
            }
        });

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

    // Start HTTP health check server
    let health_port: u16 = std::env::var("HEALTH_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let health_router = Router::new().route("/health", get(health_check));
    let health_listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", health_port)).await?;
    tokio::spawn(async move {
        if let Err(e) = axum::serve(health_listener, health_router).await {
            error!("Health check server error: {}", e);
        }
    });
    info!("Health check server listening on port {}", health_port);

    // Start background scheduler
    let mut scheduler_rx = scheduler::spawn_scheduler(scheduler_db.clone(), 30);
    info!("Background scheduler started (polling every 30s)");

    // Signal health check interval (every 60 minutes)
    // This refreshes prekeys to prevent silent send failures
    let mut signal_health_interval = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
    signal_health_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Skip the first immediate tick
    signal_health_interval.tick().await;
    info!("Signal health check scheduled (every 60 minutes)");

    // Main event loop
    loop {
        tokio::select! {
            // Periodic Signal health check (refresh prekeys)
            _ = signal_health_interval.tick() => {
                info!("🔄 Running Signal health check...");
                let client = signal_client.lock().await;
                if let Err(e) = client.refresh_account() {
                    warn!("Signal health check failed: {} - will retry next interval", e);
                }
            }
            // Handle scheduled task events
            Some(event) = scheduler_rx.recv() => {
                let task = event.task;
                info!("⏰ Processing scheduled task: {} ({})", task.description, task.task_type.as_str());

                // Look up the signal_identifier for this agent_id
                let signal_identifier = match agent_manager.get_signal_identifier(task.agent_id) {
                    Ok(Some(id)) => id,
                    Ok(None) => {
                        error!("No signal_identifier found for agent_id {} - cannot deliver scheduled task", task.agent_id);
                        continue;
                    }
                    Err(e) => {
                        error!("Failed to look up signal_identifier for agent_id {}: {}", task.agent_id, e);
                        continue;
                    }
                };

                // Handle different task types based on payload
                match &task.payload {
                    scheduler::TaskPayload::Message(msg_payload) => {
                        info!("📤 Sending scheduled message to {}: {}", signal_identifier, msg_payload.message);
                        let client = signal_client.lock().await;
                        if let Err(e) = client.send_message(&signal_identifier, &msg_payload.message) {
                            error!("Failed to send scheduled message: {}", e);
                        }
                    }
                    scheduler::TaskPayload::ToolCall(tool_payload) => {
                        warn!("Tool call scheduled tasks not yet implemented: {:?}", tool_payload);
                    }
                }
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

                            // Store messages SYNCHRONOUSLY so next step sees them in conversation history
                            // Only the embedding update is async (slow API call)
                            let mut msg_ids_for_embedding: Vec<(Uuid, String)> = Vec::new();
                            for response in &messages_to_store {
                                let msg_id = {
                                    let agent_guard = agent.lock().await;
                                    agent_guard.store_message_sync(&recipient, "assistant", response)
                                };
                                if let Ok(id) = msg_id {
                                    msg_ids_for_embedding.push((id, response.clone()));
                                }
                            }

                            // Async embedding updates (don't block next step)
                            if !msg_ids_for_embedding.is_empty() {
                                let agent_clone = agent.clone();
                                tokio::spawn(async move {
                                    for (msg_id, content) in msg_ids_for_embedding {
                                        let agent_guard = agent_clone.lock().await;
                                        if let Err(e) = agent_guard.update_message_embedding(msg_id, &content).await {
                                            tracing::warn!("Failed to update embedding: {}", e);
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
