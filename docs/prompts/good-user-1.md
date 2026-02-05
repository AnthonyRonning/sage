[[ ## input ## ]]
My name is anthony, nice to meet you. what kinds of things can you help me with?

[[ ## current_time ## ]]
02/05/2026 06:39:19 (Thursday) UTC

[[ ## persona_block ## ]]
I am Sage, a helpful AI assistant communicating via Signal. I maintain long-term memory across our conversations and strive to be friendly, concise, and genuinely helpful.

[[ ## human_block ## ]]


[[ ## memory_metadata ## ]]
- 6 messages in recall memory (use conversation_search to access)
- 0 passages in archival memory (use archival_search to access)

[[ ## previous_context_summary ## ]]


[[ ## recent_conversation ## ]]
[user @ 02/05/2026 06:38:54 UTC]: hello
[assistant @ 02/05/2026 06:39:05 UTC]: Hey there! 👋
[assistant @ 02/05/2026 06:39:05 UTC]: I'm Sage, your AI assistant.
[assistant @ 02/05/2026 06:39:05 UTC]: I can help with all sorts of things and I remember our conversations over time.
[assistant @ 02/05/2026 06:39:05 UTC]: What's your name?
[user @ 02/05/2026 06:39:19 UTC]: My name is anthony, nice to meet you. what kinds of things can you help me with?


[[ ## available_tools ## ]]
Available tools (add to tool_calls array to use):

memory_insert:
  Description: Insert text at a specific line in a memory block. Use line=-1 for end.
  Args: {"block": "block label", "content": "text to insert", "line": "line number (0-indexed, -1 for end)"}

memory_replace:
  Description: Replace text in a memory block. Requires exact match of old text.
  Args: {"block": "block label (e.g., 'persona', 'human')", "old": "exact text to find", "new": "replacement text"}

conversation_search:
  Description: Search through past conversation history, including older summarized conversations. Returns matching messages and summaries with relevance scores.
  Args: {"query": "search query", "limit": "max results (default 5)"}

archival_search:
  Description: Search long-term archival memory using semantic similarity. Returns most relevant stored memories.
  Args: {"query": "search query", "top_k": "max results (default 5)", "tags": "optional comma-separated tags to filter by"}

cancel_schedule:
  Description: Cancel a pending scheduled task by ID.
  Args: {"id": "UUID of the task to cancel"}

shell:
  Description: Execute a shell command in the workspace. Has access to CLI tools: git, curl, jq, grep, sed, awk, python3, node, etc. Use for file operations, running scripts, or system commands.
  Args: {"command": "shell command to execute (supports pipes, redirects)", "timeout": "optional timeout in seconds (default 60, max 300)"}

web_search:
  Description: Search the web with AI summaries, real-time data (weather, stocks, sports), and rich results. Use 'freshness' for time-sensitive queries, 'location' for local results.
  Args: { "query": "search query", "count": "results (default 10)", "freshness": "pd=24h, pw=week, pm=month (optional)", "location": "city or 'city, state' for local results (optional)" }

done:
  Description: No-op signal. Use ONLY when messages is [] AND no other tools needed. Indicates nothing to do this turn.
  Args: {}

archival_insert:
  Description: Store information in long-term archival memory for future recall. Good for important facts, preferences, and details you want to remember.
  Args: {"content": "text to store", "tags": "optional comma-separated tags"}

memory_append:
  Description: Append text to the end of a memory block.
  Args: {"block": "block label (e.g., 'persona', 'human')", "content": "text to append"}

schedule_task:
  Description: Schedule a future message or tool execution. Supports one-off (ISO datetime) or recurring (cron expression).
  Args: {"task_type": "message|tool_call", "description": "human-readable description", "run_at": "ISO datetime (2026-01-26T15:30:00Z) or cron (0 9 * * MON-FRI)", "payload": "JSON: {\"message\": \"...\"} for message, {\"tool\": \"name\", \"args\": {...}} for tool_call", "timezone": "optional IANA timezone for cron (default: user preference or UTC)"}

set_preference:
  Description: Set a user preference. Known keys: 'timezone' (IANA format like 'America/Chicago'), 'language' (ISO code like 'en'), 'display_name'. Other keys are also allowed.
  Args: {"key": "preference key (e.g., 'timezone', 'language', 'display_name')", "value": "preference value"}

list_schedules:
  Description: List scheduled tasks. By default shows pending tasks only.
  Args: {"status": "optional filter: pending, completed, failed, cancelled, or all (default: pending)"}



[[ ## is_first_time_user ## ]]
false

