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
mod marmot;
mod memory;
mod messenger;
mod sage_agent;
mod scheduler;
mod scheduler_tools;
mod schema;
mod shell_tool;
mod signal;
mod storage;
mod vision;

use agent_manager::{AgentManager, ContextType};
use config::MessengerType;
use messenger::{IncomingMessage, Messenger};
use sage_agent::SageAgent;
use signal::{run_receive_loop, run_receive_loop_tcp, SignalClient};

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

// Tools are defined in tools.rs module
mod tools;
use tools::{DoneTool, WebSearchTool};

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

    // Create channel for incoming messages
    let (tx, mut rx) = mpsc::channel::<IncomingMessage>(100);

    // Agent keyed by identity (Signal UUID or Marmot pubkey).
    // Both messengers currently use Direct (1:1 identity = 1 agent).
    // TODO: With multi-agent support, Marmot groups could each get their own
    // agent thread while sharing a parent identity for cross-thread memory.
    let context_type = ContextType::Direct;

    // Start messenger based on config
    let (messenger, receive_handle): (Arc<Mutex<dyn Messenger>>, _) = match config.messenger_type {
        MessengerType::Signal => {
            let signal_phone = match &config.signal_phone_number {
                Some(phone) => phone.clone(),
                None => {
                    warn!("SIGNAL_PHONE_NUMBER not set - cannot start Signal interface");
                    info!("Set SIGNAL_PHONE_NUMBER in .env to enable messaging.");
                    tokio::signal::ctrl_c().await?;
                    return Ok(());
                }
            };

            if let Some(ref host) = config.signal_cli_host {
                info!(
                    "Starting Signal interface (TCP mode: {}:{})...",
                    host, config.signal_cli_port
                );

                let signal_client =
                    SignalClient::connect_tcp(&signal_phone, host, config.signal_cli_port)?;
                let messenger: Arc<Mutex<dyn Messenger>> = Arc::new(Mutex::new(signal_client));

                let host = host.clone();
                let port = config.signal_cli_port;
                let account = signal_phone.clone();
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

                (messenger, receive_handle)
            } else {
                info!("Starting Signal interface (subprocess mode)...");

                let signal_client = SignalClient::spawn_subprocess(&signal_phone)?;
                let reader = signal_client.take_reader()?;
                let messenger: Arc<Mutex<dyn Messenger>> = Arc::new(Mutex::new(signal_client));

                let receive_handle =
                    tokio::spawn(async move { run_receive_loop(reader, tx).await });

                (messenger, receive_handle)
            }
        }
        MessengerType::Marmot => {
            let marmot_config = config.marmot_config();

            if marmot_config.relays.is_empty() {
                return Err(anyhow::anyhow!(
                    "MARMOT_RELAYS must be set when MESSENGER=marmot"
                ));
            }

            info!("Starting Marmot interface...");
            info!("  Relays: {:?}", marmot_config.relays);
            info!("  State dir: {}", marmot_config.state_dir);

            let (client, stdout) = marmot::spawn_marmot(&marmot_config)?;
            let writer = marmot::writer_handle(&client);
            let group_routes = marmot::group_routes_handle(&client);
            let messenger: Arc<Mutex<dyn Messenger>> = Arc::new(Mutex::new(client));

            let receive_handle = tokio::spawn(async move {
                marmot::run_marmot_receive_loop(stdout, writer, tx, marmot_config, group_routes)
                    .await
            });

            (messenger, receive_handle)
        }
    };

    // Log allowed users configuration
    let allowed_users = config.allowed_users();
    if allowed_users.iter().any(|u| u == "*") {
        info!("Allowed users: * (all users)");
    } else if allowed_users.is_empty() {
        warn!("No allowed users configured - Sage will respond to ANYONE!");
    } else {
        info!("Allowed users: {:?}", allowed_users);
    }

    info!(
        "Sage is awake and listening via {:?}!",
        config.messenger_type
    );

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

    // Messenger health check interval (every 60 minutes)
    let mut health_interval = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
    health_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    health_interval.tick().await;
    info!("Messenger health check scheduled (every 60 minutes)");

    // Main event loop
    loop {
        tokio::select! {
            // Periodic messenger health check
            _ = health_interval.tick() => {
                let client = messenger.lock().await;
                if let Err(e) = client.refresh() {
                    warn!("Messenger health check failed: {} - will retry next interval", e);
                }
            }
            // Handle scheduled task events
            Some(event) = scheduler_rx.recv() => {
                let task = event.task;
                info!("Processing scheduled task: {} ({})", task.description, task.task_type.as_str());

                let signal_identifier = match agent_manager.get_signal_identifier(task.agent_id) {
                    Ok(Some(id)) => id,
                    Ok(None) => {
                        error!("No identifier found for agent_id {} - cannot deliver scheduled task", task.agent_id);
                        continue;
                    }
                    Err(e) => {
                        error!("Failed to look up identifier for agent_id {}: {}", task.agent_id, e);
                        continue;
                    }
                };

                let task_result: Result<(), String> = match &task.payload {
                    scheduler::TaskPayload::Message(msg_payload) => {
                        info!("Sending scheduled message to {}: {}", signal_identifier, msg_payload.message);
                        let client = messenger.lock().await;
                        if let Err(e) = client.send_message(&signal_identifier, &msg_payload.message) {
                            Err(format!("Failed to send scheduled message: {}", e))
                        } else {
                            Ok(())
                        }
                    }
                    scheduler::TaskPayload::ToolCall(tool_payload) => {
                        Err(format!("Tool call scheduled tasks not yet implemented: {:?}", tool_payload))
                    }
                };

                match task_result {
                    Ok(()) => {
                        if let Err(e) = scheduler::complete_task(&scheduler_db, &task) {
                            error!("Failed to mark task {} as completed: {}", task.id, e);
                        }
                    }
                    Err(err) => {
                        error!("{}", err);
                        if let Err(e) = scheduler::fail_task(&scheduler_db, &task, &err) {
                            error!("Failed to mark task {} as failed: {}", task.id, e);
                        }
                    }
                }
            }

            // Handle incoming messages
            Some(msg) = rx.recv() => {
                // Check if sender is allowed
                if !is_user_allowed(&msg.source, config.allowed_users()) {
                    warn!("Ignoring message from unauthorized user: {}", msg.source);
                    continue;
                }

                let user_name = msg.source_name.as_deref().unwrap_or(&msg.source);
                info!("Processing message from {}...", user_name);

                // Get or create agent for this conversation
                // For Signal: keyed by user UUID (reply_to == source)
                // For Marmot: keyed by sender pubkey (reply_to == from_pubkey)
                let (agent_id, agent) = match agent_manager.get_or_create_agent(
                    &msg.reply_to,
                    context_type,
                    msg.source_name.as_deref(),
                ).await {
                    Ok(result) => result,
                    Err(e) => {
                        error!("Failed to get/create agent for {}: {}", msg.reply_to, e);
                        continue;
                    }
                };

                info!("Using agent {} for user {}", agent_id, user_name);

                // Send typing indicator early
                {
                    let client = messenger.lock().await;
                    let _ = client.send_typing(&msg.reply_to, false);
                }

                // Check for image attachments and run vision pre-processing
                let attachment_text = {
                    let image_attachment = msg.attachments.iter().find(|a| vision::is_supported_image(&a.content_type));
                    if let Some(attachment) = image_attachment {
                        let attachment_path = format!(
                            "/signal-cli-data/.local/share/signal-cli/attachments/{}",
                            attachment.file
                        );
                        info!("Image attachment detected: {} ({}) at {}", attachment.file, attachment.content_type, attachment_path);

                        let recent_context = {
                            let agent_guard = agent.lock().await;
                            match agent_guard.get_recent_messages_for_vision(6) {
                                Ok(ctx) => ctx,
                                Err(e) => {
                                    warn!("Failed to get recent messages for vision context: {}", e);
                                    String::new()
                                }
                            }
                        };

                        match vision::describe_image(
                            &config.maple_api_url,
                            config.maple_api_key.as_deref().unwrap_or(""),
                            &config.maple_vision_model,
                            &attachment_path,
                            &attachment.content_type,
                            &msg.message,
                            &recent_context,
                        ).await {
                            Ok(description) => {
                                info!("Image described ({} chars)", description.len());
                                Some(description)
                            }
                            Err(e) => {
                                error!("Failed to describe image: {}", e);
                                Some("[Image attached but could not be processed]".to_string())
                            }
                        }
                    } else {
                        None
                    }
                };

                let user_message = if let Some(ref desc) = attachment_text {
                    if msg.message.is_empty() {
                        format!("[Uploaded Image: {}]", desc)
                    } else {
                        format!("{}\n\n[Uploaded Image: {}]", msg.message, desc)
                    }
                } else {
                    msg.message.clone()
                };

                // Store incoming message
                let user_msg_id = {
                    let agent_guard = agent.lock().await;
                    match agent_guard.store_message_sync_with_attachment(
                        &msg.source,
                        "user",
                        &msg.message,
                        attachment_text.as_deref(),
                    ) {
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

                if let Some(msg_id) = user_msg_id {
                    let agent_clone = agent.clone();
                    let embed_content = user_message.clone();
                    tokio::spawn(async move {
                        let agent_guard = agent_clone.lock().await;
                        if let Err(e) = agent_guard.update_message_embedding(msg_id, &embed_content).await {
                            tracing::warn!("Failed to update embedding for user message: {}", e);
                        }
                    });
                }

                // Process message with agent
                let recipient = msg.reply_to.clone();

                let mut had_error = false;
                let max_steps = 10;

                for step_num in 0..max_steps {
                    let step_result = {
                        let mut agent_guard = agent.lock().await;
                        agent_guard.step(&user_message, step_num == 0).await
                    };

                    match step_result {
                        Ok(result) => {
                            let msg_count = result.messages.len();
                            let mut messages_to_store: Vec<String> = Vec::new();

                            for (i, response) in result.messages.iter().enumerate() {
                                let log_preview: String = response.chars().take(50).collect();
                                info!("Sending response ({}/{}): {}...", i + 1, msg_count, log_preview);

                                {
                                    let client = messenger.lock().await;
                                    if let Err(e) = client.send_message(&recipient, response) {
                                        error!("Failed to send reply: {}", e);
                                    }
                                }

                                messages_to_store.push(response.clone());

                                if i < msg_count - 1 {
                                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                                    {
                                        let client = messenger.lock().await;
                                        let _ = client.send_typing(&recipient, false);
                                    }
                                    tokio::time::sleep(tokio::time::Duration::from_millis(1450)).await;
                                }
                            }

                            if msg_count > 0 {
                                let client = messenger.lock().await;
                                let _ = client.send_typing(&recipient, true);
                            }

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
                                info!("Queued {} tool calls for storage", result.executed_tools.len());
                            }

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
                    let client = messenger.lock().await;
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
