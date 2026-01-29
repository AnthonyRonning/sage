-- Core memory blocks table
-- Each agent has editable memory blocks (persona, human, etc.)

CREATE TABLE blocks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id TEXT NOT NULL,  -- Links to agent (using TEXT for now, can be UUID later)
    label VARCHAR(100) NOT NULL,
    description TEXT,
    value TEXT NOT NULL DEFAULT '',
    char_limit INT NOT NULL DEFAULT 20000,
    read_only BOOLEAN NOT NULL DEFAULT FALSE,
    version INT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    -- Each agent can only have one block per label
    UNIQUE(agent_id, label)
);

-- Index for fast lookup by agent
CREATE INDEX idx_blocks_agent ON blocks(agent_id);

-- Trigger to auto-update updated_at timestamp
CREATE OR REPLACE FUNCTION update_blocks_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    NEW.version = OLD.version + 1;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER blocks_updated_at_trigger
    BEFORE UPDATE ON blocks
    FOR EACH ROW
    EXECUTE FUNCTION update_blocks_updated_at();
