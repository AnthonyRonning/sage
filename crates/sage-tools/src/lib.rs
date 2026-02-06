//! Sage Tools - capabilities that Sage can use
//!
//! Tools are organized by category:
//! - brave: Brave Search API client
//! - web_search: Web search tool using Brave

pub mod brave;
pub mod web_search;

pub use brave::{BraveClient, SearchOptions, SearchResponse};
pub use web_search::WebSearch;

/// Tool execution result
#[derive(Debug)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

impl ToolResult {
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            success: true,
            output: output.into(),
            error: None,
        }
    }

    pub fn error(error: impl Into<String>) -> Self {
        Self {
            success: false,
            output: String::new(),
            error: Some(error.into()),
        }
    }
}
