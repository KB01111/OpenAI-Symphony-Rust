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
    detect_all_providers, start_agent_session, AgentLaunchConfig, AgentStreamEvent, ApprovalGate,
    DynamicToolHandler,
};
use symphony_core::{
    compute_backoff_delay_ms, redact_secrets, render_continuation_prompt, render_workflow_prompt,
    sort_candidate_issues, AgentEvent, ApprovalDecision, ApprovalRequest, ApprovalStatus,
    CodexTotals, HardwareSnapshot, Issue, LiveSession, MergeReadinessReport,
    OrchestratorStateSnapshot, Project, ProviderCapabilities, ProviderKind, PullRequestReview,
    RetryEntry, RunAttempt, RunAttemptPhase, RunningSessionSummary, ServiceConfig, StateCounts,
    SymphonyError, SymphonyResult, Task, TaskStatus, TokenUsage, ValidationResult,
    WorkflowDefinition,
};
use symphony_platform::{HardwareMonitor, SecretStore};
use symphony_storage::Storage;
use symphony_trackers::{GitHubTracker, LinearTracker, TrackerAdapter};
use symphony_workspaces::WorkspaceManager;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, RwLock};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// SPEC.md Section 13.7.2 `/api/v1/<issue_identifier>` response contract
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub path: Option<String>,
    pub branch_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptSummary {
    pub restart_count: u32,
    pub current_retry_attempt: u32,
}

/// SPEC.md Section 13.7.2 `POST /api/v1/refresh` response contract
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshResponse {
    pub queued: bool,
    pub coalesced: bool,
    pub requested_at: DateTime<Utc>,
    pub operations: Vec<String>,
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
                    info!(path = %path.display(), "Reloaded WORKFLOW.md configuration");
                }
                Err(err) => {
                    warn!(error = %err, "Invalid WORKFLOW.md config on reload; keeping last known good configuration");
                    let mut mem = self.memory.write().await;
                    mem.workflow_error = Some(err.to_string());
                }
            },
            Err(err) => {
                warn!(error = %err, "Failed to parse WORKFLOW.md on reload; keeping last known good configuration");
                let mut mem = self.memory.write().await;
                mem.workflow_error = Some(err.to_string());
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

    pub fn detect_providers(&self) -> Vec<ProviderCapabilities> {
        detect_all_providers()
            .into_iter()
            .map(|p| p.provider.default_capabilities())
            .collect()
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
        self.storage.delete_project(id).await
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
        let cancel_tx = {
            let mut mem = self.memory.write().await;
            mem.retry_attempts.remove(task_id);
            mem.running.remove(task_id).map(|h| h.cancel_tx)
        };
        if let Some(tx) = cancel_tx {
            let _ = tx.send(reason.to_string());
        }

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

    /// Explicitly approve a Pull Request review for human merge completion (Section 4.F)
    pub async fn approve_pr_merge(&self, review_id: &str) -> SymphonyResult<PullRequestReview> {
        let mut review = self
            .storage
            .get_pr_review(review_id)
            .await?
            .ok_or_else(|| SymphonyError::NotFound(format!("PR review {review_id} not found")))?;

        review.human_approved = true;
        review.state = "approved_for_merge".to_string();
        review.updated_at = Utc::now();
        self.storage.upsert_pr_review(&review).await?;

        if let Ok(mut task) = self.get_task(&review.task_id).await {
            task.status = TaskStatus::Completed;
            task.completed_at = Some(Utc::now());
            task.updated_at = Utc::now();
            self.storage.upsert_task(&task).await?;

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

        Ok(review)
    }

    pub async fn get_workflow_document(
        &self,
        custom_path: Option<&Path>,
    ) -> SymphonyResult<(String, ServiceConfig, String)> {
        let wf = match custom_path {
            Some(p) if p.exists() => WorkflowDefinition::load_from_path(p)?,
            _ => self.memory.read().await.active_workflow.clone(),
        };
        let cfg = ServiceConfig::from_workflow(&wf)?;
        let sample_issue = Issue {
            id: "sample-1".into(),
            native_ref: None,
            identifier: "SYM-101".into(),
            title: "Add resilient retry backoff to worker pool".into(),
            description: Some(
                "Ensure exponential backoff caps at max_retry_backoff_ms and emits structured telemetry."
                    .into(),
            ),
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
        let preview = render_workflow_prompt(&wf, &sample_issue, Some(1))?;
        let raw_content = match &wf.source_path {
            Some(p) if p.exists() => {
                std::fs::read_to_string(p).unwrap_or_else(|_| wf.prompt_template.clone())
            }
            _ => wf.prompt_template.clone(),
        };
        Ok((raw_content, cfg, preview))
    }

    pub async fn save_workflow_document(
        &self,
        path: &Path,
        raw_content: &str,
    ) -> SymphonyResult<ServiceConfig> {
        let wf = WorkflowDefinition::parse(raw_content, Some(path.to_path_buf()))?;
        let cfg = ServiceConfig::from_workflow(&wf)?;
        cfg.validate_for_dispatch()?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, raw_content)?;

        let mut mem = self.memory.write().await;
        mem.active_workflow = wf;
        mem.active_config = cfg.clone();
        mem.active_workflow_path = Some(path.to_path_buf());
        mem.workflow_error = None;
        Ok(cfg)
    }

    // =========================================================================
    // Tracker Synchronization & Reconciliation Loop (SPEC.md Sections 8 & 16)
    // =========================================================================

    fn build_tracker_for_project(&self, project: &Project) -> Option<Box<dyn TrackerAdapter>> {
        match project.tracker_kind.to_lowercase().as_str() {
            "github" => {
                let repo = project.github_repo.clone()?;
                let token = self
                    .secret_store
                    .get_secret("GITHUB_TOKEN")
                    .or_else(|| std::env::var("GITHUB_TOKEN").ok())
                    .or_else(|| std::env::var("GH_TOKEN").ok());
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
                GitHubTracker::new(
                    None,
                    repo.clone(),
                    self.secret_store
                        .get_secret("GITHUB_TOKEN")
                        .or_else(|| std::env::var("GITHUB_TOKEN").ok()),
                    None,
                )
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
            extra_env: HashMap::new(),
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
                self.finish_failed_attempt(
                    &mut task,
                    &mut attempt_record,
                    RunAttemptPhase::CanceledByReconciliation,
                    format!("Cancelled: {cancel_reason}"),
                )
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

            let outcome_res = tokio::select! {
                res = agent_session.run_turn(&prompt, stream_tx.clone()) => res,
                cancel_res = &mut cancel_rx => {
                    let _ = agent_session.cancel().await;
                    let reason = cancel_res.unwrap_or_else(|_| "cancelled".to_string());
                    self.finish_failed_attempt(
                        &mut task,
                        &mut attempt_record,
                        RunAttemptPhase::CanceledByReconciliation,
                        reason,
                    ).await;
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
        let now = Utc::now();
        let elapsed = (now - attempt_record.started_at).num_milliseconds().max(0) as f64 / 1000.0;
        task.duration_seconds += elapsed;
        task.status = TaskStatus::Failed;
        task.current_phase = Some(phase);
        task.last_error = Some(error_msg.clone());
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
            "attempt/failed",
            Some(phase),
            format!("Task {} failed: {}", task.issue.identifier, error_msg),
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
