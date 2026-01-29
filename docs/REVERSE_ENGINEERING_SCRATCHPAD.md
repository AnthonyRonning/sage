# Letta Reverse-Engineering Scratchpad

Purpose: messy, append-only-ish notes while reading the codebase. This is meant to survive context compaction so we don’t re-learn the same things.

Repo: https://github.com/letta-ai/letta

## High-level first pass (2026-01-20)

### Project metadata

- Package: `letta` (v0.16.1)
- Python: `>=3.11,<3.14`
- Entrypoint: `letta = letta.main:app` (Typer CLI)
- Server (extra): FastAPI + Uvicorn (`server` extra)
- Core stack: Pydantic v2, SQLAlchemy async + SQLModel, Alembic

Notable dependencies that imply architecture choices:

- `llama-index` (+ `llama-index-embeddings-openai`) suggests they sometimes lean on LlamaIndex abstractions for ingestion/RAG.
- Multi-LLM providers: `openai`, `anthropic`, `google-genai`, `mistralai` (+ optional `bedrock`).
- Vector DB / storage options via extras: `postgres` (pgvector), `sqlite` (sqlite-vec), `pinecone`.
- Ops/observability: Sentry (`sentry-sdk[fastapi]`), OpenTelemetry, Datadog (`ddtrace`, `datadog`).
- Orchestration: `temporalio` (Temporal).
- Tools/search: `tavily-python`, `exa-py`.
- “MCP” tooling: `mcp[cli]` (likely Model Context Protocol integration).

### Repo/package layout (from `letta/`)

Top-level modules/directories worth mapping:

- `agent.py` (large): core agent runtime + context window handling.
- `agents/`: concrete agent variants (e.g. `letta_agent.py`, `voice_sleeptime_agent.py`).
- `prompts/`: system prompts + prompt generator.
- `functions/`: tool/function-set definitions.
- `services/`: manager/service layer (agent_manager, passage_manager, tool_executor, etc.).
- `orm/`: SQLAlchemy models for persistence.
- `schemas/`: Pydantic schemas (Agent, Memory, Passage, etc.).
- `server/`: FastAPI server.
- `cli/`: Typer CLI commands.
- `llm_api/`, `local_llm/`, `interfaces/`: provider clients and streaming wrappers.
- `data_sources/`: ingestion / sources (likely files/web/etc).
- `plugins/`: extension hooks.
- `jobs/`: background/scheduled work.

### Memory system: confirmed terminology (early evidence)

From code references and schema docs, Letta uses at least these “memory buckets”:

1. **Core memory**
   - Composed of editable **memory blocks** (each block has a `label`, `value`, and a char `limit`).
   - Blocks are persisted in DB (see ORM: `blocks_agents`, `block`, agent relationship docstrings).
   - Agent has tools like `core_memory_append` / `core_memory_replace` to mutate block content.
   - There’s explicit logic to rebuild the *system message* only when core memory changes (to avoid flooding recall storage).

2. **Archival memory** (long-term)
   - Insert/search tools exist (`archival_memory_insert`, `archival_memory_search`).
   - Search is semantic/embedding-based; agent_manager has a shared method: “Search archival memory using semantic (embedding-based) search with optional temporal filtering.”
   - There is an enum `VectorDBProvider` described as “Supported vector database providers for archival memory.”
   - There is a Turbopuffer client (`helpers/tpuf_client.py`) described as “archival memory storage” using a vector DB.

3. **Recall memory** (conversation history)
   - Schemas mention “previous messages between you and the user are stored in recall memory.”
   - There are tools in local LLM grammars for `conversation_search` and `conversation_search_date`.

4. **Summary memory**
   - `agent.py` token accounting explicitly includes “summary of ongoing conversation”.
   - This likely acts as a running summary to keep long chats within the context window.

Likely "4 types" you were thinking of = **core + recall + archival + summary**.

### Prompt assembly / system message structure (early evidence)

