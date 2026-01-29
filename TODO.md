# Sage Development TODO

> Last updated: 2026-01-29
> Status: Core system complete, in production

---

## Completed

### Core Implementation ✅
- Pure Rust implementation (no Python/Letta dependency)
- PostgreSQL with pgvector for memory storage
- Signal integration via signal-cli JSON-RPC
- Multi-user support with isolated memory per conversation
- Auto-reconnect and retry logic for Signal reliability

### Memory System ✅
- Core memory blocks (persona, human) - always in context
- Recall memory - full conversation history with embeddings
- Archival memory - long-term semantic storage (pgvector)
- Summary memory - auto-compaction when context overflows
- Conversation search with hybrid keyword + semantic

### Tools ✅
- `web_search` - Brave Search with AI summaries
- `shell` - Execute commands in workspace
- `memory_replace/append/insert` - Edit memory blocks
- `archival_insert/search` - Long-term memory
- `conversation_search` - Search history
- `set_preference/get_preference` - User preferences
- `schedule_task/list_schedules/cancel_schedule` - Reminders

### Infrastructure ✅
- Docker/Podman containerization
- Nix flake for development
- Diesel ORM with migrations
- DSRs (DSPy in Rust) for structured output

---

## Next Up

### Improvements 🔜
- [ ] Streaming LLM responses (avoid Cloudflare timeouts)
- [ ] Group chat support
- [ ] Voice message transcription
- [ ] Image understanding
- [ ] Natural language time parsing ("in 2 hours")

### Integrations 🔜
- [ ] Gmail integration
- [ ] Google Calendar integration
- [ ] MCP (Model Context Protocol) support

### Optimization 🔜
- [ ] GEPA prompt optimization with real usage data
- [ ] Response quality metrics and evaluation

---

## Quick Reference

### Running Sage

```bash
nix develop
just start          # Start all containers
just logs           # View logs
just stop           # Stop containers
```

### Key Files

| File | Purpose |
|------|---------|
| `crates/sage-core/src/main.rs` | Entry point, message loop |
| `crates/sage-core/src/sage_agent.rs` | Agent logic, LLM interaction |
| `crates/sage-core/src/signal.rs` | Signal JSON-RPC client |
| `crates/sage-core/src/memory/` | Memory system |
| `crates/sage-core/src/agent_manager.rs` | Multi-user agent management |

### Environment Variables

| Variable | Purpose |
|----------|---------|
| `MAPLE_API_URL` | LLM endpoint |
| `MAPLE_API_KEY` | API key |
| `MAPLE_MODEL` | Model name (kimi-k2-thinking) |
| `MAPLE_EMBEDDING_MODEL` | Embedding model |
| `SIGNAL_PHONE_NUMBER` | Sage's phone number |
| `SIGNAL_ALLOWED_USERS` | Comma-separated UUIDs (or * for all) |
| `BRAVE_API_KEY` | For web search |
| `DATABASE_URL` | PostgreSQL connection |

---

## Roadmap

See `docs/roadmap.md` for full plan.
