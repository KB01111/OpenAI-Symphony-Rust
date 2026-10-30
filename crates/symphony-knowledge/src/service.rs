//! `KnowledgeService`: everything the HTTP API and the orchestrator need from the Knowledge Base.

use crate::analysis::{
    build_analysis_prompt, build_chat_prompt, number_evidence, parse_analysis_output,
    parse_chat_output, AnalysisRequest, AnalysisRunner, EventSink, ANALYSIS_PROMPT_VERSION,
    MAX_DIFF_BYTES,
};
use crate::engine::{detect_engines, import_graph, read_graph_file, EngineConfig, GraphifyEngine};
use crate::git::{short_sha, worktree_id, GitRepo};
use crate::graph::LoadedGraph;
use crate::impact::{
    build_impact, GraphifySymbolExtractor, ImpactRequest, IndexContext, SymbolSupport,
    DEFAULT_IMPACT_DEPTH,
};
use crate::model::{
    ChatMessage, ChatRole, CommitAnalysis, CommitDetail, CommitImpact, CommitSummary,
    CommitTimeline, EngineStatus, EvidenceKind, EvidenceRef, GraphLevel, GraphView, HealthSeverity,
    HealthSignal, IndexRunSummary, IndexStatus, JobStatus, KnowledgeOverview, KnowledgeWorktree,
    NodeDetail, SearchHit, SymbolReferences,
};
use crate::snapshot::{materialize, SnapshotLimits};
use crate::store::KnowledgeStore;
use crate::timeline::{build_timeline, list_worktrees, ProjectRef, TaskLinks};
use chrono::Utc;
use futures::future::join_all;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use symphony_core::{redact_secrets, SymphonyError, SymphonyResult};
use tokio::sync::Semaphore;

const INDEX_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const RUNS_TO_KEEP: i64 = 2;
const CHAT_HISTORY: i64 = 40;
pub const MAX_QUESTION_CHARS: usize = 2000;
const SNIPPET_LINES: usize = 40;

/// Event types published on the runtime's event stream.
pub const EVENT_INDEX_PROGRESS: &str = "knowledge/index_progress";
pub const EVENT_INDEX_COMPLETED: &str = "knowledge/index_completed";
pub const EVENT_INDEX_FAILED: &str = "knowledge/index_failed";
pub const EVENT_ANALYSIS_COMPLETED: &str = "knowledge/analysis_completed";
pub const EVENT_CHAT_COMPLETED: &str = "knowledge/chat_completed";
pub const EVENT_REPO_CHANGED: &str = "knowledge/repo_changed";

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Directory layout under `<data dir>/knowledge`.
#[derive(Debug, Clone)]
pub struct KnowledgePaths {
    root: PathBuf,
}

/// A project id as a single safe path component.
fn safe_component(id: &str) -> String {
    let clean: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if clean.is_empty() || clean != id {
        use sha2::{Digest, Sha256};
        format!(
            "{clean}-{}",
            &hex::encode(Sha256::digest(id.as_bytes()))[..8]
        )
    } else {
        clean
    }
}

/// The Knowledge Base holds copies of repository source, so its directory is closed to other local
/// users where the platform has modes (the data directory it lives in already is; this does not
/// rely on that). Best effort: a failure leaves the directory as it was.
fn restrict_to_owner(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = dir;
}

impl KnowledgePaths {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            root: data_dir.join("knowledge"),
        }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn db_path(&self) -> PathBuf {
        self.root.join("knowledge.db")
    }
    pub fn snapshot_dir(&self, project_id: &str) -> PathBuf {
        self.root
            .join("snapshots")
            .join(safe_component(project_id))
            .join("current")
    }
    pub fn engine_out(&self, project_id: &str) -> PathBuf {
        self.root.join("engine").join(safe_component(project_id))
    }
    /// Serena's own copy of the indexed commit (it writes `.serena` beside the code it reads).
    pub fn serena_dir(&self, project_id: &str) -> PathBuf {
        self.root.join("serena").join(safe_component(project_id))
    }
    pub fn work_root(&self) -> PathBuf {
        self.root.join("work")
    }
    pub fn scratch_root(&self) -> PathBuf {
        self.root.join("scratch")
    }
}

/// How the graph endpoint should cut the index.
#[derive(Debug, Clone)]
pub struct GraphQuery {
    pub level: GraphLevel,
    /// Module to scope the file level to.
    pub module: Option<String>,
    /// Node to centre the symbol level on.
    pub focus: Option<String>,
    pub depth: u32,
    pub include_inferred: bool,
    pub include_external: bool,
}

pub struct KnowledgeService {
    store: KnowledgeStore,
    paths: KnowledgePaths,
    config: EngineConfig,
    graphs: Mutex<HashMap<i64, Arc<LoadedGraph>>>,
    project_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    engine_slots: Arc<Semaphore>,
    runner: RwLock<Option<Arc<dyn AnalysisRunner>>>,
    events: RwLock<Option<Arc<dyn EventSink>>>,
    /// `(project, sha)` analyses and project chats being produced right now.
    in_flight: Mutex<HashSet<String>>,
    /// Projects removed while an index run was still going; that run purges them when it ends.
    forgotten: Mutex<HashSet<String>>,
    /// Projects whose repository is being watched for live updates.
    pub(crate) watchers: Mutex<HashMap<String, Arc<crate::watch::WatchState>>>,
    /// Running Serena sessions by project; idle ones are closed by [`Self::reap_idle_navigators`].
    navigators: Mutex<HashMap<String, Arc<tokio::sync::Mutex<Option<LiveNavigator>>>>>,
}

/// A Serena session and what it was started for.
struct LiveNavigator {
    session: crate::serena::SerenaSession,
    commit_sha: String,
    last_used: std::time::Instant,
}

/// Serena sessions idle for this long are closed (a language server is not cheap to keep).
const NAVIGATOR_IDLE: Duration = Duration::from_secs(10 * 60);

