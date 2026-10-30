-- Knowledge Base index. Everything here is derived from a repository and can be rebuilt, which is
-- why it lives in its own database file (not in the main database, its backups or its migrations).

CREATE TABLE IF NOT EXISTS kb_index_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL,
    ref_name TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    engine TEXT NOT NULL,
    engine_version TEXT NOT NULL,
    status TEXT NOT NULL,
    node_count INTEGER NOT NULL DEFAULT 0,
    edge_count INTEGER NOT NULL DEFAULT 0,
    file_count INTEGER NOT NULL DEFAULT 0,
    languages_json TEXT NOT NULL DEFAULT '[]',
    error TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    duration_ms INTEGER
);
CREATE INDEX IF NOT EXISTS idx_kb_runs_project ON kb_index_runs(project_id, id);

CREATE TABLE IF NOT EXISTS kb_nodes (
    run_id INTEGER NOT NULL REFERENCES kb_index_runs(id) ON DELETE CASCADE,
    node_id TEXT NOT NULL,
    label TEXT NOT NULL,
    kind TEXT NOT NULL,
    file TEXT,
    line INTEGER,
    module TEXT,
    degree INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run_id, node_id)
);
CREATE INDEX IF NOT EXISTS idx_kb_nodes_file ON kb_nodes(run_id, file, line);

CREATE TABLE IF NOT EXISTS kb_edges (
    run_id INTEGER NOT NULL REFERENCES kb_index_runs(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    target TEXT NOT NULL,
    relation TEXT NOT NULL,
    confidence TEXT NOT NULL,
    line INTEGER
);
CREATE INDEX IF NOT EXISTS idx_kb_edges_run ON kb_edges(run_id);

-- Full-text search over symbol and file names.
CREATE VIRTUAL TABLE IF NOT EXISTS kb_nodes_fts USING fts5(
    label,
    file,
    run_id UNINDEXED,
    node_id UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Changed symbols and the affected set of a commit, per index run.
CREATE TABLE IF NOT EXISTS kb_commit_impact (
    project_id TEXT NOT NULL,
    sha TEXT NOT NULL,
    run_id INTEGER NOT NULL REFERENCES kb_index_runs(id) ON DELETE CASCADE,
    depth INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (project_id, sha, run_id, depth)
);

-- AI commit analyses, cached by commit and analysis configuration.
CREATE TABLE IF NOT EXISTS kb_analyses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL,
    sha TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL DEFAULT '',
    prompt_version INTEGER NOT NULL,
    status TEXT NOT NULL,
    result_json TEXT,
    evidence_json TEXT NOT NULL DEFAULT '[]',
    dropped_findings INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    finished_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_kb_analyses_commit ON kb_analyses(project_id, sha, id);

CREATE TABLE IF NOT EXISTS kb_chat_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    evidence_json TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL,
    provider TEXT,
    model TEXT,
    error TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_kb_chat_project ON kb_chat_messages(project_id, id);
