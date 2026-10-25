-- 0003_task_follow_ups.sql: operator follow-up instructions waiting for the next attempt of a task.
--
-- `continue_task` stores the instructions before it queues the task; the attempt that starts the
-- agent with them removes the row. They live in the database, not in memory, so a restart between
-- the two keeps them: crash recovery queues the task again and that attempt still finds them.
CREATE TABLE IF NOT EXISTS task_follow_ups (
    task_id TEXT PRIMARY KEY NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    instructions TEXT NOT NULL,
    created_at TEXT NOT NULL
);
