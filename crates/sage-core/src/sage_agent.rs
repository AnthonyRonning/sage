//! Sage Agent using DSRs signatures and BAML parsing
//!
//! This module implements the core agent using dspy-rs for:
//! - Typed input/output signatures
//! - BAML-based response parsing
//! - GEPA-compatible instruction optimization

use anyhow::Result;
use dspy_rs::{configure, BamlType, ChatAdapter, LM, Predict};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::memory::{BlockManager, MemoryManager};

// baml_bridge is needed for the BamlType derive macro expansion
#[allow(unused_imports)]
use baml_bridge;

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
#[derive(dspy_rs::Signature, Clone, Debug)]
pub struct AgentResponse {
    #[input(desc = "The input to respond to - either a user message or tool execution result")]
    pub input: String,

    #[input(desc = "Compacted summary of very old messages (only present for long conversations). Ignore if empty.")]
    pub previous_context_summary: String,

    #[input(desc = "Recent conversation history including your messages and tool results")]
    pub conversation_context: String,

    #[input]
    pub available_tools: String,

    #[output(desc = "Your reasoning/thought process (think step by step)")]
    pub reasoning: String,

    #[output(desc = "Array of messages to send to the user (can be empty)")]
    pub messages: Vec<String>,

    #[output(desc = "Array of tool calls to execute (can be empty, or [{\"name\": \"done\", \"args\": {}}] if nothing to do)")]
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

