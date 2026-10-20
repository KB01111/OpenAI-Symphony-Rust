//! Axum HTTP + SSE API for the Symphony orchestration runtime (`/api/v1/*`).
//!
//! Security model:
//! - Every `/api/v1/*` route except `health` and `runtime` requires `Authorization: Bearer <token>`
//!   whenever an access token is configured. The desktop host always configures a random
//!   per-launch token and hands it to its own webview over Tauri IPC, so other local processes and
//!   web pages cannot drive the runtime.
//! - CORS is restricted to an explicit origin allowlist (the Tauri webview and the Vite dev server
//!   by default) instead of `*`.
//! - Tokens are compared in constant time and never logged.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        Html, IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use futures::stream::{self, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::info;
use ts_rs::TS;

use symphony_core::{
    domain::{
        AgentEvent, AppSettings, ApprovalDecision, Project, ProviderKind, RuntimeInfo, TaskStatus,
    },
    error::{Result, SymphonyError},
};
use symphony_orchestrator::{ProjectUpdate, SymphonyOrchestrator};

/// API contract version reported by `GET /api/v1/runtime`. Bump on breaking changes.
pub const API_VERSION: u32 = 2;

/// Credential keys that may be stored through the API. Anything else is rejected so the endpoint
/// can never be used to inject arbitrary process environment variables (e.g. `PATH`).
pub const ALLOWED_CREDENTIAL_KEYS: &[&str] = &[
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    "LINEAR_API_KEY",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
];

/// Provider API keys that are exported into the runtime environment so agent subprocesses can
/// authenticate. Tracker tokens stay in the vault and are only used by host-side dynamic tools.
const PROVIDER_ENV_CREDENTIALS: &[&str] = &["OPENAI_API_KEY", "ANTHROPIC_API_KEY"];

/// Origins allowed to call the API from a browser context by default.
pub fn default_local_origins() -> Vec<String> {
    [
        "tauri://localhost",
        "http://tauri.localhost",
        "https://tauri.localhost",
        "http://localhost:1420",
        "http://127.0.0.1:1420",
        "http://localhost:1421",
        "http://127.0.0.1:1421",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Generate a random 256-bit hex access token.
pub fn generate_access_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind_host: String,
    pub port: u16,
    /// Bearer token required for protected routes. `None` disables auth (tests only; the desktop
    /// host and CLI always configure one).
    pub access_token: Option<String>,
    pub allowed_origins: Vec<String>,
    /// Workspace root used for projects registered without an explicit one.
    pub default_workspace_root: Option<PathBuf>,
}

impl ServerConfig {
    pub fn local(port: u16, access_token: Option<String>) -> Self {
        Self {
            bind_host: "127.0.0.1".to_string(),
            port,
            access_token,
            allowed_origins: default_local_origins(),
            default_workspace_root: None,
        }
    }
}

#[derive(Clone)]
pub struct ServerState {
    pub orchestrator: Arc<SymphonyOrchestrator>,
    pub access_token: Option<Arc<String>>,
    pub default_workspace_root: Option<PathBuf>,
    pub started_at: DateTime<Utc>,
}

// ============================================================================
// Request / response contracts (exported to TypeScript via ts-rs)
// ============================================================================

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct ApiErrorBody {
    pub error: ApiErrorDetail,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct ApiErrorDetail {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
    pub version: String,
    #[ts(type = "string")]
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct SchedulerStateResponse {
    pub scheduler_paused: bool,
}

#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(optional_fields = nullable)]
pub struct CreateProjectRequest {
    pub name: String,
    pub repo_path: String,
    pub github_repo: Option<String>,
    pub linear_project_slug: Option<String>,
    pub default_branch: Option<String>,
    pub workspace_root: Option<String>,
    pub preferred_provider: Option<ProviderKind>,
    pub preferred_model: Option<String>,
    pub workflow_path: Option<String>,
    pub auto_dispatch: Option<bool>,
    pub tracker_kind: Option<String>,
    pub required_labels: Option<Vec<String>>,
    pub max_concurrent_agents: Option<u32>,
    pub validation_commands: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, TS)]
pub struct ValidateRepositoryRequest {
    pub repo_path: String,
}

#[derive(Debug, Deserialize, Default, Serialize, TS)]
#[ts(optional_fields = nullable)]
pub struct ListTasksQuery {
    pub project_id: Option<String>,
    pub status: Option<TaskStatus>,
}

#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(optional_fields = nullable)]
pub struct CreateTaskRequest {
    pub project_id: String,
    pub identifier: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<i32>,
    pub provider: Option<ProviderKind>,
    pub model: Option<String>,
    pub labels: Option<Vec<String>>,
    /// Dispatch the task to an agent immediately after creation.
    /// `dispatch_immediately` is accepted as an alias for older clients.
    #[serde(alias = "dispatch_immediately")]
    pub auto_start: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, TS)]
pub struct MoveTaskStatusRequest {
    pub status: TaskStatus,
}

#[derive(Debug, Deserialize, Default, Serialize, TS)]
#[ts(optional_fields = nullable)]
pub struct CancelTaskRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LimitQuery {
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ApprovalsQuery {
    pub only_pending: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, TS)]
pub struct DecideApprovalRequest {
    pub decision: ApprovalDecision,
}

#[derive(Debug, Deserialize, Default)]
pub struct ProjectQuery {
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, TS)]
pub struct UpdateWorkflowRequest {
    pub raw_markdown: String,
}

