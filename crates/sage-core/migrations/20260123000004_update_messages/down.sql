DROP INDEX IF EXISTS idx_messages_embedding;
DROP INDEX IF EXISTS idx_messages_agent_seq;
ALTER TABLE messages DROP COLUMN IF EXISTS tool_results;
ALTER TABLE messages DROP COLUMN IF EXISTS tool_calls;
ALTER TABLE messages DROP COLUMN IF EXISTS embedding;
ALTER TABLE messages DROP COLUMN IF EXISTS sequence_id;
ALTER TABLE messages DROP COLUMN IF EXISTS agent_id;
