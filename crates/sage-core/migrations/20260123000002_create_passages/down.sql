DROP INDEX IF EXISTS idx_passages_embedding;
DROP INDEX IF EXISTS idx_passages_tags;
DROP INDEX IF EXISTS idx_passages_agent;
DROP TABLE IF EXISTS passages;
-- Note: We don't drop the vector extension as other tables might use it
