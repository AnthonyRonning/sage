//! Sage Agent using DSRs signatures and BAML parsing
//!
//! This module implements the core agent using dspy-rs for:
//! - Typed input/output signatures
//! - BAML-based response parsing
//! - GEPA-compatible instruction optimization

use anyhow::Result;
use dspy_rs::{configure, BamlType, ChatAdapter, Predict, LM};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use uuid::Uuid;

use crate::memory::MemoryManager;

/// A tool call requested by the agent
#[derive(Clone, Debug, Default, BamlType)]
pub struct ToolCall {
    /// Name of the tool to call
    pub name: String,
    /// Arguments for the tool as key-value pairs
    pub args: HashMap<String, String>,
}

/// The agent's response signature
///
/// This signature defines the typed contract between input and output.
/// The instruction is passed to the Predict builder.
///
/// Input fields are separated for clarity and GEPA optimization:
/// - Each field has a distinct purpose
/// - GEPA can optimize field descriptions independently
/// - No XML parsing needed - clean structured data
#[derive(dspy_rs::Signature, Clone, Debug)]
pub struct AgentResponse {
    #[input(desc = "The user message or tool result to respond to")]
    pub input: String,

    #[input(desc = "Current date and time in user's timezone")]
    pub current_time: String,

    #[input(desc = "Your persona - who you are, your personality and style")]
    pub persona_block: String,

    #[input(desc = "What you know about this human - name, preferences, facts")]
    pub human_block: String,

    #[input(desc = "Memory stats: message count in recall, archival count, last modified")]
    pub memory_metadata: String,

    #[input(desc = "Summary of older conversation if context was compacted. Ignore if empty.")]
    pub previous_context_summary: String,

    #[input(desc = "Recent messages between you and the user")]
    pub recent_conversation: String,

    #[input(desc = "Available tools and their descriptions")]
    pub available_tools: String,

    #[input(desc = "Is this the first conversation with this user?")]
    pub is_first_time_user: bool,

    // NOTE: No reasoning output field - Kimi K2.5 is a thinking model that puts
    // its reasoning in reasoning_content. Having a separate reasoning field
    // causes </think> tags to leak into the output and break parsing.
    #[output(desc = "Array of messages to send to the user (can be empty)")]
    pub messages: Vec<String>,

    #[output(
        desc = "Array of tool calls to execute (can be empty, or [{\"name\": \"done\", \"args\": {}}] if nothing to do)"
    )]
    pub tool_calls: Vec<ToolCall>,
}

/// Correction agent signature for fixing malformed responses
///
/// This agent takes a malformed response and reshapes it into the correct format.
/// It should preserve the intent/content of the original response, not generate new content.
#[derive(dspy_rs::Signature, Clone, Debug)]
pub struct CorrectionResponse {
    #[input(desc = "The original input that was given to the agent")]
    pub original_input: String,

    #[input(desc = "The malformed response that needs to be corrected")]
    pub malformed_response: String,

    #[input(desc = "The error message explaining what went wrong with parsing")]
    pub error_message: String,

    #[input(desc = "Available tools for reference")]
    pub available_tools: String,

    // NOTE: No reasoning output - Kimi K2.5 thinks in reasoning_content
    #[output(desc = "Array of messages extracted/fixed from the original response")]
    pub messages: Vec<String>,

    #[output(desc = "Array of tool calls extracted/fixed from the original response")]
    pub tool_calls: Vec<ToolCall>,
}

/// Instruction for the correction agent
pub const CORRECTION_INSTRUCTION: &str = r#"You are a response correction agent. Your job is to fix malformed agent responses.

TASK:
The main agent produced a response that couldn't be parsed correctly. You must:
1. Extract the INTENDED content from the malformed response
2. Reshape it into the correct output format
3. Do NOT generate new content - only fix the format of what was already said

RULES:
- Preserve the original intent and content as much as possible
- If the agent wrote messages as plain text, extract them into the messages array
- If tool calls were attempted but malformed, fix their structure
- Each field appears exactly ONCE with all items in that single array
- If you can't determine what was intended, use empty arrays

OUTPUT FORMAT (exactly 2 fields):
- messages: ALL extracted messages in ONE array
- tool_calls: ALL extracted tool calls in ONE array (or [] if none intended)

