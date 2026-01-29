-- Revert to original messages table structure
DROP INDEX IF EXISTS idx_messages_role;
DROP INDEX IF EXISTS idx_messages_embedding;
DROP INDEX IF EXISTS idx_messages_agent_seq;
DROP INDEX IF EXISTS idx_messages_user_created;
DROP TABLE IF EXISTS messages;

CREATE TABLE messages (
    id BIGSERIAL PRIMARY KEY,
    user_id TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_messages_user_created ON messages(user_id, created_at DESC);
