-- Scheduled tasks for reminders, recurring tool calls, etc.
CREATE TABLE scheduled_tasks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    
    -- What to do: 'message' or 'tool_call'
    task_type VARCHAR(20) NOT NULL,
    -- Payload: {message: "..."} for messages, {tool: "name", args: {...}} for tool calls
    payload JSONB NOT NULL,
    
    -- When to do it
    next_run_at TIMESTAMPTZ NOT NULL,
    -- NULL = one-off, cron expression = recurring (e.g., "0 9 * * MON-FRI")
    cron_expression VARCHAR(100),
    -- Timezone for cron interpretation (IANA format, e.g., "America/Chicago")
    timezone VARCHAR(50) NOT NULL DEFAULT 'UTC',
    
    -- Status: pending, running, completed, failed, cancelled
    status VARCHAR(20) NOT NULL DEFAULT 'pending',
    last_run_at TIMESTAMPTZ,
    run_count INT NOT NULL DEFAULT 0,
    last_error TEXT,
    
    -- Human-readable description (e.g., "remind me to call mom")
    description TEXT NOT NULL,
    
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for efficient polling of due tasks
CREATE INDEX idx_scheduled_tasks_pending ON scheduled_tasks(next_run_at) 
    WHERE status = 'pending';

-- Index for listing tasks by agent
CREATE INDEX idx_scheduled_tasks_agent ON scheduled_tasks(agent_id, status);
