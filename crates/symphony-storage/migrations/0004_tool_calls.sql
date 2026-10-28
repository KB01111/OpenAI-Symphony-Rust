-- 0004_tool_calls.sql: tool-call history (one row per tool call an agent made).
--
-- The start and the result of a call are merged into one row (`id` = "<task id>:<call id>").
-- Arguments and output are redacted before they are written and output is capped at 64 KiB.
-- Rows of finished tasks follow the activity-log retention (`event_retention_days`).
CREATE TABLE IF NOT EXISTS tool_calls (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    session_id TEXT,
    provider TEXT NOT NULL,
    call_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    kind TEXT NOT NULL,
    arguments_json TEXT,
    output TEXT,
    output_truncated INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    duration_ms INTEGER
);

CREATE INDEX IF NOT EXISTS idx_tool_calls_task ON tool_calls(task_id, started_at);
CREATE INDEX IF NOT EXISTS idx_tool_calls_started_at ON tool_calls(started_at);
