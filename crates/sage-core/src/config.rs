use anyhow::{Context, Result};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub maple_api_url: String,
    pub maple_api_key: Option<String>,
    pub maple_model: String,
    pub maple_embedding_model: String,
    pub maple_vision_model: String,

    pub letta_api_url: String,

    pub valkey_url: String,
    pub database_url: String,

    pub signal_phone_number: Option<String>,
    pub signal_allowed_users: Vec<String>,
    /// If set, connect to signal-cli daemon via TCP instead of spawning subprocess
    pub signal_cli_host: Option<String>,
    pub signal_cli_port: u16,

    pub brave_api_key: Option<String>,

    /// Workspace directory for shell commands and file operations
    pub workspace_path: String,

    pub http_port: u16,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            maple_api_url: std::env::var("MAPLE_API_URL")
                .unwrap_or_else(|_| "http://localhost:8080/v1".to_string()),
            maple_api_key: std::env::var("MAPLE_API_KEY").ok(),
            maple_model: std::env::var("MAPLE_MODEL").unwrap_or_else(|_| "kimi-k2".to_string()),
            maple_embedding_model: std::env::var("MAPLE_EMBEDDING_MODEL")
                .unwrap_or_else(|_| "nomic-embed-text".to_string()),
            maple_vision_model: std::env::var("MAPLE_VISION_MODEL").unwrap_or_else(|_| {
                std::env::var("MAPLE_MODEL").unwrap_or_else(|_| "kimi-k2-5".to_string())
            }),

            letta_api_url: std::env::var("LETTA_API_URL")
                .unwrap_or_else(|_| "http://localhost:8283".to_string()),

            valkey_url: std::env::var("VALKEY_URL")
                .unwrap_or_else(|_| "redis://localhost:6379".to_string()),
            database_url: std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?,

            signal_phone_number: std::env::var("SIGNAL_PHONE_NUMBER").ok(),
            signal_allowed_users: std::env::var("SIGNAL_ALLOWED_USERS")
                .map(|s| s.split(',').map(|u| u.trim().to_string()).collect())
                .unwrap_or_default(),
            signal_cli_host: std::env::var("SIGNAL_CLI_HOST").ok(),
            signal_cli_port: std::env::var("SIGNAL_CLI_PORT")
                .unwrap_or_else(|_| "7583".to_string())
                .parse()
                .unwrap_or(7583),

            brave_api_key: std::env::var("BRAVE_API_KEY").ok(),

            workspace_path: std::env::var("SAGE_WORKSPACE")
                .unwrap_or_else(|_| "/workspace".to_string()),

            http_port: std::env::var("HTTP_PORT")
                .unwrap_or_else(|_| "3000".to_string())
                .parse()
                .context("HTTP_PORT must be a valid port number")?,
        })
    }
}