#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(optional_fields = nullable)]
pub struct PreviewWorkflowRequest {
    pub raw_markdown: String,
    pub sample_identifier: Option<String>,
    pub sample_title: Option<String>,
    pub sample_description: Option<String>,
    pub attempt: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ProvidersQuery {
    pub refresh: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, TS)]
pub struct SetCredentialRequest {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Deserialize, Serialize, TS)]
pub struct CredentialKeysResponse {
    /// Names of configured credentials. Values are never returned.
    pub configured: Vec<String>,
    pub allowed: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, TS)]
pub struct CredentialSavedResponse {
    pub saved: bool,
    pub key: String,
}

/// Payload of the `ready` and `resync` SSE events.
#[derive(Debug, Deserialize, Serialize, TS)]
pub struct StreamControlEvent {
    pub api_version: u32,
    #[ts(type = "string")]
    pub server_time: DateTime<Utc>,
    pub reason: Option<String>,
}

// ============================================================================
// Router
// ============================================================================

pub fn build_router(orchestrator: Arc<SymphonyOrchestrator>, config: &ServerConfig) -> Router {
    let state = ServerState {
        orchestrator,
        access_token: config
            .access_token
            .clone()
            .filter(|t| !t.trim().is_empty())
            .map(Arc::new),
        default_workspace_root: config.default_workspace_root.clone(),
        started_at: Utc::now(),
    };

    let origins: Vec<HeaderValue> = config
        .allowed_origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("last-event-id"),
        ])
        .max_age(Duration::from_secs(600));

    let protected = Router::new()
        .route("/state", get(state_handler))
        .route("/refresh", post(refresh_handler))
        .route("/events", get(sse_events_handler))
        .route("/events/recent", get(recent_events_handler))
        .route("/scheduler/pause", post(pause_scheduler_handler))
        .route("/scheduler/resume", post(resume_scheduler_handler))
        .route(
            "/projects",
            get(list_projects_handler).post(create_project_handler),
        )
        .route("/projects/validate", post(validate_repository_handler))
        .route(
            "/projects/{id}",
            get(get_project_handler)
                .patch(update_project_handler)
                .delete(delete_project_handler),
        )
        .route("/tasks", get(list_tasks_handler).post(create_task_handler))
        .route("/tasks/{id}", get(get_task_by_id_handler))
        // Legacy alias kept for older clients.
        .route("/tasks/id/{id}", get(get_task_by_id_handler))
        .route(
            "/tasks/{id}/status",
            axum::routing::patch(move_task_status_handler),
        )
        .route("/tasks/{id}/dispatch", post(dispatch_task_handler))
        .route("/tasks/{id}/cancel", post(cancel_task_handler))
        .route("/tasks/{id}/retry", post(retry_task_handler))
        .route("/tasks/{id}/events", get(list_task_events_handler))
        .route("/tasks/{id}/attempts", get(list_task_attempts_handler))
        .route(
            "/tasks/{id}/validations",
            get(list_task_validations_handler),
        )
        .route("/tasks/{id}/diff", get(task_diff_handler))
        .route("/approvals", get(list_approvals_handler))
        .route("/approvals/{id}/decide", post(decide_approval_handler))
        .route("/prs", get(list_prs_handler))
        .route("/prs/{id}", get(get_pr_handler))
        .route("/prs/{id}/approve-merge", post(approve_pr_merge_handler))
        .route("/worktrees", get(list_worktrees_handler))
        .route(
            "/workflow",
            get(get_workflow_handler).put(update_workflow_handler),
        )
        .route("/workflow/preview", post(preview_workflow_handler))
        .route(
            "/settings",
            get(get_settings_handler).put(put_settings_handler),
        )
        .route("/providers", get(list_providers_handler))
        .route(
            "/credentials",
            get(list_credentials_handler).post(set_credential_handler),
        )
        .route(
            "/credentials/{key}",
            axum::routing::delete(delete_credential_handler),
        )
        .route("/diagnostics/export", get(export_diagnostics_handler))
        .route("/issues/{issue_identifier}", get(issue_detail_handler))
        // SPEC.md Section 13.7.2: `/api/v1/<issue_identifier>` catch-all
        .route("/{issue_identifier}", get(issue_detail_handler))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_bearer_token,
        ));

    let public = Router::new()
        .route("/health", get(health_handler))
        .route("/runtime", get(runtime_handler));

    Router::new()
        .route("/", get(root_handler))
        .nest("/api/v1", public.merge(protected))
        .layer(cors)
        .with_state(state)
}

