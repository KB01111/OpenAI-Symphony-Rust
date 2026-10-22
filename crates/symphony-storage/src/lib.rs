use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::{ConnectOptions, Connection, Row, SqlitePool};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;
use symphony_core::{
    redact_secrets, AgentEvent, AppSettings, ApprovalDecision, ApprovalRequest, ApprovalStatus,
    Issue, Project, ProviderKind, ProviderProfile, PullRequestReview, RunAttempt, RunAttemptPhase,
    SymphonyError, SymphonyResult, Task, TaskStatus, ThreadUsageMark, TokenUsage, ValidationResult,
};

pub mod backup;

#[derive(Clone)]
pub struct Storage {
    pool: SqlitePool,
    /// The database file, `None` for in-memory databases.
    db_path: Option<PathBuf>,
}

/// One forward-only schema migration. Versions are applied in order, each inside a transaction, and
/// recorded in `schema_migrations`; a migration that fails leaves the database untouched.
struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    // 0001 only uses `CREATE ... IF NOT EXISTS`, so it is also the baseline of databases created
    // before versions were recorded.
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "usage_accounting",
        sql: include_str!("../migrations/0002_usage_accounting.sql"),
    },
];

/// The schema version this build reads and writes.
pub const SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticBundle {
    pub exported_at: DateTime<Utc>,
    pub settings: AppSettings,
    pub projects: Vec<Project>,
    pub tasks: Vec<Task>,
    pub recent_attempts: Vec<RunAttempt>,
    pub recent_events: Vec<AgentEvent>,
    pub validations: Vec<ValidationResult>,
}

impl Storage {
    /// Connect to a SQLite database file with WAL mode enabled and apply schema migrations.
    ///
    /// Migrations run on a dedicated connection that is closed before the pool opens: a pooled
    /// connection that cached the old schema would otherwise misread rows of altered tables.
    pub async fn connect(db_path: impl AsRef<Path>) -> SymphonyResult<Self> {
        let path_ref = db_path.as_ref();
        if let Some(parent) = path_ref.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let options = SqliteConnectOptions::new()
            .filename(path_ref)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(10))
            .foreign_keys(true);

