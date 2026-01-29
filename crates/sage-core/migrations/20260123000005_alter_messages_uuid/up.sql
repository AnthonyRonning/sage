-- Alter messages table to use UUID primary key for consistency with memory system
-- This is a breaking change - existing data will be lost

-- Drop existing indexes that depend on the table structure
DROP INDEX IF EXISTS idx_messages_embedding;
DROP INDEX IF EXISTS idx_messages_agent_seq;
DROP INDEX IF EXISTS idx_messages_user_created;

-- Drop and recreate the messages table with UUID
DROP TABLE IF EXISTS messages;

CREATE TABLE messages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id UUID NOT NULL,
    user_id TEXT NOT NULL,  -- Signal phone number or identifier
    role TEXT NOT NULL,     -- 'user', 'assistant', 'tool', 'system'
    content TEXT NOT NULL,
    embedding VECTOR(768),  -- nomic-embed-text embeddings (always computed)
    sequence_id BIGSERIAL,  -- Monotonic ordering within agent
    tool_calls JSONB,       -- For assistant messages with tool calls
    tool_results JSONB,     -- For tool result messages
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for loading messages by agent in sequence order
CREATE INDEX idx_messages_agent_seq ON messages(agent_id, sequence_id);

-- Index for user-based queries (Signal conversations)
CREATE INDEX idx_messages_user_created ON messages(user_id, created_at DESC);

-- Vector similarity index for semantic recall search
CREATE INDEX idx_messages_embedding ON messages 
    USING ivfflat (embedding vector_cosine_ops)
    WITH (lists = 100);

-- Index for role filtering (e.g., find all user messages)
CREATE INDEX idx_messages_role ON messages(agent_id, role);
