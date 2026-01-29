-- Create summaries table for context window compaction
-- Stores rolling summaries of older conversation history

CREATE TABLE summaries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    
    -- Range of messages this summary covers (sequence IDs)
    from_sequence_id BIGINT NOT NULL,
    to_sequence_id BIGINT NOT NULL,
    
    -- The summary content
    content TEXT NOT NULL,
    
    -- Embedding for semantic search in conversation_search
    embedding VECTOR(768),
    
    -- Chain: previous summary that was absorbed into this one
    previous_summary_id UUID REFERENCES summaries(id) ON DELETE SET NULL,
    
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    -- Ensure valid range
    CONSTRAINT valid_sequence_range CHECK (from_sequence_id <= to_sequence_id)
);

-- Index for finding latest summary for an agent
CREATE INDEX idx_summaries_agent_latest ON summaries(agent_id, to_sequence_id DESC);

-- Index for chain traversal
CREATE INDEX idx_summaries_previous ON summaries(previous_summary_id);

-- Vector similarity index for semantic search
CREATE INDEX idx_summaries_embedding ON summaries 
    USING ivfflat (embedding vector_cosine_ops)
    WITH (lists = 50);