        let mut migration_conn = options
            .connect()
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;
        let migrated = apply_migrations(
            &mut migration_conn,
            Some(path_ref),
            MIGRATIONS,
            SCHEMA_VERSION,
        )
        .await;
        let _ = migration_conn.close().await;
        migrated?;

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        Ok(Self {
            pool,
            db_path: Some(path_ref.to_path_buf()),
        })
    }

    /// Connect to an in-memory SQLite database for isolated unit and integration tests.
    pub async fn connect_in_memory() -> SymphonyResult<Self> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        let storage = Self {
            pool,
            db_path: None,
        };
        storage.run_migrations().await?;
        Ok(storage)
    }

    /// Apply pending schema migrations (see [`apply_migrations`]); a no-op when up to date.
    pub async fn run_migrations(&self) -> SymphonyResult<()> {
        let mut conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;
        apply_migrations(
            &mut conn,
            self.db_path.as_deref(),
            MIGRATIONS,
            SCHEMA_VERSION,
        )
        .await
    }

    /// Write a transactionally consistent copy of the database to `destination` (SQLite
    /// `VACUUM INTO`): readers and writers keep working, committed WAL content is included and the
    /// result is a self-contained database file. Fails if `destination` already exists.
    pub async fn backup_to(&self, destination: &Path) -> SymphonyResult<()> {
        let mut conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;
        backup_connection_to(&mut conn, destination).await
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // =========================================================================
    // Projects
    // =========================================================================

    pub async fn upsert_project(&self, project: &Project) -> SymphonyResult<()> {
        let labels_json = serde_json::to_string(&project.required_labels)
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;
        let cmds_json = serde_json::to_string(&project.validation_commands)
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO projects (
                id, name, repo_path, default_branch, workspace_root, workflow_path,
                preferred_provider, preferred_model, tracker_kind, github_repo,
                linear_project_slug, required_labels_json, auto_dispatch,
                max_concurrent_agents, validation_commands_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                repo_path = excluded.repo_path,
                default_branch = excluded.default_branch,
                workspace_root = excluded.workspace_root,
                workflow_path = excluded.workflow_path,
                preferred_provider = excluded.preferred_provider,
                preferred_model = excluded.preferred_model,
                tracker_kind = excluded.tracker_kind,
                github_repo = excluded.github_repo,
                linear_project_slug = excluded.linear_project_slug,
                required_labels_json = excluded.required_labels_json,
                auto_dispatch = excluded.auto_dispatch,
                max_concurrent_agents = excluded.max_concurrent_agents,
                validation_commands_json = excluded.validation_commands_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&project.id)
        .bind(&project.name)
        .bind(&project.repo_path)
        .bind(&project.default_branch)
        .bind(&project.workspace_root)
        .bind(&project.workflow_path)
        .bind(project.preferred_provider.as_str())
        .bind(&project.preferred_model)
        .bind(&project.tracker_kind)
        .bind(&project.github_repo)
        .bind(&project.linear_project_slug)
        .bind(labels_json)
        .bind(project.auto_dispatch as i64)
        .bind(project.max_concurrent_agents as i64)
        .bind(cmds_json)
        .bind(project.created_at.to_rfc3339())
        .bind(project.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        Ok(())
    }

    pub async fn list_projects(&self) -> SymphonyResult<Vec<Project>> {
        let rows = sqlx::query("SELECT * FROM projects ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        rows.into_iter().map(row_to_project).collect()
    }

    pub async fn get_project(&self, id: &str) -> SymphonyResult<Option<Project>> {
        let row = sqlx::query("SELECT * FROM projects WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        row.map(row_to_project).transpose()
    }

    pub async fn delete_project(&self, id: &str) -> SymphonyResult<()> {
        sqlx::query("DELETE FROM projects WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;
        Ok(())
    }

    // =========================================================================
    // Tasks
    // =========================================================================

    pub async fn upsert_task(&self, task: &Task) -> SymphonyResult<()> {
        let issue_json = serde_json::to_string(&task.issue)
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;
        let phase_str = task.current_phase.map(|p| p.to_string());

        sqlx::query(
            r#"
            INSERT INTO tasks (
                id, project_id, issue_id, issue_identifier, issue_json, status,
                provider, model, workspace_path, branch_name, attempt_count,
                current_session_id, current_thread_id, current_turn_id, current_phase,
                pr_number, pr_url, input_tokens, output_tokens, total_tokens,
                estimated_cost_usd, duration_seconds, last_error, next_retry_at,
                created_at, updated_at, started_at, completed_at,
                thread_usage_thread_id, thread_usage_input_tokens, thread_usage_output_tokens,
                thread_usage_total_tokens, usage_estimated
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28,
                ?29, ?30, ?31, ?32, ?33
            )
            ON CONFLICT(id) DO UPDATE SET
                issue_id = excluded.issue_id,
                issue_identifier = excluded.issue_identifier,
                issue_json = excluded.issue_json,
                status = excluded.status,
                provider = excluded.provider,
                model = excluded.model,
                workspace_path = excluded.workspace_path,
                branch_name = excluded.branch_name,
                attempt_count = excluded.attempt_count,
                current_session_id = excluded.current_session_id,
                current_thread_id = excluded.current_thread_id,
                current_turn_id = excluded.current_turn_id,
                current_phase = excluded.current_phase,
                pr_number = excluded.pr_number,
                pr_url = excluded.pr_url,
                input_tokens = excluded.input_tokens,
                output_tokens = excluded.output_tokens,
                total_tokens = excluded.total_tokens,
                estimated_cost_usd = excluded.estimated_cost_usd,
                duration_seconds = excluded.duration_seconds,
                last_error = excluded.last_error,
                next_retry_at = excluded.next_retry_at,
                updated_at = excluded.updated_at,
                started_at = excluded.started_at,
                completed_at = excluded.completed_at,
                thread_usage_thread_id = excluded.thread_usage_thread_id,
                thread_usage_input_tokens = excluded.thread_usage_input_tokens,
                thread_usage_output_tokens = excluded.thread_usage_output_tokens,
                thread_usage_total_tokens = excluded.thread_usage_total_tokens,
                usage_estimated = excluded.usage_estimated
            "#,
        )
        .bind(&task.id)
        .bind(&task.project_id)
        .bind(&task.issue.id)
        .bind(&task.issue.identifier)
        .bind(issue_json)
        .bind(task.status.as_str())
        .bind(task.provider.as_str())
        .bind(&task.model)
        .bind(&task.workspace_path)
        .bind(&task.branch_name)
        .bind(task.attempt_count as i64)
        .bind(&task.current_session_id)
        .bind(&task.current_thread_id)
        .bind(&task.current_turn_id)
        .bind(phase_str)
        .bind(task.pr_number.map(|n| n as i64))
        .bind(&task.pr_url)
        .bind(task.input_tokens as i64)
        .bind(task.output_tokens as i64)
        .bind(task.total_tokens as i64)
        .bind(task.estimated_cost_usd)
        .bind(task.duration_seconds)
        .bind(&task.last_error)
        .bind(task.next_retry_at.map(|d| d.to_rfc3339()))
        .bind(task.created_at.to_rfc3339())
        .bind(task.updated_at.to_rfc3339())
        .bind(task.started_at.map(|d| d.to_rfc3339()))
        .bind(task.completed_at.map(|d| d.to_rfc3339()))
        .bind(task.thread_usage.as_ref().map(|m| m.thread_id.clone()))
        .bind(
            task.thread_usage
                .as_ref()
                .map(|m| m.usage.input_tokens as i64),
        )
        .bind(
            task.thread_usage
                .as_ref()
                .map(|m| m.usage.output_tokens as i64),
        )
        .bind(
            task.thread_usage
                .as_ref()
                .map(|m| m.usage.total_tokens as i64),
        )
        .bind(task.usage_estimated as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        Ok(())
    }

    pub async fn list_tasks(&self, project_id: Option<&str>) -> SymphonyResult<Vec<Task>> {
        let rows = if let Some(pid) = project_id {
            sqlx::query("SELECT * FROM tasks WHERE project_id = ?1 ORDER BY updated_at DESC")
                .bind(pid)
                .fetch_all(&self.pool)
                .await
        } else {
            sqlx::query("SELECT * FROM tasks ORDER BY updated_at DESC")
                .fetch_all(&self.pool)
                .await
        }
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        rows.into_iter().map(row_to_task).collect()
    }

    pub async fn get_task(&self, id: &str) -> SymphonyResult<Option<Task>> {
        let row = sqlx::query("SELECT * FROM tasks WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        row.map(row_to_task).transpose()
    }

    /// Add usage to a task's totals in one atomic statement, without touching any other column.
    /// For code that settles usage on behalf of a stopped worker whose in-memory copy of the task
    /// is stale (rewriting the whole row would overwrite a concurrent status change).
    pub async fn add_task_usage(
        &self,
        task_id: &str,
        delta: &TokenUsage,
        estimated_cost_usd: f64,
        estimated: bool,
        thread_usage: Option<&ThreadUsageMark>,
    ) -> SymphonyResult<()> {
        sqlx::query(
            "UPDATE tasks SET
                input_tokens = input_tokens + ?2,
                output_tokens = output_tokens + ?3,
                total_tokens = total_tokens + ?4,
                estimated_cost_usd = estimated_cost_usd + ?5,
                usage_estimated = usage_estimated OR ?6,
                thread_usage_thread_id = COALESCE(?7, thread_usage_thread_id),
                thread_usage_input_tokens = COALESCE(?8, thread_usage_input_tokens),
                thread_usage_output_tokens = COALESCE(?9, thread_usage_output_tokens),
                thread_usage_total_tokens = COALESCE(?10, thread_usage_total_tokens)
             WHERE id = ?1",
        )
        .bind(task_id)
        .bind(delta.input_tokens as i64)
        .bind(delta.output_tokens as i64)
        .bind(delta.total_tokens as i64)
        .bind(estimated_cost_usd)
        .bind(estimated as i64)
        .bind(thread_usage.map(|m| m.thread_id.clone()))
        .bind(thread_usage.map(|m| m.usage.input_tokens as i64))
        .bind(thread_usage.map(|m| m.usage.output_tokens as i64))
        .bind(thread_usage.map(|m| m.usage.total_tokens as i64))
        .execute(&self.pool)
        .await
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;
        Ok(())
    }

    pub async fn get_task_by_identifier(&self, identifier: &str) -> SymphonyResult<Option<Task>> {
        let row = sqlx::query("SELECT * FROM tasks WHERE issue_identifier = ?1 LIMIT 1")
            .bind(identifier)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        row.map(row_to_task).transpose()
    }

    pub async fn delete_task(&self, id: &str) -> SymphonyResult<()> {
        sqlx::query("DELETE FROM tasks WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;
        Ok(())
    }

    /// Crash recovery helper (Section 5):
    /// Finds tasks left in `Running` status from a previous interrupted process run,
    /// marks their stale attempts as interrupted, and moves them to `Queued` with a retry schedule
    /// so they are clearly distinguished from completed tasks and safely recovered.
    pub async fn recover_stale_running_tasks(&self) -> SymphonyResult<Vec<Task>> {
        let rows = sqlx::query("SELECT * FROM tasks WHERE status = 'running'")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        let mut recovered = Vec::new();
        let now = Utc::now();

        for row in rows {
            let mut task = row_to_task(row)?;
            task.status = TaskStatus::Queued;
            task.current_phase = None;
            task.current_session_id = None;
            task.last_error = Some(
                "Interrupted execution detected on startup recovery; scheduled for safe reconciliation/retry."
                    .to_string(),
            );
            task.next_retry_at = Some(now);
            task.updated_at = now;
            self.upsert_task(&task).await?;

            // Also mark any unfinished run_attempts for this task as failed/interrupted
            sqlx::query(
                "UPDATE run_attempts SET phase = 'failed', finished_at = ?1, error = 'Interrupted by host restart' WHERE task_id = ?2 AND finished_at IS NULL",
            )
            .bind(now.to_rfc3339())
            .bind(&task.id)
            .execute(&self.pool)
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

            recovered.push(task);
        }

        Ok(recovered)
    }

    // =========================================================================
    // Run Attempts
    // =========================================================================

    pub async fn upsert_run_attempt(&self, attempt: &RunAttempt) -> SymphonyResult<()> {
        sqlx::query(
            r#"
            INSERT INTO run_attempts (
                id, task_id, issue_id, issue_identifier, attempt_number, provider,
                workspace_path, branch_name, session_id, thread_id, turn_id,
                turn_count, phase, started_at, finished_at, error,
                input_tokens, output_tokens, total_tokens, usage_estimated
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20
            )
            ON CONFLICT(id) DO UPDATE SET
                session_id = excluded.session_id,
                thread_id = excluded.thread_id,
                turn_id = excluded.turn_id,
                turn_count = excluded.turn_count,
                phase = excluded.phase,
                finished_at = excluded.finished_at,
                error = excluded.error,
                input_tokens = excluded.input_tokens,
                output_tokens = excluded.output_tokens,
                total_tokens = excluded.total_tokens,
                usage_estimated = excluded.usage_estimated
            "#,
        )
        .bind(&attempt.id)
        .bind(&attempt.task_id)
        .bind(&attempt.issue_id)
        .bind(&attempt.issue_identifier)
        .bind(attempt.attempt_number as i64)
        .bind(attempt.provider.as_str())
        .bind(&attempt.workspace_path)
        .bind(&attempt.branch_name)
        .bind(&attempt.session_id)
        .bind(&attempt.thread_id)
        .bind(&attempt.turn_id)
        .bind(attempt.turn_count as i64)
        .bind(attempt.phase.to_string())
        .bind(attempt.started_at.to_rfc3339())
        .bind(attempt.finished_at.map(|d| d.to_rfc3339()))
        .bind(&attempt.error)
        .bind(attempt.input_tokens as i64)
        .bind(attempt.output_tokens as i64)
        .bind(attempt.total_tokens as i64)
        .bind(attempt.usage_estimated as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        Ok(())
    }

    pub async fn list_run_attempts(&self, task_id: &str) -> SymphonyResult<Vec<RunAttempt>> {
        let rows = sqlx::query(
            "SELECT * FROM run_attempts WHERE task_id = ?1 ORDER BY attempt_number DESC",
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        rows.into_iter().map(row_to_run_attempt).collect()
    }

    // =========================================================================
    // Agent Events (Bounded history with automatic pruning)
    // =========================================================================

    pub async fn record_event(&self, event: &AgentEvent) -> SymphonyResult<()> {
        let payload_json = event
            .payload
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let redacted_msg = redact_secrets(&event.message, &[]);

        sqlx::query(
            r#"
            INSERT INTO agent_events (
                id, task_id, issue_identifier, session_id, provider,
                event_type, phase, message, payload_json, timestamp
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(&event.id)
        .bind(&event.task_id)
        .bind(&event.issue_identifier)
        .bind(&event.session_id)
        .bind(event.provider.map(|p| p.as_str()))
        .bind(&event.event_type)
        .bind(event.phase.map(|p| p.to_string()))
        .bind(redacted_msg)
        .bind(payload_json)
        .bind(event.timestamp.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        Ok(())
    }

    pub async fn list_events(
        &self,
        task_id: Option<&str>,
        limit: u32,
    ) -> SymphonyResult<Vec<AgentEvent>> {
        let capped = limit.clamp(1, 500) as i64;
        let rows = if let Some(tid) = task_id {
            sqlx::query(
                "SELECT * FROM agent_events WHERE task_id = ?1 ORDER BY timestamp DESC LIMIT ?2",
            )
            .bind(tid)
            .bind(capped)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query("SELECT * FROM agent_events ORDER BY timestamp DESC LIMIT ?1")
                .bind(capped)
                .fetch_all(&self.pool)
                .await
        }
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        rows.into_iter().map(row_to_event).collect()
    }

    // =========================================================================
    // Approvals
    // =========================================================================

    pub async fn upsert_approval(&self, req: &ApprovalRequest) -> SymphonyResult<()> {
        let status_str = serde_json::to_value(req.status)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "pending".to_string());
        let decision_str = req
            .decision
            .and_then(|d| serde_json::to_value(d).ok())
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        sqlx::query(
            r#"
            INSERT INTO approval_requests (
                id, task_id, issue_identifier, session_id, provider, kind,
                title, description, command_or_diff, status, created_at, resolved_at, decision
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(id) DO UPDATE SET
                status = excluded.status,
                resolved_at = excluded.resolved_at,
                decision = excluded.decision
            "#,
        )
        .bind(&req.id)
        .bind(&req.task_id)
        .bind(&req.issue_identifier)
        .bind(&req.session_id)
        .bind(req.provider.as_str())
        .bind(&req.kind)
        .bind(&req.title)
        .bind(&req.description)
        .bind(&req.command_or_diff)
        .bind(status_str)
        .bind(req.created_at.to_rfc3339())
        .bind(req.resolved_at.map(|d| d.to_rfc3339()))
        .bind(decision_str)
        .execute(&self.pool)
        .await
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        Ok(())
    }

    pub async fn list_approvals(&self, only_pending: bool) -> SymphonyResult<Vec<ApprovalRequest>> {
        let rows = if only_pending {
            sqlx::query(
                "SELECT * FROM approval_requests WHERE status = 'pending' ORDER BY created_at DESC",
            )
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query("SELECT * FROM approval_requests ORDER BY created_at DESC LIMIT 100")
                .fetch_all(&self.pool)
                .await
        }
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        rows.into_iter().map(row_to_approval).collect()
    }

    // =========================================================================
    // Validation Results & Pull Request Reviews
    // =========================================================================

    pub async fn save_validation_result(&self, val: &ValidationResult) -> SymphonyResult<()> {
        sqlx::query(
            r#"
            INSERT INTO validation_results (
                id, task_id, attempt_number, command, exit_code,
                passed, stdout, stderr, duration_ms, executed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(&val.id)
        .bind(&val.task_id)
        .bind(val.attempt_number as i64)
        .bind(&val.command)
        .bind(val.exit_code as i64)
        .bind(val.passed as i64)
        .bind(redact_secrets(&val.stdout, &[]))
        .bind(redact_secrets(&val.stderr, &[]))
        .bind(val.duration_ms as i64)
        .bind(val.executed_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        Ok(())
    }

    pub async fn list_validation_results(
        &self,
        task_id: &str,
    ) -> SymphonyResult<Vec<ValidationResult>> {
        let rows = sqlx::query(
            "SELECT * FROM validation_results WHERE task_id = ?1 ORDER BY executed_at DESC",
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(ValidationResult {
                id: r.get("id"),
                task_id: r.get("task_id"),
                attempt_number: r.get::<i64, _>("attempt_number") as u32,
                command: r.get("command"),
                exit_code: r.get::<i64, _>("exit_code") as i32,
                passed: r.get::<i64, _>("passed") != 0,
                stdout: r.get("stdout"),
                stderr: r.get("stderr"),
                duration_ms: r.get::<i64, _>("duration_ms") as u64,
                executed_at: parse_dt(r.get("executed_at")),
            });
        }
        Ok(out)
    }

    pub async fn upsert_pr_review(&self, pr: &PullRequestReview) -> SymphonyResult<()> {
        let review_json =
            serde_json::to_string(pr).map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO pull_request_reviews (id, task_id, project_id, pr_number, review_json, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(id) DO UPDATE SET
                pr_number = excluded.pr_number,
                review_json = excluded.review_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&pr.id)
        .bind(&pr.task_id)
        .bind(&pr.project_id)
        .bind(pr.pr_number as i64)
        .bind(review_json)
        .bind(pr.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        Ok(())
    }

    pub async fn list_pr_reviews(
        &self,
        project_id: Option<&str>,
    ) -> SymphonyResult<Vec<PullRequestReview>> {
        let rows = if let Some(pid) = project_id {
            sqlx::query(
                "SELECT review_json FROM pull_request_reviews WHERE project_id = ?1 ORDER BY updated_at DESC",
            )
            .bind(pid)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?
        } else {
            sqlx::query("SELECT review_json FROM pull_request_reviews ORDER BY updated_at DESC")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| SymphonyError::StorageError(e.to_string()))?
        };

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let json_str: String = r.get("review_json");
            let mut pr: PullRequestReview = serde_json::from_str(&json_str)
                .map_err(|e| SymphonyError::StorageError(e.to_string()))?;
            pr.normalize_legacy();
            out.push(pr);
        }
        Ok(out)
    }

    pub async fn get_pr_review(&self, id: &str) -> SymphonyResult<Option<PullRequestReview>> {
        let row = sqlx::query("SELECT review_json FROM pull_request_reviews WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        match row {
            Some(r) => {
                let json_str: String = r.get("review_json");
                let mut pr: PullRequestReview = serde_json::from_str(&json_str)
                    .map_err(|e| SymphonyError::StorageError(e.to_string()))?;
                pr.normalize_legacy();
                Ok(Some(pr))
            }
            None => Ok(None),
        }
    }

    // =========================================================================
    // Provider Profiles & App Settings
    // =========================================================================

    pub async fn upsert_provider_profile(&self, profile: &ProviderProfile) -> SymphonyResult<()> {
        sqlx::query(
            r#"
            INSERT INTO provider_profiles (
                id, name, provider, model, command_override,
                approval_policy, sandbox_mode, credential_ref, is_default
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                provider = excluded.provider,
                model = excluded.model,
                command_override = excluded.command_override,
                approval_policy = excluded.approval_policy,
                sandbox_mode = excluded.sandbox_mode,
                credential_ref = excluded.credential_ref,
                is_default = excluded.is_default
            "#,
        )
        .bind(&profile.id)
        .bind(&profile.name)
        .bind(profile.provider.as_str())
        .bind(&profile.model)
        .bind(&profile.command_override)
        .bind(&profile.approval_policy)
        .bind(&profile.sandbox_mode)
        .bind(&profile.credential_ref)
        .bind(profile.is_default as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        Ok(())
    }

    pub async fn list_provider_profiles(&self) -> SymphonyResult<Vec<ProviderProfile>> {
        let rows =
            sqlx::query("SELECT * FROM provider_profiles ORDER BY is_default DESC, name ASC")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let prov_str: String = r.get("provider");
            out.push(ProviderProfile {
                id: r.get("id"),
                name: r.get("name"),
                provider: ProviderKind::from_str(&prov_str).unwrap_or(ProviderKind::Codex),
                model: r.get("model"),
                command_override: r.get("command_override"),
                approval_policy: r.get("approval_policy"),
                sandbox_mode: r.get("sandbox_mode"),
                credential_ref: r.get("credential_ref"),
                is_default: r.get::<i64, _>("is_default") != 0,
            });
        }
        Ok(out)
    }

    pub async fn get_settings(&self) -> SymphonyResult<AppSettings> {
        let row = sqlx::query("SELECT value_json FROM app_settings WHERE key = 'global'")
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;

        if let Some(r) = row {
            let json_str: String = r.get("value_json");
            serde_json::from_str(&json_str).map_err(|e| SymphonyError::StorageError(e.to_string()))
        } else {
            Ok(AppSettings::default())
        }
    }

    pub async fn save_settings(&self, settings: &AppSettings) -> SymphonyResult<()> {
        let value_json = serde_json::to_string(settings)
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;
        sqlx::query(
            r#"
            INSERT INTO app_settings (key, value_json, updated_at)
            VALUES ('global', ?1, ?2)
            ON CONFLICT(key) DO UPDATE SET
                value_json = excluded.value_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(value_json)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;
        Ok(())
    }

    /// Export a diagnostic bundle with automatic secret redaction (Section 11).
    pub async fn export_diagnostics(&self, extra_secrets: &[String]) -> SymphonyResult<String> {
        let mut settings = self.get_settings().await?;
        for val in settings.extra_env_vars.values_mut() {
            *val = "[REDACTED]".to_string();
        }
        let projects = self.list_projects().await?;
        let tasks = self.list_tasks(None).await?;
        let recent_events = self.list_events(None, 200).await?;

        let mut recent_attempts = Vec::new();
        let mut validations = Vec::new();
        for t in tasks.iter().take(25) {
            if let Ok(mut att) = self.list_run_attempts(&t.id).await {
                recent_attempts.append(&mut att);
            }
            if let Ok(mut vals) = self.list_validation_results(&t.id).await {
                validations.append(&mut vals);
            }
        }

        let bundle = DiagnosticBundle {
            exported_at: Utc::now(),
            settings,
            projects,
            tasks,
            recent_attempts,
            recent_events,
            validations,
        };

        let raw_json = serde_json::to_string_pretty(&bundle)
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;
        Ok(redact_secrets(&raw_json, extra_secrets))
    }
}

fn parse_dt(s: String) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn parse_opt_dt(s: Option<String>) -> Option<DateTime<Utc>> {
    s.and_then(|val| {
        DateTime::parse_from_rfc3339(&val)
            .map(|dt| dt.with_timezone(&Utc))
            .ok()
    })
}

fn row_to_project(r: sqlx::sqlite::SqliteRow) -> SymphonyResult<Project> {
    let labels_json: String = r.get("required_labels_json");
    let cmds_json: String = r.get("validation_commands_json");
    let prov_str: String = r.get("preferred_provider");

    Ok(Project {
        id: r.get("id"),
        name: r.get("name"),
        repo_path: r.get("repo_path"),
        default_branch: r.get("default_branch"),
        workspace_root: r.get("workspace_root"),
        workflow_path: r.get("workflow_path"),
        preferred_provider: ProviderKind::from_str(&prov_str).unwrap_or(ProviderKind::Codex),
        preferred_model: r.get("preferred_model"),
        tracker_kind: r.get("tracker_kind"),
        github_repo: r.get("github_repo"),
        linear_project_slug: r.get("linear_project_slug"),
        required_labels: serde_json::from_str(&labels_json).unwrap_or_default(),
        auto_dispatch: r.get::<i64, _>("auto_dispatch") != 0,
        max_concurrent_agents: r.get::<i64, _>("max_concurrent_agents") as u32,
        validation_commands: serde_json::from_str(&cmds_json).unwrap_or_default(),
        created_at: parse_dt(r.get("created_at")),
        updated_at: parse_dt(r.get("updated_at")),
    })
}

/// Apply pending schema migrations on `conn`.
///
/// * A database written by a newer build is refused: guessing at a newer schema could corrupt it.
/// * Before the first pending migration touches a database that already holds data, a consistent
///   snapshot is written next to it (`<db>.pre-migration-v<N>-<time>.bak`), so a migration that
///   goes wrong can always be undone by restoring that file.
/// * Every migration runs in its own transaction together with its `schema_migrations` row.
async fn apply_migrations(
    conn: &mut SqliteConnection,
    db_path: Option<&Path>,
    migrations: &[Migration],
    supported_version: i64,
) -> SymphonyResult<()> {
    let storage_err = |e: sqlx::Error| SymphonyError::StorageError(e.to_string());

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL
        )",
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_err)?;

    let applied: Vec<i64> = sqlx::query_scalar("SELECT version FROM schema_migrations")
        .fetch_all(&mut *conn)
        .await
        .map_err(storage_err)?;
    if let Some(newest) = applied.iter().copied().max() {
        if newest > supported_version {
            return Err(SymphonyError::StorageError(format!(
                "the database uses schema version {newest}, which is newer than the version \
                 {supported_version} this build supports; update Symphony Desktop or restore a \
                 backup made by this version"
            )));
        }
    }

    let pending: Vec<&Migration> = migrations
        .iter()
        .filter(|m| !applied.contains(&m.version))
        .collect();
    if pending.is_empty() {
        return Ok(());
    }

    let has_data: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'tasks')",
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_err)?;
    if let (true, Some(path)) = (has_data, db_path) {
        let target_version = pending
            .last()
            .map(|m| m.version)
            .unwrap_or(supported_version);
        let snapshot = path.with_file_name(format!(
            "{}.pre-migration-v{}-{}.bak",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "symphony.sqlite".into()),
            target_version,
            Utc::now().format("%Y%m%dT%H%M%SZ")
        ));
        backup_connection_to(conn, &snapshot).await?;
        tracing::info!(snapshot = %snapshot.display(), "wrote pre-migration database snapshot");
    }

    for migration in pending {
        let mut tx = conn.begin().await.map_err(storage_err)?;
        sqlx::raw_sql(migration.sql)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                SymphonyError::StorageError(format!(
                    "migration {:04}_{} failed and was rolled back: {e}",
                    migration.version, migration.name
                ))
            })?;
        sqlx::query(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
        )
        .bind(migration.version)
        .bind(migration.name)
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(storage_err)?;
        tx.commit().await.map_err(storage_err)?;
        tracing::info!(
            version = migration.version,
            name = migration.name,
            "applied schema migration"
        );
    }
    Ok(())
}

async fn backup_connection_to(
    conn: &mut SqliteConnection,
    destination: &Path,
) -> SymphonyResult<()> {
    if destination.exists() {
        return Err(SymphonyError::StorageError(format!(
            "backup destination {} already exists",
            destination.display()
        )));
    }
    if let Some(parent) = destination.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    sqlx::query("VACUUM INTO ?1")
        .bind(destination.to_string_lossy().into_owned())
        .execute(conn)
        .await
        .map_err(|e| SymphonyError::StorageError(format!("database backup failed: {e}")))?;
    Ok(())
}

/// The recorded thread usage baseline, `None` for rows without one (legacy tasks, tasks that never
/// ran a provider thread).
fn thread_usage_from_row(r: &sqlx::sqlite::SqliteRow) -> Option<ThreadUsageMark> {
    let thread_id: Option<String> = r.get("thread_usage_thread_id");
    let input: Option<i64> = r.get("thread_usage_input_tokens");
    let output: Option<i64> = r.get("thread_usage_output_tokens");
    let total: Option<i64> = r.get("thread_usage_total_tokens");
    Some(ThreadUsageMark {
        thread_id: thread_id?,
        usage: TokenUsage {
            input_tokens: input? as u64,
            output_tokens: output? as u64,
            total_tokens: total? as u64,
        },
    })
}

fn row_to_task(r: sqlx::sqlite::SqliteRow) -> SymphonyResult<Task> {
    let issue_json: String = r.get("issue_json");
    let issue: Issue = serde_json::from_str(&issue_json)
        .map_err(|e| SymphonyError::StorageError(e.to_string()))?;
    let status_str: String = r.get("status");
    let prov_str: String = r.get("provider");
    let phase_opt: Option<String> = r.get("current_phase");

    Ok(Task {
        id: r.get("id"),
        project_id: r.get("project_id"),
        issue,
        status: TaskStatus::from_str(&status_str).unwrap_or(TaskStatus::Ready),
        provider: ProviderKind::from_str(&prov_str).unwrap_or(ProviderKind::Codex),
        model: r.get("model"),
        workspace_path: r.get("workspace_path"),
        branch_name: r.get("branch_name"),
        attempt_count: r.get::<i64, _>("attempt_count") as u32,
        current_session_id: r.get("current_session_id"),
        current_thread_id: r.get("current_thread_id"),
        current_turn_id: r.get("current_turn_id"),
        current_phase: phase_opt
            .and_then(|p| serde_json::from_value(serde_json::Value::String(p)).ok()),
        pr_number: r.get::<Option<i64>, _>("pr_number").map(|n| n as u64),
        pr_url: r.get("pr_url"),
        input_tokens: r.get::<i64, _>("input_tokens") as u64,
        output_tokens: r.get::<i64, _>("output_tokens") as u64,
        total_tokens: r.get::<i64, _>("total_tokens") as u64,
        estimated_cost_usd: r.get("estimated_cost_usd"),
        thread_usage: thread_usage_from_row(&r),
        usage_estimated: r.get::<i64, _>("usage_estimated") != 0,
        duration_seconds: r.get("duration_seconds"),
        last_error: r.get("last_error"),
        next_retry_at: parse_opt_dt(r.get("next_retry_at")),
        created_at: parse_dt(r.get("created_at")),
        updated_at: parse_dt(r.get("updated_at")),
        started_at: parse_opt_dt(r.get("started_at")),
        completed_at: parse_opt_dt(r.get("completed_at")),
    })
}

fn row_to_run_attempt(r: sqlx::sqlite::SqliteRow) -> SymphonyResult<RunAttempt> {
    let prov_str: String = r.get("provider");
    let phase_str: String = r.get("phase");
    let phase = serde_json::from_value(serde_json::Value::String(phase_str))
        .unwrap_or(RunAttemptPhase::Failed);

    Ok(RunAttempt {
        id: r.get("id"),
        task_id: r.get("task_id"),
        issue_id: r.get("issue_id"),
        issue_identifier: r.get("issue_identifier"),
        attempt_number: r.get::<i64, _>("attempt_number") as u32,
        provider: ProviderKind::from_str(&prov_str).unwrap_or(ProviderKind::Codex),
        workspace_path: r.get("workspace_path"),
        branch_name: r.get("branch_name"),
        session_id: r.get("session_id"),
        thread_id: r.get("thread_id"),
        turn_id: r.get("turn_id"),
        turn_count: r.get::<i64, _>("turn_count") as u32,
        phase,
        started_at: parse_dt(r.get("started_at")),
        finished_at: parse_opt_dt(r.get("finished_at")),
        error: r.get("error"),
        input_tokens: r.get::<i64, _>("input_tokens") as u64,
        output_tokens: r.get::<i64, _>("output_tokens") as u64,
        total_tokens: r.get::<i64, _>("total_tokens") as u64,
        usage_estimated: r.get::<i64, _>("usage_estimated") != 0,
    })
}

fn row_to_event(r: sqlx::sqlite::SqliteRow) -> SymphonyResult<AgentEvent> {
    let prov_opt: Option<String> = r.get("provider");
    let phase_opt: Option<String> = r.get("phase");
    let payload_opt: Option<String> = r.get("payload_json");

    Ok(AgentEvent {
        id: r.get("id"),
        task_id: r.get("task_id"),
        issue_identifier: r.get("issue_identifier"),
        session_id: r.get("session_id"),
        provider: prov_opt.and_then(|s| ProviderKind::from_str(&s).ok()),
        event_type: r.get("event_type"),
        phase: phase_opt.and_then(|p| serde_json::from_value(serde_json::Value::String(p)).ok()),
        message: r.get("message"),
        payload: payload_opt.and_then(|s| serde_json::from_str(&s).ok()),
        timestamp: parse_dt(r.get("timestamp")),
    })
}

fn row_to_approval(r: sqlx::sqlite::SqliteRow) -> SymphonyResult<ApprovalRequest> {
    let prov_str: String = r.get("provider");
    let status_str: String = r.get("status");
    let decision_opt: Option<String> = r.get("decision");

    let status: ApprovalStatus = serde_json::from_value(serde_json::Value::String(status_str))
        .unwrap_or(ApprovalStatus::Pending);
    let decision: Option<ApprovalDecision> =
        decision_opt.and_then(|d| serde_json::from_value(serde_json::Value::String(d)).ok());

    Ok(ApprovalRequest {
        id: r.get("id"),
        task_id: r.get("task_id"),
        issue_identifier: r.get("issue_identifier"),
        session_id: r.get("session_id"),
        provider: ProviderKind::from_str(&prov_str).unwrap_or(ProviderKind::Codex),
        kind: r.get("kind"),
        title: r.get("title"),
        description: r.get("description"),
        command_or_diff: r.get("command_or_diff"),
        status,
        created_at: parse_dt(r.get("created_at")),
        resolved_at: parse_opt_dt(r.get("resolved_at")),
        decision,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_storage_crud_and_crash_recovery() {
        let storage = Storage::connect_in_memory().await.unwrap();
        let now = Utc::now();

        let project = Project {
            id: "proj-1".into(),
            name: "Symphony Test".into(),
            repo_path: "C:/repos/symphony".into(),
            default_branch: "main".into(),
            workspace_root: "C:/repos/workspaces".into(),
            workflow_path: None,
            preferred_provider: ProviderKind::Codex,
            preferred_model: Some("gpt-5-codex".into()),
            tracker_kind: "manual".into(),
            github_repo: None,
            linear_project_slug: None,
            required_labels: vec!["symphony".into()],
            auto_dispatch: true,
            max_concurrent_agents: 2,
            validation_commands: vec!["cargo test".into()],
            created_at: now,
            updated_at: now,
        };
        storage.upsert_project(&project).await.unwrap();

        let mut task = Task::new_manual(
            project.id.clone(),
            "SYM-1".into(),
            "Fix bug".into(),
            Some("Details".into()),
            Some(1),
            ProviderKind::Codex,
            None,
            vec!["symphony".into()],
        );
        task.status = TaskStatus::Running;
        storage.upsert_task(&task).await.unwrap();

        // Simulate crash recovery
        let recovered = storage.recover_stale_running_tasks().await.unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].status, TaskStatus::Queued);
        assert!(recovered[0]
            .last_error
            .as_ref()
            .unwrap()
            .contains("Interrupted"));
    }
}

#[cfg(test)]
mod migration_tests {
    use super::*;

    #[tokio::test]
    async fn a_failing_migration_rolls_back_completely() {
        let storage = Storage::connect_in_memory().await.unwrap();
        let broken = [Migration {
            version: 99,
            name: "broken",
            // The first statement succeeds, the second fails: neither may persist.
            sql: "CREATE TABLE half_applied (id INTEGER); INSERT INTO missing_table VALUES (1);",
        }];
        let mut conn = storage.pool().acquire().await.unwrap();
        let err = apply_migrations(&mut conn, None, &broken, 99)
            .await
            .expect_err("migration must fail");
        drop(conn);
        assert!(err.to_string().contains("rolled back"), "{err}");

        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = 'half_applied')",
        )
        .fetch_one(storage.pool())
        .await
        .unwrap();
        assert!(!exists, "partial DDL must be rolled back");
        let recorded: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM schema_migrations WHERE version = 99")
                .fetch_one(storage.pool())
                .await
                .unwrap();
        assert_eq!(recorded, 0);
    }
}
