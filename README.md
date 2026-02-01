# Sage

**Privacy-first personal AI agent with persistent memory, built in Rust.**

> ⚠️ **Experimental** - This is a proof of concept / personal project exploring ideas around private, memory-augmented AI agents. It works, but expect rough edges.

## What is Sage?

Sage is an AI assistant that prioritizes **privacy** and **data sovereignty**. It's designed to be a trusted companion that remembers your conversations, learns about you over time, and can take actions on your behalf - all while keeping your data under your control.

**Key Features:**
- **End-to-end encrypted messaging** via Signal
- **Long-term memory** that persists across conversations
- **Confidential compute** - LLM inference runs in a TEE (Trusted Execution Environment)
- **Self-hosted** - all data stays on your machine
- **Multi-user support** with isolated memory per conversation

## Why Build This?

Most AI assistants are stateless - they forget everything after each conversation. The few that have memory send your data to cloud servers you don't control. Sage takes a different approach:

- Your conversations stay on **your** PostgreSQL instance
- LLM inference happens in **confidential compute** (Maple/TEE) - the inference provider can't see your prompts
- Communication happens over **Signal's E2E encryption**
- The agent runs in **your container** on your infrastructure

## Technical Highlights

This project explores several unconventional design choices:

### No Native Tool Calling

Instead of relying on LLM providers' function calling APIs (which are buggy and provider-specific), Sage uses **structured output parsing** via [DSRs](https://github.com/krypticmouse/DSRs) (DSPy in Rust) with BAML. The LLM outputs natural text that gets parsed into typed Rust structs. This approach:
- Works identically across all LLM providers
- Is immune to vLLM/provider-specific tool calling bugs
- Is fully debuggable (just look at the text output)

### Regenerated Context, Not Append-Only

Rather than maintaining an ever-growing message log, Sage **regenerates the full context** on each request:
- Single system prompt with injected memory blocks
- Recent conversation history (not the full log)
- No KV cache dependency - works with any provider

### Letta-Inspired Memory Architecture

