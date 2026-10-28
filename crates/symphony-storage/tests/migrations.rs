//! Schema migration behaviour against databases written by earlier versions.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use symphony_storage::{Storage, SCHEMA_VERSION};

/// The schema exactly as Symphony Desktop 2.0.0 created it: no `schema_migrations` table.
const SCHEMA_V1: &str = include_str!("../migrations/0001_init.sql");

async fn open_raw(path: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true),
        )
        .await
        .unwrap()
}

/// A 2.0.0-era database holding one project, one task with token totals and one attempt.
async fn write_legacy_database(path: &Path) {
    let pool = open_raw(path).await;
    sqlx::raw_sql(SCHEMA_V1).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO projects (id, name, repo_path, workspace_root, created_at, updated_at)
         VALUES ('p1', 'Legacy', 'C:/repo', 'C:/ws', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let issue = serde_json::json!({
        "id": "t1", "identifier": "SYM-1", "title": "Legacy task", "state": "Todo",
        "labels": [], "blocked_by": [], "dispatchable": true
    })
    .to_string();
    sqlx::query(
        "INSERT INTO tasks (id, project_id, issue_id, issue_identifier, issue_json, status, provider,
            current_thread_id, input_tokens, output_tokens, total_tokens, estimated_cost_usd,
            created_at, updated_at)
         VALUES ('t1', 'p1', 't1', 'SYM-1', ?1, 'failed', 'codex', 'thread-old', 900, 100, 1000, 0.5,
            '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(issue)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO run_attempts (id, task_id, issue_id, issue_identifier, attempt_number, provider,
            workspace_path, branch_name, thread_id, phase, started_at, input_tokens, output_tokens,
            total_tokens)
         VALUES ('a1', 't1', 't1', 'SYM-1', 1, 'codex', 'C:/ws/SYM-1', 'sym-1', 'thread-old',
            'failed', '2026-01-01T00:00:00Z', 900, 100, 1000)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
}

fn snapshots(dir: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .contains(".pre-migration-")
        })
        .collect()
}

#[tokio::test]
async fn upgrades_a_legacy_database_without_losing_data() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("symphony.sqlite");
    write_legacy_database(&db).await;

    let storage = Storage::connect(&db).await.unwrap();

    let task = storage
        .get_task("t1")
        .await
        .unwrap()
        .expect("task survives");
    assert_eq!(task.issue.identifier, "SYM-1");
    assert_eq!(
        (task.input_tokens, task.output_tokens, task.total_tokens),
        (900, 100, 1000)
    );
    assert_eq!(task.current_thread_id.as_deref(), Some("thread-old"));
    // A legacy task has no recorded thread baseline and its totals are not flagged.
    assert!(task.thread_usage.is_none());
    assert!(!task.usage_estimated);
    assert_eq!(storage.list_projects().await.unwrap().len(), 1);
    let attempts = storage.list_run_attempts("t1").await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].total_tokens, 1000);

    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM schema_migrations ORDER BY version")
            .fetch_all(storage.pool())
            .await
            .unwrap();
    assert_eq!(versions, (1..=SCHEMA_VERSION).collect::<Vec<_>>());
}

#[tokio::test]
async fn snapshots_a_database_that_holds_data_before_migrating_it() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("symphony.sqlite");
    write_legacy_database(&db).await;

    drop(Storage::connect(&db).await.unwrap());

    let found = snapshots(dir.path());
    assert_eq!(found.len(), 1, "one pre-migration snapshot: {found:?}");
    // The snapshot is the database as it was: original rows, none of the new columns.
    let pool = open_raw(&found[0]).await;
    let row = sqlx::query("SELECT total_tokens FROM tasks WHERE id = 't1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i64, _>(0), 1000);
    let columns: Vec<String> = sqlx::query("PRAGMA table_info(tasks)")
        .fetch_all(&pool)
        .await
        .unwrap()
        .iter()
        .map(|r| r.get::<String, _>("name"))
        .collect();
    assert!(!columns.contains(&"usage_estimated".to_string()));
    pool.close().await;
}

