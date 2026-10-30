//! The Knowledge Base database (`knowledge.db`): index runs with their graph, the commit-impact
//! cache, AI analyses and chat history.
//!
//! It is a separate SQLite file with its own migration table. Its content is derived from the
//! repositories, so it is excluded from the main database's backups and schema chain, and deleting
//! it only costs a re-index.

use crate::model::{
    ChatMessage, ChatRole, CommitAnalysis, CommitAnalysisResult, CommitImpact, Confidence,
    EvidenceRef, GraphEdge, GraphNode, IndexRunSummary, IndexStatus, JobStatus, LanguageStat,
    NodeKind,
};
use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use symphony_core::{ProviderKind, SymphonyError, SymphonyResult};

const SCHEMA_VERSION: i64 = 1;
const MIGRATIONS: &[(i64, &str, &str)] =
    &[(1, "init", include_str!("../migrations/0001_init.sql"))];
/// Rows per multi-row INSERT (SQLite's variable limit is the constraint).
const INSERT_CHUNK: usize = 400;

fn db_err(e: sqlx::Error) -> SymphonyError {
    SymphonyError::StorageError(e.to_string())
}

fn json_err(e: serde_json::Error) -> SymphonyError {
    SymphonyError::JsonError(e.to_string())
}

fn parse_time(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_default()
}

fn provider_from_str(value: &str) -> ProviderKind {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .unwrap_or(ProviderKind::Simulated)
}

#[derive(Clone)]
pub struct KnowledgeStore {
    pool: SqlitePool,
}

