-- Agents table
-- Stores agent configuration and context window state

CREATE TABLE agents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(255) NOT NULL,
    system_prompt TEXT NOT NULL,
    
    -- In-context message IDs (the current context window)
    -- This is the key invariant from Letta: context window is persisted state
    message_ids UUID[] NOT NULL DEFAULT '{}',
    
    -- LLM configuration (stored as JSON for flexibility)
    llm_config JSONB NOT NULL DEFAULT '{}',
    
    -- Memory state
    last_memory_update TIMESTAMPTZ,
    
    -- Context window settings
    max_context_tokens INT NOT NULL DEFAULT 256000,  -- Kimi K2 default
    compaction_threshold REAL NOT NULL DEFAULT 0.80,
    
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Trigger to auto-update updated_at timestamp
CREATE OR REPLACE FUNCTION update_agents_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER agents_updated_at_trigger
    BEFORE UPDATE ON agents
    FOR EACH ROW
    EXECUTE FUNCTION update_agents_updated_at();