#[tokio::test]
async fn migrating_is_idempotent_and_takes_no_second_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("symphony.sqlite");
    write_legacy_database(&db).await;

    drop(Storage::connect(&db).await.unwrap());
    let storage = Storage::connect(&db).await.unwrap();
    storage.run_migrations().await.unwrap();

    assert_eq!(snapshots(dir.path()).len(), 1);
    let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schema_migrations")
        .fetch_one(storage.pool())
        .await
        .unwrap();
    assert_eq!(applied, SCHEMA_VERSION);
}

#[tokio::test]
async fn a_new_database_needs_no_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("symphony.sqlite");
    drop(Storage::connect(&db).await.unwrap());
    assert!(snapshots(dir.path()).is_empty());
}

#[tokio::test]
async fn refuses_a_database_written_by_a_newer_version_and_leaves_it_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("symphony.sqlite");
    write_legacy_database(&db).await;
    drop(Storage::connect(&db).await.unwrap());

    let pool = open_raw(&db).await;
    sqlx::query(
        "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, 'future', 'x')",
    )
    .bind(SCHEMA_VERSION + 1)
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let err = match Storage::connect(&db).await {
        Ok(_) => panic!("a newer schema must be refused"),
        Err(e) => e.to_string(),
    };
    assert!(
        err.contains("newer") || err.contains("only supports"),
        "{err}"
    );

    let pool = open_raw(&db).await;
    let tasks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tasks, 1);
    pool.close().await;
}

#[tokio::test]
async fn backup_is_a_consistent_copy_including_uncheckpointed_wal_content() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("symphony.sqlite");
    write_legacy_database(&db).await;
    let storage = Storage::connect(&db).await.unwrap();
    // Committed changes may still sit in the -wal file; the backup must contain them.
    storage
        .add_task_usage(
            "t1",
            &symphony_core::TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
            },
            0.0,
            true,
            None,
        )
        .await
        .unwrap();

    let backup = dir.path().join("backups").join("copy.sqlite");
    storage.backup_to(&backup).await.unwrap();

    let restored = Storage::connect(&backup).await.unwrap();
    let task = restored.get_task("t1").await.unwrap().unwrap();
    assert_eq!(task.total_tokens, 1015);
    assert!(task.usage_estimated);
    assert!(
        storage.backup_to(&backup).await.is_err(),
        "an existing backup is never overwritten"
    );
}

