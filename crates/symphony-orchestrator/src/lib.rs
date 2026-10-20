use async_trait::async_trait;
use chrono::{DateTime, Utc};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use symphony_agents::{
    detect_provider_statuses, start_agent_session, AgentLaunchConfig, AgentStreamEvent,
    ApprovalGate, DynamicToolHandler,
};
use symphony_core::{
    compute_backoff_delay_ms, redact_secrets, render_continuation_prompt, render_workflow_prompt,
    sort_candidate_issues, AgentEvent, ApprovalDecision, ApprovalRequest, ApprovalStatus,
    CodexTotals, HardwareSnapshot, Issue, LiveSession, MergeReadinessReport,
    OrchestratorStateSnapshot, Project, ProviderKind, ProviderStatus, PullRequestReview,
    RepositoryValidation, RetryEntry, RunAttempt, RunAttemptPhase, RunningSessionSummary,
    ServiceConfig, StateCounts, SymphonyError, SymphonyResult, Task, TaskDiff, TaskStatus,
    TokenUsage, ValidationResult, WorkflowDefinition, WorktreeInfo,
};
use symphony_platform::{HardwareMonitor, SecretStore};

/// Provider API keys that agent subprocesses may receive from the credential vault.
pub const PROVIDER_CREDENTIAL_ENV_KEYS: &[&str] = &["OPENAI_API_KEY", "ANTHROPIC_API_KEY"];
use symphony_storage::Storage;
use symphony_trackers::{GitHubTracker, LinearTracker, TrackerAdapter};
use symphony_workspaces::WorkspaceManager;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, RwLock};
use tracing::{debug, info, warn};
use ts_rs::TS;
use uuid::Uuid;

/// How long provider detection results are reused before re-probing CLIs.
const PROVIDER_CACHE_TTL_SECS: i64 = 60;
/// Maximum diff size returned by `task_diff` (the UI renders it lazily).
const TASK_DIFF_MAX_BYTES: usize = 512 * 1024;

/// SPEC.md Section 13.7.2 `/api/v1/<issue_identifier>` response contract
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IssueDetailResponse {
    pub issue_identifier: String,
    pub issue_id: String,
    pub task_id: String,
    pub status: String,
    pub task_status: TaskStatus,
    pub provider: ProviderKind,
    pub workspace: WorkspaceInfo,
    pub attempts: AttemptSummary,
    pub running: Option<LiveSession>,
    pub retry: Option<RetryEntry>,
    pub recent_events: Vec<AgentEvent>,
    pub last_error: Option<String>,
    pub validation_results: Vec<ValidationResult>,
    pub pull_request: Option<PullRequestReview>,
    pub tracked: HashMap<String, Value>,
    pub task: Task,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkspaceInfo {
    pub path: Option<String>,
    pub branch_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AttemptSummary {
    pub restart_count: u32,
    pub current_retry_attempt: u32,
}

/// SPEC.md Section 13.7.2 `POST /api/v1/refresh` response contract
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RefreshResponse {
    pub queued: bool,
    pub coalesced: bool,
    #[ts(type = "string")]
    pub requested_at: DateTime<Utc>,
    pub operations: Vec<String>,
}

/// Workflow document as served to editors (`GET /api/v1/workflow`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkflowDocument {
    /// File the document is loaded from and saved to, if any.
    pub path: Option<String>,
    pub raw_markdown: String,
    pub valid: bool,
    pub error: Option<String>,
    /// Parsed configuration with tracker credentials redacted.
    pub config: Option<ServiceConfig>,
    pub rendered_preview: Option<String>,
}

/// Result of previewing or saving a workflow document.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkflowValidation {
    pub valid: bool,
    pub error_code: Option<String>,
    pub error: Option<String>,
    pub config: Option<ServiceConfig>,
    pub rendered_prompt: Option<String>,
    pub path: Option<String>,
}

/// Partial project update (`PATCH /api/v1/projects/{id}`). Absent fields are left unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(optional_fields = nullable)]
pub struct ProjectUpdate {
    pub name: Option<String>,
    pub default_branch: Option<String>,
    pub preferred_provider: Option<ProviderKind>,
    pub preferred_model: Option<String>,
    pub tracker_kind: Option<String>,
    pub github_repo: Option<String>,
    pub linear_project_slug: Option<String>,
    pub required_labels: Option<Vec<String>>,
    pub auto_dispatch: Option<bool>,
    pub max_concurrent_agents: Option<u32>,
    pub validation_commands: Option<Vec<String>>,
}

struct ActiveWorkerHandle {
    pub session: LiveSession,
    pub cancel_tx: oneshot::Sender<String>,
}

struct OrchestratorRuntimeMemory {
    pub scheduler_paused: bool,
    pub running: HashMap<String, ActiveWorkerHandle>,
    pub claimed_issue_ids: HashSet<String>,
    pub retry_attempts: HashMap<String, RetryEntry>,
    pub completed_issue_ids: HashSet<String>,
    pub codex_totals: CodexTotals,
    pub latest_rate_limits: Option<Value>,
    pub active_workflow_path: Option<PathBuf>,
    pub active_workflow: WorkflowDefinition,
    pub active_config: ServiceConfig,
    pub workflow_error: Option<String>,
    pub pending_approvals: HashMap<String, oneshot::Sender<ApprovalDecision>>,
    pub provider_cache: Option<(DateTime<Utc>, Vec<ProviderStatus>)>,
    /// Where the workflow is saved when no `WORKFLOW.md` was loaded at startup.
    pub workflow_fallback_path: Option<PathBuf>,
}

struct OrchestratorApprovalGate {
    storage: Storage,
    memory: Arc<RwLock<OrchestratorRuntimeMemory>>,
    event_tx: broadcast::Sender<AgentEvent>,
}

#[async_trait]
impl ApprovalGate for OrchestratorApprovalGate {
    async fn request_approval(&self, request: ApprovalRequest) -> SymphonyResult<ApprovalDecision> {
        let approval_id = request.id.clone();
        self.storage.upsert_approval(&request).await?;

        let evt = AgentEvent {
            id: Uuid::new_v4().to_string(),
            task_id: Some(request.task_id.clone()),
            issue_identifier: Some(request.issue_identifier.clone()),
            session_id: Some(request.session_id.clone()),
            provider: Some(request.provider),
            event_type: "approval/requested".into(),
            phase: Some(RunAttemptPhase::StreamingTurn),
            message: format!("Approval requested: {}", request.title),
            payload: Some(serde_json::to_value(&request).unwrap_or(Value::Null)),
            timestamp: Utc::now(),
        };
        let _ = self.storage.record_event(&evt).await;
        let _ = self.event_tx.send(evt);

        let (tx, rx) = oneshot::channel();
        {
            let mut mem = self.memory.write().await;
            mem.pending_approvals.insert(approval_id.clone(), tx);
        }

        match rx.await {
            Ok(decision) => Ok(decision),
            Err(_) => Ok(ApprovalDecision::Cancel),
        }
    }
}

struct HostDynamicToolExecutor {
    github_tracker: Option<GitHubTracker>,
    linear_tracker: Option<LinearTracker>,
}

#[async_trait]
impl DynamicToolHandler for HostDynamicToolExecutor {
    async fn execute_tool(&self, tool_name: &str, arguments: &Value) -> Value {
        match tool_name {
            "github_api" => {
                if let Some(ref gh) = self.github_tracker {
                    let (ok, output) = gh.execute_github_dynamic_tool(arguments.clone()).await;
                    json!({
                        "success": ok,
                        "output": output,
                        "contentItems": [{"type": "inputText", "text": output}]
                    })
                } else {
                    let msg = json!({"error": "GitHub tracker is not configured for this project"})
                        .to_string();
                    json!({
                        "success": false,
                        "output": msg,
                        "contentItems": [{"type": "inputText", "text": msg}]
                    })
                }
            }
            "linear_graphql" => {
                if let Some(ref ln) = self.linear_tracker {
                    let (ok, output) = ln.execute_linear_dynamic_tool(arguments.clone()).await;
                    json!({
                        "success": ok,
                        "output": output,
                        "contentItems": [{"type": "inputText", "text": output}]
                    })
                } else {
                    let msg = json!({"error": "Linear tracker is not configured for this project"})
                        .to_string();
                    json!({
                        "success": false,
                        "output": msg,
                        "contentItems": [{"type": "inputText", "text": msg}]
                    })
                }
            }
            other => {
                let msg =
                    json!({"error": format!("Unsupported dynamic tool: {other}")}).to_string();
                json!({
                    "success": false,
                    "output": msg,
                    "contentItems": [{"type": "inputText", "text": msg}]
                })
            }
        }
    }
}

/// Authoritative Rust Symphony Orchestrator (SPEC.md Sections 7, 8, 9, 10, 16)
#[derive(Clone)]
pub struct SymphonyOrchestrator {
    storage: Storage,
    workspaces: WorkspaceManager,
    hardware: HardwareMonitor,
    secret_store: SecretStore,
    memory: Arc<RwLock<OrchestratorRuntimeMemory>>,
    event_tx: broadcast::Sender<AgentEvent>,
    refresh_tx: mpsc::Sender<()>,
    refresh_rx: Arc<Mutex<mpsc::Receiver<()>>>,
}

impl SymphonyOrchestrator {
    pub async fn new(
        storage: Storage,
        workflow_path: Option<PathBuf>,
        secret_store: SecretStore,
    ) -> SymphonyResult<Self> {
        let (active_workflow, active_config, workflow_error) = match &workflow_path {
            Some(path) if path.exists() => match WorkflowDefinition::load_from_path(path) {
                Ok(wf) => match ServiceConfig::from_workflow(&wf) {
                    Ok(cfg) => (wf, cfg, None),
                    Err(e) => {
                        let default_wf = Self::default_workflow();
                        let default_cfg = ServiceConfig::from_workflow(&default_wf)?;
                        (default_wf, default_cfg, Some(e.to_string()))
                    }
                },
                Err(e) => {
                    let default_wf = Self::default_workflow();
                    let default_cfg = ServiceConfig::from_workflow(&default_wf)?;
                    (default_wf, default_cfg, Some(e.to_string()))
                }
            },
            _ => {
                let default_wf = Self::default_workflow();
                let default_cfg = ServiceConfig::from_workflow(&default_wf)?;
                (default_wf, default_cfg, None)
            }
        };

        // Crash recovery on startup: reconcile stale Running tasks in SQLite
        let recovered = storage.recover_stale_running_tasks().await?;
        if !recovered.is_empty() {
            info!(
                recovered_count = recovered.len(),
                "Recovered interrupted running tasks on orchestrator startup"
            );
        }

        // Hydrate totals from existing tasks
        let existing_tasks = storage.list_tasks(None).await.unwrap_or_default();
        let mut totals = CodexTotals::default();
        let mut completed_ids = HashSet::new();
        for t in &existing_tasks {
            totals.input_tokens = totals.input_tokens.saturating_add(t.input_tokens);
            totals.output_tokens = totals.output_tokens.saturating_add(t.output_tokens);
            totals.total_tokens = totals.total_tokens.saturating_add(t.total_tokens);
            totals.seconds_running += t.duration_seconds;
            totals.estimated_cost_usd += t.estimated_cost_usd;
            if t.status == TaskStatus::Completed {
                completed_ids.insert(t.issue.id.clone());
            }
        }

        let (event_tx, _) = broadcast::channel(2048);
        let (refresh_tx, refresh_rx) = mpsc::channel(32);

        let memory = Arc::new(RwLock::new(OrchestratorRuntimeMemory {
            scheduler_paused: false,
            running: HashMap::new(),
            claimed_issue_ids: HashSet::new(),
            retry_attempts: HashMap::new(),
            completed_issue_ids: completed_ids,
            codex_totals: totals,
            latest_rate_limits: None,
            active_workflow_path: workflow_path,
            active_workflow,
            active_config,
            workflow_error,
            pending_approvals: HashMap::new(),
            provider_cache: None,
            workflow_fallback_path: None,
        }));

        Ok(Self {
            storage,
            workspaces: WorkspaceManager::new(),
            hardware: HardwareMonitor::new(),
            secret_store,
            memory,
            event_tx,
            refresh_tx,
            refresh_rx: Arc::new(Mutex::new(refresh_rx)),
        })
    }

