//! Sage Core Library
//!
//! Shared types and modules for the Sage AI agent.

pub mod agent_manager;
pub mod config;
pub mod marmot;
pub mod memory;
pub mod messenger;
pub mod sage_agent;
pub mod scheduler;
pub mod scheduler_tools;
pub mod schema;
pub mod shell_tool;
pub mod signal;
pub mod storage;
pub mod tools;
pub mod vision;

// Re-export key types for convenience
pub use config::Config;
pub use sage_agent::{
    AgentResponse, AgentResponseInput, ToolCall, ToolRegistry, AGENT_INSTRUCTION,
};
pub use tools::{DoneTool, WebSearchTool};