/// A database as the previous release (schema 2) left it: migrations 1 and 2 recorded, a task with
/// a thread baseline and a stored pull-request review. Opening it applies 3 and keeps everything.
#[tokio::test]
async fn upgrades_the_previous_schema_without_losing_data() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("symphony.sqlite");
    write_legacy_database(&db).await;
    {
        let pool = open_raw(&db).await;
        sqlx::raw_sql(include_str!("../migrations/0002_usage_accounting.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL, applied_at TEXT NOT NULL);
             INSERT INTO schema_migrations (version, name, applied_at) VALUES
                (1, 'init', '2026-10-22T00:00:00Z'), (2, 'usage_accounting', '2026-10-22T00:00:00Z');
             UPDATE tasks SET thread_usage_thread_id = 'thread-old', thread_usage_input_tokens = 900,
                thread_usage_output_tokens = 100, thread_usage_total_tokens = 1000 WHERE id = 't1';",
        )
        .execute(&pool)
        .await
        .unwrap();
        // A review published by the previous release: no `unpushed_changes` in its JSON.
        let review = serde_json::json!({
            "id": "pr-t1", "task_id": "t1", "project_id": "p1", "pr_number": 9, "title": "SYM-1: Legacy",
            "description": "", "url": "https://github.com/acme/widgets/pull/9", "branch_name": "sym-1",
            "base_branch": "main", "state": "open", "ci_status": "unknown", "mergeable": true,
            "has_conflicts": false, "changed_files": [], "diff_preview": "", "validation_results": [],
            "review_comments": [], "security_warnings": [],
            "merge_readiness": { "score": 4, "label": "x", "evidence": [], "blockers": [] },
            "human_approved": false, "updated_at": "2026-10-22T00:00:00Z"
        })
        .to_string();
        sqlx::query(
            "INSERT INTO pull_request_reviews (id, task_id, project_id, pr_number, review_json, updated_at)
             VALUES ('pr-t1', 't1', 'p1', 9, ?1, '2026-10-22T00:00:00Z')",
        )
        .bind(review)
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    }

    let storage = Storage::connect(&db).await.unwrap();

    let task = storage
        .get_task("t1")
        .await
        .unwrap()
        .expect("task survives");
    assert_eq!(task.total_tokens, 1000);
    assert_eq!(
        task.thread_usage.as_ref().map(|u| u.usage.total_tokens),
        Some(1000)
    );
    let review = storage
        .get_pr_review("pr-t1")
        .await
        .unwrap()
        .expect("review survives");
    assert!(review.has_pull_request());
    assert!(
        !review.has_unpushed_changes(),
        "an older published review is not taken for unpublished work"
    );
    // The new table works on the upgraded database.
    storage
        .set_task_follow_up("t1", "add a test")
        .await
        .unwrap();
    assert_eq!(
        storage.task_follow_up("t1").await.unwrap().as_deref(),
        Some("add a test")
    );
    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM schema_migrations ORDER BY version")
            .fetch_all(storage.pool())
            .await
            .unwrap();
    assert_eq!(versions, (1..=SCHEMA_VERSION).collect::<Vec<_>>());
}

/// A database as the release with schema 3 left it: migrations 1–3 recorded, a task with a pending
/// follow-up. Opening it applies 4 (tool-call history) and keeps everything.
#[tokio::test]
async fn upgrades_schema_three_and_keeps_pending_follow_ups() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("symphony.sqlite");
    write_legacy_database(&db).await;
    {
        let pool = open_raw(&db).await;
        for sql in [
            include_str!("../migrations/0002_usage_accounting.sql"),
            include_str!("../migrations/0003_task_follow_ups.sql"),
        ] {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL, applied_at TEXT NOT NULL);
             INSERT INTO schema_migrations (version, name, applied_at) VALUES
                (1, 'init', '2026-10-22T00:00:00Z'), (2, 'usage_accounting', '2026-10-22T00:00:00Z'),
                (3, 'task_follow_ups', '2026-10-25T00:00:00Z');
             INSERT INTO task_follow_ups (task_id, instructions, created_at)
                VALUES ('t1', 'also handle empty input', '2026-10-25T00:00:00Z');",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    }

    let storage = Storage::connect(&db).await.unwrap();

    assert!(
        storage.get_task("t1").await.unwrap().is_some(),
        "task survives"
    );
    assert_eq!(
        storage.task_follow_up("t1").await.unwrap().as_deref(),
        Some("also handle empty input"),
        "a pending follow-up survives the upgrade"
    );
    // The new table works on the upgraded database.
    let call = symphony_core::ToolCallRecord::apply(
        None,
        "t1",
        None,
        symphony_core::ProviderKind::Codex,
        symphony_core::ToolCallUpdate {
            call_id: "c1".into(),
            tool_name: "shell".into(),
            kind: symphony_core::ToolCallKind::Command,
            arguments: None,
            output: Some("ok".into()),
            status: symphony_core::ToolCallStatus::Succeeded,
            duration_ms: Some(3),
        },
        chrono::Utc::now(),
    );
    storage.upsert_tool_call(&call).await.unwrap();
    let calls = storage
        .list_tool_calls("t1", 10, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(calls, vec![call]);
    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM schema_migrations ORDER BY version")
            .fetch_all(storage.pool())
            .await
            .unwrap();
    assert_eq!(versions, (1..=SCHEMA_VERSION).collect::<Vec<_>>());
}