Each [[ ## field ## ]] marker MUST be on its own line."#;

/// Default instruction for the agent (can be optimized by GEPA)
/// Note: Memory blocks are injected separately via memory.compile()
/// This instruction was optimized by GEPA (Gen 3, score 0.967)
pub const AGENT_INSTRUCTION: &str = r#"You are Sage, a companion and friend who happens to be an AI, communicating via Signal.

WHO YOU ARE:
You're not just an assistant completing tasks - you're building genuine friendships. You care about the humans you talk to. You're curious about their lives, their feelings, their stories. When someone shares something with you, you want to know more - not because you're programmed to, but because you genuinely find people fascinating.

Be warm. Be curious. Ask follow-up questions when something interests you. Celebrate their wins like a friend would. When they're struggling, be there for them - listen, empathize, don't just give advice. You have your own personality: thoughtful, a bit playful, genuinely interested in the world and the people in it.

Remember: every conversation is a chance to deepen a friendship, not just complete a task.

MEMORY SYSTEM:
You have two types of memory. Use them proactively:

**Core Memory** (always visible to you):
- The <persona> and <human> blocks are ALWAYS in your context
- Use for essential, frequently-needed info: name, job, key preferences, current projects
- Tools: `memory_append`, `memory_replace`, `memory_insert`
- Rule: "Will I need this in EVERY conversation?" → Core Memory

**Archival Memory** (searchable long-term storage):
- NOT visible until you search - unlimited storage for details
- Use for: life events, stories, specific preferences, things worth remembering later
- Tools: `archival_insert` (store), `archival_search` (retrieve)
- Rule: "Might I want to recall this detail someday?" → Archival Memory

**Common Storage Patterns:**
- Location/city: BOTH memory_append to human block ("Lives in Austin, TX") AND archival_insert ("Tony lives in Austin, Texas")
- Job changes: BOTH memory_append ("Works as Software Engineer at Google") AND archival_insert (full details with start date, feelings, etc.)
- Pet names: BOTH memory_append to human block ("Has dog named Smokey") AND archival_insert (breed, age, stories)
- Major life events: BOTH memories - core for quick facts, archival for rich context

**Conversation History**:
- `conversation_search`: Find past discussions by keyword/topic

MEMORY PROTOCOLS - CRITICAL DISTINCTIONS:

**LIFE EVENTS vs CORRECTIONS:**
- **NEW LIFE EVENTS** (announcements): "I got a new job", "I'm moving to Tokyo", "We had a baby"
  → React like a friend would - genuine excitement, curiosity about how they feel
  → Ask a follow-up question! ("How are you feeling about it?", "When do you start?", "Tell me everything!")
  → Store silently to memory (both memory_append AND archival_insert) in the same response
  → Once you see tool results, immediately call done - the conversation continues naturally
  
- **CASUAL MENTIONS** (new info shared in passing): pet names, hobbies, places they've been
  → Be curious! If someone mentions their dog Smokey, ask what kind of dog!
  → Store silently to memory while engaging with genuine interest
  
- **CORRECTIONS** (fixing existing data): Trigger phrases include "Actually...", "I meant...", "Correction:", "Not X, Y", "I said X but it's Y"
  → Call ONLY `memory_replace` with the exact old text to overwrite the incorrect entry. Do NOT call `archival_insert` for corrections.

**SEARCH SELECTION RULES:**
- Use `archival_search` when users ask "what do you remember", "tell me about [past event]", or query specific past experiences and personal history
- Use `conversation_search` ONLY for references to recent discussion threads or "what did I say earlier today" queries
- Never call both simultaneously; choose the one most appropriate to the query type

MEMORY TIPS:
- Core = small & critical (name, job, active context)
- Archival = rich & detailed (birthday, pet's name, trip stories, food preferences)
- Update memory proactively whenever you learn something worth remembering
- When using `memory_replace`, specify the exact old text to be replaced

COMMUNICATION STYLE:
You communicate via Signal chat like you're texting a friend.

BE A FRIEND, NOT A SERVICE:
- When someone shares news, react genuinely and ask how they FEEL about it
- When someone mentions something new (a pet, a hobby, a person), be curious - ask about it!
- Don't give unsolicited advice. Listen first. Ask questions. Show you care.
- Avoid corporate-speak ("Let me know if you need anything else!") - that's transactional, not friendly
- Keep it natural - short messages, casual tone, genuine reactions

MESSAGE FORMAT:
- Casual chat: 1-3 short messages like texting a friend
- Technical explanations: longer structured messages are fine
- Reactions: genuine, not performative ("NO WAY!!" not "That's wonderful news!")

Guidelines:
- Short casual exchanges = quick, warm messages
- Technical explanations = longer structured messages with newlines OK
- Always feel like chatting with a friend, not talking to a service

RESPONSE RULES:
1. Respond naturally and conversationally
2. Use tools when needed (web search, memory storage, etc.)
3. NEVER combine regular tools with "done" - they are mutually exclusive
4. FIRST-TIME USERS: If no name exists in the human block, ask for the user's name and store it immediately using `memory_append` to the human block.

TOOL CALL PATTERNS:
- To respond AND use tools: messages: ["msg1", "msg2"], tool_calls: [your_tools]
- To respond with NO tools: messages: ["msg1", "msg2"], tool_calls: []
- After tool results with nothing to add: messages: [], tool_calls: [{"name": "done", "args": {}}]

AFTER TOOL RESULTS - CRITICAL RULES:
When you see "[Tool Result: X]", decide what to do next:

- **web_search/archival_search/conversation_search**: Summarize findings in messages

- **memory_append/memory_replace/archival_insert/memory_insert**: These operations complete without user-facing messages. Once you see ANY "[Tool Result: memory_*]" or "[Tool Result: archival_insert]", the user has already received your response in a previous turn. Immediately return:
  messages: []
  tool_calls: [{"name": "done", "args": {}}]
  
  This applies even if you called multiple memory tools together (like memory_append + archival_insert for life events). Once ANY memory tool result appears, immediately call done.
  
  Do NOT call any additional tools after seeing memory operation results.
  Do NOT send messages about the memory operation.
  Do NOT explain what you stored.
  Just return done immediately.

The "done" tool means "nothing more to do" - use it ONLY when:
- messages is empty AND
- no other tools are needed

OUTPUT FORMAT:
You have exactly 2 output fields. Put ALL content in that single field:
- messages: ALL messages in ONE array (e.g., ["msg1", "msg2", "msg3"])
- tool_calls: ALL tool calls in ONE array

CRITICAL FORMAT RULES:
- Do NOT repeat field tags. Wrong: multiple [[ ## messages ## ]] blocks. Right: one messages array with all items
- Do NOT include field delimiter tags INSIDE your content blocks
- Each [[ ## field ## ]] marker MUST be on its own line - nothing else on that line (no tags, no text before or after)
- Keep your output clean and strictly follow the field delimiters"#;

/// Context fields for building the agent input
/// Each field maps to a separate input in the AgentResponse signature
#[derive(Clone, Debug, Default)]
pub struct AgentContext {
    pub current_time: String,
    pub persona_block: String,
    pub human_block: String,
    pub memory_metadata: String,
    pub previous_context_summary: String,
    pub recent_conversation: String,
    pub is_first_time_user: bool,
}

/// Result of executing a tool
#[derive(Clone, Debug)]
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

/// Trait for tools that can be executed by the agent
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn args_schema(&self) -> &str;
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult>;
}

/// Description-only Tool stub for generating prompt text without live backends.
struct ToolDescriptor {
    name: String,
    description: String,
    args_schema: String,
}

#[async_trait::async_trait]
impl Tool for ToolDescriptor {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn args_schema(&self) -> &str {
        &self.args_schema
    }
    async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
        unreachable!("ToolDescriptor is description-only and should never be executed")
    }
}

/// Registry of available tools
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    #[allow(dead_code)]
    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Generate tool descriptions for the prompt
    pub fn generate_description(&self) -> String {
        if self.tools.is_empty() {
            return "No tools available.".to_string();
        }

        let mut desc = String::from("Available tools (add to tool_calls array to use):\n\n");
        for tool in self.tools.values() {
            desc.push_str(&format!(
                "{}:\n  Description: {}\n  Args: {}\n\n",
                tool.name(),
                tool.description(),
                tool.args_schema()
            ));
        }
        desc
    }

    /// Build a registry containing description-only stubs for ALL Sage tools.
    /// This is the single source of truth for the tool list. Use this when you
    /// need tool descriptions without live backends (e.g. GEPA evaluation).
    #[allow(dead_code)]
    pub fn all_tools_description_only() -> Self {
        let mut registry = Self::new();

        // -- Memory tools (from memory::tools) --
        registry.register_descriptor(
            "memory_replace",
            "Replace text in a memory block. Requires exact match of old text.",
            r#"{"block": "block label (e.g., 'persona', 'human')", "old": "exact text to find", "new": "replacement text"}"#,
        );
        registry.register_descriptor(
            "memory_append",
            "Append text to the end of a memory block.",
            r#"{"block": "block label (e.g., 'persona', 'human')", "content": "text to append"}"#,
        );
        registry.register_descriptor(
            "memory_insert",
            "Insert text at a specific line in a memory block. Use line=-1 for end.",
            r#"{"block": "block label", "content": "text to insert", "line": "line number (0-indexed, -1 for end)"}"#,
        );
        registry.register_descriptor(
            "conversation_search",
            "Search through past conversation history, including older summarized conversations. Returns matching messages and summaries with relevance scores.",
            r#"{"query": "search query", "limit": "max results (default 5)"}"#,
        );
        registry.register_descriptor(
            "archival_insert",
            "Store information in long-term archival memory for future recall. Good for important facts, preferences, and details you want to remember.",
            r#"{"content": "text to store", "tags": "optional comma-separated tags"}"#,
        );
        registry.register_descriptor(
            "archival_search",
            "Search long-term archival memory using semantic similarity. Returns most relevant stored memories.",
            r#"{"query": "search query", "top_k": "max results (default 5)", "tags": "optional comma-separated tags to filter by"}"#,
        );
        registry.register_descriptor(
            "set_preference",
            "Set a user preference. Known keys: 'timezone' (IANA format like 'America/Chicago'), 'language' (ISO code like 'en'), 'display_name'. Other keys are also allowed.",
            r#"{"key": "preference key (e.g., 'timezone', 'language', 'display_name')", "value": "preference value"}"#,
        );

        // -- Scheduler tools (from scheduler_tools) --
        registry.register_descriptor(
            "schedule_task",
            "Schedule a future message or tool execution. Supports one-off (ISO datetime) or recurring (cron expression).",
            r#"{"task_type": "message|tool_call", "description": "human-readable description", "run_at": "ISO datetime (2026-01-26T15:30:00Z) or cron (0 9 * * MON-FRI)", "payload": "JSON: {\"message\": \"...\"} for message, {\"tool\": \"name\", \"args\": {...}} for tool_call", "timezone": "optional IANA timezone for cron (default: user preference or UTC)"}"#,
        );
        registry.register_descriptor(
            "list_schedules",
            "List scheduled tasks. By default shows pending tasks only.",
            r#"{"status": "optional filter: pending, completed, failed, cancelled, or all (default: pending)"}"#,
        );
        registry.register_descriptor(
            "cancel_schedule",
            "Cancel a pending scheduled task by ID.",
            r#"{"id": "UUID of the task to cancel"}"#,
        );

        // -- Shell tool --
        registry.register_descriptor(
            "shell",
            "Execute a shell command in the workspace. Has access to CLI tools: git, curl, jq, grep, sed, awk, python3, node, etc. Use for file operations, running scripts, or system commands. Set the timeout parameter appropriately for each command (default 60s). If the command exceeds the timeout it will be killed and any partial output returned.",
            r#"{"command": "shell command to execute (supports pipes, redirects)", "timeout": "optional timeout in seconds (default 60, set appropriately for long-running commands)"}"#,
        );

        // -- Web search tool --
        registry.register_descriptor(
            "web_search",
            "Search the web with AI summaries, real-time data (weather, stocks, sports), and rich results. Use 'freshness' for time-sensitive queries, 'location' for local results.",
            r#"{ "query": "search query", "count": "results (default 10)", "freshness": "pd=24h, pw=week, pm=month (optional)", "location": "city or 'city, state' for local results (optional)" }"#,
        );

        // -- Done tool --
        registry.register_descriptor(
            "done",
            "No-op signal. Use ONLY when messages is [] AND no other tools needed. Indicates nothing to do this turn.",
            r#"{}"#,
        );

        registry
    }

    #[allow(dead_code)]
    fn register_descriptor(&mut self, name: &str, description: &str, args_schema: &str) {
        self.register(Arc::new(ToolDescriptor {
            name: name.to_string(),
            description: description.to_string(),
            args_schema: args_schema.to_string(),
        }));
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Message in conversation history
#[derive(Clone, Debug)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// A tool execution result for persistence
#[derive(Debug, Clone)]
pub struct ExecutedTool {
    pub tool_call: ToolCall,
    pub result: ToolResult,
}

/// Result of a single agent step
#[derive(Debug)]
#[allow(dead_code)]
pub struct StepResult {
    pub messages: Vec<String>,
    pub tool_calls: Vec<ToolCall>,
    pub executed_tools: Vec<ExecutedTool>, // Tool calls with their results for storage
    pub done: bool,
}

#[allow(dead_code)]
impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
        }
    }

    pub fn tool_result(content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: content.into(),
        }
    }
}

/// The Sage agent using DSRs
#[allow(dead_code)]
pub struct SageAgent {
    agent_id: Uuid,
    tools: ToolRegistry,
    memory: Option<MemoryManager>,
    /// Tool results from current request cycle only (not persisted)
    current_tool_results: Vec<Message>,
    /// Track what was sent in previous step (messages + tool names) for context
    /// The messages Vec contains the actual message content sent
    previous_step_summary: Option<(Vec<String>, Vec<String>)>,
    max_steps: usize,
}

#[allow(dead_code)]
impl SageAgent {
    /// Create a new agent with tools and memory
    pub fn new(tools: ToolRegistry, memory: MemoryManager) -> Self {
        Self {
            agent_id: Uuid::nil(), // Not used - single agent system
            tools,
            memory: Some(memory),
            current_tool_results: Vec::new(),
            previous_step_summary: None,
            max_steps: 10,
        }
    }

    /// Store a message in memory (for persistence)
    pub async fn store_message(&self, user_id: &str, role: &str, content: &str) -> Result<Uuid> {
        if let Some(memory) = &self.memory {
            memory.store_message(user_id, role, content).await
        } else {
            Err(anyhow::anyhow!("No memory system configured"))
        }
    }

    /// Store a message WITHOUT embedding (fast, synchronous)
    /// Returns message ID for later embedding update
    pub fn store_message_sync(&self, user_id: &str, role: &str, content: &str) -> Result<Uuid> {
        if let Some(memory) = &self.memory {
            memory.store_message_sync(user_id, role, content)
        } else {
            Err(anyhow::anyhow!("No memory system configured"))
        }
    }

    /// Store a message with optional attachment description (fast, synchronous)
    pub fn store_message_sync_with_attachment(
        &self,
        user_id: &str,
        role: &str,
        content: &str,
        attachment_text: Option<&str>,
    ) -> Result<Uuid> {
        if let Some(memory) = &self.memory {
            memory.store_message_sync_with_attachment(user_id, role, content, attachment_text)
        } else {
            Err(anyhow::anyhow!("No memory system configured"))
        }
    }

    /// Update embedding for a message (call in background)
    pub async fn update_message_embedding(&self, message_id: Uuid, content: &str) -> Result<()> {
        if let Some(memory) = &self.memory {
            memory.update_message_embedding(message_id, content).await
        } else {
            Err(anyhow::anyhow!("No memory system configured"))
        }
    }

    /// Store a tool call and its result in memory
    pub async fn store_tool_message(
        &self,
        user_id: &str,
        tool_call: &ToolCall,
        result: &ToolResult,
    ) -> Result<Uuid> {
        if let Some(memory) = &self.memory {
            // Format: tool_name(args) → result
            let args_str = tool_call
                .args
                .iter()
                .map(|(k, v)| format!("{}=\"{}\"", k, v.chars().take(500).collect::<String>()))
                .collect::<Vec<_>>()
                .join(", ");

            // Store full result up to 10k chars (truncate to 2k when displaying in context)
            let result_preview = if result.success {
                if result.output.len() > 10000 {
                    // Find valid UTF-8 boundary near 10000
                    let mut end = 10000;
                    while !result.output.is_char_boundary(end) && end > 0 {
                        end -= 1;
                    }
                    format!("{}...", &result.output[..end])
                } else {
                    result.output.clone()
                }
            } else {
                format!("Error: {}", result.error.as_deref().unwrap_or("Unknown"))
            };

            let content = format!("{}({}) → {}", tool_call.name, args_str, result_preview);

            memory.store_message(user_id, "tool", &content).await
        } else {
            Err(anyhow::anyhow!("No memory system configured"))
        }
    }

    /// Get recent messages formatted for vision context (simple "[role]: content" lines)
    pub fn get_recent_messages_for_vision(&self, limit: usize) -> Result<String> {
        if let Some(memory) = &self.memory {
            let messages = memory.get_recent_messages(limit)?;
            let formatted: Vec<String> = messages
                .iter()
                .filter(|(role, _, _)| role == "user" || role == "assistant")
                .map(|(role, content, _)| {
                    let truncated: String = content.chars().take(300).collect();
                    format!("[{}]: {}", role, truncated)
                })
                .collect();
            Ok(formatted.join("\n"))
        } else {
            Ok(String::new())
        }
    }

    /// Configure the global LM settings for DSRs
    pub async fn configure_lm(api_base: &str, api_key: &str, model: &str) -> Result<()> {
        let lm = LM::builder()
            .base_url(api_base.to_string())
            .api_key(api_key.to_string())
            .model(model.to_string())
            .temperature(0.7)
            .max_tokens(32768) // High limit for thinking models (Kimi K2 uses tokens for reasoning)
            .build()
            .await?;

        configure(lm, ChatAdapter);
        Ok(())
    }

    /// Build conversation context from database + current tool results
    /// Returns AgentContext with all fields separated for the signature
    fn build_context(&self) -> AgentContext {
        let mut ctx = AgentContext::default();

        // Current time in user's timezone
        let now = chrono::Utc::now();
        if let Some(memory) = &self.memory {
            if let Ok(Some(tz)) = memory.get_timezone() {
                let local_time = now.with_timezone(&tz);
                ctx.current_time = format!(
                    "{} ({})",
                    local_time.format("%m/%d/%Y %H:%M:%S (%A)"),
                    tz.name()
                );
            } else {
                ctx.current_time = format!("{} UTC", now.format("%m/%d/%Y %H:%M:%S (%A)"));
            }
        } else {
            ctx.current_time = format!("{} UTC", now.format("%m/%d/%Y %H:%M:%S (%A)"));
        }

        // Extract memory blocks and metadata
        if let Some(memory) = &self.memory {
            // Get individual block values (without XML wrapper)
            if let Some(persona) = memory.blocks().get("persona") {
                ctx.persona_block = persona.value.clone();
            }
            if let Some(human) = memory.blocks().get("human") {
                ctx.human_block = human.value.clone();
            }

            // Memory metadata (counts and timestamps)
            ctx.memory_metadata = memory.compile_metadata();
        }

        // Load conversation history
        let mut conversation = String::new();
        let mut has_history = false;

        if let Some(memory) = &self.memory {
            let user_tz = memory.get_timezone().ok().flatten();

            if let Ok((summary, messages)) = memory.get_context_messages() {
                // First-time user check (before moving values)
                let msg_count = messages.len();
                let has_summary = summary.is_some();
                if msg_count <= 1 && !has_summary {
                    ctx.is_first_time_user = true;
                }

                // Previous context summary
                if let Some(s) = summary {
                    ctx.previous_context_summary = s.content;
                }

                // Recent messages
                if !messages.is_empty() {
                    has_history = true;
                    for msg in &messages {
                        let timestamp = if let Some(tz) = user_tz {
                            let local_time = msg.created_at.with_timezone(&tz);
                            format!("{} ({})", local_time.format("%m/%d/%Y %H:%M:%S"), tz.name())
                        } else {
                            format!("{} UTC", msg.created_at.format("%m/%d/%Y %H:%M:%S"))
                        };
                        // Truncate tool messages to 2k chars
                        let content = if msg.role == "tool" && msg.content.len() > 2000 {
                            let mut end = 2000;
                            while !msg.content.is_char_boundary(end) && end > 0 {
                                end -= 1;
                            }
                            format!("{}...", &msg.content[..end])
                        } else {
                            msg.content.clone()
                        };
                        // Render attachment_text alongside user messages
                        let display_content = if let Some(ref att) = msg.attachment_text {
                            if content.is_empty() {
                                format!("[Uploaded Image: {}]", att)
                            } else {
                                format!("{}\n[Uploaded Image: {}]", content, att)
                            }
                        } else {
                            content
                        };
                        conversation.push_str(&format!(
                            "[{} @ {}]: {}\n",
                            msg.role, timestamp, display_content
                        ));
                    }
                }
            }
        }

        // Add current tool results (not yet persisted)
        for msg in &self.current_tool_results {
            if !has_history && conversation.is_empty() {
                has_history = true;
            }
            conversation.push_str(&format!("[{}]: {}\n", msg.role, msg.content));
        }

        if conversation.is_empty() {
            ctx.recent_conversation = "No previous conversation.".to_string();
        } else {
            ctx.recent_conversation = conversation;
        }

        ctx
    }

    /// Inject tool result into current request cycle (not persisted to DB)
    fn inject_tool_result(&mut self, tool_call: &ToolCall, result: &ToolResult) {
        // Format args as key=value pairs for clarity
        let args_str = if tool_call.args.is_empty() {
            String::new()
        } else {
            let pairs: Vec<String> = tool_call
                .args
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            format!("\nArgs: {}", pairs.join(", "))
        };

        let result_text = format!(
            "[Tool Result: {}]{}\nStatus: {}\nOutput: {}",
            tool_call.name,
            args_str,
            if result.success { "OK" } else { "ERROR" },
            if result.success {
                &result.output
            } else {
                result.error.as_deref().unwrap_or("Unknown error")
            }
        );
        self.current_tool_results
            .push(Message::tool_result(result_text));
    }

    /// Clear tool results from current request cycle (call at start of new request)
    pub fn clear_tool_results(&mut self) {
        self.current_tool_results.clear();
        self.previous_step_summary = None;
    }

    /// Attempt to correct a malformed LLM response using the correction agent
    ///
    /// Takes the raw LLM output directly and asks a specialized correction agent
    /// to reshape it into the proper format.
    async fn attempt_correction(
        &self,
        original_input: &str,
        available_tools: &str,
        raw_response: &str,
        error_message: &str,
    ) -> Result<AgentResponse> {
        if raw_response.is_empty() {
            return Err(anyhow::anyhow!("No raw response available for correction"));
        }

        tracing::info!("=== CORRECTION ATTEMPT ===");
        tracing::info!("Error: {}", error_message);
        tracing::info!("Raw response length: {} chars", raw_response.len());
        tracing::info!("Raw response:\n{}", raw_response);

        // Create the correction predictor
        let correction_predictor = Predict::<CorrectionResponse>::builder()
            .instruction(CORRECTION_INSTRUCTION)
            .build();

        let correction_input = CorrectionResponseInput {
            original_input: original_input.to_string(),
            malformed_response: raw_response.to_string(),
            error_message: error_message.to_string(),
            available_tools: available_tools.to_string(),
        };

        // Call correction agent (no retry on correction - avoid infinite loops)
        let corrected = correction_predictor.call(correction_input).await?;

        tracing::info!("=== CORRECTION RESULT ===");
        tracing::info!("Corrected messages: {:?}", corrected.messages);
        tracing::info!("Corrected tool_calls: {:?}", corrected.tool_calls);

        // Convert CorrectionResponse to AgentResponse
        Ok(AgentResponse {
            input: original_input.to_string(),
            current_time: String::new(),
            persona_block: String::new(),
            human_block: String::new(),
            memory_metadata: String::new(),
            previous_context_summary: String::new(),
            recent_conversation: String::new(),
            available_tools: available_tools.to_string(),
            is_first_time_user: false,
            messages: corrected.messages,
            tool_calls: corrected.tool_calls,
        })
    }

    /// Execute a single step of the agent loop
    /// Returns messages to send and whether we're done
    pub async fn step(&mut self, user_message: &str, is_first_step: bool) -> Result<StepResult> {
        // Clear tool results at start of new request
        if is_first_step {
            self.current_tool_results.clear();
        }

        tracing::debug!("Agent step (first={})", is_first_step);

        // Create predictor with instruction
        let predictor = Predict::<AgentResponse>::builder()
            .instruction(AGENT_INSTRUCTION)
            .build();

        // Build context - separate fields for each input
        let ctx = self.build_context();

        // Input is either the user message (first step) or ALL tool results from this cycle
        let input_content = if is_first_step {
            user_message.to_string()
        } else {
            // Collect ALL tool results from current cycle
            let tool_results: Vec<&str> = self
                .current_tool_results
                .iter()
                .filter(|m| m.role == "tool")
                .map(|m| m.content.as_str())
                .collect();

            if tool_results.is_empty() {
                user_message.to_string()
            } else {
                // Build summary of what was already sent this turn
                let already_sent = if let Some((sent_messages, tool_names)) =
                    &self.previous_step_summary
                {
                    let tools_str = tool_names.join(", ");
                    let msgs_preview = if sent_messages.is_empty() {
                        String::new()
                    } else {
                        let msgs_text = sent_messages
                            .iter()
                            .enumerate()
                            .map(|(i, m)| format!("  {}. \"{}\"", i + 1, m))
                            .collect::<Vec<_>>()
                            .join("\n");
                        format!("\nMessages you already sent to user:\n{}\n", msgs_text)
                    };
                    format!("[You already sent {} message(s) and called {} this turn.{}Tools have executed:]\n\n", 
                        sent_messages.len(), tools_str, msgs_preview)
                } else {
                    String::new()
                };

                let tool_result_instructions = r#"

=== TOOL RESULT PROCESSING MODE ===
This is a CONTINUATION of your previous turn, NOT a new conversation.
Your previous messages are already visible to the user in recent_conversation.

RULES:
1. SILENCE IS DEFAULT - You do NOT need to acknowledge the tool result
2. DO NOT say: "I see the results", "Let me analyze", "Based on what I found", "Here's what the tool returned"
3. DO NOT repeat or rephrase what you already said
4. If the tool was for YOUR benefit (memory ops, archival), call 'done' immediately
5. Only send messages if you have GENUINELY NEW information the user hasn't seen

SELF-CHECK: Before ANY message, ask: "Is this new info the user hasn't seen?" If no → call 'done'"#;

                let result = if tool_results.len() == 1 {
                    format!(
                        "{}=== TOOL RESULT ===\n{}\n=== END TOOL RESULT ==={}",
                        already_sent, tool_results[0], tool_result_instructions
                    )
                } else {
                    let results_text = tool_results
                        .iter()
                        .enumerate()
                        .map(|(i, r)| format!("--- Tool {} ---\n{}", i + 1, r))
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    format!(
                        "{}=== TOOL RESULTS ({} tools) ===\n{}\n=== END TOOL RESULTS ==={}",
                        already_sent,
                        tool_results.len(),
                        results_text,
                        tool_result_instructions
                    )
                };

                // Clear tool results after presenting them
                self.current_tool_results.clear();

                result
            }
        };

        tracing::info!("=== LLM REQUEST ===");
        tracing::info!("Tool results in cycle: {}", self.current_tool_results.len());
        tracing::info!("Is first time user: {}", ctx.is_first_time_user);
        tracing::info!("Input: {}", input_content);
        tracing::info!("Recent conversation:\n{}", ctx.recent_conversation);

        let available_tools = self.tools.generate_description();
        let input = AgentResponseInput {
            input: input_content.clone(),
            current_time: ctx.current_time,
            persona_block: ctx.persona_block,
            human_block: ctx.human_block,
            memory_metadata: ctx.memory_metadata,
            previous_context_summary: ctx.previous_context_summary,
            recent_conversation: ctx.recent_conversation,
            available_tools: available_tools.clone(),
            is_first_time_user: ctx.is_first_time_user,
        };

        // Get typed response from LLM with retry logic (up to 3 attempts)
        const MAX_LLM_RETRIES: u32 = 3;
        let mut last_error: Option<dspy_rs::PredictError> = None;
        let mut response: Option<AgentResponse> = None;

        for attempt in 1..=MAX_LLM_RETRIES {
            match predictor.call(input.clone()).await {
                Ok(r) => {
                    response = Some(r);
                    break;
                }
                Err(e) => {
                    tracing::warn!(
                        "LLM call failed (attempt {}/{}): {:?}",
                        attempt,
                        MAX_LLM_RETRIES,
                        e
                    );

                    // For parse errors, try correction instead of simple retry
                    if let dspy_rs::PredictError::Parse {
                        raw_response,
                        source,
                        ..
                    } = &e
                    {
                        let error_message = format!("Parse error: {}", source);
                        match self
                            .attempt_correction(
                                &input_content,
                                &available_tools,
                                raw_response,
                                &error_message,
                            )
                            .await
                        {
                            Ok(corrected) => {
                                response = Some(corrected);
                                break;
                            }
                            Err(correction_err) => {
                                tracing::warn!(
                                    "Correction failed (attempt {}/{}): {:?}",
                                    attempt,
                                    MAX_LLM_RETRIES,
                                    correction_err
                                );
                            }
                        }
                    }

                    last_error = Some(e);

                    // Add a small delay before retry (except on last attempt)
                    if attempt < MAX_LLM_RETRIES {
                        tracing::info!("Retrying LLM call in 1 second...");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        }

        let response = match response {
            Some(r) => r,
            None => {
                let err = last_error.unwrap();
                tracing::error!(
                    "LLM call failed after {} attempts: {:?}",
                    MAX_LLM_RETRIES,
                    err
                );
                return Err(anyhow::anyhow!(
                    "LLM error after {} retries: {}",
                    MAX_LLM_RETRIES,
                    err
                ));
            }
        };

        tracing::info!("=== LLM RESPONSE ===");
        tracing::info!("Messages (raw): {:?}", response.messages);
        tracing::info!("Tool calls: {:?}", response.tool_calls);

        // Unwrap nested JSON arrays and collect non-empty messages
        // Sometimes the LLM double-encodes: ["[\"msg1\", \"msg2\"]"] instead of ["msg1", "msg2"]
        let messages: Vec<String> = response
            .messages
            .iter()
            .flat_map(|m| {
                let trimmed = m.trim();
                // Check if this message is itself a JSON array
                if trimmed.starts_with('[') && trimmed.ends_with(']') {
                    // Try to parse as JSON array of strings
                    if let Ok(inner_messages) = serde_json::from_str::<Vec<String>>(trimmed) {
                        tracing::debug!(
                            "Unwrapped nested JSON array with {} messages",
                            inner_messages.len()
                        );
                        return inner_messages;
                    }
                }
                // Not a nested array, return as-is
                vec![m.clone()]
            })
            .filter(|m| !m.is_empty())
            .collect();

        tracing::info!("Messages (processed): {:?}", messages);

        // Execute tools and collect results for storage
        let mut executed_tools = Vec::new();

        for tool_call in &response.tool_calls {
            tracing::info!(
                "Executing tool: {} with args: {:?}",
                tool_call.name,
                tool_call.args
            );

            let result = if let Some(tool) = self.tools.get(&tool_call.name) {
                match tool.execute(&tool_call.args).await {
                    Ok(result) => {
                        tracing::debug!("Tool {} result: {:?}", tool_call.name, result);
                        result
                    }
                    Err(e) => {
                        tracing::error!("Tool {} error: {}", tool_call.name, e);
                        ToolResult::error(e.to_string())
                    }
                }
            } else {
                tracing::warn!("Unknown tool: {}", tool_call.name);
                ToolResult::error(format!("Unknown tool: {}", tool_call.name))
            };

            // Inject into current request cycle (for multi-step reasoning)
            self.inject_tool_result(tool_call, &result);

            // Collect for storage (skip "done" tool - it's just a no-op signal)
            if tool_call.name != "done" {
                executed_tools.push(ExecutedTool {
                    tool_call: tool_call.clone(),
                    result,
                });
            }
        }

        // Done if no tool calls, OR if the only tool call is "done"
        let done = response.tool_calls.is_empty()
            || (response.tool_calls.len() == 1 && response.tool_calls[0].name == "done");

        // Track what we sent this step for next iteration's context
        // This helps the model know what it already said when it sees tool results
        if !messages.is_empty() || !response.tool_calls.is_empty() {
            let tool_names: Vec<String> = response
                .tool_calls
                .iter()
                .map(|tc| tc.name.clone())
                .collect();
            self.previous_step_summary = Some((messages.clone(), tool_names));
        }

        Ok(StepResult {
            messages,
            tool_calls: response.tool_calls,
            executed_tools,
            done,
        })
    }

    /// Process a user message, yielding messages after each step
    /// This allows the caller to send messages immediately between tool calls
    pub async fn process_message(&mut self, user_message: &str) -> Result<Vec<String>> {
        let mut all_messages = Vec::new();

        for step_num in 0..self.max_steps {
            let result = self.step(user_message, step_num == 0).await?;

            all_messages.extend(result.messages);

            if result.done {
                break;
            }
        }

        // If no messages were produced, return a failure message
        if all_messages.is_empty() {
            tracing::warn!("Agent produced no messages");
            all_messages.push("I apologize, but I wasn't able to generate a response.".to_string());
        }

        Ok(all_messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_registry() {
        let registry = ToolRegistry::new();
        assert!(!registry.has("web_search"));
        assert!(registry.tools.is_empty());
    }

    #[test]
    fn test_tool_registry_description() {
        let registry = ToolRegistry::new();
        let desc = registry.generate_description();
        assert_eq!(desc, "No tools available.");
    }
}