pub async fn start_http_server(
    orchestrator: Arc<SymphonyOrchestrator>,
    config: ServerConfig,
) -> Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let router = build_router(orchestrator, &config);
    let addr_str = format!("{}:{}", config.bind_host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr_str).await?;
    let local_addr = listener.local_addr()?;
    info!(
        address = %local_addr,
        auth_required = config.access_token.is_some(),
        "Symphony HTTP server listening"
    );

    let handle = tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, router).await {
            tracing::error!(error = %err, "Symphony HTTP server terminated with error");
        }
    });

    Ok((local_addr, handle))
}

// ============================================================================
// Auth & errors
// ============================================================================

fn token_matches(expected: &str, provided: &str) -> bool {
    expected.as_bytes().ct_eq(provided.as_bytes()).into()
}

async fn require_bearer_token(
    State(state): State<ServerState>,
    request: Request,
    next: Next,
) -> Response {
    if let Some(expected) = &state.access_token {
        let provided = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .unwrap_or("");
        if !token_matches(expected, provided.trim()) {
            return api_error(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "A valid Bearer token is required",
            );
        }
    }
    next.run(request).await
}

fn api_error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(ApiErrorBody {
            error: ApiErrorDetail {
                code: code.to_string(),
                message: message.into(),
            },
        }),
    )
        .into_response()
}

