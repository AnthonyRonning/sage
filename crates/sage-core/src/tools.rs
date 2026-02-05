//! Common tools for the Sage agent

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use crate::sage_agent::{Tool, ToolResult};

/// Canonical tool descriptions matching exactly what the live Sage agent registers.
/// Used by both the live agent (via ToolRegistry::generate_description) and GEPA evaluation
/// to ensure prompts are identical.
///
/// Format matches ToolRegistry::generate_description():
///   {name}:\n  Description: {description}\n  Args: {args_schema}\n\n
#[allow(dead_code)]
pub fn canonical_tool_descriptions() -> String {
    // Each entry: (name, description, args_schema)
    // Order and content must match what agent_manager.rs registers.
    let tools: &[(&str, &str, &str)] = &[
        (
            "memory_replace",
            "Replace text in a memory block. Requires exact match of old text.",
            r#"{"block": "block label (e.g., 'persona', 'human')", "old": "exact text to find", "new": "replacement text"}"#,
        ),
        (
            "memory_append",
            "Append text to the end of a memory block.",
            r#"{"block": "block label (e.g., 'persona', 'human')", "content": "text to append"}"#,
        ),
        (
            "memory_insert",
            "Insert text at a specific line in a memory block. Use line=-1 for end.",
            r#"{"block": "block label", "content": "text to insert", "line": "line number (0-indexed, -1 for end)"}"#,
        ),
        (
            "conversation_search",
            "Search through past conversation history, including older summarized conversations. Returns matching messages and summaries with relevance scores.",
            r#"{"query": "search query", "limit": "max results (default 5)"}"#,
        ),
        (
            "archival_insert",
            "Store information in long-term archival memory for future recall. Good for important facts, preferences, and details you want to remember.",
            r#"{"content": "text to store", "tags": "optional comma-separated tags"}"#,
        ),
        (
            "archival_search",
            "Search long-term archival memory using semantic similarity. Returns most relevant stored memories.",
            r#"{"query": "search query", "top_k": "max results (default 5)", "tags": "optional comma-separated tags to filter by"}"#,
        ),
        (
            "set_preference",
            "Set a user preference. Known keys: 'timezone' (IANA format like 'America/Chicago'), 'language' (ISO code like 'en'), 'display_name'. Other keys are also allowed.",
            r#"{"key": "preference key (e.g., 'timezone', 'language', 'display_name')", "value": "preference value"}"#,
        ),
        (
            "schedule_task",
            "Schedule a future message or tool execution. Supports one-off (ISO datetime) or recurring (cron expression).",
            r#"{"task_type": "message|tool_call", "description": "human-readable description", "run_at": "ISO datetime (2026-01-26T15:30:00Z) or cron (0 9 * * MON-FRI)", "payload": "JSON: {\"message\": \"...\"} for message, {\"tool\": \"name\", \"args\": {...}} for tool_call", "timezone": "optional IANA timezone for cron (default: user preference or UTC)"}"#,
        ),
        (
            "list_schedules",
            "List scheduled tasks. By default shows pending tasks only.",
            r#"{"status": "optional filter: pending, completed, failed, cancelled, or all (default: pending)"}"#,
        ),
        (
            "cancel_schedule",
            "Cancel a pending scheduled task by ID.",
            r#"{"id": "UUID of the task to cancel"}"#,
        ),
        (
            "shell",
            "Execute a shell command in the workspace. Has access to CLI tools: git, curl, jq, grep, sed, awk, python3, node, etc. Use for file operations, running scripts, or system commands.",
            r#"{"command": "shell command to execute (supports pipes, redirects)", "timeout": "optional timeout in seconds (default 60, max 300)"}"#,
        ),
        (
            "web_search",
            "Search the web with AI summaries, real-time data (weather, stocks, sports), and rich results. Use 'freshness' for time-sensitive queries, 'location' for local results.",
            r#"{ "query": "search query", "count": "results (default 10)", "freshness": "pd=24h, pw=week, pm=month (optional)", "location": "city or 'city, state' for local results (optional)" }"#,
        ),
        (
            "done",
            "No-op signal. Use ONLY when messages is [] AND no other tools needed. Indicates nothing to do this turn.",
            r#"{}"#,
        ),
    ];

    let mut desc = String::from("Available tools (add to tool_calls array to use):\n\n");
    for (name, description, args_schema) in tools {
        desc.push_str(&format!(
            "{}:\n  Description: {}\n  Args: {}\n\n",
            name, description, args_schema
        ));
    }
    desc
}

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
