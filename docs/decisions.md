# Architecture Decision Records

## ADR-001: Python with Letta for Agent Framework

**Status**: Accepted (Updated from Rust)

**Context**: Originally chose Rust with rig-core, but pivoted after realizing Letta provides comprehensive agent memory management that would take months to reimplement.

**Decision**: Use Python with Letta as the agent framework.

**Rationale**:
- Letta provides production-ready memory management (sliding window, summarization, archival)
- Memory as tools concept (agent can search/update its own memory)
- Tool sandbox for safe execution
- Python ecosystem better for LLM/agent experimentation
- Faster iteration than Rust for this domain

**Consequences**:
- Rust code archived in `crates/` for reference
- UV for Python dependency management
- Letta container dependency

---

## ADR-002: Letta for Memory Management

**Status**: Accepted

**Context**: Long-term memory, context injection, and conversation state are complex problems.

**Decision**: Use Letta as a separate service for memory.

**Rationale**:
- Purpose-built for LLM memory management
- Core memory blocks (persona, human) for identity
- Automatic context window management
- PostgreSQL backend for persistence
- Tool execution sandbox included

**Consequences**:
- Additional container to manage (Podman)
- Networking complexity with OrbStack (IPv6 for host access)
- Letta model handle bug requires passing configs directly

---

## ADR-003: Kimi K2 via Maple for Privacy

**Status**: Accepted

**Context**: Need an LLM that excels at tool calling while maintaining privacy.

**Decision**: Use Kimi K2 (thinking variant) through Maple proxy.

**Rationale**:
- Supports up to 200 consecutive tool calls
- Excellent at agentic tasks
- Maple provides confidential compute (TEE)
- No logs, no training on user data
- OpenAI-compatible API

**Consequences**:
- Dependent on Maple proxy / Maple Desktop
- IPv6 addressing for container → host communication
- Maple (enclave.trymaple.ai) as backend

---

## ADR-004: Signal as Primary Interface

**Status**: Accepted

**Context**: Need a communication channel that is private and works on mobile.

**Decision**: Use Signal via signal-cli JSON-RPC mode.

**Rationale**:
- End-to-end encrypted
- Works on mobile without custom app
- signal-cli provides programmatic access
- Matches "text a friend" interaction model

**Consequences**:
- signal-cli ARM64 binary needed for Apple Silicon
- JSON-RPC mode for bidirectional communication
- Typing indicators require periodic refresh (~10s)

---

## ADR-005: Brave Search for Web Search

**Status**: Accepted

**Context**: Need web search capability without compromising privacy.

**Decision**: Use Brave Search API instead of Letta's built-in Exa.

**Rationale**:
- Privacy-respecting (no tracking)
- Good search quality
- Simple API
- Aligns with project's privacy-first philosophy

**Consequences**:
- Requires BRAVE_API_KEY
- Tool executes in Letta sandbox (uses `requests` library)
- API key passed as tool execution environment variable

---

## ADR-006: UV for Python Dependencies

**Status**: Accepted

**Context**: Need reliable Python dependency management.

**Decision**: Use UV instead of pip/poetry/conda.

**Rationale**:
- Fast dependency resolution
- Lockfile support (uv.lock)
- Works well with Nix development environment
- Simple pyproject.toml configuration

**Consequences**:
- `uv sync` for installing dependencies
- `uv run` for executing scripts
- Python 3.11 pinned in .python-version

---

## ADR-007: Podman for Letta Container

**Status**: Accepted

**Context**: Need to run Letta server with PostgreSQL.

**Decision**: Use Podman (not Docker) for container management.

**Rationale**:
- Available in NixOS packages
- Rootless containers
- Docker-compatible commands
- Works with OrbStack on macOS

**Consequences**:
- IPv6 addressing required for host access from container
- `podman-compose` or direct `podman run` commands
- Letta config file for custom LLM/embedding endpoints

---

## ADR-008: Continuous Typing Indicator

**Status**: Accepted

**Context**: Signal typing indicators timeout after ~15 seconds, but tool calls can take 30+ seconds.

**Decision**: Implement background thread that refreshes typing indicator every 10 seconds during long operations.

**Rationale**:
- Better UX - user knows Sage is working
- Simple implementation with threading.Event
- Stops immediately when response is ready

**Consequences**:
- Additional thread per message processing
- More Signal API calls during long operations
- Cleaner code with `with_typing_indicator()` helper

---

## ADR-009: Per-User Agents in Letta

**Status**: Accepted

**Context**: Need to support multiple users with separate conversation histories.

**Decision**: Create one Letta agent per user (by UUID).

**Rationale**:
- Complete isolation between users
- Each user has their own memory blocks
- Agent name convention: `sage-{user_uuid[:8]}`

**Consequences**:
- Agent lookup on each message
- Agents persist in Letta PostgreSQL
- Tools must be re-attached when creating new agents

---

## ADR-010: Debug Logging for Response Parsing

**Status**: Accepted

**Context**: Letta response format can vary (tool calls, reasoning, assistant messages).

**Decision**: Add debug logging to trace response message structure.

**Rationale**:
- Helps diagnose "null" response issues
- Shows tool call → tool return → assistant message flow
- Can be disabled in production

**Consequences**:
- More verbose logs when DEBUG enabled
- Easier troubleshooting of LLM/Letta issues

---

## Historical ADRs (from Rust phase)

The following ADRs were made during the Rust implementation phase. They remain for reference but the Rust code is now archived.

### ADR-H1: Rust as Primary Language (Superseded)

Originally chose Rust with rig-core. Pivoted to Python/Letta for faster iteration and better memory management.

### ADR-H2: rig-core for Agent Orchestration (Superseded)

Used rig-core 0.27 with multi-turn support. Now using Letta's agent framework instead.

### ADR-H3: Diesel for PostgreSQL (Archived)

Implemented message history in Rust with Diesel ORM. Now using Letta's built-in PostgreSQL storage.

---

## Future Considerations

- **Valkey Integration**: For reminders and scheduled tasks
- **MCP Client**: For connecting to external tool servers
- **Sub-Agent Architecture**: For complex, long-running tasks
- **Production Deployment**: Cloudflare Tunnel, monitoring, backups