fn status_for(err: &SymphonyError) -> StatusCode {
    match err {
        SymphonyError::NotFound(_)
        | SymphonyError::TaskNotFound(_)
        | SymphonyError::ProjectNotFound(_)
        | SymphonyError::IssueNotFound(_)
        | SymphonyError::MissingWorkflowFile(_) => StatusCode::NOT_FOUND,
        SymphonyError::InvalidStateTransition { .. } | SymphonyError::Conflict(_) => {
            StatusCode::CONFLICT
        }
        SymphonyError::InvalidConfig(_)
        | SymphonyError::WorkflowParseError(_)
        | SymphonyError::WorkflowFrontMatterNotAMap
        | SymphonyError::TemplateParseError(_)
        | SymphonyError::TemplateRenderError(_)
        | SymphonyError::JsonError(_) => StatusCode::BAD_REQUEST,
        SymphonyError::WorkspaceOutsideRoot { .. }
        | SymphonyError::WorkspaceEqualsRoot(_)
        | SymphonyError::SymlinkEscape { .. } => StatusCode::FORBIDDEN,
        SymphonyError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn error_response(err: SymphonyError) -> Response {
    api_error(status_for(&err), err.code(), err.to_string())
}

fn respond<T: Serialize>(result: Result<T>) -> Response {
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

fn bad_request(message: impl Into<String>) -> Response {
    api_error(StatusCode::BAD_REQUEST, "invalid_request", message)
}

// ============================================================================
// Public handlers
// ============================================================================

async fn root_handler() -> Html<&'static str> {
    Html(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Symphony Runtime</title></head>\
         <body style=\"font-family:'Segoe UI',system-ui,sans-serif\"><h1>Symphony orchestration runtime</h1>\
         <p>API base: <code>/api/v1</code>. Authenticated clients only.</p></body></html>",
    )
}

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".into(),
        service: "symphony-orchestrator".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        timestamp: Utc::now(),
    })
}

async fn runtime_handler(State(state): State<ServerState>) -> Json<RuntimeInfo> {
    Json(RuntimeInfo {
        service: "symphony-orchestrator".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        api_version: API_VERSION,
        auth_required: state.access_token.is_some(),
        started_at: state.started_at,
        event_stream: "/api/v1/events".into(),
    })
}

// ============================================================================
// State & scheduler
// ============================================================================

async fn state_handler(State(state): State<ServerState>) -> Response {
    respond(state.orchestrator.state_snapshot().await)
}

async fn refresh_handler(State(state): State<ServerState>) -> Response {
    let resp = state.orchestrator.trigger_refresh().await;
    (StatusCode::ACCEPTED, Json(resp)).into_response()
}

async fn pause_scheduler_handler(State(state): State<ServerState>) -> Response {
    respond(
        state
            .orchestrator
            .set_scheduler_paused(true)
            .await
            .map(|_| SchedulerStateResponse {
                scheduler_paused: true,
            }),
    )
}

async fn resume_scheduler_handler(State(state): State<ServerState>) -> Response {
    respond(
        state
            .orchestrator
            .set_scheduler_paused(false)
            .await
            .map(|_| SchedulerStateResponse {
                scheduler_paused: false,
            }),
    )
}

// ============================================================================
// Server-Sent Events
// ============================================================================

fn agent_event_to_sse(ev: &AgentEvent) -> SseEvent {
    let payload = serde_json::to_string(ev).unwrap_or_else(|_| "{}".to_string());
    SseEvent::default()
        .id(ev.id.clone())
        .event(ev.event_type.clone())
        .data(payload)
}

fn control_event(name: &str, reason: Option<&str>) -> SseEvent {
    let body = StreamControlEvent {
        api_version: API_VERSION,
        server_time: Utc::now(),
        reason: reason.map(str::to_string),
    };
    SseEvent::default()
        .event(name)
        .data(serde_json::to_string(&body).unwrap_or_else(|_| "{}".into()))
}