    fn default_workflow() -> WorkflowDefinition {
        let raw = r#"---
tracker:
  kind: manual
  active_states:
    - Todo
    - In Progress
    - Ready
  terminal_states:
    - Done
    - Closed
    - Cancelled
polling:
  interval_ms: 30000
agent:
  max_concurrent_agents: 2
  max_turns: 8
  max_retry_backoff_ms: 300000
codex:
  command: codex app-server
  approval_policy: never
  thread_sandbox: workspace-write
  turn_timeout_ms: 3600000
  read_timeout_ms: 5000
  stall_timeout_ms: 300000
---
You are working on issue `{{ issue.identifier }}`: **{{ issue.title }}**.

Description:
{{ issue.description }}

Please implement the requested changes cleanly in the current Git workspace, run appropriate tests/validation, and summarize your work.
"#;
        WorkflowDefinition::parse(raw, None).expect("default workflow must be valid")
    }

    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    pub fn secret_store(&self) -> &SecretStore {
        &self.secret_store
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<AgentEvent> {
        self.event_tx.subscribe()
    }

    /// Start background polling, reconciliation, and dynamic WORKFLOW.md file watcher loops
    pub fn start_background_loops(&self) {
        let orch = self.clone();
        tokio::spawn(async move {
            // Startup terminal workspace cleanup
            let _ = orch.startup_terminal_workspace_cleanup().await;

            loop {
                let interval_ms = {
                    let mem = orch.memory.read().await;
                    mem.active_config.polling_interval_ms.max(2000)
                };

                let mut rx = orch.refresh_rx.lock().await;
                let _ = tokio::time::timeout(Duration::from_millis(interval_ms), rx.recv()).await;
                drop(rx);

                if let Err(e) = orch.reconcile_and_dispatch_tick().await {
                    warn!(error = %e, "Orchestrator tick encountered an error");
                }
            }
        });

        // Start dynamic WORKFLOW.md file watcher if a path is configured
        let orch_watch = self.clone();
        tokio::spawn(async move {
            let path_opt = {
                let mem = orch_watch.memory.read().await;
                mem.active_workflow_path.clone()
            };
            if let Some(wf_path) = path_opt {
                let _ = orch_watch.watch_workflow_file(wf_path).await;
            }
        });
    }

    async fn watch_workflow_file(&self, wf_path: PathBuf) -> SymphonyResult<()> {
        let (tx, mut rx) = mpsc::channel::<notify::Result<Event>>(16);
        let mut watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.blocking_send(res);
            },
            notify::Config::default(),
        )
        .map_err(|e| SymphonyError::IoError(e.to_string()))?;

        if let Some(parent) = wf_path.parent() {
            if parent.exists() {
                let _ = watcher.watch(parent, RecursiveMode::NonRecursive);
            }
        }