- There is a `prompts/prompt_generator.py` that generates a `<memory_metadata>` block containing:
  - current time
  - “Memory blocks were last modified” timestamp
  - recall count (“N previous messages…”)
  - archival count (“M total memories…”)
  - archival tags list
- There is a context window calculator that parses the system message into tagged sections:
  - `base_instructions`
  - `memory_blocks` (`<memory_blocks>...`)
  - `memory_metadata` (`<memory_metadata>...`)
- System prompts live in `prompts/system_prompts/` (e.g. `letta_v1.py`).

### Tools/function sets (early evidence)

- `functions/function_sets/base.py` defines the agent-facing tools for:
  - archival insert/search
  - core memory append/replace
- `functions/function_sets/files.py` has an `open_files` tool that loads file contents into a “files section in core memory” (max 5 simultaneously).

### Local LLM structured tool-calls

- `local_llm/grammars/json_func_calls_with_inner_thoughts.gbnf` defines a JSON grammar for function calls including `inner_thoughts` and `request_heartbeat`.
- Local LLM wrappers include reminder prefixes that explicitly teach the model:
  - core memory is limited and editable via tools
  - archival memory is unlimited and searchable
  - the model should update memory immediately on new important info

## Questions / things to validate by reading code

- What exactly is in `Memory.compile(...)` and how it formats blocks (line-numbered mode, labels, etc.)?
- How recall memory is stored/indexed and how `conversation_search(_date)` works (DB? full-text? embeddings?).
- Archival memory implementation details:
  - schema for passages
  - embedding generation pipeline
  - provider switching (native vs pgvector vs sqlite-vec vs pinecone vs turbopuffer)
  - how tags + timestamps are stored and used
- Summary memory:
  - when/how it’s updated
  - prompt used for summarization
  - whether it is persisted and where
- Agent runtime:
  - exact “step loop” flow (where tools are executed, how memory updates trigger system prompt rebuild)
  - stop reasons + errors
- Server/API:
  - endpoints for agent creation/runs
  - streaming protocol
  - auth/tenancy (orgs/users)
- Extension points:
  - plugins
  - function/tool registration
  - data sources
  - custom system prompts/personas

## Deeper notes (continued 2026-01-20)

### Core memory blocks (schemas + ORM)

- `letta/schemas/block.py`
  - `BaseBlock`: `{value, limit, label, description, metadata, read_only, hidden}` + template-related fields.
  - Enforces per-block char limit via a `@model_validator(mode="before")` (raises `Edit failed: Exceeds {limit} character limit`).
  - `DEFAULT_BLOCKS = [Human(value=""), Persona(value="")]`.
- `letta/orm/block.py`
  - DB model `Block`: `{label, value, limit, description, metadata_, read_only, hidden}`.
  - `version` column used for optimistic locking.
  - SQLAlchemy event `validate_value_length` enforces `len(value) <= limit` on insert/update (DB-level guard).
  - Agent↔Block is many-to-many via `blocks_agents` with a uniqueness constraint enforcing **one block per label per agent**.

### Memory prompt compilation

- `letta/schemas/memory.py`
  - `Memory.compile(...)` renders:
    - `<memory_blocks>` (unless `agent_type` is `react_agent`/`workflow_agent`)
    - `<tool_usage_rules>` (compiled tool rule prompts)
    - `<directories>` (open/attached files; rendering differs for react/workflow)
  - **Does not** include `<memory_metadata>` (that’s added by `PromptGenerator`).
  - Line-numbered memory rendering is enabled only when:
    - provider is **Anthropic** (`llm_config.model_endpoint_type == "anthropic"`), and
    - agent type is one of `{sleeptime_agent, memgpt_v2_agent, letta_v1_agent}`.

### System prompt assembly

- `letta/prompts/prompt_generator.py`
  - `compile_memory_metadata_block(...)` generates `<memory_metadata>` containing:
    - current system date (in agent timezone)
    - last memory edit timestamp
    - recall memory count (messages not in-context)
    - archival memory count + tags (if present)
  - `get_system_message_from_compiled_memory(...)` injects `{CORE_MEMORY}` (appends it if missing).

