//! Common tools for the Sage agent

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use crate::sage_agent::{Tool, ToolResult};

/// Done tool - signals the agent is finished and doesn't need to send another message
pub struct DoneTool;

#[async_trait]
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

    async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
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

#[async_trait]
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

    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
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