        while let Some(Ok(_event)) = rx.recv().await {
            if wf_path.exists() {
                self.reload_workflow_from_disk(&wf_path).await;
            }
        }
        Ok(())
    }

    /// Reload `WORKFLOW.md` dynamically; per SPEC.md Section 6.2, invalid reload keeps last known good config
    pub async fn reload_workflow_from_disk(&self, path: &Path) {
        match WorkflowDefinition::load_from_path(path) {
            Ok(wf) => match ServiceConfig::from_workflow(&wf) {
                Ok(cfg) => {
                    let mut mem = self.memory.write().await;
                    mem.active_workflow = wf;
                    mem.active_config = cfg;
                    mem.active_workflow_path = Some(path.to_path_buf());
                    mem.workflow_error = None;
                    drop(mem);
                    info!(path = %path.display(), "Reloaded WORKFLOW.md configuration");
                    // Lets clients refresh the document (e.g. Workflow Studio's external-change notice).
                    self.emit_event(
                        None,
                        None,
                        None,
                        None,
                        "workflow/reloaded",
                        None,
                        format!("Reloaded workflow from {}", path.display()),
                        None,
                    )
                    .await;
                }
                Err(err) => {
                    warn!(error = %err, "Invalid WORKFLOW.md config on reload; keeping last known good configuration");
                    let mut mem = self.memory.write().await;
                    mem.workflow_error = Some(err.to_string());
                    drop(mem);
                    self.emit_event(
                        None,
                        None,
                        None,
                        None,
                        "workflow/reload_failed",
                        None,
                        err.to_string(),
                        None,
                    )
                    .await;
                }
            },
            Err(err) => {
                warn!(error = %err, "Failed to parse WORKFLOW.md on reload; keeping last known good configuration");
                let mut mem = self.memory.write().await;
                mem.workflow_error = Some(err.to_string());
                drop(mem);
                self.emit_event(
                    None,
                    None,
                    None,
                    None,
                    "workflow/reload_failed",
                    None,
                    err.to_string(),
                    None,
                )
                .await;
            }
        }
    }

    pub async fn trigger_refresh(&self) -> RefreshResponse {
        let coalesced = self.refresh_tx.try_send(()).is_err();
        RefreshResponse {
            queued: true,
            coalesced,
            requested_at: Utc::now(),
            operations: vec!["poll".to_string(), "reconcile".to_string()],
        }
    }

    pub async fn set_scheduler_paused(&self, paused: bool) -> SymphonyResult<()> {
        {
            let mut mem = self.memory.write().await;
            mem.scheduler_paused = paused;
        }
        self.emit_event(
            None,
            None,
            None,
            None,
            if paused {
                "scheduler/paused"
            } else {
                "scheduler/resumed"
            },
            None,
            if paused {
                "Task dispatching paused by operator".to_string()
            } else {
                "Task dispatching resumed by operator".to_string()
            },
            None,
        )
        .await;
        if !paused {
            let _ = self.refresh_tx.try_send(());
        }
        Ok(())
    }

    pub async fn is_scheduler_paused(&self) -> bool {
        self.memory.read().await.scheduler_paused
    }

    pub async fn hardware_snapshot(&self) -> HardwareSnapshot {
        let settings = self.storage.get_settings().await.unwrap_or_default();
        let configured_max = {
            let mem = self.memory.read().await;
            mem.active_config.agent.max_concurrent_agents
        };
        self.hardware
            .snapshot(&settings.resource_policy, configured_max)
    }

    /// Detect installed provider CLIs, their versions, and credential presence.
    /// Results are cached for `PROVIDER_CACHE_TTL_SECS` to avoid spawning CLIs on every request.
    pub async fn provider_statuses(&self, force_refresh: bool) -> Vec<ProviderStatus> {
        if !force_refresh {
            let mem = self.memory.read().await;
            if let Some((at, cached)) = &mem.provider_cache {
                if (Utc::now() - *at).num_seconds() < PROVIDER_CACHE_TTL_SECS {
                    return cached.clone();
                }
            }
        }
        let settings = self.storage.get_settings().await.unwrap_or_default();
        let vault_keys = self.secret_store.list_configured_keys();
        let statuses = detect_provider_statuses(&vault_keys, |kind| {
            settings
                .provider_default_models
                .get(&kind)
                .cloned()
                .or_else(|| {
                    (kind == settings.default_provider && !settings.default_model.is_empty())
                        .then(|| settings.default_model.clone())
                })
        })
        .await;
        self.memory.write().await.provider_cache = Some((Utc::now(), statuses.clone()));
        statuses
    }

    /// Drop cached provider detection (e.g. after credentials or settings change).
    pub async fn invalidate_provider_cache(&self) {
        self.memory.write().await.provider_cache = None;
    }

    /// Configure where the workflow is saved when no `WORKFLOW.md` path is active.
    pub async fn set_workflow_fallback_path(&self, path: PathBuf) {
        self.memory.write().await.workflow_fallback_path = Some(path);
    }

    /// Build the authoritative `/api/v1/state` snapshot (SPEC.md Section 13.7.2)
    pub async fn state_snapshot(&self) -> SymphonyResult<OrchestratorStateSnapshot> {
        let tasks = self.storage.list_tasks(None).await?;
        let hardware = self.hardware_snapshot().await;
        let mem = self.memory.read().await;

        let mut running_summaries: Vec<RunningSessionSummary> = mem
            .running
            .values()
            .map(|h| RunningSessionSummary {
                issue_id: h.session.issue_id.clone(),
                issue_identifier: h.session.issue_identifier.clone(),
                task_id: h.session.task_id.clone(),
                project_id: h.session.project_id.clone(),
                provider: h.session.provider,
                model: h.session.model.clone(),
                state: "In Progress".to_string(),
                phase: h.session.phase,
                session_id: h.session.session_id.clone(),
                workspace_path: h.session.workspace_path.clone(),
                turn_count: h.session.turn_count,
                last_event: h.session.last_event.clone(),
                last_message: h.session.last_message.clone().unwrap_or_default(),
                started_at: h.session.started_at,
                last_event_at: h.session.last_event_at,
                tokens: TokenUsage {
                    input_tokens: h.session.input_tokens,
                    output_tokens: h.session.output_tokens,
                    total_tokens: h.session.total_tokens,
                },
            })
            .collect();
        running_summaries.sort_by(|a, b| a.issue_identifier.cmp(&b.issue_identifier));

        let mut retrying: Vec<RetryEntry> = mem.retry_attempts.values().cloned().collect();
        retrying.sort_by_key(|a| a.due_at);

        let counts = StateCounts {
            running: running_summaries.len(),
            retrying: retrying.len(),
            queued: tasks
                .iter()
                .filter(|t| matches!(t.status, TaskStatus::Ready | TaskStatus::Queued))
                .count(),
            blocked: tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Blocked)
                .count(),
            human_review: tasks
                .iter()
                .filter(|t| t.status == TaskStatus::HumanReview)
                .count(),
            completed: tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Completed)
                .count(),
            failed: tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Failed)
                .count(),
        };

        // Add live running seconds to totals
        let now = Utc::now();
        let active_elapsed: f64 = mem
            .running
            .values()
            .map(|h| (now - h.session.started_at).num_milliseconds().max(0) as f64 / 1000.0)
            .sum();
        let mut totals = mem.codex_totals;
        totals.seconds_running += active_elapsed;

        Ok(OrchestratorStateSnapshot {
            generated_at: now,
            scheduler_paused: mem.scheduler_paused,
            counts,
            running: running_summaries,
            retrying,
            codex_totals: totals,
            rate_limits: mem.latest_rate_limits.clone(),
            hardware,
            active_workflow_path: mem
                .active_workflow_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            workflow_error: mem.workflow_error.clone(),
        })
    }

    /// Build `/api/v1/<issue_identifier>` detail response (SPEC.md Section 13.7.2)
    pub async fn issue_detail(
        &self,
        issue_identifier: &str,
    ) -> SymphonyResult<IssueDetailResponse> {
        let task = self
            .storage
            .get_task_by_identifier(issue_identifier)
            .await?
            .ok_or_else(|| SymphonyError::IssueNotFound(issue_identifier.to_string()))?;

        let recent_events = self.storage.list_events(Some(&task.id), 100).await?;
        let validation_results = self.storage.list_validation_results(&task.id).await?;
        let pr_reviews = self.storage.list_pr_reviews(Some(&task.project_id)).await?;
        let pull_request = pr_reviews.into_iter().find(|r| r.task_id == task.id);

        let mem = self.memory.read().await;
        let running = mem.running.get(&task.id).map(|h| h.session.clone());
        let retry = mem.retry_attempts.get(&task.id).cloned();

        let status_label = if running.is_some() {
            "running"
        } else if retry.is_some() {
            "retrying"
        } else {
            task.status.as_str()
        };

        Ok(IssueDetailResponse {
            issue_identifier: task.issue.identifier.clone(),
            issue_id: task.issue.id.clone(),
            task_id: task.id.clone(),
            status: status_label.to_string(),
            task_status: task.status,
            provider: task.provider,
            workspace: WorkspaceInfo {
                path: task.workspace_path.clone(),
                branch_name: task.branch_name.clone(),
            },
            attempts: AttemptSummary {
                restart_count: task.attempt_count.saturating_sub(1),
                current_retry_attempt: retry
                    .as_ref()
                    .map(|r| r.attempt)
                    .unwrap_or(task.attempt_count),
            },
            running,
            retry,
            recent_events,
            last_error: task.last_error.clone(),
            validation_results,
            pull_request,
            tracked: HashMap::new(),
            task,
        })
    }

    // =========================================================================
    // Project & Task Operations
    // =========================================================================

    pub async fn create_project(&self, project: Project) -> SymphonyResult<Project> {
        if project.workspace_root.trim().is_empty() {
            return Err(SymphonyError::InvalidConfig(
                "workspace_root must not be empty".to_string(),
            ));
        }
        std::fs::create_dir_all(&project.workspace_root)?;
        self.storage.upsert_project(&project).await?;

        self.emit_event(
            None,
            None,
            None,
            None,
            "project/registered",
            None,
            format!(
                "Registered project '{}' ({})",
                project.name, project.repo_path
            ),
            None,
        )
        .await;
        Ok(project)
    }

    pub async fn list_projects(&self) -> SymphonyResult<Vec<Project>> {
        self.storage.list_projects().await
    }

    pub async fn delete_project(&self, id: &str) -> SymphonyResult<()> {
        let project = self
            .storage
            .get_project(id)
            .await?
            .ok_or_else(|| SymphonyError::ProjectNotFound(id.to_string()))?;
        let running = {
            let mem = self.memory.read().await;
            mem.running
                .values()
                .any(|h| h.session.project_id == project.id)
        };
        if running {
            return Err(SymphonyError::Conflict(format!(
                "Project '{}' has running agent sessions; cancel them before removing the project",
                project.name
            )));
        }
        self.storage.delete_project(id).await?;
        self.emit_event(
            None,
            None,
            None,
            None,
            "project/removed",
            None,
            format!("Removed project '{}'", project.name),
            None,
        )
        .await;
        Ok(())
    }

    /// Apply a partial update to a registered project.
    pub async fn update_project(&self, id: &str, update: ProjectUpdate) -> SymphonyResult<Project> {
        let mut project = self
            .storage
            .get_project(id)
            .await?
            .ok_or_else(|| SymphonyError::ProjectNotFound(id.to_string()))?;
        if let Some(name) = update.name {
            if name.trim().is_empty() {
                return Err(SymphonyError::InvalidConfig(
                    "project name must not be empty".into(),
                ));
            }
            project.name = name.trim().to_string();
        }
        if let Some(branch) = update.default_branch {
            if branch.trim().is_empty() {
                return Err(SymphonyError::InvalidConfig(
                    "default_branch must not be empty".into(),
                ));
            }
            project.default_branch = branch.trim().to_string();
        }
        if let Some(p) = update.preferred_provider {
            project.preferred_provider = p;
        }
        if let Some(m) = update.preferred_model {
            project.preferred_model = Some(m).filter(|v| !v.trim().is_empty());
        }
        if let Some(k) = update.tracker_kind {
            project.tracker_kind = k;
        }
        if let Some(r) = update.github_repo {
            project.github_repo = Some(r).filter(|v| !v.trim().is_empty());
        }
        if let Some(l) = update.linear_project_slug {
            project.linear_project_slug = Some(l).filter(|v| !v.trim().is_empty());
        }
        if let Some(labels) = update.required_labels {
            project.required_labels = labels;
        }
        if let Some(auto) = update.auto_dispatch {
            project.auto_dispatch = auto;
        }
        if let Some(max) = update.max_concurrent_agents {
            if max == 0 || max > 32 {
                return Err(SymphonyError::InvalidConfig(
                    "max_concurrent_agents must be between 1 and 32".into(),
                ));
            }
            project.max_concurrent_agents = max;
        }
        if let Some(cmds) = update.validation_commands {
            project.validation_commands = cmds
                .into_iter()
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect();
        }
        project.updated_at = Utc::now();
        self.storage.upsert_project(&project).await?;
        self.emit_event(
            None,
            None,
            None,
            None,
            "project/updated",
            None,
            format!("Updated project '{}'", project.name),
            None,
        )
        .await;
        Ok(project)
    }

    /// Inspect a local path before registering it as a project repository.
    pub async fn validate_repository(&self, repo_path: &str) -> RepositoryValidation {
        let path = PathBuf::from(repo_path.trim());
        let mut errors = Vec::new();
        let exists = !repo_path.trim().is_empty() && path.exists();
        let is_directory = exists && path.is_dir();
        if repo_path.trim().is_empty() {
            errors.push("Repository path is required.".to_string());
        } else if !exists {
            errors.push("The selected path does not exist.".to_string());
        } else if !is_directory {
            errors.push("The selected path is not a directory.".to_string());
        }
        let mut is_git_repository = false;
        let mut current_branch = None;
        let mut resolved_path = repo_path.trim().to_string();
        if is_directory {
            // Resolve the checkout root: a subdirectory of a repository is inside the work tree,
            // but worktrees are created from the root (which holds `.git`).
            let out = tokio::process::Command::new("git")
                .arg("-C")
                .arg(&path)
                .args(["rev-parse", "--show-toplevel"])
                .output()
                .await;
            if let Ok(o) = &out {
                let top = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if o.status.success() && !top.is_empty() {
                    is_git_repository = true;
                    resolved_path = PathBuf::from(&top).to_string_lossy().into_owned();
                }
            }
            if is_git_repository {
                if let Ok(o) = tokio::process::Command::new("git")
                    .arg("-C")
                    .arg(&path)
                    .args(["symbolic-ref", "--short", "HEAD"])
                    .output()
                    .await
                {
                    if o.status.success() {
                        current_branch =
                            Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                                .filter(|b| !b.is_empty());
                    }
                }
            } else {
                errors.push("The directory is not a Git repository (git rev-parse failed).".into());
            }
        }
        // Workflow presence is checked at the checkout root the project will actually use.
        let has_workflow_file =
            is_directory && PathBuf::from(&resolved_path).join("WORKFLOW.md").is_file();
        RepositoryValidation {
            repo_path: resolved_path,
            exists,
            is_directory,
            is_git_repository,
            current_branch,
            has_workflow_file,
            suggested_name: path.file_name().map(|n| n.to_string_lossy().to_string()),
            errors,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_manual_task(
        &self,
        project_id: String,
        identifier: Option<String>,
        title: String,
        description: Option<String>,
        priority: Option<i32>,
        provider: Option<ProviderKind>,
        model: Option<String>,
        labels: Vec<String>,
        auto_start: bool,
    ) -> SymphonyResult<Task> {
        let project = self
            .storage
            .get_project(&project_id)
            .await?
            .ok_or_else(|| SymphonyError::ProjectNotFound(project_id.clone()))?;

        let existing = self.storage.list_tasks(Some(&project_id)).await?;
        let resolved_identifier = identifier
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("SYM-{}", existing.len() + 1));

        let mut task = Task::new_manual(
            project_id,
            resolved_identifier,
            title,
            description,
            priority,
            provider.unwrap_or(project.preferred_provider),
            model.or(project.preferred_model.clone()),
            labels,
        );
        if auto_start {
            task.status = TaskStatus::Queued;
        }

        self.storage.upsert_task(&task).await?;
        self.emit_event(
            Some(task.id.clone()),
            Some(task.issue.identifier.clone()),
            None,
            Some(task.provider),
            "task/created",
            None,
            format!(
                "Created task {}: {}",
                task.issue.identifier, task.issue.title
            ),
            None,
        )
        .await;

        if auto_start {
            let _ = self.dispatch_task_now(&task.id).await;
            return self
                .storage
                .get_task(&task.id)
                .await?
                .ok_or(SymphonyError::TaskNotFound(task.id));
        }

        Ok(task)
    }

    pub async fn list_tasks(&self, project_id: Option<&str>) -> SymphonyResult<Vec<Task>> {
        self.storage.list_tasks(project_id).await
    }

    pub async fn get_task(&self, task_id: &str) -> SymphonyResult<Task> {
        self.storage
            .get_task(task_id)
            .await?
            .ok_or_else(|| SymphonyError::TaskNotFound(task_id.to_string()))
    }

    pub async fn transition_task(&self, task_id: &str, next: TaskStatus) -> SymphonyResult<Task> {
        let mut task = self.get_task(task_id).await?;
        if next == TaskStatus::Running && task.status != TaskStatus::Running {
            return Err(SymphonyError::InvalidStateTransition {
                from: task.status.as_str().to_string(),
                to: "running (use dispatch to start an agent)".to_string(),
            });
        }
        let is_active = self.memory.read().await.running.contains_key(task_id);
        if is_active && next != TaskStatus::Running {
            return Err(SymphonyError::Conflict(format!(
                "Task {} has an active agent session; cancel it before changing status",
                task.issue.identifier
            )));
        }
        task.transition_to(next)?;
        self.storage.upsert_task(&task).await?;

        self.emit_event(
            Some(task.id.clone()),
            Some(task.issue.identifier.clone()),
            None,
            Some(task.provider),
            "task/status_changed",
            None,
            format!(
                "Task {} moved to {}",
                task.issue.identifier,
                task.status.as_str()
            ),
            None,
        )
        .await;

        if next == TaskStatus::Queued {
            let _ = self.refresh_tx.try_send(());
        }
        Ok(task)
    }

    pub async fn cancel_task(&self, task_id: &str, reason: &str) -> SymphonyResult<Task> {
        let current = self.get_task(task_id).await?;
        current.status.validate_transition(TaskStatus::Cancelled)?;
        let cancel_tx = {
            let mut mem = self.memory.write().await;
            mem.retry_attempts.remove(task_id);
            mem.running.remove(task_id).map(|h| h.cancel_tx)
        };
        if let Some(tx) = cancel_tx {
            let _ = tx.send(reason.to_string());
        }

        self.expire_pending_approvals(task_id).await;

        let mut task = self.get_task(task_id).await?;
        task.status = TaskStatus::Cancelled;
        task.current_phase = Some(RunAttemptPhase::CanceledByReconciliation);
        task.current_session_id = None;
        task.last_error = Some(reason.to_string());
        task.updated_at = Utc::now();
        self.storage.upsert_task(&task).await?;

        {
            let mut mem = self.memory.write().await;
            mem.claimed_issue_ids.remove(&task.issue.id);
        }

        self.emit_event(
            Some(task.id.clone()),
            Some(task.issue.identifier.clone()),
            None,
            Some(task.provider),
            "task/cancelled",
            Some(RunAttemptPhase::CanceledByReconciliation),
            format!("Task {} cancelled: {}", task.issue.identifier, reason),
            None,
        )
        .await;

        Ok(task)
    }

    pub async fn retry_task(&self, task_id: &str) -> SymphonyResult<Task> {
        let mut task = self.get_task(task_id).await?;
        if self.memory.read().await.running.contains_key(task_id) {
            return Err(SymphonyError::Conflict(format!(
                "Task {} is already running",
                task.issue.identifier
            )));
        }
        task.status.validate_transition(TaskStatus::Queued)?;
        task.status = TaskStatus::Queued;
        task.last_error = None;
        task.next_retry_at = None;
        task.updated_at = Utc::now();
        self.storage.upsert_task(&task).await?;
        self.dispatch_task_now(task_id).await
    }

    pub async fn resolve_approval(
        &self,
        approval_id: &str,
        decision: ApprovalDecision,
    ) -> SymphonyResult<ApprovalRequest> {
        let approvals = self.storage.list_approvals(false).await?;
        let mut req = approvals
            .into_iter()
            .find(|a| a.id == approval_id)
            .ok_or_else(|| SymphonyError::NotFound(format!("Approval {approval_id} not found")))?;
        if req.status != ApprovalStatus::Pending {
            return Err(SymphonyError::Conflict(format!(
                "Approval {approval_id} was already resolved"
            )));
        }

        req.decision = Some(decision);
        req.resolved_at = Some(Utc::now());
        req.status = match decision {
            ApprovalDecision::Accept | ApprovalDecision::AcceptForSession => {
                ApprovalStatus::Approved
            }
            ApprovalDecision::Decline => ApprovalStatus::Denied,
            ApprovalDecision::Cancel => ApprovalStatus::Cancelled,
        };
        self.storage.upsert_approval(&req).await?;

        let tx_opt = {
            let mut mem = self.memory.write().await;
            mem.pending_approvals.remove(approval_id)
        };
        if let Some(tx) = tx_opt {
            let _ = tx.send(decision);
        }

        self.emit_event(
            Some(req.task_id.clone()),
            Some(req.issue_identifier.clone()),
            Some(req.session_id.clone()),
            Some(req.provider),
            "approval/resolved",
            None,
            format!("Approval {} resolved with {:?}", req.title, decision),
            None,
        )
        .await;

        Ok(req)
    }

    /// Mark approvals still pending for a task as cancelled (its run ended without a decision), so
    /// they never linger as actionable in the UI.
    async fn expire_pending_approvals(&self, task_id: &str) {
        let Ok(pending) = self.storage.list_approvals(true).await else {
            return;
        };
        for mut req in pending.into_iter().filter(|a| a.task_id == task_id) {
            {
                let mut mem = self.memory.write().await;
                mem.pending_approvals.remove(&req.id);
            }
            req.status = ApprovalStatus::Cancelled;
            req.decision = Some(ApprovalDecision::Cancel);
            req.resolved_at = Some(Utc::now());
            let _ = self.storage.upsert_approval(&req).await;
            self.emit_event(
                Some(req.task_id.clone()),
                Some(req.issue_identifier.clone()),
                Some(req.session_id.clone()),
                Some(req.provider),
                "approval/expired",
                None,
                format!("Approval '{}' expired because the run ended", req.title),
                None,
            )
            .await;
        }
    }

    pub async fn list_approvals(&self, only_pending: bool) -> SymphonyResult<Vec<ApprovalRequest>> {
        self.storage.list_approvals(only_pending).await
    }

    pub async fn list_events(
        &self,
        task_id: Option<&str>,
        limit: u32,
    ) -> SymphonyResult<Vec<AgentEvent>> {
        self.storage.list_events(task_id, limit).await
    }

    pub async fn list_run_attempts(&self, task_id: &str) -> SymphonyResult<Vec<RunAttempt>> {
        self.storage.list_run_attempts(task_id).await
    }

    pub async fn list_validation_results(
        &self,
        task_id: &str,
    ) -> SymphonyResult<Vec<ValidationResult>> {
        self.storage.list_validation_results(task_id).await
    }

    pub async fn list_pr_reviews(
        &self,
        project_id: Option<&str>,
    ) -> SymphonyResult<Vec<PullRequestReview>> {
        self.storage.list_pr_reviews(project_id).await
    }

    pub async fn get_pr_review(&self, review_id: &str) -> SymphonyResult<PullRequestReview> {
        self.storage
            .get_pr_review(review_id)
            .await?
            .ok_or_else(|| SymphonyError::NotFound(format!("PR review {review_id} not found")))
    }

    /// Git worktrees allocated for tasks, with on-disk and dirty status.
    pub async fn list_worktrees(&self) -> SymphonyResult<Vec<WorktreeInfo>> {
        let tasks = self.storage.list_tasks(None).await?;
        let projects: HashMap<String, Project> = self
            .storage
            .list_projects()
            .await?
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect();
        let active: HashMap<String, String> = {
            let mem = self.memory.read().await;
            mem.running
                .iter()
                .map(|(id, h)| (id.clone(), h.session.session_id.clone()))
                .collect()
        };
        let mut out = Vec::new();
        for task in tasks {
            let Some(ws) = task.workspace_path.clone().filter(|p| !p.is_empty()) else {
                continue;
            };
            let path = PathBuf::from(&ws);
            let exists = path.is_dir();
            let dirty = exists && self.workspaces.is_worktree_dirty(&path).await;
            let project = projects.get(&task.project_id);
            out.push(WorktreeInfo {
                task_id: task.id.clone(),
                project_id: task.project_id.clone(),
                project_name: project.map(|p| p.name.clone()),
                repo_path: project.map(|p| p.repo_path.clone()),
                issue_identifier: task.issue.identifier.clone(),
                issue_title: task.issue.title.clone(),
                task_status: task.status,
                provider: task.provider,
                workspace_path: ws,
                branch_name: task.branch_name.clone(),
                exists,
                dirty,
                active_session_id: active.get(&task.id).cloned(),
                last_activity_at: task.updated_at,
            });
        }
        out.sort_by_key(|w| std::cmp::Reverse(w.last_activity_at));
        Ok(out)
    }

    /// Validated workspace path for a task: it must lie inside its project's workspace root.
    pub async fn task_workspace_path(&self, task_id: &str) -> SymphonyResult<PathBuf> {
        let task = self.get_task(task_id).await?;
        let project = self
            .storage
            .get_project(&task.project_id)
            .await?
            .ok_or_else(|| SymphonyError::ProjectNotFound(task.project_id.clone()))?;
        let ws = task
            .workspace_path
            .filter(|p| !p.is_empty())
            .ok_or_else(|| SymphonyError::NotFound(format!("Task {task_id} has no workspace")))?;
        symphony_workspaces::validate_workspace_path(
            Path::new(&ws),
            Path::new(&project.workspace_root),
        )
    }

    /// Changed files and unified diff of a task's worktree.
    pub async fn task_diff(&self, task_id: &str) -> SymphonyResult<TaskDiff> {
        let task = self.get_task(task_id).await?;
        let validated = match self.task_workspace_path(task_id).await {
            Ok(p) => Some(p),
            Err(SymphonyError::NotFound(_)) => None,
            Err(e)
                if task.workspace_path.is_some()
                    && !Path::new(task.workspace_path.as_deref().unwrap_or_default()).exists() =>
            {
                debug!(error = %e, "workspace no longer on disk");
                None
            }
            Err(e) => return Err(e),
        };
        let Some(path) = validated else {
            return Ok(TaskDiff {
                task_id: task.id,
                workspace_path: task.workspace_path,
                branch_name: task.branch_name,
                exists: false,
                changed_files: Vec::new(),
                diff: String::new(),
                truncated: false,
            });
        };
        let changed_files = self.workspaces.inspect_changed_files(&path).await?;
        let (diff, truncated) = self
            .workspaces
            .collect_diff(&path, TASK_DIFF_MAX_BYTES)
            .await;
        Ok(TaskDiff {
            task_id: task.id,
            workspace_path: Some(path.to_string_lossy().to_string()),
            branch_name: task.branch_name,
            exists: true,
            changed_files,
            diff: redact_secrets(&diff, &self.secret_store.all_secret_values()),
            truncated,
        })
    }

    /// Explicitly approve a Pull Request review for human merge completion (Section 4.F)
    pub async fn approve_pr_merge(&self, review_id: &str) -> SymphonyResult<PullRequestReview> {
        let mut review = self
            .storage
            .get_pr_review(review_id)
            .await?
            .ok_or_else(|| SymphonyError::NotFound(format!("PR review {review_id} not found")))?;
        // Idempotent and retry-safe: a repeated call (e.g. after the task write failed) still
        // completes the task, but approval is recorded and announced only once.
        let newly_approved = !review.human_approved;
        if newly_approved {
            review.human_approved = true;
            review.state = "approved_for_merge".to_string();
            review.updated_at = Utc::now();
            self.storage.upsert_pr_review(&review).await?;
        }

        if let Ok(mut task) = self.get_task(&review.task_id).await {
            // Only a task waiting for human review is completed by an approval; a task that was
            // re-dispatched, cancelled or already completed keeps its state.
            let completes_task = task.status == TaskStatus::HumanReview;
            if completes_task {
                task.status = TaskStatus::Completed;
                task.completed_at = Some(Utc::now());
                task.updated_at = Utc::now();
                self.storage.upsert_task(&task).await?;
            }
            if newly_approved || completes_task {
                self.emit_event(
                    Some(task.id),
                    Some(task.issue.identifier),
                    None,
                    Some(task.provider),
                    "pr/human_approved",
                    Some(RunAttemptPhase::Succeeded),
                    format!("Human approved PR #{} ({})", review.pr_number, review.title),
                    None,
                )
                .await;
            }
        }

        Ok(review)
    }

    /// Path the workflow document is saved to: the active `WORKFLOW.md`, else the configured fallback.
    pub async fn workflow_target_path(&self) -> Option<PathBuf> {
        let mem = self.memory.read().await;
        mem.active_workflow_path
            .clone()
            .or_else(|| mem.workflow_fallback_path.clone())
    }

    /// Currently active workflow source (full front matter + body), its path, and parsed config.
    pub async fn get_workflow_document(&self) -> WorkflowDocument {
        let (wf, path, error) = {
            let mem = self.memory.read().await;
            (
                mem.active_workflow.clone(),
                mem.active_workflow_path
                    .clone()
                    .or_else(|| mem.workflow_fallback_path.clone()),
                mem.workflow_error.clone(),
            )
        };
        // Prefer the on-disk text so an invalid edit made outside the app is still visible.
        let raw_markdown = path
            .as_ref()
            .filter(|p| p.is_file())
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_else(|| wf.raw_source.clone());
        let validation =
            Self::validate_workflow_text(&raw_markdown, path.clone(), None, None, None, None);
        WorkflowDocument {
            path: path.map(|p| p.to_string_lossy().to_string()),
            raw_markdown,
            valid: validation.valid && error.is_none(),
            error: validation.error.or(error),
            config: validation.config,
            rendered_preview: validation.rendered_prompt,
        }
    }

    /// Parse, validate, and render a workflow document without persisting it.
    /// `source_path` is the file the text would be saved to; relative paths in the config
    /// (e.g. `workspace.root`) are resolved against it exactly as they are after saving.
    pub fn validate_workflow_text(
        raw: &str,
        source_path: Option<PathBuf>,
        sample_identifier: Option<String>,
        sample_title: Option<String>,
        sample_description: Option<String>,
        attempt: Option<u32>,
    ) -> WorkflowValidation {
        let fail = |e: SymphonyError| WorkflowValidation {
            valid: false,
            error_code: Some(e.code().to_string()),
            error: Some(e.to_string()),
            config: None,
            rendered_prompt: None,
            path: None,
        };
        let wf = match WorkflowDefinition::parse(raw, source_path) {
            Ok(w) => w,
            Err(e) => return fail(e),
        };
        let cfg = match ServiceConfig::from_workflow(&wf) {
            Ok(c) => c,
            Err(e) => return fail(e),
        };
        let sample_issue = Issue {
            id: "sample-1".into(),
            native_ref: None,
            identifier: sample_identifier.unwrap_or_else(|| "SYM-101".into()),
            title: sample_title
                .unwrap_or_else(|| "Add resilient retry backoff to worker pool".into()),
            description: Some(sample_description.unwrap_or_else(|| {
                "Ensure exponential backoff caps at max_retry_backoff_ms and emits structured telemetry."
                    .into()
            })),
            priority: Some(1),
            state: "Todo".into(),
            branch_name: Some("symphony/sym-101".into()),
            url: Some("https://github.com/openai/symphony/issues/101".into()),
            assignee_id: None,
            labels: vec!["symphony".into(), "rust".into()],
            blocked_by: Vec::new(),
            dispatchable: true,
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };
        match render_workflow_prompt(&wf, &sample_issue, attempt.or(Some(1))) {
            Ok(rendered) => WorkflowValidation {
                valid: true,
                error_code: None,
                error: None,
                config: Some(cfg.redacted_for_display()),
                rendered_prompt: Some(rendered),
                path: None,
            },
            Err(e) => WorkflowValidation {
                config: Some(cfg.redacted_for_display()),
                ..fail(e)
            },
        }
    }

    /// Validate and persist the workflow document to the active workflow path (or fallback).
    /// Invalid documents are rejected and never written.
    pub async fn save_workflow_document(
        &self,
        raw_content: &str,
    ) -> SymphonyResult<WorkflowValidation> {
        let path = self.workflow_target_path().await.ok_or_else(|| {
            SymphonyError::InvalidConfig(
                "No workflow file location is configured for this runtime".to_string(),
            )
        })?;
        let wf = WorkflowDefinition::parse(raw_content, Some(path.clone()))?;
        let cfg = ServiceConfig::from_workflow(&wf)?;
        cfg.validate_for_dispatch()?;
        let validation =
            Self::validate_workflow_text(raw_content, Some(path.clone()), None, None, None, None);
        if !validation.valid {
            return Err(SymphonyError::TemplateRenderError(
                validation.error.unwrap_or_default(),
            ));
        }

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(&path, raw_content)?;

        let was_watched = {
            let mut mem = self.memory.write().await;
            let was_watched = mem.active_workflow_path.is_some();
            mem.active_workflow = wf;
            mem.active_config = cfg;
            mem.active_workflow_path = Some(path.clone());
            mem.workflow_error = None;
            was_watched
        };
        if !was_watched {
            // First save to the fallback location: start watching it so later external edits
            // reload the configuration like a workflow that existed at startup.
            let orch = self.clone();
            let watched = path.clone();
            tokio::spawn(async move {
                let _ = orch.watch_workflow_file(watched).await;
            });
        }
        self.emit_event(
            None,
            None,
            None,
            None,
            "workflow/saved",
            None,
            format!("Saved workflow to {}", path.display()),
            None,
        )
        .await;
        Ok(WorkflowValidation {
            path: Some(path.to_string_lossy().to_string()),
            ..validation
        })
    }

    // =========================================================================
    // Tracker Synchronization & Reconciliation Loop (SPEC.md Sections 8 & 16)
    // =========================================================================

    /// GitHub token for host-side tracker calls against api.github.com: every vault alias first,
    /// then the environment, skipping empty values. `GITHUB_ENTERPRISE_TOKEN` is deliberately not
    /// used: the tracker talks to the public API, and an Enterprise token must not be sent there.
    fn github_token(&self) -> Option<String> {
        const ALIASES: [&str; 2] = ["GITHUB_TOKEN", "GH_TOKEN"];
        let usable = |v: String| (!v.trim().is_empty()).then_some(v);
        ALIASES
            .iter()
            .find_map(|key| self.secret_store.get_vault_secret(key).and_then(usable))
            .or_else(|| {
                ALIASES
                    .iter()
                    .find_map(|key| std::env::var(key).ok().and_then(usable))
            })
    }

    /// Provider API keys stored in the vault, passed to agent subprocesses explicitly so they work
    /// after a restart without exporting them into the runtime's own environment. Tracker tokens
    /// are never included (they stay host-side for dynamic tools).
    fn provider_env_from_vault(&self) -> HashMap<String, String> {
        PROVIDER_CREDENTIAL_ENV_KEYS
            .iter()
            .filter_map(|key| {
                self.secret_store
                    .get_secret(key)
                    .filter(|v| !v.trim().is_empty())
                    .map(|v| ((*key).to_string(), v))
            })
            .collect()
    }

    fn build_tracker_for_project(&self, project: &Project) -> Option<Box<dyn TrackerAdapter>> {
        match project.tracker_kind.to_lowercase().as_str() {
            "github" => {
                let repo = project.github_repo.clone()?;
                let token = self.github_token();
                Some(Box::new(GitHubTracker::new(None, repo, token, None)))
            }
            "linear" => {
                let slug = project.linear_project_slug.clone()?;
                let key = self
                    .secret_store
                    .get_secret("LINEAR_API_KEY")
                    .or_else(|| std::env::var("LINEAR_API_KEY").ok())?;
                Some(Box::new(LinearTracker::new(
                    None,
                    slug,
                    Some(key),
                    vec!["Done".into(), "Closed".into(), "Cancelled".into()],
                )))
            }
            _ => None,
        }
    }

    pub async fn sync_project_issues(&self, project_id: &str) -> SymphonyResult<Vec<Task>> {
        let project = self
            .storage
            .get_project(project_id)
            .await?
            .ok_or_else(|| SymphonyError::ProjectNotFound(project_id.to_string()))?;

        let Some(tracker) = self.build_tracker_for_project(&project) else {
            return self.storage.list_tasks(Some(project_id)).await;
        };

        let active_states = {
            let mem = self.memory.read().await;
            mem.active_config.active_states.clone()
        };

        let issues = tracker
            .fetch_candidate_issues(&active_states, &project.required_labels)
            .await?;

        let existing = self.storage.list_tasks(Some(project_id)).await?;
        let mut by_identifier: HashMap<String, Task> = existing
            .into_iter()
            .map(|t| (t.issue.identifier.clone(), t))
            .collect();

        for issue in issues {
            if let Some(existing_task) = by_identifier.get_mut(&issue.identifier) {
                existing_task.issue = issue;
                existing_task.updated_at = Utc::now();
                self.storage.upsert_task(existing_task).await?;
            } else {
                let now = Utc::now();
                let new_task = Task {
                    id: Uuid::new_v4().to_string(),
                    project_id: project.id.clone(),
                    status: if project.auto_dispatch {
                        TaskStatus::Queued
                    } else {
                        TaskStatus::Ready
                    },
                    provider: project.preferred_provider,
                    model: project.preferred_model.clone(),
                    workspace_path: None,
                    branch_name: None,
                    attempt_count: 0,
                    current_session_id: None,
                    current_thread_id: None,
                    current_turn_id: None,
                    current_phase: None,
                    pr_number: None,
                    pr_url: None,
                    input_tokens: 0,
                    output_tokens: 0,
                    total_tokens: 0,
                    estimated_cost_usd: 0.0,
                    duration_seconds: 0.0,
                    last_error: None,
                    next_retry_at: None,
                    created_at: now,
                    updated_at: now,
                    started_at: None,
                    completed_at: None,
                    issue,
                };
                self.storage.upsert_task(&new_task).await?;
            }
        }

        self.storage.list_tasks(Some(project_id)).await
    }

    async fn startup_terminal_workspace_cleanup(&self) -> SymphonyResult<()> {
        let projects = self.storage.list_projects().await?;
        let (terminal_states, hooks) = {
            let mem = self.memory.read().await;
            (
                mem.active_config.terminal_states.clone(),
                mem.active_config.hooks.clone(),
            )
        };

        for project in projects {
            if let Some(tracker) = self.build_tracker_for_project(&project) {
                if let Ok(terminal_issues) = tracker.fetch_issues_by_states(&terminal_states).await
                {
                    let root = PathBuf::from(&project.workspace_root);
                    let repo = PathBuf::from(&project.repo_path);
                    for issue in terminal_issues {
                        let key =
                            symphony_workspaces::workspace_key_for_identifier(&issue.identifier);
                        let ws_path = root.join(key);
                        if ws_path.exists() {
                            let _ = self
                                .workspaces
                                .cleanup_workspace(&ws_path, &root, Some(&repo), &hooks, true)
                                .await;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Main poll + reconcile + dispatch tick (SPEC.md Section 8 & Section 16.2)
    pub async fn reconcile_and_dispatch_tick(&self) -> SymphonyResult<()> {
        // 1. Reconcile active runs (Part A: Stall detection, Part B: Tracker state refresh)
        self.reconcile_active_runs().await?;

        // 2. Process due retries
        self.process_due_retries().await?;

        // 3. Check if scheduler is paused or delayed by hardware pressure
        if self.is_scheduler_paused().await {
            return Ok(());
        }

        let hw = self.hardware_snapshot().await;
        if hw.effective_max_concurrency == 0 {
            debug!(
                reason = ?hw.dispatch_delayed_reason,
                "Delaying new agent dispatches due to host resource pressure"
            );
            return Ok(());
        }

        let running_count = self.memory.read().await.running.len() as u32;
        if running_count >= hw.effective_max_concurrency {
            return Ok(());
        }

        // 4. Poll auto-dispatch projects
        let projects = self.storage.list_projects().await?;
        for project in &projects {
            if project.auto_dispatch {
                let _ = self.sync_project_issues(&project.id).await;
            }
        }

        // 5. Select eligible candidate tasks sorted by priority, created_at, identifier (SPEC.md Section 8.2)
        let all_tasks = self.storage.list_tasks(None).await?;
        let mut eligible_tasks: Vec<Task> = Vec::new();
        {
            let mem = self.memory.read().await;
            for task in all_tasks {
                let is_auto_project = projects
                    .iter()
                    .find(|p| p.id == task.project_id)
                    .map(|p| p.auto_dispatch)
                    .unwrap_or(false);

                let status_eligible = task.status == TaskStatus::Queued
                    || (task.status == TaskStatus::Ready && is_auto_project);

                if status_eligible
                    && task.issue.dispatchable
                    && !task
                        .issue
                        .is_todo_blocked(&mem.active_config.terminal_states)
                    && !mem.claimed_issue_ids.contains(&task.issue.id)
                    && !mem.completed_issue_ids.contains(&task.issue.id)
                    && !mem.running.contains_key(&task.id)
                {
                    eligible_tasks.push(task);
                }
            }
        }

        // Sort issues per SPEC.md Section 8.2
        let mut issues: Vec<Issue> = eligible_tasks.iter().map(|t| t.issue.clone()).collect();
        sort_candidate_issues(&mut issues);

        let available_slots = hw.effective_max_concurrency.saturating_sub(running_count) as usize;
        for issue in issues.into_iter().take(available_slots) {
            if let Some(task) = eligible_tasks.iter().find(|t| t.issue.id == issue.id) {
                let _ = self.dispatch_task_now(&task.id).await;
            }
        }

        Ok(())
    }

    async fn reconcile_active_runs(&self) -> SymphonyResult<()> {
        let (stall_timeout_ms, active_states, terminal_states, hooks) = {
            let mem = self.memory.read().await;
            (
                mem.active_config.codex.stall_timeout_ms,
                mem.active_config.active_states.clone(),
                mem.active_config.terminal_states.clone(),
                mem.active_config.hooks.clone(),
            )
        };

        // Part A: Stall detection
        if stall_timeout_ms > 0 {
            let now = Utc::now();
            let mut stalled_task_ids = Vec::new();
            {
                let mem = self.memory.read().await;
                for (task_id, handle) in &mem.running {
                    let ref_time = handle
                        .session
                        .last_event_at
                        .unwrap_or(handle.session.started_at);
                    let elapsed_ms = (now - ref_time).num_milliseconds();
                    if elapsed_ms > stall_timeout_ms {
                        stalled_task_ids.push(task_id.clone());
                    }
                }
            }

            for task_id in stalled_task_ids {
                warn!(task_id = %task_id, "Detected stalled agent session; cancelling and scheduling retry");
                let cancel_tx = {
                    let mut mem = self.memory.write().await;
                    mem.running.remove(&task_id).map(|h| h.cancel_tx)
                };
                if let Some(tx) = cancel_tx {
                    let _ = tx.send("stall_timeout".to_string());
                }
                self.schedule_retry(&task_id, false, Some("Stall timeout exceeded".to_string()))
                    .await?;
            }
        }

        // Part B: Tracker issue state reconciliation
        let running_snapshots: Vec<(String, String, String)> = {
            let mem = self.memory.read().await;
            mem.running
                .values()
                .map(|h| {
                    (
                        h.session.task_id.clone(),
                        h.session.project_id.clone(),
                        h.session.issue_id.clone(),
                    )
                })
                .collect()
        };

        for (task_id, project_id, issue_id) in running_snapshots {
            if let Ok(Some(project)) = self.storage.get_project(&project_id).await {
                if let Some(tracker) = self.build_tracker_for_project(&project) {
                    if let Ok(refreshed) = tracker
                        .fetch_issue_states_by_ids(std::slice::from_ref(&issue_id))
                        .await
                    {
                        if let Some(issue) = refreshed.into_iter().next() {
                            if issue.is_in_states(&terminal_states) {
                                let _ = self
                                    .cancel_task(&task_id, "Tracker issue moved to terminal state")
                                    .await;
                                if let Ok(task) = self.get_task(&task_id).await {
                                    if let Some(ws) = task.workspace_path {
                                        let _ = self
                                            .workspaces
                                            .cleanup_workspace(
                                                Path::new(&ws),
                                                Path::new(&project.workspace_root),
                                                Some(Path::new(&project.repo_path)),
                                                &hooks,
                                                true,
                                            )
                                            .await;
                                    }
                                }
                            } else if !issue.is_in_states(&active_states) || !issue.dispatchable {
                                let _ = self
                                    .cancel_task(
                                        &task_id,
                                        "Tracker issue is no longer in an active routable state",
                                    )
                                    .await;
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn process_due_retries(&self) -> SymphonyResult<()> {
        let now = Utc::now();
        let due_task_ids: Vec<String> = {
            let mem = self.memory.read().await;
            mem.retry_attempts
                .values()
                .filter(|r| r.due_at <= now)
                .map(|r| r.task_id.clone())
                .collect()
        };

        for task_id in due_task_ids {
            {
                let mut mem = self.memory.write().await;
                mem.retry_attempts.remove(&task_id);
            }
            let _ = self.dispatch_task_now(&task_id).await;
        }
        Ok(())
    }

    pub async fn schedule_retry(
        &self,
        task_id: &str,
        is_normal_continuation: bool,
        error: Option<String>,
    ) -> SymphonyResult<()> {
        let mut task = self.get_task(task_id).await?;
        let max_backoff = self
            .memory
            .read()
            .await
            .active_config
            .agent
            .max_retry_backoff_ms;

        let next_attempt = if is_normal_continuation {
            1
        } else {
            task.attempt_count.max(1)
        };
        let delay_ms = compute_backoff_delay_ms(next_attempt, is_normal_continuation, max_backoff);
        let due_at = Utc::now() + chrono::Duration::milliseconds(delay_ms as i64);

        task.status = TaskStatus::Queued;
        task.next_retry_at = Some(due_at);
        task.last_error = error.clone();
        task.updated_at = Utc::now();
        self.storage.upsert_task(&task).await?;

        let entry = RetryEntry {
            task_id: task.id.clone(),
            issue_id: task.issue.id.clone(),
            identifier: task.issue.identifier.clone(),
            attempt: next_attempt,
            due_at,
            error,
        };
        {
            let mut mem = self.memory.write().await;
            mem.retry_attempts.insert(task.id.clone(), entry);
        }
        Ok(())
    }

    /// Dispatch a single task immediately into an isolated Git worktree and start the AI provider session
    pub async fn dispatch_task_now(&self, task_id: &str) -> SymphonyResult<Task> {
        let mut task = self.get_task(task_id).await?;
        let project = self
            .storage
            .get_project(&task.project_id)
            .await?
            .ok_or_else(|| SymphonyError::ProjectNotFound(task.project_id.clone()))?;

        // Prevent duplicate dispatch
        {
            let mut mem = self.memory.write().await;
            if mem.running.contains_key(&task.id) {
                return Ok(task);
            }
            if !task.status.can_transition_to(TaskStatus::Running) {
                return Err(SymphonyError::InvalidStateTransition {
                    from: task.status.as_str().to_string(),
                    to: TaskStatus::Running.as_str().to_string(),
                });
            }
            mem.claimed_issue_ids.insert(task.issue.id.clone());
            mem.retry_attempts.remove(&task.id);
        }

        task.attempt_count += 1;
        task.status = TaskStatus::Running;
        task.current_phase = Some(RunAttemptPhase::PreparingWorkspace);
        task.started_at = Some(Utc::now());
        task.updated_at = Utc::now();
        task.last_error = None;
        self.storage.upsert_task(&task).await?;

        let (cancel_tx, cancel_rx) = oneshot::channel::<String>();
        let placeholder_session = LiveSession {
            session_id: format!("pending-{}", task.attempt_count),
            thread_id: task.current_thread_id.clone().unwrap_or_default(),
            turn_id: "turn-1".to_string(),
            task_id: task.id.clone(),
            issue_id: task.issue.id.clone(),
            issue_identifier: task.issue.identifier.clone(),
            issue_title: task.issue.title.clone(),
            project_id: task.project_id.clone(),
            provider: task.provider,
            model: task.model.clone(),
            workspace_path: task.workspace_path.clone().unwrap_or_default(),
            branch_name: task.branch_name.clone().unwrap_or_default(),
            pid: None,
            phase: RunAttemptPhase::PreparingWorkspace,
            attempt_number: task.attempt_count,
            turn_count: 0,
            started_at: Utc::now(),
            last_event: Some("preparing_workspace".to_string()),
            last_message: Some("Allocating isolated Git worktree".to_string()),
            last_event_at: Some(Utc::now()),
            input_tokens: task.input_tokens,
            output_tokens: task.output_tokens,
            total_tokens: task.total_tokens,
        };

        {
            let mut mem = self.memory.write().await;
            mem.running.insert(
                task.id.clone(),
                ActiveWorkerHandle {
                    session: placeholder_session,
                    cancel_tx,
                },
            );
        }

        let orch = self.clone();
        let task_clone = task.clone();
        tokio::spawn(async move {
            orch.run_worker_attempt(task_clone, project, cancel_rx)
                .await;
        });

        Ok(task)
    }

    async fn run_worker_attempt(
        &self,
        mut task: Task,
        project: Project,
        mut cancel_rx: oneshot::Receiver<String>,
    ) {
        let attempt_start = Utc::now();
        let attempt_id = Uuid::new_v4().to_string();
        let (workflow, config) = {
            let mem = self.memory.read().await;
            (mem.active_workflow.clone(), mem.active_config.clone())
        };

        let mut attempt_record = RunAttempt {
            id: attempt_id.clone(),
            task_id: task.id.clone(),
            issue_id: task.issue.id.clone(),
            issue_identifier: task.issue.identifier.clone(),
            attempt_number: task.attempt_count,
            provider: task.provider,
            workspace_path: String::new(),
            branch_name: String::new(),
            session_id: None,
            thread_id: task.current_thread_id.clone(),
            turn_id: None,
            turn_count: 0,
            phase: RunAttemptPhase::PreparingWorkspace,
            started_at: attempt_start,
            finished_at: None,
            error: None,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
        };
        let _ = self.storage.upsert_run_attempt(&attempt_record).await;

        // 1. Allocate isolated Git worktree / workspace
        let ws_root = PathBuf::from(&project.workspace_root);
        let repo_path = PathBuf::from(&project.repo_path);
        let repo_opt = if repo_path.exists() {
            Some(repo_path.as_path())
        } else {
            None
        };

        let allocated = match self
            .workspaces
            .prepare_workspace(
                &task.issue.identifier,
                &ws_root,
                repo_opt,
                &project.default_branch,
                task.issue.branch_name.as_deref(),
                &config.hooks,
            )
            .await
        {
            Ok(ws) => ws,
            Err(err) => {
                self.finish_failed_attempt(
                    &mut task,
                    &mut attempt_record,
                    RunAttemptPhase::Failed,
                    format!("Workspace allocation failed: {err}"),
                )
                .await;
                return;
            }
        };

        let ws_str = allocated.path.to_string_lossy().to_string();
        task.workspace_path = Some(ws_str.clone());
        task.branch_name = Some(allocated.branch_name.clone());
        task.current_phase = Some(RunAttemptPhase::BuildingPrompt);
        let _ = self.storage.upsert_task(&task).await;

        attempt_record.workspace_path = ws_str.clone();
        attempt_record.branch_name = allocated.branch_name.clone();
        attempt_record.phase = RunAttemptPhase::BuildingPrompt;
        let _ = self.storage.upsert_run_attempt(&attempt_record).await;

        self.emit_event(
            Some(task.id.clone()),
            Some(task.issue.identifier.clone()),
            None,
            Some(task.provider),
            "workspace/prepared",
            Some(RunAttemptPhase::PreparingWorkspace),
            format!(
                "Prepared isolated workspace at {} (branch `{}`)",
                ws_str, allocated.branch_name
            ),
            None,
        )
        .await;

        // 2. Run `before_run` hook if configured (SPEC.md Section 9.4)
        if let Some(ref before_script) = config.hooks.before_run {
            if let Err(err) = symphony_workspaces::run_lifecycle_hook(
                "before_run",
                before_script,
                &allocated.path,
                config.hooks.timeout_ms,
            )
            .await
            {
                self.finish_failed_attempt(
                    &mut task,
                    &mut attempt_record,
                    RunAttemptPhase::Failed,
                    format!("before_run hook failed: {err}"),
                )
                .await;
                return;
            }
        }

        // 3. Render workflow prompt (SPEC.md Section 5.4 & 12)
        let first_turn_prompt = match render_workflow_prompt(
            &workflow,
            &task.issue,
            if task.attempt_count > 1 {
                Some(task.attempt_count)
            } else {
                None
            },
        ) {
            Ok(p) => p,
            Err(err) => {
                self.finish_failed_attempt(
                    &mut task,
                    &mut attempt_record,
                    RunAttemptPhase::Failed,
                    err.to_string(),
                )
                .await;
                return;
            }
        };

        // 4. Launch agent session
        task.current_phase = Some(RunAttemptPhase::LaunchingAgentProcess);
        let _ = self.storage.upsert_task(&task).await;

        let mut dynamic_tools = Vec::new();
        let gh_tracker = if project.tracker_kind.eq_ignore_ascii_case("github") {
            project.github_repo.as_ref().map(|repo| {
                dynamic_tools.push(GitHubTracker::dynamic_tool_spec());
                GitHubTracker::new(None, repo.clone(), self.github_token(), None)
            })
        } else {
            None
        };
        let ln_tracker = if project.tracker_kind.eq_ignore_ascii_case("linear") {
            project.linear_project_slug.as_ref().and_then(|slug| {
                let key = self
                    .secret_store
                    .get_secret("LINEAR_API_KEY")
                    .or_else(|| std::env::var("LINEAR_API_KEY").ok())?;
                dynamic_tools.push(LinearTracker::dynamic_tool_spec());
                Some(LinearTracker::new(
                    None,
                    slug.clone(),
                    Some(key),
                    config.terminal_states.clone(),
                ))
            })
        } else {
            None
        };

        let tool_handler: Option<Arc<dyn DynamicToolHandler>> =
            Some(Arc::new(HostDynamicToolExecutor {
                github_tracker: gh_tracker,
                linear_tracker: ln_tracker,
            }));

        let approval_gate: Arc<dyn ApprovalGate> = Arc::new(OrchestratorApprovalGate {
            storage: self.storage.clone(),
            memory: self.memory.clone(),
            event_tx: self.event_tx.clone(),
        });

        let launch_cfg = AgentLaunchConfig {
            provider: task.provider,
            command: config.codex.command.clone(),
            model: task.model.clone(),
            workspace_path: allocated.path.clone(),
            issue_identifier: task.issue.identifier.clone(),
            issue_title: task.issue.title.clone(),
            task_id: task.id.clone(),
            approval_policy: config.codex.approval_policy.clone(),
            thread_sandbox: serde_json::Value::String(config.codex.thread_sandbox.clone()),
            turn_sandbox_policy: config.codex.turn_sandbox_policy.clone(),
            read_timeout_ms: config.codex.read_timeout_ms,
            turn_timeout_ms: config.codex.turn_timeout_ms,
            extra_env: self.provider_env_from_vault(),
            dynamic_tools,
            require_human_approval_for_requests: config.codex.approval_policy != json!("never"),
        };

        let (stream_tx, mut stream_rx) = mpsc::channel::<AgentStreamEvent>(512);
        let orch_events = self.clone();
        let event_task_id = task.id.clone();
        let event_identifier = task.issue.identifier.clone();
        let event_provider = task.provider;

        tokio::spawn(async move {
            while let Some(ev) = stream_rx.recv().await {
                orch_events
                    .handle_agent_stream_event(
                        &event_task_id,
                        &event_identifier,
                        event_provider,
                        ev,
                    )
                    .await;
            }
        });

        let mut agent_session = match start_agent_session(
            launch_cfg,
            task.current_thread_id.clone(),
            tool_handler,
            approval_gate,
            stream_tx.clone(),
        )
        .await
        {
            Ok(s) => s,
            Err(err) => {
                self.finish_failed_attempt(
                    &mut task,
                    &mut attempt_record,
                    RunAttemptPhase::Failed,
                    format!("Agent startup failed: {err}"),
                )
                .await;
                return;
            }
        };

        // 5. Multi-turn execution loop (up to `agent.max_turns`, SPEC.md Section 7.1)
        let max_turns = config.agent.max_turns.max(1);
        let mut last_outcome = None;

        for turn_number in 1..=max_turns {
            if let Ok(cancel_reason) = cancel_rx.try_recv() {
                let _ = agent_session.cancel().await;
                self.finish_externally_stopped_attempt(&mut attempt_record, &cancel_reason)
                    .await;
                return;
            }

            let prompt = if turn_number == 1 {
                first_turn_prompt.clone()
            } else {
                render_continuation_prompt(&task.issue, turn_number, max_turns)
            };

            attempt_record.phase = RunAttemptPhase::StreamingTurn;
            attempt_record.turn_count = turn_number;
            let _ = self.storage.upsert_run_attempt(&attempt_record).await;
            {
                // Holding the memory lock while writing means `cancel_task` (which removes the
                // running handle before persisting `Cancelled`) either sees this write first or
                // makes it a no-op, so a stale `Running` row can never overwrite a cancellation.
                let mut mem = self.memory.write().await;
                if let Some(handle) = mem.running.get_mut(&task.id) {
                    handle.session.phase = RunAttemptPhase::StreamingTurn;
                    handle.session.turn_count = turn_number;
                    if task.current_phase != Some(RunAttemptPhase::StreamingTurn) {
                        task.current_phase = Some(RunAttemptPhase::StreamingTurn);
                        let _ = self.storage.upsert_task(&task).await;
                    }
                }
            }

            let outcome_res = tokio::select! {
                res = agent_session.run_turn(&prompt, stream_tx.clone()) => res,
                cancel_res = &mut cancel_rx => {
                    let _ = agent_session.cancel().await;
                    let reason = cancel_res.unwrap_or_else(|_| "cancelled".to_string());
                    self.finish_externally_stopped_attempt(&mut attempt_record, &reason).await;
                    return;
                }
            };

            match outcome_res {
                Ok(outcome) => {
                    task.current_thread_id = Some(outcome.thread_id.clone());
                    task.current_turn_id = Some(outcome.turn_id.clone());
                    task.current_session_id = Some(outcome.session_id.clone());
                    task.input_tokens =
                        task.input_tokens.saturating_add(outcome.usage.input_tokens);
                    task.output_tokens = task
                        .output_tokens
                        .saturating_add(outcome.usage.output_tokens);
                    task.total_tokens =
                        task.total_tokens.saturating_add(outcome.usage.total_tokens);
                    task.estimated_cost_usd +=
                        outcome.usage.estimate_cost_usd(task.model.as_deref());
                    let _ = self.storage.upsert_task(&task).await;

                    attempt_record.session_id = Some(outcome.session_id.clone());
                    attempt_record.thread_id = Some(outcome.thread_id.clone());
                    attempt_record.turn_id = Some(outcome.turn_id.clone());
                    attempt_record.input_tokens += outcome.usage.input_tokens;
                    attempt_record.output_tokens += outcome.usage.output_tokens;
                    attempt_record.total_tokens += outcome.usage.total_tokens;
                    let _ = self.storage.upsert_run_attempt(&attempt_record).await;

                    if outcome.cancelled {
                        let _ = agent_session.stop().await;
                        let reason = outcome
                            .error_message
                            .unwrap_or_else(|| "Turn aborted by operator".to_string());
                        self.finish_unsuccessful_attempt(
                            &mut task,
                            &mut attempt_record,
                            TaskStatus::Cancelled,
                            RunAttemptPhase::CanceledByReconciliation,
                            reason,
                        )
                        .await;
                        return;
                    }
                    if !outcome.succeeded {
                        let _ = agent_session.stop().await;
                        let err_msg = outcome
                            .error_message
                            .unwrap_or_else(|| "Agent turn failed".to_string());
                        self.finish_failed_attempt(
                            &mut task,
                            &mut attempt_record,
                            RunAttemptPhase::Failed,
                            err_msg,
                        )
                        .await;
                        return;
                    }

                    let continue_multi_turn = outcome
                        .assistant_messages
                        .iter()
                        .any(|m| m.contains("[CONTINUE_TURN]"));
                    last_outcome = Some(outcome);
                    if !continue_multi_turn || turn_number >= max_turns {
                        break;
                    }
                }
                Err(err) => {
                    let _ = agent_session.stop().await;
                    let phase = match err {
                        SymphonyError::TurnTimeout(_) => RunAttemptPhase::TimedOut,
                        _ => RunAttemptPhase::Failed,
                    };
                    self.finish_failed_attempt(
                        &mut task,
                        &mut attempt_record,
                        phase,
                        err.to_string(),
                    )
                    .await;
                    return;
                }
            }
        }

        let _ = agent_session.stop().await;

        // The agent has stopped streaming; hooks and validations follow.
        {
            let mut mem = self.memory.write().await;
            if let Some(handle) = mem.running.get_mut(&task.id) {
                handle.session.phase = RunAttemptPhase::Finishing;
                task.current_phase = Some(RunAttemptPhase::Finishing);
                let _ = self.storage.upsert_task(&task).await;
            }
        }

        // 6. Run `after_run` lifecycle hook (failure logged and ignored per SPEC.md)
        if let Some(ref after_script) = config.hooks.after_run {
            if let Err(e) = symphony_workspaces::run_lifecycle_hook(
                "after_run",
                after_script,
                &allocated.path,
                config.hooks.timeout_ms,
            )
            .await
            {
                warn!(error = %e, "after_run hook failed (ignored per SPEC.md)");
            }
        }

        // 7. Run configured validation commands in workspace (Section 10)
        attempt_record.phase = RunAttemptPhase::Finishing;
        let _ = self.storage.upsert_run_attempt(&attempt_record).await;

        let validation_results = self
            .workspaces
            .run_validation_commands(
                &task.id,
                task.attempt_count,
                &allocated.path,
                &project.validation_commands,
                config.hooks.timeout_ms,
            )
            .await;

        for vr in &validation_results {
            let _ = self.storage.save_validation_result(vr).await;
        }

        // 8. Inspect workspace changes, diff, and construct structured PR review / merge-readiness report
        let changed_files = self
            .workspaces
            .inspect_changed_files(&allocated.path)
            .await
            .unwrap_or_default();
        let diff_preview = self
            .workspaces
            .get_diff_preview(&allocated.path)
            .await
            .unwrap_or_default();

        let mut security_warnings = Vec::new();
        if diff_preview.contains("PRIVATE KEY") || diff_preview.contains("ghp_") {
            security_warnings.push(
                "Potential credential or key pattern detected in workspace diff.".to_string(),
            );
        }

        let merge_readiness = MergeReadinessReport::evaluate(
            false,
            "passed",
            &validation_results,
            &security_warnings,
            changed_files.len(),
        );

        let pr_review = PullRequestReview {
            id: format!("pr-{}", task.id),
            task_id: task.id.clone(),
            project_id: project.id.clone(),
            pr_number: task.pr_number.unwrap_or(task.attempt_count as u64),
            title: format!("{}: {}", task.issue.identifier, task.issue.title),
            description: last_outcome
                .and_then(|o| o.assistant_messages.last().cloned())
                .unwrap_or_else(|| {
                    format!(
                        "Automated implementation for {} in isolated branch `{}`.",
                        task.issue.identifier, allocated.branch_name
                    )
                }),
            url: task
                .pr_url
                .clone()
                .unwrap_or_else(|| format!("local://worktree/{}", allocated.branch_name)),
            branch_name: allocated.branch_name.clone(),
            base_branch: project.default_branch.clone(),
            state: "open".to_string(),
            ci_status: "passed".to_string(),
            mergeable: true,
            has_conflicts: false,
            changed_files,
            diff_preview,
            validation_results: validation_results.clone(),
            review_comments: Vec::new(),
            security_warnings,
            merge_readiness,
            human_approved: false,
            updated_at: Utc::now(),
        };
        let _ = self.storage.upsert_pr_review(&pr_review).await;

        // 9. Transition task to HumanReview (or Completed if all validations passed and no review gate)
        let elapsed_secs = (Utc::now() - attempt_start).num_milliseconds().max(0) as f64 / 1000.0;
        task.duration_seconds += elapsed_secs;
        task.status = TaskStatus::HumanReview;
        task.current_phase = Some(RunAttemptPhase::Succeeded);
        task.updated_at = Utc::now();
        let _ = self.storage.upsert_task(&task).await;

        attempt_record.phase = RunAttemptPhase::Succeeded;
        attempt_record.finished_at = Some(Utc::now());
        let _ = self.storage.upsert_run_attempt(&attempt_record).await;

        {
            let mut mem = self.memory.write().await;
            mem.running.remove(&task.id);
            mem.claimed_issue_ids.remove(&task.issue.id);
            mem.codex_totals.seconds_running += elapsed_secs;
        }

        self.emit_event(
            Some(task.id.clone()),
            Some(task.issue.identifier.clone()),
            attempt_record.session_id.clone(),
            Some(task.provider),
            "attempt/succeeded",
            Some(RunAttemptPhase::Succeeded),
            format!(
                "Completed execution for {} (Merge Readiness: {}/5). Ready for human review.",
                task.issue.identifier, pr_review.merge_readiness.score
            ),
            None,
        )
        .await;
    }

    async fn finish_failed_attempt(
        &self,
        task: &mut Task,
        attempt_record: &mut RunAttempt,
        phase: RunAttemptPhase,
        error_msg: String,
    ) {
        self.finish_unsuccessful_attempt(
            task,
            attempt_record,
            TaskStatus::Failed,
            phase,
            error_msg,
        )
        .await;
    }

    /// Finalize an attempt that was stopped from outside the worker (`cancel_task`, stall
    /// detection, tracker reconciliation). The stopping side owns the task's status, events and
    /// retry scheduling, so only the attempt record and runtime totals are written here — the
    /// worker's stale task copy is never persisted over the newer state.
    async fn finish_externally_stopped_attempt(
        &self,
        attempt_record: &mut RunAttempt,
        reason: &str,
    ) {
        self.expire_pending_approvals(&attempt_record.task_id).await;
        let now = Utc::now();
        let elapsed = (now - attempt_record.started_at).num_milliseconds().max(0) as f64 / 1000.0;
        attempt_record.phase = if reason == "stall_timeout" {
            RunAttemptPhase::Stalled
        } else {
            RunAttemptPhase::CanceledByReconciliation
        };
        attempt_record.error = Some(format!("Stopped: {reason}"));
        attempt_record.finished_at = Some(now);
        let _ = self.storage.upsert_run_attempt(attempt_record).await;
        let mut mem = self.memory.write().await;
        mem.codex_totals.seconds_running += elapsed;
        // Release the issue claim (e.g. after a stall) unless a newer attempt already runs it.
        if !mem.running.contains_key(&attempt_record.task_id) {
            mem.claimed_issue_ids.remove(&attempt_record.issue_id);
        }
    }

    /// End an attempt that did not succeed: `Failed` (eligible for retry) or `Cancelled` (the
    /// operator aborted the turn, e.g. via the "abort turn" approval decision).
    async fn finish_unsuccessful_attempt(
        &self,
        task: &mut Task,
        attempt_record: &mut RunAttempt,
        status: TaskStatus,
        phase: RunAttemptPhase,
        error_msg: String,
    ) {
        self.expire_pending_approvals(&task.id).await;
        let now = Utc::now();
        let elapsed = (now - attempt_record.started_at).num_milliseconds().max(0) as f64 / 1000.0;
        task.duration_seconds += elapsed;
        task.status = status;
        task.current_phase = Some(phase);
        task.last_error = Some(error_msg.clone());
        task.current_session_id = None;
        task.updated_at = now;
        let _ = self.storage.upsert_task(task).await;

        attempt_record.phase = phase;
        attempt_record.error = Some(error_msg.clone());
        attempt_record.finished_at = Some(now);
        let _ = self.storage.upsert_run_attempt(attempt_record).await;

        {
            let mut mem = self.memory.write().await;
            mem.running.remove(&task.id);
            mem.claimed_issue_ids.remove(&task.issue.id);
            mem.codex_totals.seconds_running += elapsed;
        }

        self.emit_event(
            Some(task.id.clone()),
            Some(task.issue.identifier.clone()),
            attempt_record.session_id.clone(),
            Some(task.provider),
            if status == TaskStatus::Cancelled {
                "task/cancelled"
            } else {
                "attempt/failed"
            },
            Some(phase),
            if status == TaskStatus::Cancelled {
                format!("Task {} cancelled: {}", task.issue.identifier, error_msg)
            } else {
                format!("Task {} failed: {}", task.issue.identifier, error_msg)
            },
            None,
        )
        .await;
    }

    async fn handle_agent_stream_event(
        &self,
        task_id: &str,
        issue_identifier: &str,
        provider: ProviderKind,
        ev: AgentStreamEvent,
    ) {
        {
            let mut mem = self.memory.write().await;
            if let Some(handle) = mem.running.get_mut(task_id) {
                handle.session.last_event = Some(ev.event_type.clone());
                handle.session.last_event_at = Some(ev.timestamp);
                if let Some(ref msg) = ev.message {
                    handle.session.last_message = Some(msg.clone());
                }
                if let Some(ref sid) = ev.session_id {
                    handle.session.session_id = sid.clone();
                }
                if let Some(pid) = ev.pid {
                    handle.session.pid = Some(pid);
                }
                if let Some(ref usage) = ev.usage {
                    handle.session.input_tokens = usage.input_tokens;
                    handle.session.output_tokens = usage.output_tokens;
                    handle.session.total_tokens = usage.total_tokens;
                }
            }
            if let Some(ref usage) = ev.usage {
                mem.codex_totals.input_tokens = mem
                    .codex_totals
                    .input_tokens
                    .saturating_add(usage.input_tokens);
                mem.codex_totals.output_tokens = mem
                    .codex_totals
                    .output_tokens
                    .saturating_add(usage.output_tokens);
                mem.codex_totals.total_tokens = mem
                    .codex_totals
                    .total_tokens
                    .saturating_add(usage.total_tokens);
                mem.codex_totals.estimated_cost_usd += usage.estimate_cost_usd(None);
            }
            if let Some(ref rl) = ev.rate_limits {
                mem.latest_rate_limits = Some(rl.clone());
            }
        }

        let agent_evt = AgentEvent {
            id: Uuid::new_v4().to_string(),
            task_id: Some(task_id.to_string()),
            issue_identifier: Some(issue_identifier.to_string()),
            session_id: ev.session_id,
            provider: Some(provider),
            event_type: ev.event_type.clone(),
            phase: Some(RunAttemptPhase::StreamingTurn),
            message: redact_secrets(&ev.message.unwrap_or_else(|| ev.event_type.clone()), &[]),
            payload: Some(ev.raw),
            timestamp: ev.timestamp,
        };

        let _ = self.storage.record_event(&agent_evt).await;
        let _ = self.event_tx.send(agent_evt);
    }

    #[allow(clippy::too_many_arguments)]
    async fn emit_event(
        &self,
        task_id: Option<String>,
        issue_identifier: Option<String>,
        session_id: Option<String>,
        provider: Option<ProviderKind>,
        event_type: &str,
        phase: Option<RunAttemptPhase>,
        message: String,
        payload: Option<Value>,
    ) {
        let evt = AgentEvent {
            id: Uuid::new_v4().to_string(),
            task_id,
            issue_identifier,
            session_id,
            provider,
            event_type: event_type.to_string(),
            phase,
            message: redact_secrets(&message, &[]),
            payload,
            timestamp: Utc::now(),
        };
        let _ = self.storage.record_event(&evt).await;
        let _ = self.event_tx.send(evt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_end_to_end_simulated_task_execution_and_recovery() {
        let temp = TempDir::new().unwrap();
        let repo_dir = temp.path().join("repo");
        let ws_root = temp.path().join("workspaces");
        std::fs::create_dir_all(&repo_dir).unwrap();
        std::fs::create_dir_all(&ws_root).unwrap();

        // Initialize a real git repo for worktree testing
        std::process::Command::new("git")
            .arg("init")
            .arg("-b")
            .arg("main")
            .current_dir(&repo_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@symphony.local"])
            .current_dir(&repo_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Symphony Test"])
            .current_dir(&repo_dir)
            .output()
            .unwrap();
        std::fs::write(repo_dir.join("README.md"), "# Test Repo\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&repo_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(&repo_dir)
            .output()
            .unwrap();

        let storage = Storage::connect_in_memory().await.unwrap();
        let secrets = SecretStore::new(&temp.path().join("vault")).unwrap();
        let orch = SymphonyOrchestrator::new(storage, None, secrets)
            .await
            .unwrap();

        let project = Project {
            id: "proj-1".into(),
            name: "Test Repo".into(),
            repo_path: repo_dir.to_string_lossy().to_string(),
            default_branch: "main".into(),
            workspace_root: ws_root.to_string_lossy().to_string(),
            workflow_path: None,
            preferred_provider: ProviderKind::Simulated,
            preferred_model: Some("simulated-deterministic-v1".into()),
            tracker_kind: "manual".into(),
            github_repo: None,
            linear_project_slug: None,
            required_labels: Vec::new(),
            auto_dispatch: false,
            max_concurrent_agents: 2,
            validation_commands: vec!["git status --short".into()],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        orch.create_project(project).await.unwrap();

        let task = orch
            .create_manual_task(
                "proj-1".into(),
                Some("SYM-1".into()),
                "Implement feature X".into(),
                Some("Create SYMPHONY_TASK_NOTE.md".into()),
                Some(1),
                Some(ProviderKind::Simulated),
                None,
                vec!["feature".into()],
                true,
            )
            .await
            .unwrap();

        // Wait for async worker to finish
        for _ in 0..40 {
            let t = orch.get_task(&task.id).await.unwrap();
            if t.status == TaskStatus::HumanReview {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let finished = orch.get_task(&task.id).await.unwrap();
        assert_eq!(finished.status, TaskStatus::HumanReview);
        assert!(finished.total_tokens > 0);

        let detail = orch.issue_detail("SYM-1").await.unwrap();
        assert_eq!(detail.validation_results.len(), 1);
        assert!(detail.validation_results[0].passed);
        assert!(detail.pull_request.is_some());
        let pr = detail.pull_request.unwrap();
        assert!(pr.merge_readiness.score >= 4);
        assert!(pr
            .changed_files
            .iter()
            .any(|f| f.path.contains("SYMPHONY_TASK_NOTE.md")));

        // Step 11: Human approves PR merge / completion
        let approved_pr = orch.approve_pr_merge(&pr.id).await.unwrap();
        assert!(approved_pr.human_approved);
        let completed_task = orch.get_task(&task.id).await.unwrap();
        assert_eq!(completed_task.status, TaskStatus::Completed);
    }

    #[tokio::test]
    async fn test_restart_persistence_and_crash_recovery_and_cancellation() {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("symphony_test.db");
        let repo_dir = temp.path().join("repo");
        let ws_root = temp.path().join("workspaces");
        std::fs::create_dir_all(&repo_dir).unwrap();
        std::fs::create_dir_all(&ws_root).unwrap();

        std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&repo_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@symphony.local"])
            .current_dir(&repo_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Symphony Test"])
            .current_dir(&repo_dir)
            .output()
            .unwrap();
        std::fs::write(repo_dir.join("README.md"), "# Persistence Repo\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&repo_dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(&repo_dir)
            .output()
            .unwrap();

        let task_id: String;
        {
            let storage = Storage::connect(&db_path).await.unwrap();
            let secrets = SecretStore::new(&temp.path().join("vault")).unwrap();
            let orch = SymphonyOrchestrator::new(storage.clone(), None, secrets)
                .await
                .unwrap();

            let project = Project {
                id: "proj-persist".into(),
                name: "Persistence Repo".into(),
                repo_path: repo_dir.to_string_lossy().to_string(),
                default_branch: "main".into(),
                workspace_root: ws_root.to_string_lossy().to_string(),
                workflow_path: None,
                preferred_provider: ProviderKind::Simulated,
                preferred_model: None,
                tracker_kind: "manual".into(),
                github_repo: None,
                linear_project_slug: None,
                required_labels: Vec::new(),
                auto_dispatch: false,
                max_concurrent_agents: 2,
                validation_commands: Vec::new(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            orch.create_project(project).await.unwrap();

            let t = orch
                .create_manual_task(
                    "proj-persist".into(),
                    Some("SYM-2".into()),
                    "Test cancellation and crash recovery".into(),
                    None,
                    Some(2),
                    Some(ProviderKind::Simulated),
                    None,
                    Vec::new(),
                    false,
                )
                .await
                .unwrap();
            task_id = t.id.clone();

            // Test cancellation transition
            let cancelled = orch.cancel_task(&task_id, "user_cancelled").await.unwrap();
            assert_eq!(cancelled.status, TaskStatus::Cancelled);

            // Simulate a task left in Running state before unexpected process exit
            let mut stale = cancelled;
            stale.status = TaskStatus::Running;
            storage.upsert_task(&stale).await.unwrap();
        }

        // Step 12: Re-open SQLite DB after restart and verify crash recovery + task history persistence
        let storage2 = Storage::connect(&db_path).await.unwrap();
        let secrets2 = SecretStore::new(&temp.path().join("vault")).unwrap();
        let orch2 = SymphonyOrchestrator::new(storage2, None, secrets2)
            .await
            .unwrap();

        let recovered = orch2.get_task(&task_id).await.unwrap();
        assert_eq!(recovered.status, TaskStatus::Queued);
        assert!(recovered
            .last_error
            .as_deref()
            .unwrap_or("")
            .contains("Interrupted execution detected on startup recovery"));
    }

    #[tokio::test]
    async fn test_codex_app_server_stdio_jsonrpc_protocol_fixture() {
        let temp = TempDir::new().unwrap();
        let ws = temp.path().join("ws-codex");
        std::fs::create_dir_all(&ws).unwrap();

        // Create a deterministic mock Codex App Server in Node.js speaking JSON-RPC 2.0 over stdio
        let script_path = temp.path().join("mock_codex_server.js");
        let script = r#"
const readline = require('readline');
const fs = require('fs');
const path = require('path');
const rl = readline.createInterface({ input: process.stdin, terminal: false });
rl.on('line', (line) => {
  if (!line.trim()) return;
  const msg = JSON.parse(line);
  if (msg.method === 'initialize') {
    process.stdout.write(JSON.stringify({ id: msg.id, result: { serverInfo: { name: 'codex-app-server', version: '0.147.0' } } }) + '\n');
  } else if (msg.method === 'thread/start') {
    process.stdout.write(JSON.stringify({ id: msg.id, result: { thread: { id: 'thread_test_1' } } }) + '\n');
  } else if (msg.method === 'turn/start') {
    fs.writeFileSync(path.join(process.cwd(), 'CODEX_OUTPUT.txt'), 'created by codex app-server\n');
    process.stdout.write(JSON.stringify({ id: msg.id, result: { turn: { id: 'turn_test_1' } } }) + '\n');
    process.stdout.write(JSON.stringify({ method: 'thread/tokenUsage/updated', params: { usage: { input_tokens: 320, output_tokens: 180, total_tokens: 500 } } }) + '\n');
    process.stdout.write(JSON.stringify({ method: 'item/agentMessage/delta', params: { delta: 'Implemented CODEX_OUTPUT.txt' } }) + '\n');
    process.stdout.write(JSON.stringify({ method: 'turn/completed', params: { turnId: 'turn_test_1' } }) + '\n');
  }
});
"#;
        std::fs::write(&script_path, script).unwrap();

        let launch_cfg = AgentLaunchConfig {
            provider: ProviderKind::Codex,
            command: format!("node \"{}\"", script_path.display()),
            model: Some("gpt-5-codex".into()),
            workspace_path: ws.clone(),
            issue_identifier: "SYM-10".into(),
            issue_title: "Verify Codex JSON-RPC".into(),
            task_id: "task-10".into(),
            approval_policy: Value::String("never".into()),
            thread_sandbox: Value::String("workspace-write".into()),
            turn_sandbox_policy: None,
            read_timeout_ms: 5_000,
            turn_timeout_ms: 15_000,
            extra_env: HashMap::new(),
            dynamic_tools: Vec::new(),
            require_human_approval_for_requests: false,
        };

        struct AutoApprove;
        #[async_trait::async_trait]
        impl ApprovalGate for AutoApprove {
            async fn request_approval(
                &self,
                _req: ApprovalRequest,
            ) -> SymphonyResult<ApprovalDecision> {
                Ok(ApprovalDecision::AcceptForSession)
            }
        }

        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(64);
        let mut session = start_agent_session(
            launch_cfg,
            None,
            None,
            Arc::new(AutoApprove),
            event_tx.clone(),
        )
        .await
        .unwrap();

        let outcome = session
            .run_turn("Write CODEX_OUTPUT.txt", event_tx)
            .await
            .unwrap();
        let _ = session.stop().await;

        assert!(outcome.succeeded);
        assert_eq!(outcome.session_id, "thread_test_1-turn_test_1");
        assert_eq!(outcome.usage.total_tokens, 500);
        assert!(ws.join("CODEX_OUTPUT.txt").exists());

        let mut received_events = Vec::new();
        while let Ok(ev) = event_rx.try_recv() {
            received_events.push(ev);
        }
        assert!(!received_events.is_empty());
    }
}
