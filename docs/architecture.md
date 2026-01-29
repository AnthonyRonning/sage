# Sage Architecture

## Philosophy

Sage is a privacy-first personal AI agent. You interact with Sage like a trusted friend via Signal. Sage handles everything else using confidential compute (Maple/TEE) and long-term memory (PostgreSQL/pgvector).

## Current Implementation

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              YOUR PHONE                                     │
│                         Signal Messenger App                                │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      │ Signal Protocol (E2E encrypted)
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         signal-cli (Container)                              │
│                      JSON-RPC daemon on port 7583                           │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      │ TCP JSON-RPC
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           SAGE (Rust)                                       │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                         main.rs                                      │   │
│  │  - Message loop with typing indicators                               │   │
│  │  - Multi-user agent management                                       │   │
│  │  - Auto-reconnect on connection failures                             │   │
│  └──────────────────────────────┬──────────────────────────────────────┘   │
│                                 │                                           │
│  ┌──────────────────┐          │          ┌──────────────────────────┐    │
│  │  signal.rs       │◄─────────┴─────────►│  sage_agent.rs           │    │
│  │  - TCP JSON-RPC  │                      │  - DSRs signatures       │    │
│  │  - Typing        │                      │  - Tool execution        │    │
│  │    indicators    │                      │  - Response generation   │    │
│  │  - Retry logic   │                      └───────────┬──────────────┘    │
│  └──────────────────┘                                  │                    │
│                                                        │                    │
│  ┌──────────────────┐          ┌──────────────────────┴──────────────┐    │
│  │  config.rs       │          │  memory/                            │    │
│  │  - Environment   │          │  - block.rs (core memory)           │    │
│  │    variables     │          │  - recall.rs (conversation)         │    │
│  │  - Workspace     │          │  - archival.rs (long-term)          │    │
│  └──────────────────┘          │  - compaction.rs (summaries)        │    │
│                                └─────────────────────────────────────┘    │
│                                                                             │
│  ┌──────────────────────────────────────────────────────────────────────┐  │
│  │  Tools                                                                │  │
│  │  - web_search (Brave)    - shell (commands)                          │  │
│  │  - memory_* (blocks)     - archival_* (long-term)                    │  │
│  │  - schedule_* (tasks)    - preference_* (settings)                   │  │
│  └──────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      │ Diesel ORM
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                     PostgreSQL + pgvector (Container)                       │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────────────────┐ │
│  │  messages       │  │  blocks         │  │  passages                   │ │
│  │  (history +     │  │  (core memory)  │  │  (archival with             │ │
│  │   embeddings)   │  │                 │  │   vector embeddings)        │ │
│  └─────────────────┘  └─────────────────┘  └─────────────────────────────┘ │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────────────────┐ │
│  │  agents         │  │  preferences    │  │  scheduled_tasks            │ │
│  │  (per-user)     │  │  (user settings)│  │  (reminders)                │ │
│  └─────────────────┘  └─────────────────┘  └─────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      │ OpenAI-compatible API
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                     MAPLE (Confidential Compute)                            │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │  Kimi K2 (thinking variant)                                          │   │
│  │  - Running in TEE (Trusted Execution Environment)                    │   │
│  │  - enclave.trymaple.ai backend                                       │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Core Components

### 1. Sage Core (Rust)

The main process that runs Sage:

- **main.rs**: Entry point, message loop, multi-user management
- **signal.rs**: Signal JSON-RPC interface with auto-reconnect
- **sage_agent.rs**: DSRs-based agent with tool execution
- **agent_manager.rs**: Per-user agent isolation
- **memory/**: 4-tier memory system
- **config.rs**: Environment-based configuration

### 2. Signal Interface

Primary communication channel via signal-cli:
- TCP JSON-RPC mode for container communication
- Auto-reconnect on broken pipe
- Retry logic (up to 3 attempts)
- Typing indicators
- User allowlist for security

### 3. Memory System

Four-tier memory inspired by Letta/MemGPT:

- **Core Memory**: Editable blocks (persona, human) always in context
- **Recall Memory**: Full conversation history with embeddings
- **Archival Memory**: Long-term semantic storage (pgvector)
- **Summary Memory**: Auto-compaction when context exceeds threshold

### 4. Maple (LLM Backend)

Confidential compute for privacy:
- Kimi K2 model optimized for tool calling
- Runs in TEE (Trusted Execution Environment)
- All inference is private - no logs, no training on your data

## Data Flows

### Message Flow (You → Sage → You)

```
1. You send Signal message
2. signal-cli receives, forwards via TCP JSON-RPC
3. Sage starts typing indicator
4. Agent manager routes to user's agent
5. Memory context assembled (blocks + recent history)
6. LLM processes with tools available
7. If tool needed: execute and continue
8. Response parsed, sent as Signal messages
9. Conversation persisted with embeddings
```

### Multi-User Isolation

Each Signal user gets:
- Separate agent instance
- Isolated memory blocks
- Private conversation history
- Own archival memory
- Separate preferences and scheduled tasks

## Security Model

- **Signal**: End-to-end encrypted messaging
- **Maple/TEE**: LLM inference in confidential compute
- **User Allowlist**: Only approved users can interact (or `*` for all)
- **Brave Search**: Privacy-respecting web search (no tracking)
- **Local PostgreSQL**: All memory stays on your machine

## Current Capabilities

| Capability | Status | Notes |
|------------|--------|-------|
| Signal messaging | ✅ Working | TCP JSON-RPC with auto-reconnect |
| Multi-user support | ✅ Working | Isolated agents per user |
| Long-term memory | ✅ Working | Core blocks + archival |
| Conversation history | ✅ Working | With semantic search |
| Web search | ✅ Working | Brave Search |
| Shell commands | ✅ Working | In workspace directory |
| Typing indicators | ✅ Working | During processing |
| Scheduled tasks | ✅ Working | Cron and one-off reminders |
| User preferences | ✅ Working | Timezone, etc. |

## Networking (Container Setup)

When running with Podman/Docker:
- Sage runs in container with `--network host`
- signal-cli runs in separate container on port 7583
- PostgreSQL runs in container on port 5434
- Maple accessed via configured API URL

## Future Architecture

See `roadmap.md` for planned additions:
- Gmail/Calendar integration
- Group chat support
- Voice messages
- MCP (Model Context Protocol)
