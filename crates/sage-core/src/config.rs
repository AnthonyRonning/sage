use anyhow::{Context, Result};

use crate::marmot::MarmotConfig;

#[derive(Debug, Clone, PartialEq)]
pub enum MessengerType {
    Signal,
    Marmot,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub maple_api_url: String,
    pub maple_api_key: Option<String>,
    pub maple_model: String,
    pub maple_embedding_model: String,
    pub maple_vision_model: String,

    pub database_url: String,

    /// Which messaging provider to use
    pub messenger_type: MessengerType,

    // Signal-specific config
    pub signal_phone_number: Option<String>,
    pub signal_allowed_users: Vec<String>,
    /// If set, connect to signal-cli daemon via TCP instead of spawning subprocess
    pub signal_cli_host: Option<String>,
    pub signal_cli_port: u16,

    // Marmot-specific config
    pub marmot_binary: String,
    pub marmot_relays: Vec<String>,
    pub marmot_state_dir: String,
    pub marmot_allowed_pubkeys: Vec<String>,
    pub marmot_auto_accept_welcomes: bool,

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

            database_url: std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?,

            messenger_type: match std::env::var("MESSENGER")
                .unwrap_or_else(|_| "signal".to_string())
                .to_lowercase()
                .as_str()
            {
                "marmot" => MessengerType::Marmot,
                _ => MessengerType::Signal,
            },

            signal_phone_number: std::env::var("SIGNAL_PHONE_NUMBER").ok(),
            signal_allowed_users: std::env::var("SIGNAL_ALLOWED_USERS")
                .map(|s| s.split(',').map(|u| u.trim().to_string()).collect())
                .unwrap_or_default(),
            signal_cli_host: std::env::var("SIGNAL_CLI_HOST").ok(),
            signal_cli_port: std::env::var("SIGNAL_CLI_PORT")
                .unwrap_or_else(|_| "7583".to_string())
                .parse()
                .unwrap_or(7583),

            marmot_binary: std::env::var("MARMOT_BINARY").unwrap_or_else(|_| "marmotd".to_string()),
            marmot_relays: std::env::var("MARMOT_RELAYS")
                .map(|s| {
                    s.split(',')
                        .map(|r| r.trim().to_string())
                        .filter(|r| !r.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            marmot_state_dir: std::env::var("MARMOT_STATE_DIR")
                .unwrap_or_else(|_| "/data/marmot-state".to_string()),
            marmot_allowed_pubkeys: std::env::var("MARMOT_ALLOWED_PUBKEYS")
                .map(|s| {
                    s.split(',')
                        .map(|p| p.trim().to_string())
                        .filter(|p| !p.is_empty())
                        .map(|p| {
                            if p == "*" {
                                p
                            } else {
                                crate::marmot::normalize_pubkey(&p).unwrap_or(p)
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
            marmot_auto_accept_welcomes: std::env::var("MARMOT_AUTO_ACCEPT_WELCOMES")
                .map(|s| s != "false" && s != "0")
                .unwrap_or(true),

            brave_api_key: std::env::var("BRAVE_API_KEY").ok(),

            workspace_path: std::env::var("SAGE_WORKSPACE")
                .unwrap_or_else(|_| "/workspace".to_string()),

            http_port: std::env::var("HTTP_PORT")
                .unwrap_or_else(|_| "3000".to_string())
                .parse()
                .context("HTTP_PORT must be a valid port number")?,
        })
    }

    pub fn marmot_config(&self) -> MarmotConfig {
        MarmotConfig {
            binary_path: self.marmot_binary.clone(),
            relays: self.marmot_relays.clone(),
            state_dir: self.marmot_state_dir.clone(),
            allowed_pubkeys: self.marmot_allowed_pubkeys.clone(),
            auto_accept_welcomes: self.marmot_auto_accept_welcomes,
        }
    }

    pub fn allowed_users(&self) -> &[String] {
        match self.messenger_type {
            MessengerType::Signal => &self.signal_allowed_users,
            MessengerType::Marmot => &self.marmot_allowed_pubkeys,
        }
    }
}