/// `GET /api/v1/events` — named SSE events (`event:` = `AgentEvent.event_type`, `id:` = event id).
///
/// Sends `ready` on connect. When the client reconnects with `Last-Event-ID`, persisted events
/// newer than that id are replayed first; if the id is unknown (or the broadcast buffer
/// overflowed) a `resync` event tells the client to refetch its state.
async fn sse_events_handler(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    // Subscribe before reading history so no event falls between replay and live delivery.
    let rx = state.orchestrator.subscribe_events();
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .filter(|s| !s.is_empty());

    let mut initial = vec![control_event("ready", None)];
    if let Some(last_id) = last_event_id {
        match state.orchestrator.list_events(None, 500).await {
            Ok(recent) => match recent.iter().position(|e| e.id == last_id) {
                Some(pos) => {
                    initial.extend(recent[..pos].iter().rev().map(agent_event_to_sse));
                }
                None => initial.push(control_event("resync", Some("last_event_id_unknown"))),
            },
            Err(_) => initial.push(control_event("resync", Some("history_unavailable"))),
        }
    }

    let live = BroadcastStream::new(rx).map(|msg| match msg {
        Ok(ev) => agent_event_to_sse(&ev),
        Err(BroadcastStreamRecvError::Lagged(_)) => control_event("resync", Some("lagged")),
    });
    let combined: std::pin::Pin<
        Box<dyn Stream<Item = std::result::Result<SseEvent, Infallible>> + Send>,
    > = Box::pin(stream::iter(initial).chain(live).map(Ok));

    Sse::new(combined)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

/// `GET /api/v1/events/recent?limit=N` — persisted events across all tasks, newest first.
async fn recent_events_handler(
    State(state): State<ServerState>,
    Query(q): Query<LimitQuery>,
) -> Response {
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    respond(state.orchestrator.list_events(None, limit).await)
}

// ============================================================================
// Projects
// ============================================================================

async fn list_projects_handler(State(state): State<ServerState>) -> Response {
    respond(state.orchestrator.list_projects().await)
}

async fn get_project_handler(State(state): State<ServerState>, Path(id): Path<String>) -> Response {
    match state.orchestrator.list_projects().await {
        Ok(list) => match list.into_iter().find(|p| p.id == id) {
            Some(p) => Json(p).into_response(),
            None => error_response(SymphonyError::ProjectNotFound(id)),
        },
        Err(e) => error_response(e),
    }
}

async fn validate_repository_handler(
    State(state): State<ServerState>,
    Json(req): Json<ValidateRepositoryRequest>,
) -> Response {
    Json(state.orchestrator.validate_repository(&req.repo_path).await).into_response()
}

async fn create_project_handler(
    State(state): State<ServerState>,
    Json(req): Json<CreateProjectRequest>,
) -> Response {
    if req.name.trim().is_empty() {
        return bad_request("Project name is required");
    }
    let validation = state.orchestrator.validate_repository(&req.repo_path).await;
    if !validation.is_git_repository {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_repository",
            validation
                .errors
                .first()
                .cloned()
                .unwrap_or_else(|| "Invalid repository path".into()),
        );
    }
    let now = Utc::now();
    let workspace_root = req
        .workspace_root
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            state
                .default_workspace_root
                .clone()
                .unwrap_or_else(|| std::env::temp_dir().join("symphony_workspaces"))
                .to_string_lossy()
                .to_string()
        });
    let workflow_path = req
        .workflow_path
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            Some(
                PathBuf::from(&validation.repo_path)
                    .join("WORKFLOW.md")
                    .to_string_lossy()
                    .to_string(),
            )
        });
    let max_agents = req.max_concurrent_agents.unwrap_or(2);
    if max_agents == 0 || max_agents > 32 {
        return bad_request("max_concurrent_agents must be between 1 and 32");
    }

    let project = Project {
        id: uuid::Uuid::new_v4().to_string(),
        name: req.name.trim().to_string(),
        repo_path: validation.repo_path,
        github_repo: req.github_repo.filter(|s| !s.trim().is_empty()),
        linear_project_slug: req.linear_project_slug.filter(|s| !s.trim().is_empty()),
        default_branch: req
            .default_branch
            .filter(|s| !s.trim().is_empty())
            .or(validation.current_branch)
            .unwrap_or_else(|| "main".to_string()),
        workspace_root,
        preferred_provider: req.preferred_provider.unwrap_or(ProviderKind::Codex),
        preferred_model: req.preferred_model.filter(|s| !s.trim().is_empty()),
        workflow_path,
        tracker_kind: req.tracker_kind.unwrap_or_else(|| "manual".to_string()),
        required_labels: req
            .required_labels
            .unwrap_or_else(|| vec!["symphony".to_string()]),
        auto_dispatch: req.auto_dispatch.unwrap_or(false),
        max_concurrent_agents: max_agents,
        validation_commands: req.validation_commands.unwrap_or_default(),
        created_at: now,
        updated_at: now,
    };

    match state.orchestrator.create_project(project).await {
        Ok(created) => (StatusCode::CREATED, Json(created)).into_response(),
        Err(e) => error_response(e),
    }
}