Custom implementation of a 4-tier memory system (inspired by [Letta](https://github.com/letta-ai/letta)/MemGPT):

| Layer | Purpose | Storage |
|-------|---------|---------|
| **Core Memory** | Always in context (persona, user info) | PostgreSQL |
| **Recall Memory** | Searchable conversation history | PostgreSQL + TEE embeddings |
| **Archival Memory** | Long-term semantic storage | pgvector + TEE embeddings |
| **Summary Memory** | Auto-compaction when context overflows | PostgreSQL |

All embeddings are generated via Maple's TEE-based embedding API (nomic-embed-text), meaning your memory content stays private even during vector encoding.

### Built for Prompt Optimization

The codebase is structured around [DSRs](https://github.com/krypticmouse/DSRs) signatures, enabling future **GEPA (Genetic-Pareto) optimization** of prompts based on collected traces and feedback metrics.

### DSRs Signature Architecture

Sage uses typed DSRs signatures to define the contract between inputs and outputs. This makes the agent's interface explicit, debuggable, and optimizable.

**Main Agent Signature (`AgentResponse`):**

```rust
#[derive(dspy_rs::Signature)]
pub struct AgentResponse {
    // Inputs
    #[input(desc = "The input to respond to - either a user message or tool execution result")]
    pub input: String,

    #[input(desc = "Compacted summary of very old messages (only present for long conversations)")]
    pub previous_context_summary: String,

    #[input(desc = "Recent conversation history including your messages and tool results")]
    pub conversation_context: String,

    #[input]
    pub available_tools: String,

    // Outputs
    #[output(desc = "Your reasoning/thought process (think step by step)")]
    pub reasoning: String,

    #[output(desc = "Array of messages to send to the user (can be empty)")]
    pub messages: Vec<String>,

    #[output(desc = "Array of tool calls to execute (can be empty)")]
    pub tool_calls: Vec<ToolCall>,
}
```

**How it works:** DSRs compiles this signature + instruction into a single prompt with field markers (`[[ ## field ## ]]`). The LLM outputs structured text that gets parsed back into typed Rust structs via BAML.

<details>
<summary><strong>Example: Compiled Prompt → LLM Response</strong></summary>

When Sage processes a message, DSRs generates a prompt like this:

```
Your input fields are:
1. `input` (string): The input to respond to - either a user message or tool execution result
2. `previous_context_summary` (string): Compacted summary of very old messages
3. `conversation_context` (string): Recent conversation history
4. `available_tools` (string)

Your output fields are:
1. `reasoning` (string): Your reasoning/thought process
2. `messages` (string[]): Array of messages to send to the user
3. `tool_calls` (ToolCall[]): Array of tool calls to execute

[[ ## input ## ]]
What's the weather like in Austin today?

[[ ## previous_context_summary ## ]]

[[ ## conversation_context ## ]]
Current time: 02/01/2026 10:30:00 (Sunday) (America/Chicago)

<memory_blocks>
<persona>
I am Sage, a helpful AI assistant communicating via Signal.
</persona>
<human>
Name: Alex
Location: Austin, TX
Preferences: Prefers concise responses
</human>
</memory_blocks>

Recent conversation:
[user @ 01/31/2026 18:45:00]: hey sage, can you help me with something tomorrow?
[assistant @ 01/31/2026 18:45:12]: Of course! Just let me know what you need.
...

[[ ## available_tools ## ]]
Available tools:
web_search:
  Description: Search the web with AI summaries
  Args: {"query": "search query", "location": "city for local results"}
...

[[ ## reasoning ## ]]
[[ ## messages ## ]]
[[ ## tool_calls ## ]]
[[ ## completed ## ]]
```

The LLM responds with structured output:

```
[[ ## reasoning ## ]]
Alex is asking about weather in Austin. I should use web_search with their location
to get current conditions. I'll keep my response concise per their preferences.

[[ ## messages ## ]]
["Let me check the current weather for you."]

[[ ## tool_calls ## ]]
[{"name": "web_search", "args": {"query": "weather Austin TX today", "location": "Austin, TX"}}]

[[ ## completed ## ]]
```

DSRs parses this back into a typed `AgentResponse` struct that Sage uses to execute tools and send messages.

</details>

**Other signatures in the codebase:**
- `SummarizeConversation` - Compacts old messages when context window fills
- `CorrectionResponse` - Fixes malformed LLM outputs (self-healing)

## Stack

| Component | Choice | Why |
|-----------|--------|-----|
| Language | **Rust** | Performance, type safety, reliability |
| LLM | **Kimi K2** (thinking variant) | Strong tool use, 128k context |
| Inference | **Maple** | TEE-based confidential compute |
| Embeddings | **nomic-embed-text** | Via Maple |
| Messaging | **Signal** (signal-cli) | E2E encrypted, works on mobile |
| Database | **PostgreSQL + pgvector** | Structured data + vector search |
| Framework | **DSRs** (dspy-rs) | Typed signatures, BAML parsing |

## Tools

| Tool | Description |
|------|-------------|
| `web_search` | Brave Search with AI summaries |
| `shell` | Execute commands in workspace |
| `memory_replace/append/insert` | Edit core memory blocks |
| `archival_insert/search` | Long-term semantic memory |
| `conversation_search` | Search conversation history |
| `schedule_task` | Reminders (cron or one-off) |
| `set_preference` | User preferences (timezone, etc.) |

## Quick Start

### Prerequisites

- [Podman](https://podman.io/) or Docker
- signal-cli registered with a phone number
- Maple API access (or compatible OpenAI endpoint)

### Option 1: Docker (Recommended)

Pre-built images are available for `linux/amd64` and `linux/arm64`:

```bash
# Pull the latest image
docker pull ghcr.io/anthonyronning/sage:latest

# Clone for docker-compose and configs
git clone https://github.com/AnthonyRonning/sage.git
cd sage

# Configure environment
cp .env.example .env
# Edit .env with your settings

# Initialize signal-cli data volume (requires existing signal-cli registration)
just signal-init

# Start all services (postgres, signal-cli, sage)
docker compose up -d
```

Or use the image directly in your own compose setup:

```yaml
services:
  sage:
    image: ghcr.io/anthonyronning/sage:latest
    environment:
      - DATABASE_URL=postgres://sage:sage@postgres:5432/sage
      - MAPLE_API_URL=https://your-maple-endpoint
      - MAPLE_API_KEY=your-api-key
      - SIGNAL_CLI_HOST=signal-cli
      - SIGNAL_CLI_PORT=7583
      - SIGNAL_PHONE_NUMBER=+1234567890
```

### Option 2: Build from Source

Requires [Nix](https://nixos.org/download.html) with flakes enabled:

```bash
git clone https://github.com/AnthonyRonning/sage.git
cd sage
nix develop

cp .env.example .env
# Edit .env with your settings

just signal-init  # Copy signal-cli data to volume
just build        # Build container
just start        # Start all services
```

### Configuration

```bash
# Required
MAPLE_API_URL=https://your-maple-endpoint
MAPLE_API_KEY=your-api-key
MAPLE_MODEL=maple/kimi-k2-thinking
SIGNAL_PHONE_NUMBER=+1234567890

# Optional
BRAVE_API_KEY=your-brave-key  # For web search
SIGNAL_ALLOWED_USERS=*        # Or comma-separated UUIDs
```

## Architecture

```
┌─────────────────┐     Signal      ┌─────────────────┐
│   Your Phone    │◄──────────────►│   signal-cli    │
└─────────────────┘    (encrypted)  └────────┬────────┘
                                             │ JSON-RPC
                                             ▼
┌─────────────────────────────────────────────────────┐
│                    Sage (Rust)                      │
│  ┌─────────────┐  ┌─────────────┐  ┌────────────┐  │
│  │   Agent     │  │   Memory    │  │   Tools    │  │
│  │   Manager   │  │   System    │  │            │  │
│  └─────────────┘  └─────────────┘  └────────────┘  │
└─────────────────────────┬───────────────────────────┘
                          │
        ┌─────────────────┼─────────────────┐
        ▼                 ▼                 ▼
┌───────────────┐ ┌───────────────┐ ┌───────────────┐
│  PostgreSQL   │ │    Maple      │ │ Brave Search  │
│  + pgvector   │ │    (TEE)      │ │               │
└───────────────┘ └───────────────┘ └───────────────┘
```

## Privacy Model

| Layer | Protection |
|-------|------------|
| **Transport** | Signal E2E encryption |
| **Inference** | Maple TEE (confidential compute) |
| **Embeddings** | Maple TEE (memory vectors generated privately) |
| **Storage** | Local PostgreSQL (your machine) |
| **Search** | Brave (privacy-respecting, no tracking) |

## Project Status

**Working:**
- Multi-user conversations with memory isolation
- Web search, shell commands, scheduling
- Auto-reconnect on Signal connection drops
- Context compaction when approaching limits

**Future:**
- GEPA prompt optimization
- Gmail/Calendar integration
- Group chat support
- Voice messages

## Related Projects

- [Letta](https://github.com/letta-ai/letta) - Memory management inspiration
- [DSRs](https://github.com/krypticmouse/DSRs) - DSPy in Rust
- [signal-cli](https://github.com/AsamK/signal-cli) - Signal CLI interface
- [Maple](https://www.trymaple.ai/) - Confidential compute LLM inference

## License

MIT
