-- Enable pgvector extension for vector similarity search
CREATE EXTENSION IF NOT EXISTS vector;

-- Archival memory passages table
-- Stores long-term memories with embeddings for semantic search

CREATE TABLE passages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id TEXT NOT NULL,  -- Links to agent
    content TEXT NOT NULL,
    embedding VECTOR(768),   -- nomic-embed-text dimension
    tags TEXT[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for fast lookup by agent
CREATE INDEX idx_passages_agent ON passages(agent_id);

-- Index for tag filtering (GIN for array containment)
CREATE INDEX idx_passages_tags ON passages USING GIN(tags);

-- Vector similarity index (IVFFlat for approximate nearest neighbor)
-- Using cosine distance (vector_cosine_ops)
CREATE INDEX idx_passages_embedding ON passages 
    USING ivfflat (embedding vector_cosine_ops)
    WITH (lists = 100);