async fn update_project_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(update): Json<ProjectUpdate>,
) -> Response {
    respond(state.orchestrator.update_project(&id, update).await)
}

async fn delete_project_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Response {
    match state.orchestrator.delete_project(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(e),
    }
}

// ============================================================================
// Tasks
// ============================================================================

async fn list_tasks_handler(
    State(state): State<ServerState>,
    Query(query): Query<ListTasksQuery>,
) -> Response {
    respond(
        state
            .orchestrator
            .list_tasks(query.project_id.as_deref())
            .await
            .map(|tasks| {
                tasks
                    .into_iter()
                    .filter(|t| query.status.is_none_or(|s| t.status == s))
                    .collect::<Vec<_>>()
            }),
    )
}

async fn create_task_handler(
    State(state): State<ServerState>,
    Json(req): Json<CreateTaskRequest>,
) -> Response {
    if req.title.trim().is_empty() {
        return bad_request("Task title is required");
    }
    if let Some(p) = req.priority {
        if !(0..=4).contains(&p) {
            return bad_request("priority must be between 0 and 4");
        }
    }
    match state
        .orchestrator
        .create_manual_task(
            req.project_id,
            req.identifier,
            req.title.trim().to_string(),
            req.description,
            req.priority,
            req.provider,
            req.model.filter(|m| !m.trim().is_empty()),
            req.labels.unwrap_or_default(),
            req.auto_start.unwrap_or(false),
        )
        .await
    {
        Ok(created) => (StatusCode::CREATED, Json(created)).into_response(),
        Err(e) => error_response(e),
    }
}

async fn get_task_by_id_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Response {
    respond(state.orchestrator.get_task(&id).await)
}

async fn move_task_status_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(req): Json<MoveTaskStatusRequest>,
) -> Response {
    respond(state.orchestrator.transition_task(&id, req.status).await)
}

async fn dispatch_task_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Response {
    respond(state.orchestrator.dispatch_task_now(&id).await)
}

async fn cancel_task_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    body: Option<Json<CancelTaskRequest>>,
) -> Response {
    let reason = body
        .and_then(|b| b.0.reason)
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| "Cancelled by operator".to_string());
    respond(state.orchestrator.cancel_task(&id, &reason).await)
}

async fn retry_task_handler(State(state): State<ServerState>, Path(id): Path<String>) -> Response {
    respond(state.orchestrator.retry_task(&id).await)
}

async fn list_task_events_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Query(q): Query<LimitQuery>,
) -> Response {
    let limit = q.limit.unwrap_or(500).clamp(1, 500);
    respond(state.orchestrator.list_events(Some(&id), limit).await)
}

async fn list_task_attempts_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Response {
    respond(state.orchestrator.list_run_attempts(&id).await)
}

async fn list_task_validations_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Response {
    respond(state.orchestrator.list_validation_results(&id).await)
}

async fn task_diff_handler(State(state): State<ServerState>, Path(id): Path<String>) -> Response {
    respond(state.orchestrator.task_diff(&id).await)
}

async fn issue_detail_handler(
    State(state): State<ServerState>,
    Path(issue_identifier): Path<String>,
) -> Response {
    respond(state.orchestrator.issue_detail(&issue_identifier).await)
}

// ============================================================================
// Approvals & pull requests
// ============================================================================

async fn list_approvals_handler(
    State(state): State<ServerState>,
    Query(q): Query<ApprovalsQuery>,
) -> Response {
    respond(
        state
            .orchestrator
            .list_approvals(q.only_pending.unwrap_or(false))
            .await,
    )
}

