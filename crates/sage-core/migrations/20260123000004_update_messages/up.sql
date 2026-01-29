-- Update messages table to support the full memory system
-- Add fields needed for recall memory and context management

-- Add agent_id column (nullable for backward compatibility)
ALTER TABLE messages ADD COLUMN agent_id UUID;

-- Add sequence_id for monotonic ordering within an agent
ALTER TABLE messages ADD COLUMN sequence_id BIGSERIAL;

-- Add embedding for semantic search (optional, can be null)
ALTER TABLE messages ADD COLUMN embedding VECTOR(768);

-- Add tool_calls and tool_results as JSONB (for tool message tracking)
ALTER TABLE messages ADD COLUMN tool_calls JSONB;
ALTER TABLE messages ADD COLUMN tool_results JSONB;

-- Create index for agent-based queries
CREATE INDEX idx_messages_agent_seq ON messages(agent_id, sequence_id);

-- Create vector similarity index for semantic recall search
CREATE INDEX idx_messages_embedding ON messages 
    USING ivfflat (embedding vector_cosine_ops)
    WITH (lists = 100);