impl KnowledgeService {
    /// Open (or create) the Knowledge Base under `data_dir`.
    pub async fn open(data_dir: &Path, config: EngineConfig) -> SymphonyResult<Arc<Self>> {
        let paths = KnowledgePaths::new(data_dir);
        std::fs::create_dir_all(paths.root())?;
        restrict_to_owner(paths.root());
        // Scratch and per-commit work directories only exist while a job runs.
        for dir in [paths.work_root(), paths.scratch_root()] {
            let _ = std::fs::remove_dir_all(&dir);
        }
        let store = KnowledgeStore::open(&paths.db_path()).await?;
        store.fail_interrupted_runs().await?;
        let service = Arc::new(Self {
            store,
            paths,
            config,
            graphs: Mutex::new(HashMap::new()),
            project_locks: Mutex::new(HashMap::new()),
            engine_slots: Arc::new(Semaphore::new(2)),
            runner: RwLock::new(None),
            events: RwLock::new(None),
            in_flight: Mutex::new(HashSet::new()),
            forgotten: Mutex::new(HashSet::new()),
            watchers: Mutex::new(HashMap::new()),
            navigators: Mutex::new(HashMap::new()),
        });
        // Close language-server sessions nobody is using; the task ends with the service.
        let weak = Arc::downgrade(&service);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tick.tick().await;
                let Some(service) = weak.upgrade() else { break };
                service.reap_idle_navigators().await;
            }
        });
        Ok(service)
    }

    pub fn set_runner(&self, runner: Arc<dyn AnalysisRunner>) {
        *self.runner.write().unwrap_or_else(|e| e.into_inner()) = Some(runner);
    }

    pub fn set_event_sink(&self, sink: Arc<dyn EventSink>) {
        *self.events.write().unwrap_or_else(|e| e.into_inner()) = Some(sink);
    }

    /// Projects currently watched for live updates.
    pub fn watchers_len(&self) -> usize {
        self.watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    pub fn store(&self) -> &KnowledgeStore {
        &self.store
    }

    pub fn paths(&self) -> &KnowledgePaths {
        &self.paths
    }

    pub fn engine_config(&self) -> &EngineConfig {
        &self.config
    }

    pub(crate) fn publish_event(
        &self,
        event_type: &str,
        project_id: &str,
        payload: serde_json::Value,
    ) {
        self.publish(event_type, project_id, payload);
    }

    fn publish(&self, event_type: &str, project_id: &str, payload: serde_json::Value) {
        let sink = self
            .events
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(sink) = sink {
            sink.publish(event_type, project_id, payload);
        }
    }

    fn runner(&self) -> SymphonyResult<Arc<dyn AnalysisRunner>> {
        self.runner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| {
                SymphonyError::Conflict("no AI provider is available for the Knowledge Base".into())
            })
    }

    fn project_lock(&self, project_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.project_locks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(project_id.to_string())
            .or_default()
            .clone()
    }

    fn claim(&self, key: String) -> bool {
        self.in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key)
    }

    fn release(&self, key: &str) {
        self.in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
    }

    /// Drop everything stored for a project that was removed: the index, analyses, chat and the
    /// snapshot and engine caches on disk. An index run that is still going cannot be stopped from
    /// here, so the project is marked and the run cleans up after itself when it ends (otherwise it
    /// would write the snapshot back after the removal).
    pub async fn forget_project(&self, project_id: &str) {
        self.forgotten
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(project_id.to_string());
        let idle = self.project_lock(project_id).try_lock_owned().is_ok();
        if idle {
            self.forgotten
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(project_id);
        }
        self.purge_project(project_id).await;
    }

    /// Run by an index task that finishes after its project was removed.
    async fn purge_if_forgotten(&self, project_id: &str) {
        let forgotten = self
            .forgotten
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(project_id);
        if forgotten {
            self.purge_project(project_id).await;
        }
    }

    async fn purge_project(&self, project_id: &str) {
        if let Err(error) = self.store.delete_project(project_id).await {
            tracing::warn!(project = %project_id, %error, "could not remove the knowledge index of a deleted project");
        }
        self.graphs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(project_id);
        let snapshot_parent = self.paths.snapshot_dir(project_id);
        let _ = std::fs::remove_dir_all(snapshot_parent.parent().unwrap_or(&snapshot_parent));
        let _ = std::fs::remove_file(snapshot_manifest_path(&snapshot_parent));
        let _ = std::fs::remove_dir_all(self.paths.engine_out(project_id));
        self.navigators
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(project_id);
        let serena_dir = self.paths.serena_dir(project_id);
        let _ = std::fs::remove_dir_all(&serena_dir);
        let _ = std::fs::remove_file(snapshot_manifest_path(&serena_dir));
    }

    /// Close Serena sessions that have been idle for a while.
    pub async fn reap_idle_navigators(&self) {
        let sessions: Vec<_> = self
            .navigators
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        for slot in sessions {
            // A session that is answering a request holds its lock; leave it alone.
            let Ok(mut guard) = slot.try_lock() else {
                continue;
            };
            let expired = guard
                .as_mut()
                .map(|live| live.last_used.elapsed() > NAVIGATOR_IDLE || !live.session.is_alive())
                .unwrap_or(false);
            if expired {
                *guard = None;
            }
        }
    }

    /// Precise references of a symbol from Serena (a language server). Slow the first time per
    /// indexed commit, because the language server has to load the project; optional, and it says
    /// why when it cannot answer.
    pub async fn node_references(
        &self,
        project: &ProjectRef,
        node_id: &str,
    ) -> SymphonyResult<SymbolReferences> {
        use crate::serena;
        let Some(command) = self.config.serena_path() else {
            return Ok(serena::unavailable(
                node_id,
                "Serena is not installed. Install it to get precise references (see Overview > Engines).",
            ));
        };
        let (graph, run) = self.graph_for(&project.id).await?.ok_or_else(not_indexed)?;
        let node = graph.node(node_id).ok_or_else(|| {
            SymphonyError::NotFound(format!("node {node_id} not found in the index"))
        })?;
        let (Some(file), Some(line), true) = (node.file.clone(), node.line, node.kind.is_symbol())
        else {
            return Ok(serena::unavailable(
                node_id,
                "Precise references are available for functions, methods and types.",
            ));
        };
        let name = node
            .label
            .trim_matches(|c| c == '.' || c == '(' || c == ')')
            .to_string();
        if name.is_empty() || !crate::snapshot::is_safe_relative_path(&file) {
            return Ok(serena::unavailable(
                node_id,
                "This symbol cannot be looked up.",
            ));
        }
        let mut languages: Vec<&'static str> = Vec::new();
        for stat in &run.languages {
            if let Some(id) = serena::serena_language(&stat.language) {
                if !languages.contains(&id) {
                    languages.push(id);
                }
            }
        }
        languages.truncate(3);

        let slot = self
            .navigators
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(project.id.clone())
            .or_default()
            .clone();
        let mut guard = slot.lock().await;
        // A session belongs to one indexed commit; anything else (or a dead process) starts over.
        let usable = match guard.as_mut() {
            Some(live) => live.commit_sha == run.commit_sha && live.session.is_alive(),
            None => false,
        };
        if !usable {
            *guard = None;
            let dir = self.paths.serena_dir(&project.id);
            materialize(
                &GitRepo::new(&project.repo_path),
                &run.commit_sha,
                &dir,
                SnapshotLimits::default(),
            )
            .await?;
            if let Err(error) =
                serena::ensure_project(&command, &dir, "symphony-knowledge", &languages).await
            {
                return Ok(serena::unavailable(node_id, error.to_string()));
            }
            let mode = serena::mode_path(self.paths.root(), &safe_component(&project.id));
            match serena::SerenaSession::start(&command, &dir, &mode).await {
                Ok(session) => {
                    *guard = Some(LiveNavigator {
                        session,
                        commit_sha: run.commit_sha.clone(),
                        last_used: std::time::Instant::now(),
                    });
                }
                Err(error) => return Ok(serena::unavailable(node_id, error.to_string())),
            }
        }
        let Some(live) = guard.as_mut() else {
            return Ok(serena::unavailable(node_id, "Serena is not running."));
        };
        live.last_used = std::time::Instant::now();
        let outcome = async {
            let found = live
                .session
                .call_tool(
                    "find_symbol",
                    json!({ "name_path_pattern": name, "relative_path": file, "include_body": false }),
                )
                .await?;
            let candidates = serena::parse_find_symbol(&found);
            let Some(definition) = serena::pick_definition(&candidates, line).cloned() else {
                return Ok(serena::unavailable(
                    node_id,
                    "Serena did not find this symbol at the indexed location.",
                ));
            };
            let references = live
                .session
                .call_tool(
                    "find_referencing_symbols",
                    json!({ "name_path": definition.name_path, "relative_path": file }),
                )
                .await?;
            Ok::<_, SymphonyError>(serena::answer(node_id, &definition, serena::parse_references(&references)))
        }
        .await;
        match outcome {
            Ok(answer) => Ok(answer),
            Err(error) => {
                // A failed session is not reused.
                *guard = None;
                Ok(serena::unavailable(node_id, error.to_string()))
            }
        }
    }

    pub async fn engines(&self) -> Vec<EngineStatus> {
        detect_engines(&self.config).await
    }

    // -- git-backed views ----------------------------------------------------------------

    pub async fn worktrees(
        &self,
        project: &ProjectRef,
        links: &TaskLinks,
    ) -> SymphonyResult<Vec<KnowledgeWorktree>> {
        list_worktrees(project, links).await
    }

    pub async fn timeline(
        &self,
        project: &ProjectRef,
        limit: usize,
    ) -> SymphonyResult<CommitTimeline> {
        build_timeline(project, limit.clamp(1, 500)).await
    }

    async fn decorate(
        &self,
        project: &ProjectRef,
        repo: &GitRepo,
        raw: crate::git::RawCommit,
    ) -> CommitSummary {
        let sha = raw.sha.clone();
        let mut summary = raw.into_summary();
        let default_tip = repo.branch_tip(&project.default_branch).await;
        let worktrees = repo.worktrees().await.unwrap_or_default();
        let membership = join_all(worktrees.iter().filter(|w| !w.bare).map(|w| {
            let repo = repo.clone();
            let sha = sha.clone();
            let head = w.head.clone();
            async move {
                match head {
                    Some(head) if repo.is_ancestor(&sha, &head).await => Some(head),
                    _ => None,
                }
            }
        }))
        .await;
        summary.worktrees = worktrees
            .iter()
            .filter(|w| !w.bare)
            .zip(membership)
            .filter_map(|(w, head)| {
                head.map(|head| crate::model::CommitWorktreeRef {
                    worktree_id: worktree_id(&w.path),
                    branch: w.branch.clone(),
                    is_head: head == sha,
                })
            })
            .collect();
        summary
            .worktrees
            .sort_by(|a, b| (&a.branch, &a.worktree_id).cmp(&(&b.branch, &b.worktree_id)));
        summary.on_default_branch = match default_tip {
            Some(tip) => repo.is_ancestor(&sha, &tip).await,
            None => false,
        };
        if repo.has_remote_refs().await {
            summary.unpushed = Some(repo.is_unpushed(&sha).await);
        }
        summary.refs = repo
            .refs_by_commit()
            .await
            .ok()
            .and_then(|mut m| m.remove(&sha))
            .unwrap_or_default();
        summary
    }

    pub async fn commit_detail(
        &self,
        project: &ProjectRef,
        sha: &str,
    ) -> SymphonyResult<CommitDetail> {
        let repo = GitRepo::new(&project.repo_path);
        let raw = repo.commit_detail(sha).await?;
        let commit = self.decorate(project, &repo, raw.commit).await;
        Ok(CommitDetail {
            commit,
            body: raw.body,
            files: raw.files,
            additions: raw.additions,
            deletions: raw.deletions,
            files_truncated: raw.files_truncated,
            diff_base: raw.diff_base,
        })
    }

    pub async fn commit_diff(
        &self,
        project: &ProjectRef,
        sha: &str,
    ) -> SymphonyResult<(String, bool)> {
        GitRepo::new(&project.repo_path)
            .diff_text(sha, 3, 512 * 1024)
            .await
    }

    // -- indexing -----------------------------------------------------------------------

    async fn graph_for(
        &self,
        project_id: &str,
    ) -> SymphonyResult<Option<(Arc<LoadedGraph>, IndexRunSummary)>> {
        let Some(run) = self.store.latest_completed_run(project_id).await? else {
            return Ok(None);
        };
        if let Some(graph) = self
            .graphs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&run.id)
            .cloned()
        {
            return Ok(Some((graph, run)));
        }
        let (nodes, edges) = self.store.load_graph(run.id).await?;
        let graph = Arc::new(LoadedGraph::new(run.id, nodes, edges));
        let mut cache = self.graphs.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= 3 {
            cache.clear();
        }
        cache.insert(run.id, graph.clone());
        Ok(Some((graph, run)))
    }

    /// Start indexing `project` in the background: its default branch, or the HEAD of one of its
    /// worktrees. Returns the running record immediately; progress and the result arrive as
    /// events.
    pub async fn start_index(
        self: &Arc<Self>,
        project: &ProjectRef,
        worktree: Option<&str>,
    ) -> SymphonyResult<IndexRunSummary> {
        let Some(graphify) = self.config.graphify_path() else {
            return Err(SymphonyError::Conflict(
                "Graphify is not installed, so the repository cannot be indexed (see Engines)"
                    .into(),
            ));
        };
        let engine = GraphifyEngine::new(graphify);
        let version = engine.version().await.ok_or_else(|| {
            SymphonyError::Conflict(
                "Graphify did not report a version; check the installation".into(),
            )
        })?;
        let guard = self
            .project_lock(&project.id)
            .try_lock_owned()
            .map_err(|_| {
                SymphonyError::Conflict("this repository is already being indexed".into())
            })?;

        let repo = GitRepo::new(&project.repo_path);
        let (ref_name, sha) = match worktree {
            Some(id) => {
                let found = repo
                    .worktrees()
                    .await?
                    .into_iter()
                    .find(|w| worktree_id(&w.path) == id)
                    .ok_or_else(|| SymphonyError::NotFound(format!("worktree {id} not found")))?;
                let head = found.head.clone().ok_or_else(|| {
                    SymphonyError::Conflict("that worktree has no commit yet".into())
                })?;
                (
                    found
                        .branch
                        .clone()
                        .unwrap_or_else(|| format!("detached {}", short_sha(&head))),
                    head,
                )
            }
            None => {
                let tip = repo
                    .branch_tip(&project.default_branch)
                    .await
                    .ok_or_else(|| {
                        SymphonyError::NotFound(format!(
                            "branch `{}` not found",
                            project.default_branch
                        ))
                    })?;
                (project.default_branch.clone(), tip)
            }
        };
        let run_id = self
            .store
            .create_run(&project.id, &ref_name, &sha, "graphify", &version)
            .await?;
        let summary = self
            .store
            .run(run_id)
            .await?
            .ok_or_else(|| SymphonyError::StorageError("the index run disappeared".into()))?;

        let this = self.clone();
        let project = project.clone();
        tokio::spawn(async move {
            let outcome = this.run_index(&project, &repo, &engine, run_id, &sha).await;
            drop(guard);
            this.purge_if_forgotten(&project.id).await;
            match outcome {
                Ok(()) => {
                    if let Ok(Some(run)) = this.store.run(run_id).await {
                        this.publish(EVENT_INDEX_COMPLETED, &project.id, json!({ "run_id": run_id, "nodes": run.node_count, "edges": run.edge_count }));
                    }
                }
                Err(error) => {
                    let message = redact_secrets(&error.to_string(), &[]);
                    tracing::warn!(project = %project.id, %message, "knowledge index failed");
                    let _ = this.store.fail_run(run_id, &message).await;
                    this.publish(
                        EVENT_INDEX_FAILED,
                        &project.id,
                        json!({ "run_id": run_id, "error": message }),
                    );
                }
            }
        });
        Ok(summary)
    }

    async fn run_index(
        &self,
        project: &ProjectRef,
        repo: &GitRepo,
        engine: &GraphifyEngine,
        run_id: i64,
        sha: &str,
    ) -> SymphonyResult<()> {
        let progress = |phase: &str| {
            self.publish(
                EVENT_INDEX_PROGRESS,
                &project.id,
                json!({ "run_id": run_id, "phase": phase }),
            );
        };
        progress("snapshot");
        let snapshot_dir = self.paths.snapshot_dir(&project.id);
        let limits = SnapshotLimits::default();
        let report = materialize(repo, sha, &snapshot_dir, limits).await?;
        if report.truncated {
            // A graph of part of the repository would show up as complete and miss dependents.
            return Err(SymphonyError::InvalidConfig(format!(
                "this repository is larger than the Knowledge Base indexes ({} files or {} MiB of source), so nothing was indexed",
                limits.max_files,
                limits.max_total_bytes / (1024 * 1024)
            )));
        }
        let known: HashSet<String> = report.files.iter().cloned().collect();

        progress("extract");
        let _slot = self
            .engine_slots
            .acquire()
            .await
            .map_err(|e| SymphonyError::Conflict(e.to_string()))?;
        let graph_path = engine
            .extract(
                &snapshot_dir,
                &self.paths.engine_out(&project.id),
                INDEX_TIMEOUT,
            )
            .await?;

        progress("import");
        let imported = tokio::task::spawn_blocking(move || {
            let bytes = read_graph_file(&graph_path)?;
            import_graph(&bytes, &known)
        })
        .await
        .map_err(|e| SymphonyError::IoError(e.to_string()))??;
        if imported.nodes.is_empty() {
            return Err(SymphonyError::InvalidConfig(
                "Graphify found no source files to index in this repository".into(),
            ));
        }

        progress("store");
        self.store
            .complete_run(
                run_id,
                &imported.nodes,
                &imported.edges,
                imported.file_count,
                &imported.languages,
            )
            .await?;
        self.store.prune_runs(&project.id, RUNS_TO_KEEP).await?;
        self.graphs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        Ok(())
    }

    // -- graph queries -------------------------------------------------------------------

    pub async fn graph(
        &self,
        project: &ProjectRef,
        query: &GraphQuery,
    ) -> SymphonyResult<GraphView> {
        let (graph, _) = self.graph_for(&project.id).await?.ok_or_else(not_indexed)?;
        match query.level {
            GraphLevel::Modules => {
                Ok(graph.module_view(query.include_external, query.include_inferred))
            }
            GraphLevel::Files => {
                Ok(graph.file_view(query.module.as_deref(), query.include_inferred))
            }
            GraphLevel::Symbols => {
                let focus = query.focus.as_deref().ok_or_else(|| {
                    SymphonyError::InvalidConfig("the symbol level needs a `focus` node".into())
                })?;
                graph
                    .neighborhood(focus, query.depth, query.include_inferred)
                    .ok_or_else(|| {
                        SymphonyError::NotFound(format!("node {focus} not found in the index"))
                    })
            }
        }
    }

    pub async fn search(
        &self,
        project: &ProjectRef,
        query: &str,
        limit: usize,
    ) -> SymphonyResult<Vec<SearchHit>> {
        let (graph, run) = self.graph_for(&project.id).await?.ok_or_else(not_indexed)?;
        let ids = self
            .store
            .search_node_ids(run.id, query, limit.clamp(1, 100))
            .await?;
        let needle = query.trim().to_lowercase();
        let mut hits: Vec<SearchHit> = ids
            .iter()
            .filter_map(|id| graph.node(id))
            .map(|n| SearchHit { node: n.clone() })
            .collect();
        // Exact name first, then code over configuration, then the best connected.
        let rank = |n: &crate::model::GraphNode| {
            let label = n
                .label
                .trim_matches(|c| c == '.' || c == '(' || c == ')')
                .to_lowercase();
            (
                std::cmp::Reverse(label == needle),
                std::cmp::Reverse(n.kind.is_symbol() || n.kind == crate::model::NodeKind::File),
                std::cmp::Reverse(n.degree),
            )
        };
        hits.sort_by_key(|h| rank(&h.node));
        Ok(hits)
    }

    pub async fn node(&self, project: &ProjectRef, node_id: &str) -> SymphonyResult<NodeDetail> {
        let (graph, _) = self.graph_for(&project.id).await?.ok_or_else(not_indexed)?;
        let mut detail = graph.detail(node_id).ok_or_else(|| {
            SymphonyError::NotFound(format!("node {node_id} not found in the index"))
        })?;
        if let (Some(file), Some(tip)) = (
            detail.node.file.clone(),
            GitRepo::new(&project.repo_path)
                .branch_tip(&project.default_branch)
                .await,
        ) {
            let repo = GitRepo::new(&project.repo_path);
            if let Ok(commits) = repo.log_for_path(&tip, &file, 8).await {
                detail.recent_commits = commits.into_iter().map(|c| c.into_summary()).collect();
            }
        }
        Ok(detail)
    }

    // -- overview ---------------------------------------------------------------------------

    pub async fn overview(
        &self,
        project: &ProjectRef,
        links: &TaskLinks,
    ) -> SymphonyResult<KnowledgeOverview> {
        let repo = GitRepo::new(&project.repo_path);
        let engines = self.engines().await;
        let worktrees = list_worktrees(project, links).await.unwrap_or_default();
        let default_tip = repo.branch_tip(&project.default_branch).await;
        let index = self.store.latest_completed_run(&project.id).await?;
        let active_run = self.store.active_or_failed_run(&project.id).await?;

        let mut health: Vec<HealthSignal> = Vec::new();
        let mut signal = |code: &str, severity: HealthSeverity, message: String| {
            health.push(HealthSignal {
                code: code.into(),
                severity,
                message,
            });
        };
        let graphify_ready = engines.iter().any(|e| e.id == "graphify" && e.available);
        if !graphify_ready {
            signal("engine_missing", HealthSeverity::Problem, "Graphify is not installed, so there is no code graph. Git history and worktrees still work.".into());
        } else if index.is_none()
            && active_run
                .as_ref()
                .map(|r| r.status != IndexStatus::Running)
                .unwrap_or(true)
        {
            signal(
                "not_indexed",
                HealthSeverity::Warning,
                "The repository has not been indexed yet.".into(),
            );
        }
        if let Some(run) = active_run
            .as_ref()
            .filter(|r| r.status == IndexStatus::Failed)
        {
            signal(
                "index_failed",
                HealthSeverity::Problem,
                format!(
                    "The last index run failed: {}",
                    run.error.as_deref().unwrap_or("unknown error")
                ),
            );
        }
        let mut index_stale = false;
        if let Some(run) = index.as_ref() {
            // The branch the index was built from: the default branch, or the branch of the
            // worktree it was started on. A detached index stays at its commit and is never stale.
            let branch = if run.ref_name == project.default_branch {
                default_tip
                    .clone()
                    .map(|tip| (project.default_branch.clone(), tip))
            } else if run.ref_name.starts_with("detached ") {
                None
            } else {
                repo.branch_tip(&run.ref_name)
                    .await
                    .map(|tip| (run.ref_name.clone(), tip))
            };
            if let Some((branch, tip)) = branch.filter(|(_, tip)| tip != &run.commit_sha) {
                index_stale = true;
                let behind = repo.ahead_count(&tip, &run.commit_sha).await;
                signal(
                    "index_stale",
                    HealthSeverity::Info,
                    match behind {
                        Some(n) if n > 0 => format!(
                            "The index was built from {}; `{branch}` is {n} commit(s) ahead.",
                            short_sha(&run.commit_sha),
                        ),
                        _ => format!(
                            "The index was built from {}, which is not on `{branch}` any more.",
                            short_sha(&run.commit_sha),
                        ),
                    },
                );
            }
        }
        let dirty = worktrees.iter().filter(|w| w.dirty).count();
        if dirty > 0 {
            signal(
                "dirty_worktrees",
                HealthSeverity::Info,
                format!(
                    "{dirty} of {} worktree(s) have uncommitted changes.",
                    worktrees.len()
                ),
            );
        }
        let detached = worktrees.iter().filter(|w| w.detached).count();
        if detached > 0 {
            signal(
                "detached_worktrees",
                HealthSeverity::Info,
                format!("{detached} worktree(s) are on a detached HEAD."),
            );
        }
        let prunable = worktrees.iter().filter(|w| w.prunable || !w.exists).count();
        if prunable > 0 {
            signal("missing_worktrees", HealthSeverity::Warning, format!("{prunable} worktree(s) point at a directory that no longer exists (`git worktree prune`)."));
        }

        let (symbol_count, top_modules, hubs) = match self.graph_for(&project.id).await? {
            Some((graph, _)) => {
                let (inferred, total) = graph.inferred_share();
                if total > 0 && inferred * 4 > total {
                    signal(
                        "inferred_edges",
                        HealthSeverity::Info,
                        format!("{inferred} of {total} dependency edges were inferred rather than read from source."),
                    );
                }
                (graph.symbol_count(), graph.module_stats(12), graph.hubs(8))
            }
            None => (0, Vec::new(), Vec::new()),
        };

        Ok(KnowledgeOverview {
            project_id: project.id.clone(),
            default_branch: project.default_branch.clone(),
            default_branch_sha: default_tip,
            index,
            active_run,
            index_stale,
            engines,
            worktrees_total: worktrees.len() as u32,
            worktrees_dirty: dirty as u32,
            symbol_count,
            top_modules,
            hubs,
            health,
            generated_at: Utc::now(),
        })
    }

    // -- commit impact ----------------------------------------------------------------------

    pub async fn commit_impact(
        &self,
        project: &ProjectRef,
        sha: &str,
        depth: Option<u32>,
        include_inferred: bool,
    ) -> SymphonyResult<CommitImpact> {
        let repo = GitRepo::new(&project.repo_path);
        let full = repo.resolve_commit(sha).await?;
        let depth = depth
            .unwrap_or(DEFAULT_IMPACT_DEPTH)
            .clamp(1, crate::impact::MAX_IMPACT_DEPTH);
        let indexed = self.graph_for(&project.id).await?;
        // Only the result that follows every kind of link is cached; asking for extracted links
        // only is computed fresh, because its dependents are a different set.
        if include_inferred {
            if let Some((_, run)) = &indexed {
                if let Some(cached) = self
                    .store
                    .cached_impact(&project.id, &full, run.id, depth)
                    .await?
                {
                    return Ok(cached);
                }
            }
        }
        let raw = repo.commit_detail(&full).await?;
        let extractor;
        let support = match self.config.graphify_path() {
            Some(path) => {
                extractor = GraphifySymbolExtractor::new(
                    GraphifyEngine::new(path),
                    self.paths.work_root(),
                    self.engine_slots.clone(),
                );
                SymbolSupport::Available(&extractor)
            }
            None => SymbolSupport::Unavailable(
                "Graphify is not installed, so changed code is listed by file only.".into(),
            ),
        };
        let context = indexed
            .as_ref()
            .map(|(graph, run)| IndexContext { graph, run });
        let parent = raw.commit.parents.first().cloned();
        let impact = build_impact(
            &repo,
            ImpactRequest {
                sha: &full,
                parent: parent.as_deref(),
                files: &raw.files,
                depth,
                include_inferred,
                patch_truncated: raw.patch_truncated,
            },
            context,
            support,
        )
        .await?;
        // Only a complete result is worth keeping: a missing engine may be installed later.
        if let (Some((_, run)), None) = (&indexed, &impact.unavailable_reason) {
            if include_inferred {
                self.store
                    .cache_impact(&project.id, run.id, depth, &impact)
                    .await?;
            }
        }
        Ok(impact)
    }

    // -- AI analysis ------------------------------------------------------------------------

    pub async fn commit_analysis(
        &self,
        project: &ProjectRef,
        sha: &str,
    ) -> SymphonyResult<Option<CommitAnalysis>> {
        let full = GitRepo::new(&project.repo_path).resolve_commit(sha).await?;
        self.store.latest_analysis(&project.id, &full).await
    }

    /// Start (or return the cached / running) AI analysis of a commit.
    pub async fn start_commit_analysis(
        self: &Arc<Self>,
        project: &ProjectRef,
        sha: &str,
        force: bool,
    ) -> SymphonyResult<CommitAnalysis> {
        let runner = self.runner()?;
        let repo = GitRepo::new(&project.repo_path);
        let full = repo.resolve_commit(sha).await?;
        let (provider, model) = runner.selection(&project.id).await?;
        if !force {
            if let Some(cached) = self
                .store
                .cached_analysis(
                    &project.id,
                    &full,
                    provider,
                    model.as_deref(),
                    ANALYSIS_PROMPT_VERSION,
                )
                .await?
            {
                return Ok(cached);
            }
        }
        let key = format!("analysis:{}:{full}", project.id);
        if !self.claim(key.clone()) {
            return self
                .store
                .latest_analysis(&project.id, &full)
                .await?
                .ok_or_else(|| {
                    SymphonyError::Conflict("an analysis of this commit is already running".into())
                });
        }
        let id = match self
            .store
            .start_analysis(
                &project.id,
                &full,
                provider,
                model.as_deref(),
                ANALYSIS_PROMPT_VERSION,
            )
            .await
        {
            Ok(id) => id,
            Err(error) => {
                self.release(&key);
                return Err(error);
            }
        };
        let this = self.clone();
        let project_for_task = project.clone();
        let sha_for_task = full.clone();
        tokio::spawn(async move {
            let outcome = this
                .run_analysis(
                    &project_for_task,
                    &sha_for_task,
                    id,
                    runner,
                    provider,
                    model,
                )
                .await;
            if let Err(error) = outcome {
                let message = redact_secrets(&error.to_string(), &[]);
                tracing::warn!(project = %project_for_task.id, sha = %sha_for_task, %message, "commit analysis failed");
                let _ = this.store.fail_analysis(id, &message).await;
            }
            this.release(&key);
            this.publish(
                EVENT_ANALYSIS_COMPLETED,
                &project_for_task.id,
                json!({ "sha": sha_for_task, "analysis_id": id }),
            );
        });
        self.store
            .latest_analysis(&project.id, &full)
            .await?
            .ok_or_else(|| SymphonyError::StorageError("the analysis record disappeared".into()))
    }

    async fn run_analysis(
        &self,
        project: &ProjectRef,
        sha: &str,
        id: i64,
        runner: Arc<dyn AnalysisRunner>,
        provider: symphony_core::ProviderKind,
        model: Option<String>,
    ) -> SymphonyResult<()> {
        let detail = self.commit_detail(project, sha).await?;
        let impact = match self.commit_impact(project, sha, None, true).await {
            Ok(impact) => impact,
            Err(error) => {
                // The analysis can still be done from the diff alone; say so in the prompt.
                tracing::info!(%error, "commit impact unavailable for the analysis");
                CommitImpact {
                    sha: sha.to_string(),
                    run_id: None,
                    indexed_sha: None,
                    unavailable_reason: Some(format!("the impact could not be computed ({error})")),
                    files: Vec::new(),
                    affected: Vec::new(),
                    depth: DEFAULT_IMPACT_DEPTH,
                    affected_truncated: false,
                    symbol_files_skipped: 0,
                    computed_at: Utc::now(),
                }
            }
        };
        let repo = GitRepo::new(&project.repo_path);
        let (diff, truncated) = repo.diff_text(sha, 3, MAX_DIFF_BYTES).await?;
        let built = build_analysis_prompt(&project.name, &detail, &impact, &diff, truncated);
        let scratch = self.new_scratch_dir()?;
        let output = runner
            .run(AnalysisRequest {
                project_id: project.id.clone(),
                provider,
                model,
                prompt: built.prompt,
                scratch_dir: scratch.clone(),
            })
            .await;
        let _ = std::fs::remove_dir_all(&scratch);
        let output = output?;
        let (result, dropped) = parse_analysis_output(&output.text, &built.evidence)?;
        // Keep only the evidence that is actually cited, so the stored record is self-contained.
        let cited: HashSet<&str> = result
            .what_changed
            .iter()
            .chain(&result.risks)
            .chain(&result.security)
            .chain(&result.regressions)
            .chain(&result.recommendations)
            .flat_map(|f| f.evidence.iter().map(String::as_str))
            .chain(
                result
                    .tests
                    .iter()
                    .flat_map(|t| t.evidence.iter().map(String::as_str)),
            )
            .collect();
        let evidence: Vec<EvidenceRef> = built
            .evidence
            .into_iter()
            .filter(|e| cited.contains(e.id.as_str()))
            .collect();
        self.store
            .finish_analysis(
                id,
                &result,
                &evidence,
                dropped,
                output.input_tokens,
                output.output_tokens,
            )
            .await
    }

    fn new_scratch_dir(&self) -> SymphonyResult<PathBuf> {
        let dir = self.paths.scratch_root().join(format!(
            "{}-{}-{}",
            std::process::id(),
            SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    // -- chat -------------------------------------------------------------------------------

    pub async fn chat_messages(&self, project: &ProjectRef) -> SymphonyResult<Vec<ChatMessage>> {
        self.store.messages(&project.id, CHAT_HISTORY).await
    }

    pub async fn clear_chat(&self, project: &ProjectRef) -> SymphonyResult<()> {
        self.store.clear_messages(&project.id).await
    }

    /// Ask a question about the repository. Returns the stored question and the (still running)
    /// answer record; the answer arrives as a `knowledge/chat_completed` event.
    pub async fn ask(
        self: &Arc<Self>,
        project: &ProjectRef,
        question: &str,
    ) -> SymphonyResult<Vec<ChatMessage>> {
        let question = question.trim();
        if question.is_empty() || question.chars().count() > MAX_QUESTION_CHARS {
            return Err(SymphonyError::InvalidConfig(format!(
                "the question must be 1 to {MAX_QUESTION_CHARS} characters"
            )));
        }
        let runner = self.runner()?;
        let (provider, model) = runner.selection(&project.id).await?;
        let key = format!("chat:{}", project.id);
        let busy =
            || SymphonyError::Conflict("the previous question is still being answered".into());
        if !self.claim(key.clone()) {
            return Err(busy());
        }
        match self.store.has_running_message(&project.id).await {
            Ok(false) => {}
            Ok(true) => {
                self.release(&key);
                return Err(busy());
            }
            Err(error) => {
                self.release(&key);
                return Err(error);
            }
        }
        let question_id = match self
            .store
            .add_message(
                &project.id,
                ChatRole::User,
                question,
                JobStatus::Completed,
                None,
                None,
            )
            .await
        {
            Ok(id) => id,
            Err(error) => {
                self.release(&key);
                return Err(error);
            }
        };
        let answer_id = match self
            .store
            .add_message(
                &project.id,
                ChatRole::Assistant,
                "",
                JobStatus::Running,
                Some(provider),
                model.as_deref(),
            )
            .await
        {
            Ok(id) => id,
            Err(error) => {
                self.release(&key);
                return Err(error);
            }
        };
        let this = self.clone();
        let project_for_task = project.clone();
        let question = question.to_string();
        tokio::spawn(async move {
            let outcome = this
                .run_chat(
                    &project_for_task,
                    &question,
                    question_id,
                    answer_id,
                    runner,
                    provider,
                    model,
                )
                .await;
            if let Err(error) = outcome {
                let message = redact_secrets(&error.to_string(), &[]);
                tracing::warn!(project = %project_for_task.id, %message, "knowledge chat failed");
                let _ = this
                    .store
                    .finish_message(answer_id, "", &[], JobStatus::Failed, Some(&message))
                    .await;
            }
            this.release(&key);
            this.publish(
                EVENT_CHAT_COMPLETED,
                &project_for_task.id,
                json!({ "message_id": answer_id }),
            );
        });
        let messages = self.store.messages(&project.id, 2).await?;
        Ok(messages)
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_chat(
        &self,
        project: &ProjectRef,
        question: &str,
        question_id: i64,
        answer_id: i64,
        runner: Arc<dyn AnalysisRunner>,
        provider: symphony_core::ProviderKind,
        model: Option<String>,
    ) -> SymphonyResult<()> {
        let history: Vec<ChatMessage> = self
            .store
            .messages(&project.id, CHAT_HISTORY)
            .await?
            .into_iter()
            .filter(|m| m.id < question_id)
            .collect();
        let retrieved = self.retrieve_evidence(project, question).await?;
        let numbered = number_evidence(retrieved);
        let prompt = build_chat_prompt(&project.name, question, &history, &numbered);
        let references: Vec<EvidenceRef> = numbered.iter().map(|e| e.reference.clone()).collect();
        let scratch = self.new_scratch_dir()?;
        let output = runner
            .run(AnalysisRequest {
                project_id: project.id.clone(),
                provider,
                model,
                prompt,
                scratch_dir: scratch.clone(),
            })
            .await;
        let _ = std::fs::remove_dir_all(&scratch);
        let output = output?;
        let answer = parse_chat_output(&output.text, &references)?;
        self.store
            .finish_message(
                answer_id,
                &answer.content,
                &answer.evidence,
                JobStatus::Completed,
                None,
            )
            .await
    }

    /// Material for a question: symbols and files whose names match its words, how they connect,
    /// a few lines of their code from the index snapshot, and commits whose messages mention it.
    async fn retrieve_evidence(
        &self,
        project: &ProjectRef,
        question: &str,
    ) -> SymphonyResult<Vec<(EvidenceRef, String)>> {
        let terms = question_terms(question);
        let mut out: Vec<(EvidenceRef, String)> = Vec::new();
        if let Some((graph, run)) = self.graph_for(&project.id).await? {
            let mut scores: HashMap<String, u32> = HashMap::new();
            for term in &terms {
                for id in self.store.search_node_ids(run.id, term, 8).await? {
                    *scores.entry(id).or_default() += 1;
                }
            }
            let mut ranked: Vec<(&String, &u32)> = scores.iter().collect();
            ranked.sort_by(|a, b| {
                let da = graph.node(a.0).map(|n| n.degree).unwrap_or(0);
                let db = graph.node(b.0).map(|n| n.degree).unwrap_or(0);
                b.1.cmp(a.1).then(db.cmp(&da)).then_with(|| a.0.cmp(b.0))
            });
            let repo = GitRepo::new(&project.repo_path);
            for (rank, (id, _)) in ranked.into_iter().take(10).enumerate() {
                let Some(detail) = graph.detail(id) else {
                    continue;
                };
                let node = &detail.node;
                let mut text = format!(
                    "{} {} — {}{}",
                    node.kind.as_str(),
                    node.label,
                    node.file.as_deref().unwrap_or("external dependency"),
                    node.line.map(|l| format!(":{l}")).unwrap_or_default(),
                );
                let names = |list: &[crate::model::NodeNeighbor]| -> String {
                    list.iter()
                        .take(6)
                        .map(|n| format!("{} ({})", n.node.label, n.relation))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                if !detail.dependencies.is_empty() {
                    text.push_str(&format!("\n  uses: {}", names(&detail.dependencies)));
                }
                if !detail.references.is_empty() {
                    text.push_str(&format!("\n  used by: {}", names(&detail.references)));
                }
                if rank < 4 {
                    if let (Some(file), Some(line)) = (node.file.as_deref(), node.line) {
                        if let Some(snippet) =
                            read_snippet(&repo, &run.commit_sha, file, line).await
                        {
                            text.push_str(&format!("\n  code:\n{snippet}"));
                        }
                    }
                }
                out.push((
                    EvidenceRef {
                        id: String::new(),
                        kind: if node.kind == crate::model::NodeKind::File {
                            EvidenceKind::File
                        } else {
                            EvidenceKind::Symbol
                        },
                        label: node.label.clone(),
                        file: node.file.clone(),
                        line: node.line,
                        node_id: Some(node.id.clone()),
                        sha: None,
                    },
                    text,
                ));
            }
        }
        let repo = GitRepo::new(&project.repo_path);
        if let Some(tip) = repo.branch_tip(&project.default_branch).await {
            let long_terms: Vec<String> = terms
                .iter()
                .filter(|t| t.len() >= 4)
                .take(4)
                .cloned()
                .collect();
            if let Ok(commits) = repo.search_commits(&tip, &long_terms, 6).await {
                for commit in commits {
                    out.push((
                        EvidenceRef {
                            id: String::new(),
                            kind: EvidenceKind::Commit,
                            label: format!("{} {}", short_sha(&commit.sha), commit.subject),
                            file: None,
                            line: None,
                            node_id: None,
                            sha: Some(commit.sha.clone()),
                        },
                        format!(
                            "commit {} ({}) by {}: {}",
                            short_sha(&commit.sha),
                            commit.committed_at.format("%Y-%m-%d"),
                            commit.author_name,
                            commit.subject
                        ),
                    ));
                }
            }
        }
        out.truncate(20);
        Ok(out)
    }
}

/// Where [`crate::snapshot`] keeps the manifest of a snapshot directory (next to it).
fn snapshot_manifest_path(dir: &Path) -> PathBuf {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "snapshot".into());
    dir.with_file_name(format!("{name}.manifest.json"))
}

fn not_indexed() -> SymphonyError {
    SymphonyError::Conflict("the repository has not been indexed yet".into())
}

const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "what", "where", "when", "which", "who", "how",
    "why", "does", "did", "are", "was", "were", "can", "could", "should", "would", "about", "into",
    "from", "have", "has", "had", "there", "their", "them", "then", "than", "work", "works",
    "used", "use", "uses", "using", "code", "function", "file", "please", "explain", "show",
    "tell", "all", "any", "not", "you", "your", "our", "its",
];

/// The words of a question worth searching for, most specific (longest) first.
pub fn question_terms(question: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut terms: Vec<String> = question
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|t| t.chars().count() >= 3)
        .map(|t| t.to_lowercase())
        .filter(|t| !STOP_WORDS.contains(&t.as_str()) && seen.insert(t.clone()))
        .collect();
    terms.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    terms.truncate(6);
    terms
}

/// A few lines of a file as it is in the indexed commit, starting at `line`, numbered. Read from
/// git at that commit (not from the shared snapshot directory, which a newer index run may be
/// rewriting), so the code always matches the graph that selected it. The path was produced by the
/// indexer; it is still checked.
async fn read_snippet(repo: &GitRepo, sha: &str, file: &str, line: u32) -> Option<String> {
    if !crate::snapshot::is_safe_relative_path(file) {
        return None;
    }
    let bytes = repo.blob(sha, file, 1024 * 1024).await.ok()??;
    let text = String::from_utf8(bytes).ok()?;
    let start = (line as usize).saturating_sub(1);
    let snippet = text
        .lines()
        .enumerate()
        .skip(start)
        .take(SNIPPET_LINES)
        .map(|(i, l)| format!("    {:>5} | {l}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    (!snippet.is_empty()).then_some(snippet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_ids_become_single_safe_path_components() {
        assert_eq!(safe_component("3f2a-uuid_1"), "3f2a-uuid_1");
        for hostile in ["../../etc", "a/b", "a\\b", "C:\\x", "", "..", "a b"] {
            let clean = safe_component(hostile);
            assert!(
                !clean.contains(['/', '\\', ':', ' ']),
                "{hostile:?} -> {clean:?}"
            );
            assert!(clean != ".." && !clean.is_empty());
        }
        assert_ne!(
            safe_component("a/b"),
            safe_component("a_b"),
            "sanitising must not merge distinct ids"
        );
    }

    #[test]
    fn question_terms_drop_filler_and_keep_identifiers() {
        let terms =
            question_terms("How does the is_worktree_dirty function work in WorkspaceManager?");
        assert_eq!(
            terms[0], "is_worktree_dirty",
            "longest, most specific first"
        );
        assert!(terms.contains(&"workspacemanager".to_string()));
        assert!(!terms.contains(&"how".to_string()) && !terms.contains(&"the".to_string()));
        assert!(question_terms("what is it").is_empty());
    }

    #[tokio::test]
    async fn snippets_come_from_the_indexed_commit_and_are_numbered() {
        use crate::timeline::tests::{commit_file, init_repo};
        let temp = tempfile::TempDir::new().unwrap();
        init_repo(temp.path());
        let sha = commit_file(
            temp.path(),
            "a.rs",
            "one
two
three
",
            "add a",
        );
        // The working tree moves on; the snippet still shows the commit the graph was built from.
        std::fs::write(
            temp.path().join("a.rs"),
            "changed
",
        )
        .unwrap();
        let repo = GitRepo::new(temp.path());
        let snippet = read_snippet(&repo, &sha, "a.rs", 2).await.unwrap();
        assert!(
            snippet.contains("    2 | two")
                && snippet.contains("    3 | three")
                && !snippet.contains("one")
                && !snippet.contains("changed")
        );
        assert!(read_snippet(&repo, &sha, "../outside.rs", 1)
            .await
            .is_none());
        assert!(read_snippet(&repo, &sha, "missing.rs", 1).await.is_none());
    }
}
