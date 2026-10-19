-- 0001_init.sql: Symphony Desktop 1.0 Persistent Schema (SQLite WAL)

CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    repo_path TEXT NOT NULL,
    default_branch TEXT NOT NULL DEFAULT 'main',
    workspace_root TEXT NOT NULL,
    workflow_path TEXT,
    preferred_provider TEXT NOT NULL DEFAULT 'codex',
    preferred_model TEXT,
    tracker_kind TEXT NOT NULL DEFAULT 'github',
    github_repo TEXT,
    linear_project_slug TEXT,
    required_labels_json TEXT NOT NULL DEFAULT '[]',
    auto_dispatch INTEGER NOT NULL DEFAULT 0,
    max_concurrent_agents INTEGER NOT NULL DEFAULT 2,
    validation_commands_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    issue_id TEXT NOT NULL,
    issue_identifier TEXT NOT NULL,
    issue_json TEXT NOT NULL,
    status TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT,
    workspace_path TEXT,
    branch_name TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    current_session_id TEXT,
    current_thread_id TEXT,
    current_turn_id TEXT,
    current_phase TEXT,
    pr_number INTEGER,
    pr_url TEXT,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    estimated_cost_usd REAL NOT NULL DEFAULT 0.0,
    duration_seconds REAL NOT NULL DEFAULT 0.0,
    last_error TEXT,
    next_retry_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_tasks_project_id ON tasks(project_id);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_tasks_issue_identifier ON tasks(issue_identifier);

CREATE TABLE IF NOT EXISTS run_attempts (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL,
    issue_id TEXT NOT NULL,
    issue_identifier TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    provider TEXT NOT NULL,
    workspace_path TEXT NOT NULL,
    branch_name TEXT NOT NULL,
    session_id TEXT,
    thread_id TEXT,
    turn_id TEXT,
    turn_count INTEGER NOT NULL DEFAULT 0,
    phase TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    error TEXT,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY(task_id) REFERENCES tasks(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_run_attempts_task_id ON run_attempts(task_id);

CREATE TABLE IF NOT EXISTS agent_events (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT,
    issue_identifier TEXT,
    session_id TEXT,
    provider TEXT,
    event_type TEXT NOT NULL,
    phase TEXT,
    message TEXT NOT NULL,
    payload_json TEXT,
    timestamp TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_events_task_id ON agent_events(task_id);
CREATE INDEX IF NOT EXISTS idx_agent_events_timestamp ON agent_events(timestamp DESC);

CREATE TABLE IF NOT EXISTS approval_requests (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL,
    issue_identifier TEXT NOT NULL,
    session_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    command_or_diff TEXT,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    resolved_at TEXT,
    decision TEXT
);

CREATE INDEX IF NOT EXISTS idx_approval_requests_status ON approval_requests(status);

CREATE TABLE IF NOT EXISTS validation_results (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    command TEXT NOT NULL,
    exit_code INTEGER NOT NULL,
    passed INTEGER NOT NULL,
    stdout TEXT NOT NULL,
    stderr TEXT NOT NULL,
    duration_ms INTEGER NOT NULL,
    executed_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_validation_results_task_id ON validation_results(task_id);

CREATE TABLE IF NOT EXISTS pull_request_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    pr_number INTEGER NOT NULL,
    review_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS provider_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT,
    command_override TEXT,
    approval_policy TEXT NOT NULL,
    sandbox_mode TEXT NOT NULL,
    credential_ref TEXT,
    is_default INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY NOT NULL,
    value_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
