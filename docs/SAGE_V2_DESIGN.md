# Sage V2: A Memory-Augmented AI Agent in Rust

## Design Document

**Version:** 1.0.0  
**Date:** January 26, 2026  
**Status:** Production

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Motivation and Problem Statement](#2-motivation-and-problem-statement)
3. [Core Design Principles](#3-core-design-principles)
4. [Architecture Overview](#4-architecture-overview)
5. [The Memory System](#5-the-memory-system)
6. [XML-Based Tool Execution](#6-xml-based-tool-execution)
7. [DSPy-Inspired Prompt Optimization](#7-dspy-inspired-prompt-optimization)
8. [Signal Integration](#8-signal-integration)
9. [Data Model](#9-data-model)
10. [Implementation Phases](#10-implementation-phases)
11. [Technology Stack](#11-technology-stack)
12. [Appendix: Letta Lessons Learned](#appendix-letta-lessons-learned)

---

## 1. Executive Summary

Sage V2 is a complete rewrite of the Sage AI assistant, moving from Python/Letta to a pure Rust implementation. The core motivation is **control and reliability**: the Letta framework, while feature-rich, proved to be a black box that made debugging and customization extremely difficult, especially when dealing with LLM provider bugs (vLLM empty responses, tool calling failures).

### Key Differentiators

1. **No Native Tool Calling**: Instead of relying on LLM providers' buggy function calling APIs, Sage V2 uses raw XML tags in the model's text output for tool invocations. This sidesteps vLLM/provider-specific tool calling bugs entirely.

2. **DSPy-Inspired Architecture**: Using the `dsrs` (DSPy in Rust) approach for structured prompting and prompt optimization via GEPA (Genetic-Pareto optimizer).

3. **Full Memory System Control**: Implementing Letta's proven 4-tier memory architecture (core, recall, archival, summary) from scratch, giving us complete control over context management and compaction.

4. **Rust Performance**: Orders of magnitude faster than Python, with strong type safety for handling LLM outputs.

### End Goal

A conversational AI assistant that:
- Communicates via Signal (end-to-end encrypted)
- Maintains long-term memory across conversations
- Uses tools (web search, file operations, shell commands) reliably via XML parsing
- Self-improves through prompt optimization
- Never loses conversation history due to bugs or context overflow

---

## 2. Motivation and Problem Statement

### 2.1 Why We're Moving Off Letta

After weeks of development with Letta, we encountered fundamental issues:

#### Tool Calling Unreliability

The combination of:
- Kimi K2 model via vLLM
- Letta's tool calling format
- Long conversation history

Resulted in **80-100% empty response rates** when context exceeded ~120 messages. The model would either:
1. Return completely empty responses (vLLM bug)
2. Promise to call tools ("Let me store that:") but end without actually calling them
3. Output wrong formats (XML instead of JSON tool calls)

From `docs/kimi-k2-vllm-tool-bug.md`:
```
120 messages:  5/5 empty (100%)  <-- THRESHOLD
153 messages:  5/5 empty (100%)
```

#### Black Box Debugging

Letta abstracts away:
- Exact prompt construction
- Message history management  
- Tool schema injection
- Context window calculations

When things broke, we had to reverse-engineer Letta to understand what was happening. The `LETTA_ARCHITECTURE_REVERSE_ENGINEERING.md` document (1740 lines) is evidence of this effort.

#### Conversation History Pollution

Once the model learned bad patterns (promising without acting), it was nearly impossible to correct without wiping history—which we refuse to do. The bad examples in history trained the model to repeat mistakes.

### 2.2 The Core Insight

The root cause of most issues was **native LLM tool calling**. This feature:
- Is implemented differently by each provider
- Has bugs in vLLM, Tinfoil, and other inference servers
- Creates complex JSON schemas that consume context
- Fails silently or produces malformed outputs

**Solution**: Don't use native tool calling. Parse XML tags from raw text instead.

### 2.3 What Letta Got Right

Despite the issues, Letta's memory architecture is excellent:

1. **Persistence-first**: Context window as state, not ephemeral
2. **4-tier memory**: Core (immediate), Recall (searchable history), Archival (semantic long-term), Summary (compaction)
3. **Structured blocks**: XML-tagged sections in system prompt for memory
4. **Heartbeat chaining**: Synthetic messages to enable tool → tool → tool → response flows

We will reimplement these patterns, but with full control.

---

## 3. Core Design Principles

### 3.1 No Native Tool Calling

**Principle**: All tool invocations are expressed as XML tags in the model's raw text output.

```
<msg>Sure, let me look that up for you.</msg>
<web_search query="latest rust news 2026"/>
```

The agent runtime parses these tags and executes the corresponding tools. This approach:
- Works identically across all LLM providers
- Is immune to vLLM/provider tool calling bugs
- Is debuggable (just look at the text output)
- Allows content + tool calls in the same response naturally

### 3.2 Text-First, Structure-Second

**Principle**: The LLM produces text. We extract structure from text.

Rather than asking the LLM to produce JSON (fragile, provider-dependent), we:
1. Let the LLM respond naturally with embedded XML tags
2. Parse the tags with a robust XML extractor
3. Execute tools and inject results back as text

### 3.3 DSPy Signatures for Structure

**Principle**: Use typed signatures to define LLM task contracts.

From dsrs:
```rust
#[Signature]
struct AgentResponse {
    /// Respond to the user's message, using tools if needed.
    #[input]
    context: String,
    
    #[input]
    user_message: String,
    
    #[output]
    response: String,  // May contain <msg>, <tool>, etc. tags
}
```

This gives us:
- Type safety for inputs/outputs
- Automatic prompt construction
- Foundation for prompt optimization

### 3.4 Memory as Explicit State

**Principle**: The agent's memory is persisted state, not runtime cache.

Every message, every memory block edit, every archival insert is persisted to the database immediately. The "context window" is a view into this state, not the state itself.

### 3.5 Graceful Degradation

**Principle**: When things fail, fail gracefully and observably.

- Empty LLM response? Retry with logging.
- Tool execution fails? Return error message to LLM, let it adapt.
- Context overflow? Compact with summary, never lose data.

### 3.6 Prompt Optimization Over Prompt Engineering

**Principle**: Don't hand-tune prompts forever. Use GEPA to optimize them.

Instead of manually iterating on system prompts, we:
1. Define evaluation metrics (tool call success rate, response quality)
2. Collect execution traces with feedback
3. Run GEPA to evolve better prompts automatically

---

## 4. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                         Sage V2 Architecture                     │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────┐     ┌──────────────┐     ┌──────────────┐    │
│  │   Signal     │────▶│    Agent     │────▶│     LLM      │    │
│  │  Interface   │◀────│    Core      │◀────│   (Maple)    │    │
│  └──────────────┘     └──────┬───────┘     └──────────────┘    │
│                              │                                   │
│                              ▼                                   │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                     Memory System                         │  │
│  │  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌────────┐ │  │
│  │  │   Core     │ │   Recall   │ │  Archival  │ │Summary │ │  │
│  │  │  Memory    │ │  Memory    │ │  Memory    │ │Memory  │ │  │
│  │  │  (Blocks)  │ │ (History)  │ │(Embeddings)│ │(Compact)│ │  │
│  │  └────────────┘ └────────────┘ └────────────┘ └────────┘ │  │
│  └──────────────────────────────────────────────────────────┘  │
│                              │                                   │
│                              ▼                                   │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                    Tool Execution                         │  │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────┐ │  │
│  │  │   Web    │ │  Shell   │ │  Memory  │ │   Custom     │ │  │
│  │  │  Search  │ │ Commands │ │   Tools  │ │    Tools     │ │  │
│  │  └──────────┘ └──────────┘ └──────────┘ └──────────────┘ │  │
│  └──────────────────────────────────────────────────────────┘  │
│                              │                                   │
│                              ▼                                   │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                     Persistence                           │  │
│  │  ┌────────────────────┐    ┌────────────────────────┐    │  │
│  │  │     PostgreSQL     │    │     Vector Store       │    │  │
│  │  │  (Messages, Blocks │    │  (pgvector/qdrant)     │    │  │
│  │  │   Agent State)     │    │  (Archival passages)   │    │  │
│  │  └────────────────────┘    └────────────────────────┘    │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 4.1 Component Breakdown

#### Signal Interface
- JSON-RPC communication with `signal-cli`
- Message receive loop (async)
- Typing indicators
- Read receipts

#### Agent Core
- Main event loop: receive message → think → act → respond
- XML tag parsing for tool extraction
- Tool execution orchestration
- Heartbeat/continuation logic

#### LLM Integration
- `rig-core` for provider abstraction
- OpenAI-compatible API (Maple/Tinfoil)
- Streaming support for real-time responses
- Retry logic for transient failures

#### Memory System (detailed in §5)
- Core Memory: In-context blocks (persona, human, custom)
- Recall Memory: Full conversation history (searchable)
- Archival Memory: Long-term semantic storage
- Summary Memory: Compaction when context overflows

#### Tool Execution (detailed in §6)
- XML tag parser
- Tool registry and dispatch
- Async execution with timeouts
- Result formatting and injection

#### Persistence
- PostgreSQL for structured data
- pgvector or Qdrant for embeddings
- Diesel ORM for type-safe queries
- Migrations for schema evolution

---

## 5. The Memory System

### 5.1 Overview

Sage V2 implements a 4-tier memory system inspired by Letta/MemGPT:

```
┌─────────────────────────────────────────────────────────────┐
│                    Context Window                            │
│  ┌─────────────────────────────────────────────────────┐   │
│  │ System Prompt                                        │   │
│  │  ├─ Base Instructions                                │   │
│  │  ├─ <memory_blocks>                                  │   │
│  │  │   ├─ <persona>...</persona>                       │   │
│  │  │   └─ <human>...</human>                           │   │
│  │  ├─ <tools>                                          │   │
│  │  │   └─ (XML tool descriptions)                      │   │
│  │  └─ <memory_metadata>                                │   │
│  │       └─ (counts, timestamps)                        │   │
│  └─────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────┐   │
│  │ Recent Messages (in-context)                         │   │
│  │  └─ [system_summary?, user, assistant, user, ...]   │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    External Memory                           │
│  ┌──────────────────┐  ┌──────────────────────────────┐    │
│  │  Recall Memory   │  │     Archival Memory          │    │
│  │  (All messages   │  │  (Embedding-indexed          │    │
│  │   in database)   │  │   long-term passages)        │    │
│  └──────────────────┘  └──────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

### 5.2 Core Memory (Blocks)

Core memory consists of editable "blocks" that are always present in the system prompt. Each block has:

```rust
struct MemoryBlock {
    id: Uuid,
    label: String,           // e.g., "persona", "human", "project"
    description: String,     // How this block should influence behavior
    value: String,           // The actual content
    limit: usize,            // Character limit
    read_only: bool,         // Can the agent edit this?
    version: i32,            // Optimistic locking
}
```

Default blocks:
- **persona**: Who the agent is, personality, style
- **human**: Information about the user

The agent can edit blocks via memory tools:
```xml
<memory_replace block="human" old="likes coffee" new="prefers tea"/>
<memory_insert block="human" line="-1">Works at Maple AI as CTO</memory_insert>
```

### 5.3 Recall Memory (Conversation History)

Every message exchanged is persisted:

```rust
struct Message {
    id: Uuid,
    agent_id: Uuid,
    role: MessageRole,       // System, User, Assistant, Tool
    content: String,
    tool_calls: Option<Vec<ToolCall>>,  // Parsed from content
    tool_results: Option<Vec<ToolResult>>,
    created_at: DateTime<Utc>,
    sequence_id: i64,        // Monotonic ordering
}
```

Only a subset of messages are "in-context" (visible to the LLM). The rest are accessible via search:

```xml
<conversation_search query="what did we discuss about databases" limit="5"/>
```

Search is hybrid: keyword matching + semantic similarity (embeddings).

### 5.4 Archival Memory (Long-Term Semantic)

For information that should persist indefinitely and be semantically searchable:

```rust
struct ArchivalPassage {
    id: Uuid,
    agent_id: Uuid,
    content: String,
    embedding: Vec<f32>,     // 768-dim (nomic-embed-text)
    tags: Vec<String>,
    created_at: DateTime<Utc>,
}
```

The agent can insert and search archival memory:
```xml
<archival_insert tags="personal,pets">
Buddy is Alice's 5-year-old golden retriever. He loves fetch and swimming.
</archival_insert>

<archival_search query="information about Alice's dog" top_k="5"/>
```

### 5.5 Summary Memory (Compaction)

When the context window approaches its limit, we "compact" by:

1. Summarizing older messages
2. Inserting a summary as a special `system_alert` message
3. Removing the summarized messages from in-context (but keeping in DB)

```rust
struct SummaryMessage {
    summary: String,
    messages_summarized: Vec<Uuid>,  // Which messages this covers
    created_at: DateTime<Utc>,
}
```

The summary is injected near the start of the message history:
```
[System Prompt]
[Summary of earlier conversation...]
[Recent messages...]
```

### 5.6 Context Window Management

The agent tracks token usage and manages the context window:

```rust
struct ContextWindow {
    max_tokens: usize,           // e.g., 100000 for Kimi K2
    system_prompt_tokens: usize,
    message_tokens: usize,
    
    // In-context message IDs (ordered)
    message_ids: Vec<Uuid>,
}

impl ContextWindow {
    fn needs_compaction(&self) -> bool {
        self.total_tokens() > self.max_tokens * 0.8  // 80% threshold
    }
    
    fn compact(&mut self, summarizer: &Summarizer) -> Result<()> {
        // 1. Select messages to summarize (oldest N messages)
        // 2. Generate summary
        // 3. Insert summary message
        // 4. Remove summarized message IDs from in-context
        // 5. Messages remain in DB for recall search
    }
}
```

### 5.7 System Prompt Assembly

The system prompt is constructed from templates + injected memory:

```rust
fn build_system_prompt(agent: &Agent) -> String {
    let base = SYSTEM_PROMPT_TEMPLATE;
    
    let memory_blocks = agent.memory.compile_blocks();
    let tools = compile_tool_descriptions(&agent.tools);
    let metadata = compile_memory_metadata(agent);
    
    format!(
        "{base}\n\n\
        <memory_blocks>\n{memory_blocks}\n</memory_blocks>\n\n\
        <tools>\n{tools}\n</tools>\n\n\
        <memory_metadata>\n{metadata}\n</memory_metadata>"
    )
}
```

The rebuild only happens when memory blocks change, to avoid churning the system message on every turn.

---

## 6. DSRs/BAML-Based Structured Output

### 6.1 Design Philosophy

**UPDATE (2026-01-23)**: We're moving from raw XML parsing to DSRs (DSPy in Rust) with BAML for structured output parsing. This gives us:
- **Type safety**: Rust structs for inputs and outputs
- **Robust parsing**: BAML handles messy LLM outputs with retries and fuzzy matching
- **Optimization**: GEPA can directly optimize the instruction field
- **No native tool calling**: Still avoiding provider-specific function calling APIs

Instead of parsing raw XML like:
```xml
<msg>Let me search for that.</msg>
<web_search query="rust news"/>
```

We use typed signatures that output structured data:
```rust
#[Signature]
struct AgentResponse {
    #[input] user_message: String,
    #[input] context: String,
    
    #[output] messages: Vec<String>,
    #[output] tool_calls: Vec<ToolCall>,
    #[output] reasoning: String,
}
```

Benefits:
- **Typed**: `messages` is `Vec<String>`, `tool_calls` is `Vec<ToolCall>`
- **Parseable**: BAML extracts structured data from natural LLM output
- **Optimizable**: The instruction (doc comment) can be evolved by GEPA
- **Debuggable**: Still human-readable in logs

### 6.2 Signature Definition

The core agent signature defines what inputs we provide and what outputs we expect:

```rust
use dspy_rs::Signature;

#[derive(Signature, Clone)]
struct AgentResponse {
    /// You are Sage, a helpful AI assistant communicating via Signal.
    /// 
    /// Respond to the user's message. You can:
    /// - Send messages (add strings to the messages array)
    /// - Call tools (add ToolCall objects to the tool_calls array)
    /// - Use reasoning to explain your thought process
    ///
    /// Guidelines:
    /// - Be friendly, concise, and helpful
    /// - Use tools when you need current information
    /// - You can send multiple messages and call multiple tools
    /// - Always provide at least one message to the user

    #[input]
    pub user_message: String,
    
    #[input]
    pub conversation_context: String,
    
    #[input]
    pub available_tools: String,
    
    #[output]
    pub messages: Vec<String>,
    
    #[output]
    pub tool_calls: Vec<ToolCall>,
    
    #[output]
    pub reasoning: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ToolCall {
    name: String,
    args: HashMap<String, String>,
}
```

The doc comment on the struct becomes the **instruction** that GEPA can optimize.

### 6.3 Tool Registry

Tools are registered with descriptions that become part of the prompt:

```rust
trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn args_schema(&self) -> &str;  // JSON schema or example
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult>;
}

struct ToolResult {
    success: bool,
    output: String,
    error: Option<String>,
}
```

Example tool implementation:
```rust
struct WebSearchTool {
    client: BraveClient,
}

impl Tool for WebSearchTool {
    fn name(&self) -> &str { "web_search" }
    
    fn description(&self) -> &str {
        "Search the web for current information, news, facts, etc."
    }
    
    fn args_schema(&self) -> &str {
        r#"{ "query": "search query string", "count": "number of results (default 5)" }"#
    }
    
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let query = args.get("query")
            .ok_or_else(|| anyhow!("query argument required"))?;
        let count = args.get("count")
            .and_then(|c| c.parse().ok())
            .unwrap_or(5);
        
        let results = self.client.search(query, count).await?;
        Ok(ToolResult::success(results.format()))
    }
}
```

Tool descriptions are compiled into the `available_tools` input field.

### 6.4 Built-in Tools

Note: With the typed signature approach, tools are called via `tool_calls` array rather than XML tags.

#### Web
- `web_search` - Search the web for current information
  - args: `query`, `count`

#### Shell
- `shell` - Execute a shell command
  - args: `command`, `timeout`

#### Memory (Core)
- `memory_replace` - Replace text in a memory block
  - args: `block`, `old`, `new`
- `memory_append` - Append text to a memory block
  - args: `block`, `content`

#### Memory (Archival)
- `archival_insert` - Store information in long-term memory
  - args: `content`, `tags`
- `archival_search` - Search long-term memory
  - args: `query`, `top_k`, `tags`

#### Memory (Recall)
- `conversation_search` - Search conversation history
  - args: `query`, `limit`

### 6.5 Tool Prompt Generation

Tool descriptions are passed as the `available_tools` input to the signature:

```
Available tools (add to tool_calls array to use):

web_search:
  Description: Search the web for current information, news, facts, etc.
  Args: { "query": "search query string", "count": "number of results (default 5)" }

archival_insert:
  Description: Store information in long-term memory for future recall
  Args: { "content": "the information to store", "tags": "comma-separated tags" }

conversation_search:
  Description: Search through past conversation history
  Args: { "query": "search query", "limit": "max results (default 5)" }
```

BAML will parse the LLM's response into the typed `AgentResponse` struct.

### 6.6 Parsing and Execution

The agent loop using DSRs:

```rust
async fn process_turn(&mut self, user_message: &str) -> Result<()> {
    let predictor = Predict::<AgentResponse>::new();
    
    // Build input
    let input = AgentResponseInput {
        user_message: user_message.to_string(),
        conversation_context: self.build_context(),
        available_tools: self.tools.generate_description(),
    };
    
    // Get typed response (BAML parses the LLM output)
    let response = predictor.call(input).await?;
    
    // Send messages to user
    for msg in &response.messages {
        self.signal.send_message(&self.user_id, msg).await?;
    }
    
    // Execute tools
    for tool_call in &response.tool_calls {
        if let Some(tool) = self.tools.get(&tool_call.name) {
            let result = tool.execute(&tool_call.args).await?;
            
            // If tools were called, continue with results
            self.inject_tool_result(&tool_call, &result).await?;
        }
    }
    
    Ok(())
}
```

Key difference: BAML handles parsing the LLM's natural language response into typed `messages: Vec<String>` and `tool_calls: Vec<ToolCall>`. No manual XML parsing needed.

### 6.7 Tool Result Injection

After a tool executes, we inject the result into the conversation context for the next turn:

```rust
fn inject_tool_result(&mut self, tool_call: &ToolCall, result: &ToolResult) {
    let result_text = format!(
        "[Tool Result: {}]\nStatus: {}\nOutput: {}",
        tool_call.name,
        if result.success { "OK" } else { "ERROR" },
        result.output
    );
    self.conversation_history.push(Message::tool_result(result_text));
}
```

This becomes part of the conversation context, allowing the model to:
- React to results in subsequent turns
- Chain additional tool calls
- Formulate a final response based on tool outputs

### 6.8 Continuation (Heartbeat)

If the model calls tools, we continue with another turn to let it process results:

```rust
async fn step(&mut self, user_message: &str) -> Result<()> {
    for _ in 0..MAX_STEPS {
        let response = self.process_turn(user_message).await?;
        
        // Persist the typed response
        self.persist_response(&response).await?;
        
        let has_messages = !response.messages.is_empty();
        let has_tools = !response.tool_calls.is_empty();
        
        // Execute tools and inject results
        for tool_call in &response.tool_calls {
            if let Some(tool) = self.tools.get(&tool_call.name) {
                let result = tool.execute(&tool_call.args).await?;
                self.inject_tool_result(&tool_call, &result);
            }
        }
        
        // If we sent messages and have no pending tools, we're done
        if has_messages && !has_tools {
            break;
        }
        
        // If no messages and no tools, something went wrong
        if !has_messages && !has_tools {
            // Inject a nudge and retry
            self.inject_heartbeat();
        }
        
        // Otherwise, continue (tools were called, need to process results)
    }
    Ok(())
}
```

---

## 7. DSPy-Inspired Prompt Optimization

### 7.1 The Problem with Manual Prompting

We spent weeks manually tweaking Letta's prompts:
- "When I decide to use a tool, I CALL it - I don't just describe what I would do."
- "When I use a tool, I call it silently (no announcement), then respond after getting results."

This is:
- Time-consuming
- Fragile (changes break other behaviors)
- Not transferable across models

### 7.2 DSPy/dsrs Approach

Instead of writing prompts, we:

1. **Define signatures** - What are the inputs and outputs?
2. **Write metrics** - How do we measure success?
3. **Optimize** - Let an algorithm find the best prompts

```rust
use dsrs::*;

#[Signature]
struct AgentTurn {
    /// You are Sage, a helpful AI assistant. Respond to the user using 
    /// tools when needed. Use XML tags for tool calls.
    
    #[input]
    context: String,        // System prompt + recent messages
    
    #[input] 
    user_message: String,   // Latest user input
    
    #[output]
    response: String,       // May contain <msg>, <tool>, etc.
}
```

### 7.3 Evaluation Metrics

We define what "good" looks like:

```rust
impl FeedbackEvaluator for SageAgent {
    async fn feedback_metric(
        &self,
        example: &Example,
        prediction: &Prediction
    ) -> FeedbackMetric {
        let response = prediction.get("response").as_str();
        let expected_tools = example.get("expected_tools").as_array();
        
        // Parse actual tool calls from response
        let actual_tools = parse_xml_tags(response)
            .into_iter()
            .filter(|t| t.name != "msg")
            .collect::<Vec<_>>();
        
        // Check if expected tools were called
        let tool_match_score = compare_tool_calls(&expected_tools, &actual_tools);
        
        // Check if response has content
        let has_message = parse_xml_tags(response)
            .iter()
            .any(|t| t.name == "msg" && !t.content.unwrap_or("").is_empty());
        
        // Build feedback
        let mut feedback = String::new();
        
        if !has_message {
            feedback.push_str("Missing <msg> tag - no message sent to user\n");
        }
        
        if tool_match_score < 1.0 {
            feedback.push_str(&format!(
                "Tool mismatch:\n  Expected: {:?}\n  Got: {:?}\n",
                expected_tools, actual_tools
            ));
        }
        
        let score = (tool_match_score + if has_message { 1.0 } else { 0.0 }) / 2.0;
        
        FeedbackMetric::new(score, feedback)
    }
}
```

### 7.4 GEPA Optimization

GEPA (Genetic-Pareto optimizer) evolves prompts:

```rust
let gepa = GEPA::builder()
    .num_iterations(20)
    .minibatch_size(25)
    .temperature(0.9)
    .build();

// Training set: examples of user inputs and expected behaviors
let trainset = vec![
    Example::new(hashmap!{
        "context" => system_prompt,
        "user_message" => "Can you search for the weather?",
        "expected_tools" => vec!["web_search"],
    }),
    Example::new(hashmap!{
        "context" => system_prompt,
        "user_message" => "Remember that I like pizza",
        "expected_tools" => vec!["archival_insert"],
    }),
    // ... more examples
];

let result = gepa.compile_with_feedback(&mut agent, trainset).await?;
println!("Optimized instruction: {}", result.best_candidate.instruction);
```

### 7.5 What Gets Optimized

GEPA can optimize:
- The main system instruction
- Tool descriptions
- Few-shot examples
- Response formatting guidelines

The key insight: **don't hand-craft prompts, generate training data and let the optimizer find what works**.

### 7.6 Continuous Improvement

We can run optimization periodically:
1. Collect real conversation traces with outcomes
2. Label successes and failures
3. Run GEPA to improve prompts
4. Deploy updated prompts

This creates a flywheel: more usage → more data → better prompts → better usage.

---

## 8. Signal Integration

### 8.1 Architecture

```
┌─────────────────┐      JSON-RPC       ┌─────────────────┐
│   Sage Agent    │◄──────────────────▶│   signal-cli    │
│   (Rust)        │     stdin/stdout    │   (Java)        │
└─────────────────┘                     └─────────────────┘
                                               │
                                               │ Signal Protocol
                                               ▼
                                        ┌─────────────────┐
                                        │  Signal Server  │
                                        └─────────────────┘
```

### 8.2 Message Flow

**Incoming:**
1. `signal-cli` receives message via Signal protocol
2. Emits JSON-RPC notification to stdout
3. Sage parses notification, extracts message
4. Triggers agent processing

**Outgoing:**
1. Agent produces response (parsed from `<msg>` tags)
2. Sage sends JSON-RPC request to `signal-cli`
3. `signal-cli` sends via Signal protocol

### 8.3 Features

- **Typing indicators**: Show when agent is "thinking"
- **Read receipts**: Confirm message receipt
- **Multi-message responses**: Send multiple `<msg>` tags as separate messages with delays
- **Group support**: (future) Respond in group chats

### 8.4 Implementation

```rust
pub struct SignalClient {
    process: Child,
    stdin: Mutex<ChildStdin>,
    account: String,
}

impl SignalClient {
    pub async fn send_message(&self, recipient: &str, message: &str) -> Result<()> {
        self.send_request("send", json!({
            "recipient": [recipient],
            "message": message
        })).await
    }
    
    pub async fn send_typing(&self, recipient: &str, stop: bool) -> Result<()> {
        self.send_request("sendTyping", json!({
            "recipient": [recipient],
            "stop": stop
        })).await
    }
    
    pub async fn send_read_receipt(&self, recipient: &str, timestamp: u64) -> Result<()> {
        self.send_request("sendReceipt", json!({
            "recipient": [recipient],
            "type": "read",
            "targetTimestamp": [timestamp]
        })).await
    }
}
```

---

## 9. Data Model

### 9.1 Entity Relationship

```
┌─────────────┐       ┌─────────────┐       ┌─────────────┐
│   Agent     │──────▶│   Message   │       │   Block     │
│             │       │             │       │             │
│ id          │       │ id          │       │ id          │
│ name        │       │ agent_id    │◀──────│ agent_id    │
│ system      │       │ role        │       │ label       │
│ message_ids │       │ content     │       │ value       │
│ block_ids   │       │ sequence_id │       │ limit       │
│ llm_config  │       │ created_at  │       │ version     │
└─────────────┘       └─────────────┘       └─────────────┘
       │                                           
       │              ┌─────────────┐       
       └─────────────▶│  Passage    │       
                      │  (Archival) │       
                      │             │       
                      │ id          │       
                      │ agent_id    │       
                      │ content     │       
                      │ embedding   │       
                      │ tags        │       
                      └─────────────┘       
```

### 9.2 SQL Schema

```sql
-- Agents
CREATE TABLE agents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(255) NOT NULL,
    system_prompt TEXT NOT NULL,
    llm_config JSONB NOT NULL,
    message_ids UUID[] NOT NULL DEFAULT '{}',  -- In-context messages
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Messages (recall memory substrate)
CREATE TABLE messages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id UUID NOT NULL REFERENCES agents(id),
    role VARCHAR(20) NOT NULL,  -- system, user, assistant, tool
    content TEXT NOT NULL,
    tool_calls JSONB,           -- Parsed tool calls
    tool_results JSONB,         -- Tool execution results
    sequence_id BIGSERIAL,      -- Monotonic ordering
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_messages_agent_seq ON messages(agent_id, sequence_id);

-- Memory blocks (core memory)
CREATE TABLE blocks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id UUID NOT NULL REFERENCES agents(id),
    label VARCHAR(100) NOT NULL,
    description TEXT,
    value TEXT NOT NULL DEFAULT '',
    char_limit INT NOT NULL DEFAULT 5000,
    read_only BOOLEAN NOT NULL DEFAULT FALSE,
    version INT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(agent_id, label)
);

-- Archival passages (long-term memory)
CREATE TABLE passages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id UUID NOT NULL REFERENCES agents(id),
    content TEXT NOT NULL,
    embedding VECTOR(768),      -- pgvector
    tags TEXT[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_passages_embedding ON passages 
    USING ivfflat (embedding vector_cosine_ops);
CREATE INDEX idx_passages_tags ON passages USING GIN(tags);

-- Summaries (compaction artifacts)
CREATE TABLE summaries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id UUID NOT NULL REFERENCES agents(id),
    summary TEXT NOT NULL,
    message_ids UUID[] NOT NULL,  -- Messages this summarizes
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### 9.3 Rust Types

```rust
// Core types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: Uuid,
    pub name: String,
    pub system_prompt: String,
    pub llm_config: LlmConfig,
    pub message_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub role: MessageRole,
    pub content: String,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_results: Option<Vec<ToolResult>>,
    pub sequence_id: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBlock {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub label: String,
    pub description: Option<String>,
    pub value: String,
    pub char_limit: i32,
    pub read_only: bool,
    pub version: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Passage {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub content: String,
    pub embedding: Vec<f32>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}
```

---

## 10. Implementation Phases

### Phase 1: Core Loop ✅ COMPLETE

**Goal**: Basic agent that can respond to messages.

- [x] Signal client (JSON-RPC with signal-cli)
- [x] DSRs/BAML structured output parsing (replaced XML)
- [x] Basic agent loop (receive → LLM → respond)
- [x] Step-based execution for immediate message delivery
- [x] Multi-message response support
- [x] Typing indicators and read receipts

**Deliverable**: Can have a basic conversation via Signal.

### Phase 2: Tool Execution ✅ COMPLETE

**Goal**: Agent can execute tools.

- [x] Tool trait and registry
- [x] Web search tool (Brave Search Pro with rich data)
- [x] Memory tools (replace, append, insert, search)
- [x] Tool result injection
- [x] Continuation/heartbeat logic (step-based)

**Deliverable**: Can search the web and run commands.

### Phase 3: Persistence ✅ COMPLETE

**Goal**: Messages and state persist across restarts.

- [x] PostgreSQL integration (Diesel)
- [x] Message storage and retrieval with embeddings
- [x] Agent state persistence
- [x] Database migrations (7 migrations)
- [x] Dedicated PostgreSQL container with pgvector

**Deliverable**: Conversations survive restarts.

### Phase 4: Memory System ✅ COMPLETE

**Goal**: Full 4-tier memory system.

- [x] Core memory blocks (persona, human)
- [x] Memory editing tools (replace, append, insert)
- [x] Recall memory (conversation search with embeddings)
- [x] Archival memory (pgvector semantic search)
- [x] pgvector integration (768-dim nomic-embed-text)
- [x] Summary/compaction (auto-summarization at 80% threshold)
- [x] User preferences system (timezone, etc.)
- [x] Scheduled tasks and reminders (cron + one-off)

**Deliverable**: Agent remembers across sessions, can search history.

### Phase 5: DSPy Integration ✅ COMPLETE (Basic)

**Goal**: Structured output with typed signatures.

- [x] DSRs/BAML integration for structured output
- [x] AgentResponse signature with typed fields
- [x] Correction agent for malformed responses
- [ ] Evaluation metrics (future)
- [ ] Training data collection (future)
- [ ] GEPA optimization runs (future)

**Deliverable**: Reliable structured output parsing without native tool calling.

### Phase 6: Polish ✅ MOSTLY COMPLETE

**Goal**: Production-ready system.

- [x] Error handling and recovery (correction agent)
- [x] Logging and observability (tracing)
- [x] Configuration management (environment variables)
- [x] Nix development environment
- [x] Documentation (README, SAGE_V2_DESIGN)
- [ ] Containerization (Docker/Podman) - partial
- [ ] Testing suite - minimal

**Deliverable**: Reliable, deployable agent.

### Phase 7: Future Enhancements 🔜

- [ ] Gmail/Calendar integration
- [ ] Natural language time parsing ("in 2 hours", "next Tuesday")
- [ ] GEPA prompt optimization with real usage data
- [ ] Group chat support
- [ ] Voice messages
- [ ] Image understanding

---

## 11. Technology Stack

### Core
- **Language**: Rust 2021 edition
- **Async Runtime**: Tokio
- **LLM Framework**: rig-core
- **DSPy**: dsrs (dspy-rs)

### Persistence
- **Database**: PostgreSQL 16
- **ORM**: Diesel
- **Migrations**: diesel_migrations
- **Vector Store**: pgvector (or Qdrant)

### Communication
- **Signal**: signal-cli (JSON-RPC)
- **HTTP**: reqwest

### LLM Provider
- **Primary**: Maple/Tinfoil (OpenAI-compatible)
- **Embeddings**: nomic-embed-text via Maple

### Observability
- **Logging**: tracing + tracing-subscriber
- **Metrics**: (future) prometheus

### Development
- **Build**: Cargo
- **Task Runner**: just
- **Containerization**: Podman/Docker

---

## Appendix: Letta Lessons Learned

### What Worked

1. **4-tier memory model** - Excellent abstraction for long-term agents
2. **Persistence-first** - Making context window explicit state
3. **XML-tagged prompt sections** - Clean separation of concerns
4. **Tool execution loop** - Heartbeat/continuation pattern
5. **Context compaction** - Summarize rather than lose data

### What Didn't Work

1. **Native tool calling** - Provider bugs make this unreliable
2. **Black box architecture** - Hard to debug when things break
3. **Python performance** - Slow for real-time chat
4. **Implicit state management** - Hard to reason about context window
5. **No prompt optimization** - Manual prompting is fragile

### Key Takeaways

1. **Control is more important than features** - Better to have fewer features that work reliably
2. **Text is the universal interface** - LLMs produce text; work with that, not against it
3. **Persistence enables recovery** - If everything is persisted, you can always recover
4. **Observability is not optional** - You must be able to see exactly what the LLM sees
5. **Automate prompt improvement** - Don't hand-tune forever; collect data and optimize

---

## Document History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 0.1.0 | 2026-01-22 | Droid | Initial draft |
| 1.0.0 | 2026-01-26 | Droid | Production release - all core features complete |

---

*Sage V2 is now in production! The document reflects the implemented system.*
