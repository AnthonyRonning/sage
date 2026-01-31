//! Web search tool using Brave Search API

use crate::brave::BraveClient;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WebSearchError {
    #[error("Search failed: {0}")]
    SearchFailed(String),
}

#[derive(Deserialize, JsonSchema)]
pub struct WebSearchArgs {
    #[schemars(description = "The search query to look up on the web")]
    pub query: String,
}

#[derive(Clone)]
pub struct WebSearch {
    client: Arc<BraveClient>,
}

impl WebSearch {
    pub fn new(client: Arc<BraveClient>) -> Self {
        Self { client }
    }
}

impl Tool for WebSearch {
    const NAME: &'static str = "web_search";
    type Error = WebSearchError;
    type Args = WebSearchArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "web_search".to_string(),
            description: "Search the web for current information, news, facts, or any topic. Use this when you need up-to-date information or don't know something.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query to look up on the web"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        use crate::brave::SearchOptions;

        let options = SearchOptions {
            count: Some(5),
            ..Default::default()
        };

        let response = self
            .client
            .search(&args.query, Some(options))
            .await
            .map_err(|e| WebSearchError::SearchFailed(e.to_string()))?;

        Ok(response.format_results())
    }
}
