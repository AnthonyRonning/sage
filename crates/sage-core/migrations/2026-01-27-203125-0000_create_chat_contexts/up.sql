-- Chat contexts map Signal identifiers to our internal agent UUIDs
-- This allows multiple users/groups to have isolated Sage instances

CREATE TABLE chat_contexts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    signal_identifier TEXT NOT NULL UNIQUE,
    context_type VARCHAR(20) NOT NULL DEFAULT 'direct',
    display_name TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for fast lookups by Signal identifier
CREATE INDEX idx_chat_contexts_signal_id ON chat_contexts(signal_identifier);

-- Note: context_type can be 'direct' (1:1) or 'group' (group chat)
-- The id here becomes the agent_id used in all other tables (messages, blocks, etc.)