impl KnowledgeStore {
    pub async fn open(path: &Path) -> SymphonyResult<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(10))
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .map_err(db_err)?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    pub async fn open_in_memory() -> SymphonyResult<Self> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .map_err(db_err)?
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(db_err)?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> SymphonyResult<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS kb_schema_migrations (
                version INTEGER PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                applied_at TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        let applied: Vec<i64> = sqlx::query_scalar("SELECT version FROM kb_schema_migrations")
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        if let Some(newest) = applied.iter().copied().max() {
            if newest > SCHEMA_VERSION {
                return Err(SymphonyError::StorageError(format!(
                    "the knowledge index uses schema version {newest}, newer than the version \
                     {SCHEMA_VERSION} this build supports; delete knowledge.db to rebuild it"
                )));
            }
        }
        for (version, name, sql) in MIGRATIONS.iter().filter(|(v, _, _)| !applied.contains(v)) {
            let mut tx = self.pool.begin().await.map_err(db_err)?;
            sqlx::raw_sql(*sql).execute(&mut *tx).await.map_err(|e| {
                SymphonyError::StorageError(format!(
                    "knowledge migration {version:04}_{name} failed: {e}"
                ))
            })?;
            sqlx::query(
                "INSERT INTO kb_schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
            )
            .bind(version)
            .bind(name)
            .bind(Utc::now().to_rfc3339())
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
            tx.commit().await.map_err(db_err)?;
        }
        Ok(())
    }

    // -- index runs ------------------------------------------------------------------------

    /// Runs left `running` by a previous process cannot be running now.
    pub async fn fail_interrupted_runs(&self) -> SymphonyResult<u64> {
        let result = sqlx::query(
            "UPDATE kb_index_runs SET status = 'failed', finished_at = ?1,
             error = 'Interrupted: the application stopped while indexing' WHERE status = 'running'",
        )
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        sqlx::query(
            "UPDATE kb_analyses SET status = 'failed', finished_at = ?1,
             error = 'Interrupted: the application stopped during the analysis' WHERE status = 'running'",
        )
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        sqlx::query(
            "UPDATE kb_chat_messages SET status = 'failed',
             error = 'Interrupted: the application stopped before the answer was complete' WHERE status = 'running'",
        )
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(result.rows_affected())
    }

    pub async fn create_run(
        &self,
        project_id: &str,
        ref_name: &str,
        commit_sha: &str,
        engine: &str,
        engine_version: &str,
    ) -> SymphonyResult<i64> {
        let result = sqlx::query(
            "INSERT INTO kb_index_runs (project_id, ref_name, commit_sha, engine, engine_version, status, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6)",
        )
        .bind(project_id)
        .bind(ref_name)
        .bind(commit_sha)
        .bind(engine)
        .bind(engine_version)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(result.last_insert_rowid())
    }

    pub async fn fail_run(&self, run_id: i64, error: &str) -> SymphonyResult<()> {
        sqlx::query(
            "UPDATE kb_index_runs SET status = 'failed', error = ?2, finished_at = ?3,
             duration_ms = CAST((julianday(?3) - julianday(started_at)) * 86400000 AS INTEGER) WHERE id = ?1",
        )
        .bind(run_id)
        .bind(error)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// Store the graph of a run and mark it completed, in one transaction: a run is never
    /// `completed` with a partial graph.
    pub async fn complete_run(
        &self,
        run_id: i64,
        nodes: &[GraphNode],
        edges: &[GraphEdge],
        file_count: u32,
        languages: &[LanguageStat],
    ) -> SymphonyResult<()> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        for chunk in nodes.chunks(INSERT_CHUNK) {
            let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
                "INSERT INTO kb_nodes (run_id, node_id, label, kind, file, line, module, degree) ",
            );
            qb.push_values(chunk, |mut b, n| {
                b.push_bind(run_id)
                    .push_bind(&n.id)
                    .push_bind(&n.label)
                    .push_bind(n.kind.as_str())
                    .push_bind(&n.file)
                    .push_bind(n.line.map(i64::from))
                    .push_bind(&n.module)
                    .push_bind(i64::from(n.degree));
            });
            qb.build().execute(&mut *tx).await.map_err(db_err)?;
        }
        for chunk in nodes
            .iter()
            .filter(|n| n.kind != NodeKind::Dependency || n.file.is_none())
            .collect::<Vec<_>>()
            .chunks(INSERT_CHUNK)
        {
            let mut qb: QueryBuilder<Sqlite> =
                QueryBuilder::new("INSERT INTO kb_nodes_fts (label, file, run_id, node_id) ");
            qb.push_values(chunk, |mut b, n| {
                b.push_bind(&n.label)
                    .push_bind(n.file.as_deref().unwrap_or(""))
                    .push_bind(run_id)
                    .push_bind(&n.id);
            });
            qb.build().execute(&mut *tx).await.map_err(db_err)?;
        }
        for chunk in edges.chunks(INSERT_CHUNK) {
            let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
                "INSERT INTO kb_edges (run_id, source, target, relation, confidence, line) ",
            );
            qb.push_values(chunk, |mut b, e| {
                b.push_bind(run_id)
                    .push_bind(&e.source)
                    .push_bind(&e.target)
                    .push_bind(&e.relation)
                    .push_bind(e.confidence.as_str())
                    .push_bind(e.line.map(i64::from));
            });
            qb.build().execute(&mut *tx).await.map_err(db_err)?;
        }
        sqlx::query(
            "UPDATE kb_index_runs SET status = 'completed', node_count = ?2, edge_count = ?3,
             file_count = ?4, languages_json = ?5, error = NULL, finished_at = ?6,
             duration_ms = CAST((julianday(?6) - julianday(started_at)) * 86400000 AS INTEGER) WHERE id = ?1",
        )
        .bind(run_id)
        .bind(nodes.len() as i64)
        .bind(edges.len() as i64)
        .bind(i64::from(file_count))
        .bind(serde_json::to_string(languages).map_err(json_err)?)
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
        tx.commit().await.map_err(db_err)
    }

    fn run_from_row(row: &sqlx::sqlite::SqliteRow) -> IndexRunSummary {
        let languages: Vec<LanguageStat> = row
            .try_get::<String, _>("languages_json")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        IndexRunSummary {
            id: row.get("id"),
            project_id: row.get("project_id"),
            ref_name: row.get("ref_name"),
            commit_sha: row.get("commit_sha"),
            engine: row.get("engine"),
            engine_version: row.get("engine_version"),
            status: IndexStatus::parse(row.get::<String, _>("status").as_str()),
            node_count: row.get::<i64, _>("node_count") as u32,
            edge_count: row.get::<i64, _>("edge_count") as u32,
            file_count: row.get::<i64, _>("file_count") as u32,
            languages,
            error: row.get("error"),
            started_at: parse_time(&row.get::<String, _>("started_at")),
            finished_at: row
                .get::<Option<String>, _>("finished_at")
                .map(|t| parse_time(&t)),
            duration_ms: row
                .get::<Option<i64>, _>("duration_ms")
                .map(|d| d.max(0) as u64),
        }
    }

    pub async fn run(&self, run_id: i64) -> SymphonyResult<Option<IndexRunSummary>> {
        let row = sqlx::query("SELECT * FROM kb_index_runs WHERE id = ?1")
            .bind(run_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(Self::run_from_row))
    }

    pub async fn latest_completed_run(
        &self,
        project_id: &str,
    ) -> SymphonyResult<Option<IndexRunSummary>> {
        let row = sqlx::query(
            "SELECT * FROM kb_index_runs WHERE project_id = ?1 AND status = 'completed' ORDER BY id DESC LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.as_ref().map(Self::run_from_row))
    }

    /// The newest run that is still running, or failed after the newest completed one.
    pub async fn active_or_failed_run(
        &self,
        project_id: &str,
    ) -> SymphonyResult<Option<IndexRunSummary>> {
        let row = sqlx::query(
            "SELECT * FROM kb_index_runs WHERE project_id = ?1 AND status != 'completed'
             AND id > COALESCE((SELECT MAX(id) FROM kb_index_runs WHERE project_id = ?1 AND status = 'completed'), 0)
             ORDER BY id DESC LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.as_ref().map(Self::run_from_row))
    }

    /// Keep the newest `keep` completed runs of a project and drop the rest (and failed ones).
    pub async fn prune_runs(&self, project_id: &str, keep: i64) -> SymphonyResult<()> {
        let stale: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM kb_index_runs WHERE project_id = ?1 AND status != 'running' AND id NOT IN
             (SELECT id FROM kb_index_runs WHERE project_id = ?1 AND status = 'completed' ORDER BY id DESC LIMIT ?2)",
        )
        .bind(project_id)
        .bind(keep)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        for id in stale {
            let mut tx = self.pool.begin().await.map_err(db_err)?;
            sqlx::query("DELETE FROM kb_nodes_fts WHERE run_id = ?1")
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
            sqlx::query("DELETE FROM kb_index_runs WHERE id = ?1")
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
            tx.commit().await.map_err(db_err)?;
        }
        Ok(())
    }

    // -- graph -----------------------------------------------------------------------------

    pub async fn load_graph(
        &self,
        run_id: i64,
    ) -> SymphonyResult<(Vec<GraphNode>, Vec<GraphEdge>)> {
        let node_rows = sqlx::query(
            "SELECT node_id, label, kind, file, line, module, degree FROM kb_nodes WHERE run_id = ?1",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        let nodes = node_rows
            .iter()
            .map(|r| GraphNode {
                id: r.get("node_id"),
                label: r.get("label"),
                kind: NodeKind::parse(r.get::<String, _>("kind").as_str()),
                file: r.get("file"),
                line: r.get::<Option<i64>, _>("line").map(|l| l as u32),
                module: r.get("module"),
                degree: r.get::<i64, _>("degree") as u32,
                members: 1,
            })
            .collect();
        let edge_rows = sqlx::query(
            "SELECT source, target, relation, confidence, line FROM kb_edges WHERE run_id = ?1",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        let edges = edge_rows
            .iter()
            .map(|r| GraphEdge {
                source: r.get("source"),
                target: r.get("target"),
                relation: r.get("relation"),
                confidence: Confidence::parse(r.get::<String, _>("confidence").as_str()),
                line: r.get::<Option<i64>, _>("line").map(|l| l as u32),
                weight: 1,
            })
            .collect();
        Ok((nodes, edges))
    }

    /// Node ids matching `query` (prefix match per word, then substring), best candidates first.
    pub async fn search_node_ids(
        &self,
        run_id: i64,
        query: &str,
        limit: usize,
    ) -> SymphonyResult<Vec<String>> {
        let tokens: Vec<String> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_lowercase())
            .collect();
        let mut ids: Vec<String> = Vec::new();
        if !tokens.is_empty() {
            let expression = tokens
                .iter()
                .map(|t| format!("\"{}\"*", t.replace('"', "")))
                .collect::<Vec<_>>()
                .join(" ");
            let rows: Vec<String> = sqlx::query_scalar(
                "SELECT node_id FROM kb_nodes_fts WHERE kb_nodes_fts MATCH ?1 AND run_id = ?2 ORDER BY rank LIMIT ?3",
            )
            .bind(expression)
            .bind(run_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
            ids.extend(rows);
        }
        let needle = query.trim();
        if ids.len() < limit && !needle.is_empty() {
            let pattern = format!(
                "%{}%",
                needle
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            );
            let rows: Vec<String> = sqlx::query_scalar(
                "SELECT node_id FROM kb_nodes WHERE run_id = ?1 AND kind != 'dependency'
                 AND (label LIKE ?2 ESCAPE '\\' OR file LIKE ?2 ESCAPE '\\') LIMIT ?3",
            )
            .bind(run_id)
            .bind(pattern)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
            for id in rows {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        ids.truncate(limit);
        Ok(ids)
    }

    // -- commit impact cache ----------------------------------------------------------------

    pub async fn cached_impact(
        &self,
        project_id: &str,
        sha: &str,
        run_id: i64,
        depth: u32,
    ) -> SymphonyResult<Option<CommitImpact>> {
        let payload: Option<String> = sqlx::query_scalar(
            "SELECT payload_json FROM kb_commit_impact WHERE project_id = ?1 AND sha = ?2 AND run_id = ?3 AND depth = ?4",
        )
        .bind(project_id)
        .bind(sha)
        .bind(run_id)
        .bind(i64::from(depth))
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(payload.and_then(|p| serde_json::from_str(&p).ok()))
    }

    pub async fn cache_impact(
        &self,
        project_id: &str,
        run_id: i64,
        depth: u32,
        impact: &CommitImpact,
    ) -> SymphonyResult<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO kb_commit_impact (project_id, sha, run_id, depth, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(project_id)
        .bind(&impact.sha)
        .bind(run_id)
        .bind(i64::from(depth))
        .bind(serde_json::to_string(impact).map_err(json_err)?)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    // -- analyses --------------------------------------------------------------------------

    fn analysis_from_row(row: &sqlx::sqlite::SqliteRow) -> CommitAnalysis {
        let result: Option<CommitAnalysisResult> = row
            .get::<Option<String>, _>("result_json")
            .and_then(|s| serde_json::from_str(&s).ok());
        let evidence: Vec<EvidenceRef> =
            serde_json::from_str(&row.get::<String, _>("evidence_json")).unwrap_or_default();
        let model: String = row.get("model");
        CommitAnalysis {
            id: row.get("id"),
            project_id: row.get("project_id"),
            sha: row.get("sha"),
            provider: provider_from_str(&row.get::<String, _>("provider")),
            model: (!model.is_empty()).then_some(model),
            prompt_version: row.get::<i64, _>("prompt_version") as u32,
            status: JobStatus::parse(row.get::<String, _>("status").as_str()),
            result,
            evidence,
            dropped_findings: row.get::<i64, _>("dropped_findings") as u32,
            error: row.get("error"),
            input_tokens: row.get::<i64, _>("input_tokens").max(0) as u64,
            output_tokens: row.get::<i64, _>("output_tokens").max(0) as u64,
            created_at: parse_time(&row.get::<String, _>("created_at")),
            finished_at: row
                .get::<Option<String>, _>("finished_at")
                .map(|t| parse_time(&t)),
        }
    }

    pub async fn start_analysis(
        &self,
        project_id: &str,
        sha: &str,
        provider: ProviderKind,
        model: Option<&str>,
        prompt_version: u32,
    ) -> SymphonyResult<i64> {
        let result = sqlx::query(
            "INSERT INTO kb_analyses (project_id, sha, provider, model, prompt_version, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6)",
        )
        .bind(project_id)
        .bind(sha)
        .bind(provider.as_str())
        .bind(model.unwrap_or(""))
        .bind(i64::from(prompt_version))
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(result.last_insert_rowid())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn finish_analysis(
        &self,
        id: i64,
        result: &CommitAnalysisResult,
        evidence: &[EvidenceRef],
        dropped: u32,
        input_tokens: u64,
        output_tokens: u64,
    ) -> SymphonyResult<()> {
        sqlx::query(
            "UPDATE kb_analyses SET status = 'completed', result_json = ?2, evidence_json = ?3,
             dropped_findings = ?4, input_tokens = ?5, output_tokens = ?6, error = NULL, finished_at = ?7 WHERE id = ?1",
        )
        .bind(id)
        .bind(serde_json::to_string(result).map_err(json_err)?)
        .bind(serde_json::to_string(evidence).map_err(json_err)?)
        .bind(i64::from(dropped))
        .bind(input_tokens as i64)
        .bind(output_tokens as i64)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    pub async fn fail_analysis(&self, id: i64, error: &str) -> SymphonyResult<()> {
        sqlx::query(
            "UPDATE kb_analyses SET status = 'failed', error = ?2, finished_at = ?3 WHERE id = ?1",
        )
        .bind(id)
        .bind(error)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// The newest analysis of a commit (any status).
    pub async fn latest_analysis(
        &self,
        project_id: &str,
        sha: &str,
    ) -> SymphonyResult<Option<CommitAnalysis>> {
        let row = sqlx::query(
            "SELECT * FROM kb_analyses WHERE project_id = ?1 AND sha = ?2 ORDER BY id DESC LIMIT 1",
        )
        .bind(project_id)
        .bind(sha)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.as_ref().map(Self::analysis_from_row))
    }

    /// A completed analysis for exactly this configuration, if one is cached.
    pub async fn cached_analysis(
        &self,
        project_id: &str,
        sha: &str,
        provider: ProviderKind,
        model: Option<&str>,
        prompt_version: u32,
    ) -> SymphonyResult<Option<CommitAnalysis>> {
        let row = sqlx::query(
            "SELECT * FROM kb_analyses WHERE project_id = ?1 AND sha = ?2 AND provider = ?3 AND model = ?4
             AND prompt_version = ?5 AND status = 'completed' ORDER BY id DESC LIMIT 1",
        )
        .bind(project_id)
        .bind(sha)
        .bind(provider.as_str())
        .bind(model.unwrap_or(""))
        .bind(i64::from(prompt_version))
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.as_ref().map(Self::analysis_from_row))
    }

    pub async fn running_analysis(
        &self,
        project_id: &str,
        sha: &str,
    ) -> SymphonyResult<Option<i64>> {
        sqlx::query_scalar(
            "SELECT id FROM kb_analyses WHERE project_id = ?1 AND sha = ?2 AND status = 'running' LIMIT 1",
        )
        .bind(project_id)
        .bind(sha)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)
    }

    // -- chat ------------------------------------------------------------------------------

    fn message_from_row(row: &sqlx::sqlite::SqliteRow) -> ChatMessage {
        ChatMessage {
            id: row.get("id"),
            project_id: row.get("project_id"),
            role: ChatRole::parse(row.get::<String, _>("role").as_str()),
            content: row.get("content"),
            evidence: serde_json::from_str(&row.get::<String, _>("evidence_json"))
                .unwrap_or_default(),
            status: JobStatus::parse(row.get::<String, _>("status").as_str()),
            provider: row
                .get::<Option<String>, _>("provider")
                .map(|p| provider_from_str(&p)),
            model: row.get("model"),
            error: row.get("error"),
            created_at: parse_time(&row.get::<String, _>("created_at")),
        }
    }

    pub async fn add_message(
        &self,
        project_id: &str,
        role: ChatRole,
        content: &str,
        status: JobStatus,
        provider: Option<ProviderKind>,
        model: Option<&str>,
    ) -> SymphonyResult<i64> {
        let result = sqlx::query(
            "INSERT INTO kb_chat_messages (project_id, role, content, status, provider, model, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(project_id)
        .bind(role.as_str())
        .bind(content)
        .bind(status.as_str())
        .bind(provider.map(|p| p.as_str()))
        .bind(model)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(result.last_insert_rowid())
    }

    pub async fn finish_message(
        &self,
        id: i64,
        content: &str,
        evidence: &[EvidenceRef],
        status: JobStatus,
        error: Option<&str>,
    ) -> SymphonyResult<()> {
        sqlx::query(
            "UPDATE kb_chat_messages SET content = ?2, evidence_json = ?3, status = ?4, error = ?5 WHERE id = ?1",
        )
        .bind(id)
        .bind(content)
        .bind(serde_json::to_string(evidence).map_err(json_err)?)
        .bind(status.as_str())
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// The newest `limit` messages of a project, oldest first.
    pub async fn messages(&self, project_id: &str, limit: i64) -> SymphonyResult<Vec<ChatMessage>> {
        let rows = sqlx::query(
            "SELECT * FROM (SELECT * FROM kb_chat_messages WHERE project_id = ?1 ORDER BY id DESC LIMIT ?2) ORDER BY id ASC",
        )
        .bind(project_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.iter().map(Self::message_from_row).collect())
    }

    pub async fn has_running_message(&self, project_id: &str) -> SymphonyResult<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM kb_chat_messages WHERE project_id = ?1 AND status = 'running'",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(count > 0)
    }

    pub async fn clear_messages(&self, project_id: &str) -> SymphonyResult<()> {
        sqlx::query("DELETE FROM kb_chat_messages WHERE project_id = ?1 AND status != 'running'")
            .bind(project_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    /// Remove everything stored for a project (when the project is deleted).
    pub async fn delete_project(&self, project_id: &str) -> SymphonyResult<()> {
        let runs: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM kb_index_runs WHERE project_id = ?1")
                .bind(project_id)
                .fetch_all(&self.pool)
                .await
                .map_err(db_err)?;
        for id in runs {
            sqlx::query("DELETE FROM kb_nodes_fts WHERE run_id = ?1")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(db_err)?;
        }
        for sql in [
            "DELETE FROM kb_index_runs WHERE project_id = ?1",
            "DELETE FROM kb_analyses WHERE project_id = ?1",
            "DELETE FROM kb_chat_messages WHERE project_id = ?1",
        ] {
            sqlx::query(sql)
                .bind(project_id)
                .execute(&self.pool)
                .await
                .map_err(db_err)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, label: &str, kind: NodeKind, file: &str, line: u32) -> GraphNode {
        GraphNode {
            id: id.into(),
            label: label.into(),
            kind,
            file: Some(file.into()),
            line: Some(line),
            module: Some("crates/a".into()),
            degree: 0,
            members: 1,
        }
    }

    fn edge(source: &str, target: &str, relation: &str) -> GraphEdge {
        GraphEdge {
            source: source.into(),
            target: target.into(),
            relation: relation.into(),
            confidence: Confidence::Extracted,
            line: Some(3),
            weight: 1,
        }
    }

    #[tokio::test]
    async fn a_run_round_trips_its_graph_and_is_searchable() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let run = store
            .create_run("p1", "main", &"a".repeat(40), "graphify", "0.9.97")
            .await
            .unwrap();
        assert!(store.latest_completed_run("p1").await.unwrap().is_none());
        let nodes = vec![
            node("f", "lib.rs", NodeKind::File, "crates/a/src/lib.rs", 1),
            node(
                "d",
                ".is_worktree_dirty()",
                NodeKind::Method,
                "crates/a/src/lib.rs",
                601,
            ),
            node(
                "w",
                "WorkspaceManager",
                NodeKind::Type,
                "crates/a/src/lib.rs",
                400,
            ),
        ];
        let edges = vec![edge("w", "d", "method"), edge("f", "w", "contains")];
        store
            .complete_run(
                run,
                &nodes,
                &edges,
                1,
                &[LanguageStat {
                    language: "Rust".into(),
                    files: 1,
                }],
            )
            .await
            .unwrap();

        let summary = store.latest_completed_run("p1").await.unwrap().unwrap();
        assert_eq!(summary.node_count, 3);
        assert_eq!(summary.edge_count, 2);
        assert_eq!(summary.languages[0].language, "Rust");
        let (loaded_nodes, loaded_edges) = store.load_graph(run).await.unwrap();
        assert_eq!(loaded_nodes.len(), 3);
        assert_eq!(loaded_edges.len(), 2);
        assert_eq!(loaded_edges[0].confidence, Confidence::Extracted);

        // Word prefix, a part of a snake_case identifier and a substring all find the method.
        for query in ["worktree", "is_worktree_dirty", "dirt", "WorkspaceMan"] {
            let ids = store.search_node_ids(run, query, 10).await.unwrap();
            assert!(!ids.is_empty(), "{query} should match something");
        }
        assert!(store
            .search_node_ids(run, "worktree", 10)
            .await
            .unwrap()
            .contains(&"d".to_string()));
        assert!(store
            .search_node_ids(run, "nothingmatches", 10)
            .await
            .unwrap()
            .is_empty());
        // Quotes and FTS operators in a query are data, not syntax.
        assert!(store
            .search_node_ids(run, "\" OR 1 NEAR(", 10)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn only_completed_runs_count_and_old_runs_are_pruned() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let sha = "b".repeat(40);
        let mut completed = Vec::new();
        for _ in 0..3 {
            let run = store
                .create_run("p1", "main", &sha, "graphify", "1")
                .await
                .unwrap();
            store
                .complete_run(
                    run,
                    &[node("n", "x", NodeKind::Function, "a.rs", 1)],
                    &[],
                    1,
                    &[],
                )
                .await
                .unwrap();
            completed.push(run);
        }
        let failed = store
            .create_run("p1", "main", &sha, "graphify", "1")
            .await
            .unwrap();
        assert_eq!(
            store
                .active_or_failed_run("p1")
                .await
                .unwrap()
                .unwrap()
                .status,
            IndexStatus::Running
        );
        store.fail_run(failed, "boom").await.unwrap();
        let active = store.active_or_failed_run("p1").await.unwrap().unwrap();
        assert_eq!(active.status, IndexStatus::Failed);
        assert_eq!(active.error.as_deref(), Some("boom"));
        assert_eq!(
            store.latest_completed_run("p1").await.unwrap().unwrap().id,
            completed[2]
        );

        store.prune_runs("p1", 1).await.unwrap();
        assert!(store.run(completed[0]).await.unwrap().is_none());
        assert!(store.run(completed[1]).await.unwrap().is_none());
        assert!(store.run(completed[2]).await.unwrap().is_some());
        assert!(store.run(failed).await.unwrap().is_none());
        // Pruned graphs leave nothing behind.
        assert!(store.load_graph(completed[0]).await.unwrap().0.is_empty());

        // A run that was still running when the process died is failed on the next start.
        let stuck = store
            .create_run("p2", "main", &sha, "graphify", "1")
            .await
            .unwrap();
        store.fail_interrupted_runs().await.unwrap();
        assert_eq!(
            store.run(stuck).await.unwrap().unwrap().status,
            IndexStatus::Failed
        );
    }

    #[tokio::test]
    async fn analyses_are_cached_by_configuration_and_chat_keeps_order() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let sha = "c".repeat(40);
        let id = store
            .start_analysis("p1", &sha, ProviderKind::Codex, Some("m"), 1)
            .await
            .unwrap();
        assert_eq!(store.running_analysis("p1", &sha).await.unwrap(), Some(id));
        assert!(store
            .cached_analysis("p1", &sha, ProviderKind::Codex, Some("m"), 1)
            .await
            .unwrap()
            .is_none());
        let result = CommitAnalysisResult {
            summary: "s".into(),
            what_changed: vec![],
            risks: vec![],
            security: vec![],
            regressions: vec![],
            tests: vec![],
            recommendations: vec![],
        };
        store
            .finish_analysis(id, &result, &[], 2, 10, 20)
            .await
            .unwrap();
        let cached = store
            .cached_analysis("p1", &sha, ProviderKind::Codex, Some("m"), 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cached.dropped_findings, 2);
        assert_eq!(cached.output_tokens, 20);
        assert!(store
            .cached_analysis("p1", &sha, ProviderKind::Codex, Some("m"), 2)
            .await
            .unwrap()
            .is_none());
        assert!(store
            .cached_analysis("p1", &sha, ProviderKind::ClaudeCode, Some("m"), 1)
            .await
            .unwrap()
            .is_none());

        let q = store
            .add_message(
                "p1",
                ChatRole::User,
                "why?",
                JobStatus::Completed,
                None,
                None,
            )
            .await
            .unwrap();
        let a = store
            .add_message(
                "p1",
                ChatRole::Assistant,
                "",
                JobStatus::Running,
                Some(ProviderKind::Codex),
                None,
            )
            .await
            .unwrap();
        assert!(store.has_running_message("p1").await.unwrap());
        store
            .finish_message(a, "because", &[], JobStatus::Completed, None)
            .await
            .unwrap();
        let messages = store.messages("p1", 10).await.unwrap();
        assert_eq!(
            messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![q, a]
        );
        assert_eq!(messages[1].content, "because");
        store.delete_project("p1").await.unwrap();
        assert!(store.messages("p1", 10).await.unwrap().is_empty());
    }
}