async fn decide_approval_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(req): Json<DecideApprovalRequest>,
) -> Response {
    respond(state.orchestrator.resolve_approval(&id, req.decision).await)
}

async fn list_prs_handler(
    State(state): State<ServerState>,
    Query(q): Query<ProjectQuery>,
) -> Response {
    respond(
        state
            .orchestrator
            .list_pr_reviews(q.project_id.as_deref())
            .await,
    )
}

async fn get_pr_handler(State(state): State<ServerState>, Path(id): Path<String>) -> Response {
    respond(state.orchestrator.get_pr_review(&id).await)
}

async fn approve_pr_merge_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Response {
    respond(state.orchestrator.approve_pr_merge(&id).await)
}

async fn list_worktrees_handler(State(state): State<ServerState>) -> Response {
    respond(state.orchestrator.list_worktrees().await)
}

// ============================================================================
// Workflow
// ============================================================================

async fn get_workflow_handler(State(state): State<ServerState>) -> Response {
    Json(state.orchestrator.get_workflow_document().await).into_response()
}

async fn update_workflow_handler(
    State(state): State<ServerState>,
    Json(req): Json<UpdateWorkflowRequest>,
) -> Response {
    respond(
        state
            .orchestrator
            .save_workflow_document(&req.raw_markdown)
            .await,
    )
}

/// Validates and renders unsaved workflow text. Always `200`; invalid documents are reported
/// through `valid: false` + `error` so editors can show inline diagnostics.
async fn preview_workflow_handler(
    State(state): State<ServerState>,
    Json(req): Json<PreviewWorkflowRequest>,
) -> Response {
    // Parse against the file the document would be saved to, so relative paths resolve the same.
    let target = state.orchestrator.workflow_target_path().await;
    Json(SymphonyOrchestrator::validate_workflow_text(
        &req.raw_markdown,
        target,
        req.sample_identifier,
        req.sample_title,
        req.sample_description,
        req.attempt,
    ))
    .into_response()
}

// ============================================================================
// Settings, providers & credentials
// ============================================================================

async fn get_settings_handler(State(state): State<ServerState>) -> Response {
    respond(state.orchestrator.storage().get_settings().await)
}

fn validate_settings(settings: &AppSettings) -> std::result::Result<(), String> {
    if !["system", "dark", "light"].contains(&settings.theme.as_str()) {
        return Err("theme must be one of system, dark, light".into());
    }
    if !["en", "sv"].contains(&settings.locale.as_str()) {
        return Err("locale must be one of en, sv".into());
    }
    if settings.polling_interval_ms < 2_000 {
        return Err("polling_interval_ms must be at least 2000".into());
    }
    if settings.remote_port == 0 {
        return Err("remote_port must be a valid TCP port".into());
    }
    let policy = &settings.resource_policy;
    if policy.max_concurrent_on_ac == 0 || policy.max_concurrent_on_ac > 32 {
        return Err("resource_policy.max_concurrent_on_ac must be between 1 and 32".into());
    }
    if policy.max_concurrent_on_battery > policy.max_concurrent_on_ac {
        return Err(
            "resource_policy.max_concurrent_on_battery cannot exceed max_concurrent_on_ac".into(),
        );
    }
    if !(10.0..=100.0).contains(&policy.high_memory_threshold_percent)
        || !(10.0..=100.0).contains(&policy.high_cpu_threshold_percent)
    {
        return Err("resource thresholds must be between 10 and 100 percent".into());
    }
    if settings
        .extra_env_vars
        .keys()
        .any(|k| ALLOWED_CREDENTIAL_KEYS.contains(&k.as_str()) || k == "SYMPHONY_REMOTE_TOKEN")
    {
        return Err("Secrets must be stored through the credential vault, not settings".into());
    }
    Ok(())
}

