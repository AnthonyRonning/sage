# Personal AI Agent Project: Brainstorm Document

## Project Vision

Build a highly private, self-hosted AI agent/companion that lives in a Docker container on a home server with full permissions to access personal data and perform actions autonomously. The agent prioritizes privacy and data sovereignty through confidential computing and self-hosted infrastructure.

## Core Requirements

- **Maximum Privacy**: All data stays local, LLM inference via confidential compute (Maple)
- **Full Permissions**: Agent can access personal files, services, and take actions on behalf of the user
- **Self-Hosted**: Runs on existing home server infrastructure
- **Open Source**: Preference for open source solutions over proprietary frameworks
- **Conversational Interface**: Primary interaction via Signal messaging

## Architecture Decisions

### Chosen Stack

```
Docker Container (main agent):
├── Agent Binary (Rust)
│   ├── rig-core         → Agent orchestration, tool dispatch, model abstraction
│   ├── axum             → HTTP server for webhooks, health checks, callbacks
│   └── signal-cli       → Chat interface integration
├── Letta                → Long-term memory, conversation state, context management
└── Valkey               → Task queue, scheduling, pub/sub

External Services (existing infrastructure):
├── PostgreSQL           → Structured data, logs, Letta backend, audit trail
└── Maple API            → Confidential compute LLM inference proxy
```

### Technology Rationale

| Component | Choice | Reasoning |
|-----------|--------|-----------|
| Language | Rust | Developer preference, reliability, existing expertise |
| Agent Framework | rig-core | Open source, Rust-native, flexible tool dispatch, supports custom model backends |
| Memory | Letta | Handles memory injection/persistence, wraps LLM calls, supports Postgres backend |
| LLM Inference | Maple | Confidential computing, privacy guarantees for personal data |
| Chat Interface | signal-cli | Encrypted messaging, no custom client needed, works on mobile |
| Database | PostgreSQL | Already running on server, Letta compatible |
| Task Queue | Valkey | Simple sorted sets for scheduled tasks, pub/sub for real-time |

### Rejected Alternatives

- **Claude Code SDK / Droid**: Closed source, conflicts with open source requirement
- **SQLite for primary DB**: PostgreSQL already available on server
- **Custom memory solution**: Letta handles the hard parts of memory management

## Technical Details

### Rig-Core Integration

- Implement `CompletionModel` trait pointing at Maple API proxy
- Define tools as rig-core tool structs
- Agent loop: receive message → build context → call LLM → parse tool calls → execute → loop

### Letta Architecture

- Runs as separate service within the container
- Acts as proxy/interception layer for LLM calls
- Injects relevant memories into context automatically
- Persists conversation state and learnings to PostgreSQL
- Latency: ~10-50ms per memory operation (negligible vs LLM inference time)

### Valkey Task Queue

Simple approach using sorted sets:
- `ZADD` with timestamp scores for scheduling
- `ZPOPMIN` to grab due tasks
- Sufficient for personal agent scale, no need for heavy job frameworks

### Signal-CLI Considerations

- Needs its own registered Signal account
- Requires periodic attention (re-linking, session management)
- Can drop messages occasionally - implement good logging
- Consider fallback interface or at minimum comprehensive audit logs

## Key Challenges to Address

### 1. Tool Design (High Priority)
The agent's usefulness depends entirely on well-designed tools. Start simple:
- Begin with read-only tools before write operations
- "Read my calendar" is easy; "help me manage my schedule" is fuzzy
- Each tool needs clear boundaries and failure modes

### 2. Tool Sandboxing
Even with full permissions, implement guardrails:
- Confirmation flow for destructive operations
- Prevent hallucinated dangerous commands
- Logging before execution for audit trail

### 3. Context Window Management
Long conversations + personal data access = context limits:
- Letta helps with memory management
- Need summarization strategies
- Retrieval patterns for relevant context

### 4. State Persistence & Recovery
- What happens on container restart?
- How to pick up conversation context?
- Graceful handling of interrupted operations

### 5. Audit Logging
Log everything:
- Every tool invocation with parameters
- Every action taken
- Decision reasoning where possible
- Essential for debugging "why did it do that?"

### 6. Prompt Engineering
- Agent personality and behavior tuning
- Tool usage patterns
- Lots of trial and error expected

## Estimated Timeline

| Milestone | Timeframe |
|-----------|-----------|
| Basic agent loop + 1-2 tools + Signal working | Few weekends |
| Memory integration with Letta tuned | 1-2 weeks additional |
| Actually useful for daily tasks | 1-2 months of iteration |

"Works at all" comes fast. "Works well enough to use daily" takes longer.

## Open Questions for Further Brainstorming

1. **First tools to build?**
   - File system access patterns (direct mount vs API?)
   - Calendar integration
   - Email access
   - Home automation
   
2. **Confirmation UX for dangerous operations?**
   - Reply-to-confirm in Signal?
   - Whitelist of safe operations?
   
3. **How to handle multi-turn tool operations?**
   - Agent autonomy level
   - When to ask for clarification vs proceed
   
4. **Backup and disaster recovery?**
   - Postgres backups
   - Container state
   - Memory/conversation history

5. **Future interfaces beyond Signal?**
   - Voice?
   - Web UI for complex interactions?
   - API for other automations?

## Next Steps for Coding Agent

1. **Validate rig-core Maple integration**
   - Check current `CompletionModel` trait API
   - Prototype basic completion through Maple proxy

2. **Set up Letta with Postgres backend**
   - Docker compose for local dev
   - Verify memory injection works with custom model backend

3. **Scaffold Rust project structure**
   - Cargo workspace if needed
   - Core agent binary structure
   - Tool trait/interface design

4. **Signal-cli proof of concept**
   - Account registration flow
   - Basic message receive/send loop
   - Error handling and reconnection

5. **Design first tool**
   - Suggest: simple file read operation
   - Implement with logging and error handling
   - Test end-to-end through Signal

## Resources

- [rig-core](https://github.com/0xPlaygrounds/rig) - Rust AI agent framework
- [Letta](https://github.com/letta-ai/letta) - Memory management for LLM agents
- [signal-cli](https://github.com/AsamK/signal-cli) - Signal messenger CLI
- [Maple](https://maple.ai) - Confidential compute LLM inference
- [Valkey](https://valkey.io) - Redis-compatible key-value store

---

*Document created from brainstorming session. Ready for handoff to coding agent for implementation planning and prototyping.*
