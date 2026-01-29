# Sage Memory System

This module implements a 4-tier memory architecture inspired by Letta/MemGPT, adapted for Sage's Rust/DSRs stack.

## Architecture Overview

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
│  │  └─ <memory_metadata>                                │   │
│  └─────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────┐   │
│  │ Recent Messages (in-context via message_ids)         │   │
│  │  └─ [summary?, user, assistant, tool, ...]          │   │
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

## The 4 Memory Tiers

### 1. Core Memory (Blocks)
- **What**: Editable text blocks always present in system prompt
- **Default blocks**: `persona` (who the agent is), `human` (info about user)
- **Char limit**: 20,000 per block (Letta default)
- **Persistence**: PostgreSQL `blocks` table
- **Agent tools**: `memory_replace`, `memory_append`, `memory_insert`

### 2. Recall Memory (Conversation History)
- **What**: Full message history, searchable
- **Storage**: PostgreSQL `messages` table
- **In-context**: Only `message_ids` subset visible to LLM
- **Search**: Hybrid (keyword + semantic via embeddings)
- **Agent tool**: `conversation_search`

### 3. Archival Memory (Long-term Semantic)
- **What**: Agent-created long-term memories with embeddings
- **Storage**: PostgreSQL `passages` table with pgvector
- **Embedding model**: `maple/nomic-embed-text`
- **Agent tools**: `archival_insert`, `archival_search`

### 4. Summary Memory (Compaction)
- **What**: Rolling summary when context overflows
- **Trigger**: 80% of context window (256k tokens for Kimi K2)
- **Implementation**: DSRs signature for summarization
- **Prompt**: Letta's SHORTER_SUMMARY_PROMPT (100 word limit)

## Design Decisions

### From Letta Research

| Decision | Value | Source |
|----------|-------|--------|
| Block char limit | 20,000 | `CORE_MEMORY_BLOCK_CHAR_LIMIT` in constants.py |
| Default blocks | `persona`, `human` | `DEFAULT_BLOCKS` in schemas/block.py |
| Persona description | "Stores details about your current persona..." | `DEFAULT_PERSONA_BLOCK_DESCRIPTION` |
| Human description | "Stores key details about the person you are conversing with..." | `DEFAULT_HUMAN_BLOCK_DESCRIPTION` |
| Summary prompt | SHORTER_SUMMARY_PROMPT (100 words) | prompts/summarizer_prompt.py |
| Compaction threshold | 80% of context | Standard practice |

### Sage-Specific Decisions

| Decision | Value | Rationale |
|----------|-------|-----------|
| Context window | 256k tokens | Kimi K2 limit |
| Token counting | tiktoken | Standard, accurate |
| Embedding provider | maple/nomic-embed-text | Available via Maple proxy |
| Vector storage | pgvector | Simpler (in PostgreSQL) |
| LLM operations | DSRs signatures | Enables GEPA optimization |
| No line numbers | Standard XML format | We're not Anthropic-only |

## Module Structure

```
memory/
├── mod.rs              # Public API (MemoryManager)
├── block.rs            # Core memory blocks
├── recall.rs           # Conversation search
├── archival.rs         # Long-term semantic storage
├── compaction.rs       # Summary/compaction (DSRs signature)
├── context.rs          # Context window management
├── tools.rs            # Memory manipulation tools
└── README.md           # This file
```

## Public API

```rust
pub struct MemoryManager {
    blocks: BlockManager,
    recall: RecallManager,
    archival: ArchivalManager,
    context: ContextManager,
}

impl MemoryManager {
    /// Create a new memory manager for an agent
    pub async fn new(agent_id: Uuid, db: &PgPool) -> Result<Self>;
    
    /// Compile memory blocks into system prompt injection
    pub fn compile(&self) -> String;
    
    /// Compile memory metadata (counts, timestamps)
    pub fn compile_metadata(&self) -> String;
    
    /// Get memory tools for the agent
    pub fn tools(&self) -> Vec<Arc<dyn Tool>>;
    
    /// Check if compaction needed
    pub fn needs_compaction(&self, current_tokens: usize) -> bool;
    
    /// Run compaction (summarize old messages)
    pub async fn compact(&mut self) -> Result<SummaryMessage>;
    
    /// Get in-context message IDs
    pub fn message_ids(&self) -> &[Uuid];
    
    /// Update in-context message IDs
    pub fn set_message_ids(&mut self, ids: Vec<Uuid>);
}
```

## DSRs Signatures

All LLM operations use typed DSRs signatures for GEPA optimization:

### Compaction/Summarization

```rust
#[derive(Signature)]
struct SummarizeConversation {
    /// Summarize the conversation to allow resumption without disruption.
    /// Keep summary under 100 words. Include:
    /// 1. Task/conversational overview
    /// 2. Current state (completed work, files, resources)  
    /// 3. Next steps
    
    #[input]
    conversation_history: String,
    
    #[output]
    summary: String,
}
```

