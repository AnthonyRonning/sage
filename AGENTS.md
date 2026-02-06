# AGENTS.md - Sage

## Project Overview

Sage is a privacy-first personal AI agent with persistent memory, built in Rust. It communicates via Signal (E2E encrypted), runs LLM inference in TEEs (Trusted Execution Environments) via Maple, and stores all data locally in PostgreSQL with pgvector. The agent uses a 4-tier memory architecture inspired by Letta/MemGPT and typed DSRs signatures instead of native LLM tool calling.

## Repository Structure

```
sage/
├── Cargo.toml                  # Workspace root (resolver = "2")
├── Cargo.lock
├── rust-toolchain.toml         # Stable toolchain with rustfmt, clippy, rust-src
├── Dockerfile                  # Multi-stage build (cargo-chef + debian:bookworm-slim)
├── docker-compose.yml          # Full stack: postgres, signal-cli, sage
├── flake.nix                   # Nix dev shell (provides podman, diesel-cli, signal-cli, etc.)
├── flake.lock
├── justfile                    # Task runner (all build/deploy/dev commands)
├── .env.example                # Environment variable template
├── .github/
│   └── workflows/
│       ├── ci.yml              # Check, test, fmt, clippy (on push/PR to master)
│       ├── release.yml         # Build binaries on tag push (v*)
│       └── docker-publish.yml  # Build + push multi-arch images to ghcr.io
├── .githooks/
│   └── pre-commit              # Runs fmt, clippy, test before commit
├── examples/
│   └── gepa/
│       ├── trainset.json       # GEPA training examples
│       └── valset.json         # GEPA validation examples
├── optimized_instructions/     # GEPA-optimized agent instructions
├── docs/                       # Design docs, architecture notes, prompt examples
└── crates/
    ├── sage-core/              # Main application crate
    │   ├── Cargo.toml
    │   ├── migrations/         # Diesel PostgreSQL migrations (11 total)
    │   ├── src/
    │   │   ├── main.rs         # Entry point: tokio runtime, event loop, Signal listener
    │   │   ├── lib.rs          # Public API re-exports
    │   │   ├── config.rs       # Config struct from environment variables
    │   │   ├── sage_agent.rs   # Core agent: DSRs signatures, tool registry, step loop
    │   │   ├── agent_manager.rs# Multi-user agent management with isolated memory
    │   │   ├── signal.rs       # Signal JSON-RPC client (TCP + subprocess modes)
    │   │   ├── tools.rs        # DoneTool, WebSearchTool implementations
    │   │   ├── shell_tool.rs   # Shell command execution with safety checks
    │   │   ├── vision.rs       # Image description via vision LLM pre-processing
    │   │   ├── scheduler.rs    # Cron + one-off task scheduling (PostgreSQL-backed)
    │   │   ├── scheduler_tools.rs # schedule_task, list_schedules, cancel_schedule tools
    │   │   ├── storage.rs      # Basic Diesel message storage
    │   │   ├── schema.rs       # Diesel schema (agents, blocks, messages, passages, summaries, etc.)
    │   │   ├── memory/
    │   │   │   ├── mod.rs      # MemoryManager: coordinates all 4 memory tiers
    │   │   │   ├── block.rs    # Core memory blocks (persona, human) - always in context
    │   │   │   ├── recall_new.rs   # Recall memory: conversation history with embeddings
    │   │   │   ├── archival_new.rs # Archival memory: long-term semantic storage (pgvector)
    │   │   │   ├── compaction.rs   # Summary/compaction when context window fills
    │   │   │   ├── context.rs  # Context window management and token estimation
    │   │   │   ├── db.rs       # Database operations for all memory tiers
    │   │   │   ├── embedding.rs# Embedding service (Maple TEE nomic-embed-text)
    │   │   │   └── tools.rs    # Memory manipulation tools for the agent
    │   │   └── bin/
    │   │       └── gepa_optimize.rs # GEPA prompt optimization CLI (~700 lines)
    └── sage-tools/             # External tool integrations
        ├── Cargo.toml
        └── src/
            ├── lib.rs          # ToolResult type, re-exports
            ├── brave.rs        # Brave Search API client (Pro) (~740 lines)
            └── web_search.rs   # WebSearch tool wrapper
```

## Development Environment

### Prerequisites