### System prompt rebuild rules

- `letta/services/agent_manager.py`
  - `rebuild_system_prompt_async(...)`:
    - Computes `curr_memory_str = agent_state.memory.compile(...)`.
    - If `curr_memory_str` is a substring of the current system message and `force=False`, it **skips** rebuilding.
      - This avoids system message churn from constantly changing metadata timestamps/counts.
    - On rebuild, uses `PromptGenerator.get_system_message_from_compiled_memory(...)` with:
      - `previous_message_count = num_messages - len(agent_state.message_ids)`
      - `archival_memory_size = num_archival_memories`
  - `update_memory_if_changed_async(...)`:
    - Detects diffs via `new_memory.compile(...) not in system_message`.
    - Persists changed block values to DB (`block_manager.update_block_async`).
    - Refreshes memory blocks from DB, then triggers `rebuild_system_prompt_async`.

### File context (“open files”)

- `letta/services/tool_executor/files_tool_executor.py`
  - `open_files(file_requests, close_all_others=False)`:
    - Loads file content (optionally a `(offset,length)` window) using `LineChunker`.
    - Enforces `agent_state.max_files_open` and LRU-evicts via `FileAgentManager`.
    - Updates `agent_state.memory.file_blocks` (these are *not* core memory blocks).
  - `grep_files`/`semantic_search_files` exist with safety limits (max file size, regex complexity, paging).

### Multi-agent “sleeptime” pattern

- `letta/groups/sleeptime_multi_agent_v4.py`
  - Foreground agent runs a normal `LettaAgentV3.step/stream`.
  - After the response, schedules background `LettaAgentV3` runs for each sleeptime agent in the group.
  - Builds a transcript and wraps it in a `<system-reminder>` instructing the background agent to update memory blocks.
  - Frequency controlled by `group.sleeptime_agent_frequency` + `GroupManager.bump_turns_counter_async`.

### Tool execution plumbing

- `letta/services/tool_executor/tool_execution_manager.py`
  - `ToolExecutorFactory` maps `ToolType -> executor`:
    - `LETTA_CORE/LETTA_MEMORY_CORE/LETTA_SLEEPTIME_CORE -> LettaCoreToolExecutor`
    - `LETTA_FILES_CORE -> LettaFileToolExecutor`
    - `LETTA_MULTI_AGENT_CORE -> LettaMultiAgentToolExecutor`
    - `LETTA_BUILTIN -> LettaBuiltinToolExecutor`
    - `EXTERNAL_MCP -> ExternalMCPToolExecutor`
    - default: `SandboxToolExecutor`
  - Truncates tool return strings beyond `tool.return_char_limit`.

### Summarization (“summary memory”)

- `letta/agents/letta_agent_v3.py`
  - `compact(...)` runs summarization using `CompactionSettings`.
  - Inserts a packed JSON `system_alert` summary message (role `user`) near the start of the in-context buffer.
  - Evicted messages remain persisted in the DB and are accessible via recall search.
- `letta/services/summarizer/*`
  - Supports `all` and `sliding_window` modes.
  - Uses model-specific token counters and a safety margin for approximate counters.
- `letta/system.py`
  - `package_summarize_message(_no_counts)` produces the packed JSON `system_alert` summary payload.

## Additional confirmed contracts (late 2026-01-20)

### Recall search tool (`conversation_search`)

- Implemented in `letta/services/tool_executor/core_tool_executor.py::LettaCoreToolExecutor.conversation_search`.
- Calls `MessageManager.search_messages_async` (Turbopuffer hybrid if enabled, else SQL fallback), then filters:
  - all `tool` messages
  - assistant messages that call `conversation_search` (prevents recursive nesting)
  - (inside `_extract_message_text`) heartbeat messages + tool messages for `send_message`/`conversation_search`