### Archival Search Reranking (optional)

```rust
#[derive(Signature)]
struct RerankPassages {
    /// Rerank archival memory passages by relevance to query.
    
    #[input]
    query: String,
    
    #[input]
    passages: Vec<String>,
    
    #[output]
    ranked_indices: Vec<usize>,
}
```

## Memory Block XML Format

The `compile()` method produces XML for system prompt injection:

```xml
<memory_blocks>
The following memory blocks are currently engaged in your core memory unit:

<persona>
<description>
Stores details about your current persona, guiding how you behave and respond.
</description>
<metadata>
- chars_current=1234
- chars_limit=20000
</metadata>
<value>
I am Sage, a helpful AI assistant communicating via Signal...
</value>
</persona>

<human>
<description>
Stores key details about the person you are conversing with.
</description>
<metadata>
- chars_current=567
- chars_limit=20000
</metadata>
<value>
Name: Alice
Preferences: Prefers concise responses...
</value>
</human>

</memory_blocks>
```

## Memory Metadata Format

```xml
<memory_metadata>
- The current system date is: 2026-01-23 10:30:00 PST
- Memory blocks were last modified: 2026-01-23 09:15:00 PST
- 150 previous messages between you and the user are stored in recall memory (use conversation_search to access)
- 42 total memories you created are stored in archival memory (use archival_search to access)
</memory_metadata>
```

## Database Schema

### blocks table
```sql
CREATE TABLE blocks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id UUID NOT NULL REFERENCES agents(id),
    label VARCHAR(100) NOT NULL,
    description TEXT,
    value TEXT NOT NULL DEFAULT '',
    char_limit INT NOT NULL DEFAULT 20000,
    read_only BOOLEAN NOT NULL DEFAULT FALSE,
    version INT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(agent_id, label)
);
```

### passages table (archival memory)
```sql
CREATE TABLE passages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id UUID NOT NULL REFERENCES agents(id),
    content TEXT NOT NULL,
    embedding VECTOR(768),  -- nomic-embed-text dimension
    tags TEXT[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_passages_embedding ON passages 
    USING ivfflat (embedding vector_cosine_ops);
CREATE INDEX idx_passages_agent ON passages(agent_id);
CREATE INDEX idx_passages_tags ON passages USING GIN(tags);
```

### agents table additions
```sql
ALTER TABLE agents ADD COLUMN message_ids UUID[] NOT NULL DEFAULT '{}';
ALTER TABLE agents ADD COLUMN last_memory_update TIMESTAMPTZ;
```

## Tool Signatures

### memory_replace
```
Replace text in a memory block.
Args: block (label), old (text to find), new (replacement text)
```

### memory_append  
```
Append text to a memory block.
Args: block (label), content (text to append)
```

### memory_insert
```
Insert text at a specific line in a memory block.
Args: block (label), content (text), line (line number, -1 for end)
```

### conversation_search
```
Search conversation history.
Args: query (search text), limit (max results, default 5)
Returns: List of matching messages with timestamps
```

### archival_insert
```
Store information in long-term memory.
Args: content (text to store), tags (optional comma-separated tags)
```

### archival_search
```
Search long-term memory.
Args: query (search text), top_k (max results, default 5), tags (optional filter)
Returns: List of matching passages with timestamps and tags
```

## Integration with SageAgent

The memory module integrates via composition:

```rust
pub struct SageAgent {
    memory: MemoryManager,
    tools: ToolRegistry,  // Includes memory.tools()
    // ...
}

impl SageAgent {
    pub async fn new(agent_id: Uuid, db: &PgPool) -> Result<Self> {
        let memory = MemoryManager::new(agent_id, db).await?;
        let mut tools = ToolRegistry::new();
        
        // Add memory tools
        for tool in memory.tools() {
            tools.register(tool);
        }
        
        // Add other tools (web_search, etc.)
        // ...
        
        Ok(Self { memory, tools, /* ... */ })
    }
    
    fn build_context(&self) -> String {
        let mut context = String::new();
        
        // Inject compiled memory blocks
        context.push_str(&self.memory.compile());
        context.push_str("\n\n");
        context.push_str(&self.memory.compile_metadata());
        context.push_str("\n\n");
        
        // Add conversation history from message_ids
        for msg_id in self.memory.message_ids() {
            // ... load and format message
        }
        
        context
    }
}
```

## System Prompt Rebuild Rules

Following Letta's approach:
1. Rebuild only when `<memory_blocks>` content changes
2. Ignore `<memory_metadata>` changes (timestamps, counts)
3. Compare compiled output, not individual block values

## References

- Letta source: https://github.com/letta-ai/letta
- Design doc: `docs/SAGE_V2_DESIGN.md`
- Letta reverse engineering: `docs/LETTA_ARCHITECTURE_REVERSE_ENGINEERING.md`