- [Nix](https://nixos.org/download.html) with flakes enabled (provides the full dev environment)
- Alternatively: Rust stable toolchain, podman/docker, diesel-cli, PostgreSQL, signal-cli

### Nix Dev Shell

The project uses a Nix flake for reproducible development. The flake provides:
- Rust stable toolchain (rustc, cargo, clippy, rustfmt, rust-analyzer)
- Podman + container tools (Linux only: conmon, slirp4netns, fuse-overlayfs)
- PostgreSQL with pgvector extension
- diesel-cli for database migrations
- signal-cli (standalone ARM64 binary on aarch64-linux, nixpkgs elsewhere)
- jq, just, valkey

```bash
# Enter the dev shell
nix develop

# Or run a single command inside the shell
nix develop --command just build
```

On Linux, the shell hook aliases `docker` to `podman` and configures container policy/runtime automatically.

### Environment Variables

Copy `.env.example` to `.env` and configure:

```bash
# Required
MAPLE_API_URL=https://your-llm-endpoint.com
MAPLE_API_KEY=your-api-key
MAPLE_MODEL=maple/kimi-k2-5
MAPLE_EMBEDDING_MODEL=maple/nomic-embed-text
SIGNAL_PHONE_NUMBER=+1234567890
DATABASE_URL=postgres://sage:sage@localhost:5434/sage

# Optional
MAPLE_VISION_MODEL=maple/kimi-k2-5   # Defaults to MAPLE_MODEL
SIGNAL_ALLOWED_USERS=uuid1,uuid2     # Or * for all
BRAVE_API_KEY=your-brave-key          # Enables web_search tool
ANTHROPIC_API_KEY=your-key            # For GEPA optimization (Claude as judge)
RUST_LOG=info                         # Logging level
HEALTH_PORT=8080                      # Health check HTTP port
SAGE_WORKSPACE=/workspace             # Shell tool working directory
```

## Build and Run

### Container Build (Primary Method)

All container commands use `podman` and must be run inside the Nix dev shell:

```bash
# Enter nix shell first
nix develop

# Build the Docker image (multi-stage: cargo-chef for caching)
just build
# Equivalent to: podman build -f Dockerfile -t sage:latest .

# Start full stack (postgres + signal-cli + sage)
just start

# Restart only the sage container (keeps postgres/signal-cli running)
just restart

# Stop all containers (data volumes preserved)
just stop

# View logs
just logs        # Sage only
just logs-all    # All containers

# Check status
just status

# Shell into container
just shell

# Connect to PostgreSQL
just psql
```

The Dockerfile uses a 4-stage build:
1. **chef** - Installs cargo-chef on rust:1.83-bookworm with nightly toolchain
2. **planner** - Analyzes dependencies and creates recipe.json
3. **builder** - Builds dependencies (cached layer), then builds source
4. **runtime** - debian:bookworm-slim with comprehensive CLI toolset (python3, node, git, ripgrep, etc.)

### Local Development (Without Containers)

```bash
nix develop

# Start local PostgreSQL (port 5434, avoids conflicts with 5432/5433)
sage-pg-start

# Build locally
just build-local    # cargo build --release
just check          # cargo check

# Run locally (uses signal-cli subprocess mode)
just run            # cargo run --release
just run-debug      # RUST_LOG=debug cargo run --release

# Stop local PostgreSQL
sage-pg-stop
```

### Database

PostgreSQL with pgvector is used for all storage. Migrations are in `crates/sage-core/migrations/` and run automatically on startup via `diesel_migrations::embed_migrations!()`.

For manual migration management:
```bash
# Inside nix develop shell
diesel migration run --database-url postgres://sage:sage@localhost:5434/sage
diesel migration revert --database-url postgres://sage:sage@localhost:5434/sage
```

### Docker Compose

`docker-compose.yml` defines the full stack with health checks and proper service ordering:
- `postgres` - pgvector/pgvector:pg17 on port 5434
- `signal-cli` - JRE image with TCP JSON-RPC daemon on port 7583
- `signal-cli-perms` - One-shot init to fix attachment permissions
- `sage` - The agent, depends on healthy postgres + signal-cli

First-time setup requires `just signal-init` to copy local signal-cli registration data into a Docker volume.

## Testing and CI

### Running Tests

```bash
# Run all tests
just test           # cargo test
cargo test --all-features

# Run specific test
cargo test test_name

# CI-equivalent full check
just ci-check
# Runs: cargo fmt --all -- --check
#        cargo clippy --all-targets --all-features -- -D warnings
#        cargo test --all-features
```

### Pre-commit Hook

Set up with `just setup-hooks` (configures `git config core.hooksPath .githooks`). The hook runs:
1. `cargo fmt --all -- --check`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test --all-features`

### CI Workflows (GitHub Actions)

- **ci.yml** - Runs on push/PR to master: check, test, fmt, clippy (4 parallel jobs)
- **docker-publish.yml** - Builds multi-arch images (linux/amd64, linux/arm64) and pushes to `ghcr.io/anthonyronning/sage`
- **release.yml** - On tag push (v*): builds binaries for linux-x86_64, linux-aarch64, macos-aarch64 and creates GitHub release

### GEPA Prompt Optimization

```bash
# Evaluate current instruction (baseline score)
just gepa-eval

# Run optimization loop (requires ANTHROPIC_API_KEY)
just gepa-optimize

# View optimized instruction
just gepa-show
```

GEPA uses Claude as the judge and Kimi as the program under test. Training data is in `examples/gepa/trainset.json`.

## Architecture and Design Patterns

### No Native Tool Calling

The agent does NOT use LLM provider function calling APIs. Instead it uses DSRs (dspy-rs) with BAML parsing. The LLM outputs structured text with field markers (`[[ ## field ## ]]`) that gets parsed into typed Rust structs. This works identically across all providers.

### DSRs Signatures

All LLM interactions are defined as typed signatures in `sage_agent.rs`:

- **`AgentResponse`** - Main agent signature with 9 input fields and 2 output fields (messages, tool_calls)
- **`CorrectionResponse`** - Self-healing: fixes malformed LLM outputs
- **`SummarizeConversation`** - Compacts old messages when context window fills (in `memory/compaction.rs`)

The `AGENT_INSTRUCTION` constant contains the full system prompt (~4KB). It was optimized by GEPA (Gen 3, score 0.967).

### Agent Step Loop

`SageAgent::step()` implements a multi-step agentic loop (max 10 steps per message):
1. Build context from database (memory blocks, conversation history, summaries)
2. Call LLM via DSRs `Predict` with retry logic (3 attempts, correction agent on parse errors)
3. Execute tool calls, inject results for next step
4. Return messages + done flag

The main event loop in `main.rs` orchestrates: Signal message reception -> agent processing -> Signal response sending, with async embedding updates and tool result storage.

### Memory System (4-Tier)

| Tier | Module | Storage | Purpose |
|------|--------|---------|---------|
| Core | `memory/block.rs` | `blocks` table | Always-in-context persona/human info |
| Recall | `memory/recall_new.rs` | `messages` table + embeddings | Searchable conversation history |
| Archival | `memory/archival_new.rs` | `passages` table + pgvector | Long-term semantic storage |
| Summary | `memory/compaction.rs` | `summaries` table | Auto-compaction at 80% of 100k token window |

Embeddings are generated via Maple TEE (nomic-embed-text). Messages are stored synchronously (fast), embeddings updated asynchronously in background.

### Multi-User Isolation

`AgentManager` creates isolated agents per Signal user/group:
- Each gets a unique `agent_id` (UUID) stored in `chat_contexts` table
- Separate memory blocks, conversation history, archival storage, preferences, scheduled tasks
- Separate workspace directory under `SAGE_WORKSPACE/<agent_id>/`
- Agents are cached in-memory after first creation

### Signal Interface

`signal.rs` supports two modes:
- **TCP mode** (production): Connects to signal-cli daemon in separate container via JSON-RPC over TCP. Includes keepalive (30s interval), auto-reconnect with exponential backoff, 24h session rotation.
- **Subprocess mode** (development): Spawns signal-cli as a child process.

### Tool System

Tools implement the `Tool` trait (`sage_agent.rs`):
```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn args_schema(&self) -> &str;
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult>;
}
```

Tools are registered in `ToolRegistry` (BTreeMap for deterministic ordering). The single source of truth for all tool descriptions is `ToolRegistry::all_tools_description_only()` in `sage_agent.rs`.

Available tools: `memory_replace`, `memory_append`, `memory_insert`, `conversation_search`, `archival_insert`, `archival_search`, `set_preference`, `schedule_task`, `list_schedules`, `cancel_schedule`, `shell`, `web_search`, `done`.

### Vision Pipeline

Image attachments from Signal are pre-processed by a vision-capable LLM (`vision.rs`). The description is injected as text alongside the user's message (e.g., `[Uploaded Image: <description>]`). Recent conversation context (last 6 messages) is provided to the vision model for relevance.

## Coding Conventions

### Rust Style

- Edition 2021, stable toolchain
- Workspace-level dependency management in root `Cargo.toml`
- `#[allow(dead_code)]` used liberally on structs/methods that are public API but not yet called everywhere
- Error handling: `anyhow::Result` for application code, `thiserror` for library errors (`sage-tools`)
- Async runtime: Tokio with `features = ["full"]`
- Database: Diesel 2.2 with PostgreSQL, raw SQL for pgvector operations
- Logging: `tracing` crate with `tracing-subscriber` env filter
- All tool args are `HashMap<String, String>` (string-typed for LLM compatibility)

### Module Organization

- One file per major concern (signal, vision, scheduler, shell_tool, etc.)
- Memory system is the only subdirectory module (`memory/`)
- `sage-tools` crate is kept minimal (only Brave Search currently)
- `sage-core` contains everything else including binaries (`sage`, `gepa-optimize`)

### Database Conventions

- All tables use UUID primary keys (`uuid::Uuid`)
- Timestamps are `DateTime<Utc>` (stored as `timestamptz`)
- `sequence_id` on messages is auto-incrementing `BIGSERIAL` for ordering
- Embeddings stored as `vector` type via pgvector, managed through raw SQL
- Schema defined in `schema.rs` (auto-generated by Diesel CLI with manual pgvector adjustments)

### Testing

- Unit tests are in-file (`#[cfg(test)]` modules)
- Tests that require database connections should be integration tests
- Current test coverage is minimal (tool registry, cron parsing, datetime parsing)
- No mocking framework in use; tests are mostly for pure functions

## Security Considerations

- Never commit `.env` files or API keys
- The shell tool (`shell_tool.rs`) blocks dangerous patterns: `rm -rf /`, fork bombs, `mkfs`, `shutdown`, etc.
- Shell output is capped at 100KB, timeout at 300s max
- Signal allowed users should be configured (`SIGNAL_ALLOWED_USERS`) to prevent unauthorized access
- All LLM inference and embedding generation happens in TEE via Maple
- Database credentials are local-only (sage:sage for development)

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `dspy-rs` | DSPy in Rust - typed signatures, BAML parsing (git dependency) |
| `baml-bridge` | BAML derive macros for tool call types (git dependency) |
| `diesel` | PostgreSQL ORM with migrations |
| `pgvector` | Vector similarity search in PostgreSQL |
| `tokio` | Async runtime |
| `axum` | HTTP server (health check endpoint) |
| `reqwest` | HTTP client (LLM API, Brave Search, embeddings) |
| `chrono` / `chrono-tz` | Time handling with timezone support |
| `cron` | Cron expression parsing for scheduler |
| `socket2` | TCP keepalive configuration for Signal |
| `serde` / `serde_json` | Serialization throughout |
| `tracing` | Structured logging |
| `anyhow` / `thiserror` | Error handling |

Note: `dspy-rs` and `baml-bridge` are git dependencies from `https://github.com/krypticmouse/DSRs.git`.

## Common Tasks

### Adding a New Tool

1. Implement the `Tool` trait (in a new file or existing module)
2. Register it in `AgentManager::create_agent()` (`agent_manager.rs`)
3. Add a description-only entry in `ToolRegistry::all_tools_description_only()` (`sage_agent.rs`)
4. Add training examples in `examples/gepa/trainset.json` if needed

### Adding a Database Migration

```bash
nix develop
diesel migration generate migration_name --database-url postgres://sage:sage@localhost:5434/sage
# Edit the generated up.sql and down.sql
# Update schema.rs if needed (diesel print-schema)
```

### Modifying the Agent Instruction

The instruction is in `AGENT_INSTRUCTION` constant in `sage_agent.rs`. After changes:
1. Run `just gepa-eval` to check baseline score
2. Consider running `just gepa-optimize` to auto-improve

### Adding a New Memory Tier or Tool Category

Follow the pattern in `memory/` - each tier has its own module with a manager struct, and tools are in `memory/tools.rs`. Register tools via `MemoryManager::tools()`.