- Returns a structured dict:
  - `{"message": "Showing N results:", "results": [...]}`
  - each result includes `timestamp`, `time_ago`, `role`, optional `relevance` (`rrf_score`, `vector_rank`, `fts_rank`, `search_mode`)
  - message payload comes from `_extract_message_text`, which returns JSON like:
    - user: `{"content": "..."}`
    - assistant w/ `send_message`: `{"thinking": "...assistant content...", "content": "...send_message args.message..."}`

### Runtime tool schema overrides

- `letta/services/helpers/tool_parser_helper.py::runtime_override_tool_json_schema`:
  - If `response_format.type != text`, replaces `send_message.message` schema so the model emits a JSON object / JSON schema inside the `message` argument.
  - When enabled, injects a **required** `request_heartbeat: boolean` param into every non-terminal tool schema.

### File tools + LRU

- Persistence: `files_agents` join table (`letta/orm/files_agents.py`) has unique constraints on `(file_id, agent_id)` and `(agent_id, file_name)` and stores `is_open`, `visible_content`, `last_accessed_at`, `start_line`, `end_line`.
- `open_files`:
  - takes `file_requests: [{file_name, offset?, length?}]` where `offset` is 0-indexed line offset
  - enforces `agent_state.max_files_open`
  - LRU eviction via `FileAgentManager.enforce_max_open_files_and_open` (oldest `last_accessed_at` first)
- `grep_files`:
  - safety defaults: 50MB/file, 200MB total scanned, regex length<=1000, timeout=30s, collect<=1000 matches
  - pagination: 20 matches/page with `offset`
  - bumps `last_accessed_at` for files with matches (affects LRU)

## Additional confirmed contracts (2026-01-22)

### Agent loop routing (V2 vs V3)

- `letta/agents/agent_loop.py::AgentLoop.load(agent_state, actor)` chooses the loop implementation:
  - If `agent_type in {letta_v1_agent, sleeptime_agent}`:
    - `enable_sleeptime=True` + group present → `SleeptimeMultiAgentV4` (V3-based)
    - else → `LettaAgentV3`
  - Else if `enable_sleeptime=True` and `agent_type != voice_convo_agent`:
    - group present → `SleeptimeMultiAgentV3` (V2-based)
    - else → `LettaAgentV2`
  - Else → `LettaAgentV2`

### Message persistence cadence differs across loops

- **V3** checkpoints every internal step: persist new messages + update `agent.message_ids` together.
- **V2** persists step messages as it goes, but typically updates `agent.message_ids` once at end-of-turn (after summarize/compact), except approval flows which update it immediately.

### Parallel tool calls in V3

- `LettaAgentV3` can process an assistant message with multiple tool calls.
- Tools opt-in to concurrent execution via `Tool.enable_parallel_execution` (default `False`).
- V3 splits multi-tool steps into:
  - parallel subset: `asyncio.gather(...)`
  - serial subset: executed sequentially
- Message shape for multi-tool steps is produced by `create_parallel_tool_messages_from_llm_response(...)`:
  - assistant message has `tool_calls=[...]` and any reasoning/text content
  - tool message has `tool_returns=[...]` and `content=[TextContent(packaged_response), ...]` (one per tool)
  - for legacy renderers, tool message also sets `tool_call_id`/`name` to the first tool

### Prefilled args from tool rules (V3)

- `ToolRulesSolver.get_allowed_tool_names(...)` caches prefilled args each step:
  - `last_prefilled_args_by_tool: Dict[tool_name, args]`
  - `last_prefilled_args_provenance: Dict[tool_name, str]`
- In `LettaAgentV3._handle_ai_response(...)`, if a tool call is allowed and there are cached prefilled args, they are merged into the tool args via `merge_and_validate_prefilled_args(...)`:
  - prefilled wins on key collisions
  - validates keys exist on the tool JSON schema + lightweight type/enum/const checks
  - invalid prefilled args stop the step with `stop_reason=invalid_tool_call` (tool not executed)

### Legacy vs modern agent code

- `letta/agent.py::Agent` is an older sync-style runtime that still contains memory-rebuild and tool plumbing; the current “modern” loops live under `letta/agents/letta_agent_v2.py` and `letta/agents/letta_agent_v3.py`.