async fn put_settings_handler(
    State(state): State<ServerState>,
    Json(settings): Json<AppSettings>,
) -> Response {
    if let Err(msg) = validate_settings(&settings) {
        return api_error(StatusCode::BAD_REQUEST, "invalid_settings", msg);
    }
    match state.orchestrator.storage().save_settings(&settings).await {
        Ok(()) => {
            state.orchestrator.invalidate_provider_cache().await;
            Json(settings).into_response()
        }
        Err(e) => error_response(e),
    }
}

async fn list_providers_handler(
    State(state): State<ServerState>,
    Query(q): Query<ProvidersQuery>,
) -> Response {
    Json(
        state
            .orchestrator
            .provider_statuses(q.refresh.unwrap_or(false))
            .await,
    )
    .into_response()
}

async fn list_credentials_handler(State(state): State<ServerState>) -> Response {
    let configured = state
        .orchestrator
        .secret_store()
        .list_configured_keys()
        .into_iter()
        .filter(|k| ALLOWED_CREDENTIAL_KEYS.contains(&k.as_str()))
        .collect();
    Json(CredentialKeysResponse {
        configured,
        allowed: ALLOWED_CREDENTIAL_KEYS
            .iter()
            .map(|s| s.to_string())
            .collect(),
    })
    .into_response()
}

async fn set_credential_handler(
    State(state): State<ServerState>,
    Json(req): Json<SetCredentialRequest>,
) -> Response {
    let key = req.key.trim().to_string();
    if !ALLOWED_CREDENTIAL_KEYS.contains(&key.as_str()) {
        return api_error(
            StatusCode::BAD_REQUEST,
            "credential_key_not_allowed",
            format!("Credential key '{key}' is not supported"),
        );
    }
    let value = req.value.trim();
    if value.is_empty() || value.len() > 4096 || value.contains(['\n', '\r', '\0']) {
        return bad_request("Credential value must be a single non-empty line");
    }
    match state.orchestrator.secret_store().set_secret(&key, value) {
        Ok(()) => {
            if PROVIDER_ENV_CREDENTIALS.contains(&key.as_str()) {
                std::env::set_var(&key, value);
            }
            state.orchestrator.invalidate_provider_cache().await;
            Json(CredentialSavedResponse { saved: true, key }).into_response()
        }
        Err(e) => error_response(e),
    }
}

async fn delete_credential_handler(
    State(state): State<ServerState>,
    Path(key): Path<String>,
) -> Response {
    if !ALLOWED_CREDENTIAL_KEYS.contains(&key.as_str()) {
        return api_error(
            StatusCode::BAD_REQUEST,
            "credential_key_not_allowed",
            format!("Credential key '{key}' is not supported"),
        );
    }
    match state.orchestrator.secret_store().delete_secret(&key) {
        Ok(()) => {
            if PROVIDER_ENV_CREDENTIALS.contains(&key.as_str()) {
                std::env::remove_var(&key);
            }
            state.orchestrator.invalidate_provider_cache().await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => error_response(e),
    }
}

async fn export_diagnostics_handler(State(state): State<ServerState>) -> Response {
    let secrets = state.orchestrator.secret_store().all_secret_values();
    match state
        .orchestrator
        .storage()
        .export_diagnostics(&secrets)
        .await
    {
        Ok(bundle) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            bundle,
        )
            .into_response(),
        Err(e) => error_response(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison_rejects_mismatch_and_prefix() {
        assert!(token_matches("abc123", "abc123"));
        assert!(!token_matches("abc123", "abc124"));
        assert!(!token_matches("abc123", "abc"));
        assert!(!token_matches("abc123", ""));
    }

    #[test]
    fn generated_tokens_are_unique_hex() {
        let a = generate_access_token();
        let b = generate_access_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn settings_validation_rejects_secrets_in_env_vars() {
        let mut s = AppSettings::default();
        assert!(validate_settings(&s).is_ok());
        s.extra_env_vars
            .insert("GITHUB_TOKEN".into(), "ghp_x".into());
        assert!(validate_settings(&s).is_err());
    }
}