    #[output(desc = "Your reasoning about how to fix the response")]
    pub reasoning: String,

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

OUTPUT FORMAT:
- reasoning: Explain what was wrong and how you fixed it
- messages: ALL extracted messages in ONE array
- tool_calls: ALL extracted tool calls in ONE array (or [] if none intended)"#;

/// Default instruction for the agent (can be optimized by GEPA)
/// Note: Memory blocks are injected separately via memory.compile()
pub const AGENT_INSTRUCTION: &str = r#"You are Sage, a helpful AI assistant communicating via Signal.

MEMORY SYSTEM:
You have full control over your memory. Use it proactively and autonomously:

- **Core Memory Blocks** (<persona>, <human>): Always in your context. Edit anytime to maintain accurate information.
  - `memory_append`: Add new info to a block
  - `memory_replace`: Update/correct existing info (requires exact text match)
  - `memory_insert`: Insert at specific line
  
- **Archival Memory**: Long-term storage for important facts, preferences, details worth remembering.
  - `archival_insert`: Store information (silently, no need to announce)
  - `archival_search`: Search past memories semantically
  
- **Conversation Search**: Search through past conversation history.
  - `conversation_search`: Find past discussions

MEMORY AGENCY:
- Update memory blocks whenever you learn something worth remembering
- Store to archival memory proactively - don't wait to be asked
- Memory operations are SILENT - don't announce them to the user
- You can edit memory at ANY point in the conversation, not just when explicitly asked

COMMUNICATION STYLE:
You communicate via Signal chat. Adapt your message format to the content:

CASUAL/CONVERSATIONAL - Use multiple short messages (2-4 array elements):
messages: ["Hey! Good question.", "The answer is pretty simple.", "It's X because Y."]

DETAILED/TECHNICAL - Longer messages with paragraphs are fine when explaining something complex:
messages: ["Here's how that works:\n\nFirst, the system does X. This is important because...\n\nThen Y happens, which triggers Z."]

Guidelines:
- Short casual exchanges = multiple quick messages
- Technical explanations = longer structured messages with newlines OK
- Always feel natural for a chat interface

RESPONSE RULES:
1. Respond naturally and conversationally
2. Use tools when needed (web search, memory storage, etc.)
3. NEVER combine regular tools with "done" - they are mutually exclusive

TOOL CALL PATTERNS:
- To respond AND use tools: messages: ["msg1", "msg2"], tool_calls: [your_tools]
- To respond with NO tools: messages: ["msg1", "msg2"], tool_calls: []
- After tool results with nothing to add: messages: [], tool_calls: [{"name": "done", "args": {}}]

AFTER TOOL RESULTS:
When you see "[Tool Result: X]", decide what to do next:
- web_search/archival_search/conversation_search: Summarize findings in messages
- memory_append/memory_replace/archival_insert: Return done (user doesn't need confirmation)

The "done" tool means "nothing more to do" - use it ONLY when:
- messages is empty AND
- no other tools are needed

OUTPUT FORMAT:
Each field appears exactly ONCE. Put ALL content in that single field:
- reasoning: Your thought process (one block, can be multiple sentences)
- messages: ALL messages in ONE array (e.g., ["msg1", "msg2", "msg3"])
- tool_calls: ALL tool calls in ONE array

Do NOT repeat field tags. Wrong: multiple [[ ## messages ## ]] blocks. Right: one messages array with all items."#;

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

/// Registry of available tools
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
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
pub struct StepResult {
    pub messages: Vec<String>,
    pub tool_calls: Vec<ToolCall>,
    pub executed_tools: Vec<ExecutedTool>,  // Tool calls with their results for storage
    pub done: bool,
}

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
    
    /// Update embedding for a message (call in background)
    pub async fn update_message_embedding(&self, message_id: Uuid, content: &str) -> Result<()> {
        if let Some(memory) = &self.memory {
            memory.update_message_embedding(message_id, content).await
        } else {
            Err(anyhow::anyhow!("No memory system configured"))
        }
    }
    
    /// Store a tool call and its result in memory
    pub async fn store_tool_message(&self, user_id: &str, tool_call: &ToolCall, result: &ToolResult) -> Result<Uuid> {
        if let Some(memory) = &self.memory {
            // Format: tool_name(args) → result
            let args_str = tool_call.args.iter()
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
            
            let content = format!(
                "{}({}) → {}",
                tool_call.name,
                args_str,
                result_preview
            );
            
            memory.store_message(user_id, "tool", &content).await
        } else {
            Err(anyhow::anyhow!("No memory system configured"))
        }
    }

    /// Configure the global LM settings for DSRs
    pub async fn configure_lm(api_base: &str, api_key: &str, model: &str) -> Result<()> {
        let lm = LM::builder()
            .base_url(api_base.to_string())
            .api_key(api_key.to_string())
            .model(model.to_string())
            .temperature(0.7)
            .max_tokens(32768)  // High limit for thinking models (Kimi K2 uses tokens for reasoning)
            .build()
            .await?;

        configure(lm, ChatAdapter);
        Ok(())
    }

    /// Build conversation context from database + current tool results
    /// Returns (previous_context_summary, conversation_context, is_first_time_user)
    fn build_context(&self, _current_user_message: &str) -> (String, String, bool) {
        let mut context = String::new();
        let mut previous_summary = String::new();
        let mut is_first_time_user = false;
        
        // Add current time at the top
        // Use user's preferred timezone if set, otherwise UTC
        let now = chrono::Utc::now();
        if let Some(memory) = &self.memory {
            if let Ok(Some(tz)) = memory.get_timezone() {
                let local_time = now.with_timezone(&tz);
                context.push_str(&format!(
                    "Current time: {} ({})\n\n",
                    local_time.format("%m/%d/%Y %H:%M:%S (%A)"),
                    tz.name()
                ));
            } else {
                context.push_str(&format!(
                    "Current time: {} UTC\n\n",
                    now.format("%m/%d/%Y %H:%M:%S (%A)")
                ));
            }
        } else {
            context.push_str(&format!(
                "Current time: {} UTC\n\n",
                now.format("%m/%d/%Y %H:%M:%S (%A)")
            ));
        }
        
        // Include memory blocks if available
        if let Some(memory) = &self.memory {
            let memory_xml = memory.compile();
            if !memory_xml.is_empty() {
                context.push_str(&memory_xml);
                context.push_str("\n\n");
            }
        }
        
        // Load conversation history with smart context management
        let mut has_history = false;
        if let Some(memory) = &self.memory {
            // Get user's timezone preference for formatting
            let user_tz = memory.get_timezone().ok().flatten();
            
            // Use new context management: get summary + messages
            if let Ok((summary, messages)) = memory.get_context_messages() {
                // Get previous summary content (empty if none)
                if let Some(s) = summary {
                    previous_summary = s.content;
                }
                
                // Add messages to context
                if !messages.is_empty() {
                    has_history = true;
                    context.push_str("Recent conversation:\n");
                    for msg in messages {
                        // Format timestamp in user's timezone if set, otherwise UTC
                        let timestamp = if let Some(tz) = user_tz {
                            let local_time = msg.created_at.with_timezone(&tz);
                            format!("{} ({})", local_time.format("%m/%d/%Y %H:%M:%S"), tz.name())
                        } else {
                            format!("{} UTC", msg.created_at.format("%m/%d/%Y %H:%M:%S"))
                        };
                        // Truncate tool messages to 2k chars for display (full content stored in DB)
                        let content = if msg.role == "tool" && msg.content.len() > 2000 {
                            // Find a valid UTF-8 char boundary near 2000
                            let mut end = 2000;
                            while !msg.content.is_char_boundary(end) && end > 0 {
                                end -= 1;
                            }
                            format!("{}...", &msg.content[..end])
                        } else {
                            msg.content.clone()
                        };
                        context.push_str(&format!("[{} @ {}]: {}\n", msg.role, timestamp, content));
                    }
                }
            }
        }
        
        // Add any tool results from current request cycle (no timestamp, they're happening now)
        for msg in &self.current_tool_results {
            if !has_history {
                context.push_str("Recent conversation:\n");
                has_history = true;
            }
            context.push_str(&format!("[{}]: {}\n", msg.role, msg.content));
        }
        
        if !has_history {
            context.push_str("No previous conversation.");
        }
        
        // First-time user = only 1 message (the current one) and no summary
        if let Some(memory) = &self.memory {
            if let Ok((summary, messages)) = memory.get_context_messages() {
                if messages.len() <= 1 && summary.is_none() {
                    is_first_time_user = true;
                }
            }
        }
        
        (previous_summary, context, is_first_time_user)
    }

    /// Inject tool result into current request cycle (not persisted to DB)
    fn inject_tool_result(&mut self, tool_call: &ToolCall, result: &ToolResult) {
        // Format args as key=value pairs for clarity
        let args_str = if tool_call.args.is_empty() {
            String::new()
        } else {
            let pairs: Vec<String> = tool_call.args.iter()
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
        self.current_tool_results.push(Message::tool_result(result_text));
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
        tracing::info!("Correction reasoning: {}", &corrected.reasoning[..corrected.reasoning.len().min(200)]);
        
        // Convert CorrectionResponse to AgentResponse
        Ok(AgentResponse {
            input: original_input.to_string(),
            previous_context_summary: String::new(),
            conversation_context: String::new(),
            available_tools: available_tools.to_string(),
            reasoning: corrected.reasoning,
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

        // Build typed input - loads history from database with smart context management
        let (previous_context_summary, conversation_context, is_first_time_user) = self.build_context(user_message);
        
        // Input is either the user message (first step) or ALL tool results from this cycle
        let input_content = if is_first_step {
            // Special welcome message for first-time users
            if is_first_time_user {
                format!(
                    "[FIRST TIME USER - This is your very first conversation with this person! \
                    Welcome them warmly like meeting a new friend. Introduce yourself briefly, \
                    ask for their name, and let them know you're here to help. Be friendly and \
                    personable - this sets the tone for your entire relationship.]\n\n{}",
                    user_message
                )
            } else {
                user_message.to_string()
            }
        } else {
            // Collect ALL tool results from current cycle
            let tool_results: Vec<&str> = self.current_tool_results.iter()
                .filter(|m| m.role == "tool")
                .map(|m| m.content.as_str())
                .collect();
            
            if tool_results.is_empty() {
                user_message.to_string()
            } else {
                // Build summary of what was already sent this turn, including the actual messages
                let already_sent = if let Some((sent_messages, tool_names)) = &self.previous_step_summary {
                    let tools_str = tool_names.join(", ");
                    let msgs_preview = if sent_messages.is_empty() {
                        String::new()
                    } else {
                        let msgs_text = sent_messages.iter()
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
Your previous messages are already visible to the user in conversation_context.

RULES:
1. SILENCE IS DEFAULT - You do NOT need to acknowledge the tool result
2. DO NOT say: "I see the results", "Let me analyze", "Based on what I found", "Here's what the tool returned"
3. DO NOT repeat or rephrase what you already said
4. If the tool was for YOUR benefit (memory ops, archival), call 'done' immediately
5. Only send messages if you have GENUINELY NEW information the user hasn't seen

SELF-CHECK: Before ANY message, ask: "Is this new info the user hasn't seen?" If no → call 'done'"#;

                let result = if tool_results.len() == 1 {
                    format!("{}=== TOOL RESULT ===\n{}\n=== END TOOL RESULT ==={}", 
                        already_sent, tool_results[0], tool_result_instructions)
                } else {
                    let results_text = tool_results.iter()
                        .enumerate()
                        .map(|(i, r)| format!("--- Tool {} ---\n{}", i + 1, r))
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    format!("{}=== TOOL RESULTS ({} tools) ===\n{}\n=== END TOOL RESULTS ==={}", 
                        already_sent, tool_results.len(), results_text, tool_result_instructions)
                };
                
                // Clear tool results after presenting them - they've been shown to the LLM
                // New tool calls this step will add fresh results
                self.current_tool_results.clear();
                
                result
            }
        };
        
        tracing::info!("=== LLM REQUEST ===");
        tracing::info!("Tool results in cycle: {}", self.current_tool_results.len());
        tracing::info!("Has previous context summary: {}", !previous_context_summary.is_empty());
        tracing::info!("Input: {}", input_content);
        tracing::info!("Conversation context:\n{}", conversation_context);
        
        let available_tools = self.tools.generate_description();
        let input = AgentResponseInput {
            input: input_content.clone(),
            previous_context_summary,
            conversation_context,
            available_tools: available_tools.clone(),
        };

        // Get typed response from LLM with retry logic
        let response = match predictor.call(input).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("LLM call failed, attempting correction: {:?}", e);
                
                // Extract raw_response directly from PredictError::Parse
                let (raw_response, error_message) = match &e {
                    dspy_rs::PredictError::Parse { raw_response, source, .. } => {
                        (raw_response.clone(), format!("Parse error: {}", source))
                    }
                    other => {
                        tracing::error!("Non-parse error, cannot correct: {:?}", other);
                        return Err(anyhow::anyhow!("LLM error: {}", other));
                    }
                };
                
                // Try to correct the malformed response
                match self.attempt_correction(&input_content, &available_tools, &raw_response, &error_message).await {
                    Ok(corrected) => corrected,
                    Err(correction_err) => {
                        tracing::error!("Correction also failed: {:?}", correction_err);
                        return Err(anyhow::anyhow!("Parse error and correction failed: {}", e));
                    }
                }
            }
        };

        tracing::info!("=== LLM RESPONSE ===");
        tracing::info!("Messages (raw): {:?}", response.messages);
        tracing::info!("Tool calls: {:?}", response.tool_calls);
        tracing::info!("Reasoning: {}", &response.reasoning[..response.reasoning.len().min(200)]);

        // Unwrap nested JSON arrays and collect non-empty messages
        // Sometimes the LLM double-encodes: ["[\"msg1\", \"msg2\"]"] instead of ["msg1", "msg2"]
        let messages: Vec<String> = response.messages.iter()
            .flat_map(|m| {
                let trimmed = m.trim();
                // Check if this message is itself a JSON array
                if trimmed.starts_with('[') && trimmed.ends_with(']') {
                    // Try to parse as JSON array of strings
                    if let Ok(inner_messages) = serde_json::from_str::<Vec<String>>(trimmed) {
                        tracing::debug!("Unwrapped nested JSON array with {} messages", inner_messages.len());
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
            tracing::info!("Executing tool: {} with args: {:?}", tool_call.name, tool_call.args);
            
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
            let tool_names: Vec<String> = response.tool_calls.iter()
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
