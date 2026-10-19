use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        Html, IntoResponse, Response,
    },
    routing::{delete, get, patch, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use symphony_core::{
    domain::{AppSettings, ApprovalDecision, Project, ProviderKind, TaskStatus},
    error::Result,
    workflow::{render_workflow_prompt, ServiceConfig, WorkflowDefinition},
};
use symphony_orchestrator::SymphonyOrchestrator;

#[derive(Clone)]
pub struct ServerState {
    pub orchestrator: Arc<SymphonyOrchestrator>,
    pub remote_bearer_token: Option<String>,
}

pub fn build_router(
    orchestrator: Arc<SymphonyOrchestrator>,
    remote_bearer_token: Option<String>,
) -> Router {
    let state = ServerState {
        orchestrator,
        remote_bearer_token,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // SPEC.md Section 13.7 core endpoints
        .route("/", get(dashboard_html_handler))
        .route("/api/v1/health", get(health_handler))
        .route("/api/v1/state", get(state_handler))
        .route("/api/v1/refresh", post(refresh_handler))
        .route("/api/v1/events", get(sse_events_handler))
        // Symphony Desktop 1.0 API endpoints
        .route("/api/v1/scheduler/pause", post(pause_scheduler_handler))
        .route("/api/v1/scheduler/resume", post(resume_scheduler_handler))
        .route(
            "/api/v1/projects",
            get(list_projects_handler).post(create_project_handler),
        )
        .route("/api/v1/projects/:id", delete(delete_project_handler))
        .route(
            "/api/v1/tasks",
            get(list_tasks_handler).post(create_task_handler),
        )
        .route("/api/v1/tasks/id/:id", get(get_task_by_id_handler))
        .route("/api/v1/tasks/:id/status", patch(move_task_status_handler))
        .route("/api/v1/tasks/:id/dispatch", post(dispatch_task_handler))
        .route("/api/v1/tasks/:id/cancel", post(cancel_task_handler))
        .route("/api/v1/tasks/:id/retry", post(retry_task_handler))
        .route("/api/v1/tasks/:id/events", get(list_task_events_handler))
        .route("/api/v1/tasks/:id/attempts", get(list_task_attempts_handler))
        .route(
            "/api/v1/tasks/:id/validations",
            get(list_task_validations_handler),
        )
        .route("/api/v1/approvals", get(list_approvals_handler))
        .route("/api/v1/approvals/:id/decide", post(decide_approval_handler))
        .route("/api/v1/prs", get(list_prs_handler))
        .route("/api/v1/prs/:id/approve-merge", post(approve_pr_merge_handler))
        .route(
            "/api/v1/workflow",
            get(get_workflow_handler).put(update_workflow_handler),
        )
        .route("/api/v1/workflow/preview", post(preview_workflow_handler))
        .route(
            "/api/v1/settings",
            get(get_settings_handler).put(put_settings_handler),
        )
        .route("/api/v1/providers", get(list_providers_handler))
        .route("/api/v1/credentials", post(set_credential_handler))
        .route("/api/v1/diagnostics/export", get(export_diagnostics_handler))
        // SPEC.md Section 13.7.2: `/api/v1/<issue_identifier>` catch-all
        .route("/api/v1/:issue_identifier", get(issue_detail_handler))
        .layer(cors)
        .with_state(state)
}

pub async fn start_http_server(
    orchestrator: Arc<SymphonyOrchestrator>,
    bind_host: &str,
    port: u16,
    remote_bearer_token: Option<String>,
) -> Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let router = build_router(orchestrator, remote_bearer_token);
    let addr_str = format!("{}:{}", bind_host, port);
    let listener = tokio::net::TcpListener::bind(&addr_str).await?;
    let local_addr = listener.local_addr()?;
    info!(address = %local_addr, "Symphony HTTP server listening");

    let handle = tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, router).await {
            tracing::error!(error = %err, "Symphony HTTP server terminated with error");
        }
    });

    Ok((local_addr, handle))
}

fn check_remote_auth(
    state: &ServerState,
    headers: &HeaderMap,
) -> std::result::Result<(), Response> {
    if let Some(expected) = &state.remote_bearer_token {
        if !expected.trim().is_empty() {
            let provided = headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer "))
                .unwrap_or("");
            if provided != expected {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(json!({
                        "error": {
                            "code": "unauthorized",
                            "message": "Valid Bearer token required for remote API access"
                        }
                    })),
                )
                    .into_response());
            }
        }
    }
    Ok(())
}

fn err_response(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({
            "error": {
                "code": code,
                "message": message.into()
            }
        })),
    )
        .into_response()
}

async fn dashboard_html_handler(State(state): State<ServerState>) -> Html<String> {
    let snapshot = state.orchestrator.state_snapshot().await;
    let (generated_at, running, retrying, total_tokens) = match snapshot {
        Ok(s) => (
            s.generated_at.to_rfc3339(),
            s.counts.running,
            s.counts.retrying,
            s.codex_totals.total_tokens,
        ),
        Err(_) => (Utc::now().to_rfc3339(), 0, 0, 0),
    };
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>Symphony Orchestrator Runtime</title>
  <style>
    body {{ font-family: "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif; background: #0d1117; color: #e6edf3; padding: 2rem; }}
    .card {{ background: #161b22; border: 1px solid #30363d; border-radius: 8px; padding: 1.25rem; margin-bottom: 1rem; }}
    h1 {{ margin-top: 0; font-size: 1.4rem; }}
    code {{ background: #21262d; padding: 0.2rem 0.4rem; border-radius: 4px; }}
  </style>
</head>
<body>
  <div class="card">
    <h1>Symphony Desktop 1.0 — Background Orchestration Runtime</h1>
    <p>Status: <strong>Active</strong> | Generated At: <code>{}</code></p>
    <p>Running Sessions: <strong>{}</strong> | Retrying: <strong>{}</strong> | Total Tokens: <strong>{}</strong></p>
    <p>API Endpoints: <code>GET /api/v1/state</code>, <code>POST /api/v1/refresh</code>, <code>GET /api/v1/events</code></p>
  </div>
</body>
</html>"#,
        generated_at, running, retrying, total_tokens
    );
    Html(html)
}

async fn health_handler() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "service": "symphony-orchestrator",
        "version": env!("CARGO_PKG_VERSION"),
        "timestamp": Utc::now()
    }))
}

async fn state_handler(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.state_snapshot().await {
        Ok(snapshot) => Json(snapshot).into_response(),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.code(), e.to_string()),
    }
}

async fn refresh_handler(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    let resp = state.orchestrator.trigger_refresh().await;
    (StatusCode::ACCEPTED, Json(resp)).into_response()
}

async fn sse_events_handler(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    let rx = state.orchestrator.subscribe_events();
    let stream = BroadcastStream::new(rx).filter_map(|msg| match msg {
        Ok(ev) => {
            let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".to_string());
            Some(Ok::<SseEvent, Infallible>(
                SseEvent::default().event(&ev.event_type).data(payload),
            ))
        }
        Err(_) => None,
    });

    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(10))
                .text("keep-alive"),
        )
        .into_response()
}

async fn pause_scheduler_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    let _ = state.orchestrator.set_scheduler_paused(true).await;
    Json(json!({ "scheduler_paused": true })).into_response()
}

async fn resume_scheduler_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    let _ = state.orchestrator.set_scheduler_paused(false).await;
    Json(json!({ "scheduler_paused": false })).into_response()
}

#[derive(Debug, Deserialize)]
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

async fn list_projects_handler(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.list_projects().await {
        Ok(projects) => Json(projects).into_response(),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.code(), e.to_string()),
    }
}

async fn create_project_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(req): Json<CreateProjectRequest>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    let now = Utc::now();
    let workspace_root = req.workspace_root.unwrap_or_else(|| {
        std::env::temp_dir()
            .join("symphony_workspaces")
            .to_string_lossy()
            .to_string()
    });
    let workflow_path = req.workflow_path.or_else(|| {
        Some(
            PathBuf::from(&req.repo_path)
                .join("WORKFLOW.md")
                .to_string_lossy()
                .to_string(),
        )
    });

    let project = Project {
        id: uuid::Uuid::new_v4().to_string(),
        name: req.name,
        repo_path: req.repo_path,
        github_repo: req.github_repo,
        linear_project_slug: req.linear_project_slug,
        default_branch: req.default_branch.unwrap_or_else(|| "main".to_string()),
        workspace_root,
        preferred_provider: req.preferred_provider.unwrap_or(ProviderKind::Codex),
        preferred_model: req.preferred_model,
        workflow_path,
        tracker_kind: req.tracker_kind.unwrap_or_else(|| "manual".to_string()),
        required_labels: req
            .required_labels
            .unwrap_or_else(|| vec!["symphony".to_string()]),
        auto_dispatch: req.auto_dispatch.unwrap_or(false),
        max_concurrent_agents: req.max_concurrent_agents.unwrap_or(2),
        validation_commands: req.validation_commands.unwrap_or_default(),
        created_at: now,
        updated_at: now,
    };

    match state.orchestrator.create_project(project).await {
        Ok(created) => (StatusCode::CREATED, Json(created)).into_response(),
        Err(e) => err_response(StatusCode::BAD_REQUEST, e.code(), e.to_string()),
    }
}

async fn delete_project_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.delete_project(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.code(), e.to_string()),
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct ListTasksQuery {
    pub project_id: Option<String>,
}

async fn list_tasks_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Query(query): Query<ListTasksQuery>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state
        .orchestrator
        .list_tasks(query.project_id.as_deref())
        .await
    {
        Ok(tasks) => Json(tasks).into_response(),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.code(), e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    pub project_id: String,
    pub identifier: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<i32>,
    pub provider: Option<ProviderKind>,
    pub preferred_provider: Option<ProviderKind>,
    pub model: Option<String>,
    pub preferred_model: Option<String>,
    pub labels: Option<Vec<String>>,
    pub auto_start: Option<bool>,
}

async fn create_task_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(req): Json<CreateTaskRequest>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }

    let provider = req.provider.or(req.preferred_provider);
    let model = req.model.or(req.preferred_model);

    match state
        .orchestrator
        .create_manual_task(
            req.project_id,
            req.identifier,
            req.title,
            req.description,
            req.priority,
            provider,
            model,
            req.labels.unwrap_or_default(),
            req.auto_start.unwrap_or(false),
        )
        .await
    {
        Ok(created) => (StatusCode::CREATED, Json(created)).into_response(),
        Err(e) => err_response(StatusCode::BAD_REQUEST, e.code(), e.to_string()),
    }
}

async fn get_task_by_id_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.get_task(&id).await {
        Ok(t) => Json(t).into_response(),
        Err(e) => err_response(StatusCode::NOT_FOUND, e.code(), e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct MoveTaskStatusRequest {
    pub status: TaskStatus,
}

async fn move_task_status_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<MoveTaskStatusRequest>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.transition_task(&id, req.status).await {
        Ok(task) => Json(task).into_response(),
        Err(e) => err_response(StatusCode::BAD_REQUEST, e.code(), e.to_string()),
    }
}

async fn dispatch_task_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.dispatch_task_now(&id).await {
        Ok(task) => Json(task).into_response(),
        Err(e) => err_response(StatusCode::BAD_REQUEST, e.code(), e.to_string()),
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct CancelTaskRequest {
    pub reason: Option<String>,
}

async fn cancel_task_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Option<Json<CancelTaskRequest>>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    let reason = body
        .and_then(|b| b.0.reason)
        .unwrap_or_else(|| "Cancelled by operator".to_string());
    match state.orchestrator.cancel_task(&id, &reason).await {
        Ok(task) => Json(task).into_response(),
        Err(e) => err_response(StatusCode::BAD_REQUEST, e.code(), e.to_string()),
    }
}

async fn retry_task_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.retry_task(&id).await {
        Ok(task) => Json(task).into_response(),
        Err(e) => err_response(StatusCode::BAD_REQUEST, e.code(), e.to_string()),
    }
}

async fn list_task_events_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.list_events(Some(&id), 500).await {
        Ok(events) => Json(events).into_response(),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.code(), e.to_string()),
    }
}

async fn list_task_attempts_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.list_run_attempts(&id).await {
        Ok(attempts) => Json(attempts).into_response(),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.code(), e.to_string()),
    }
}

async fn list_task_validations_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.list_validation_results(&id).await {
        Ok(validations) => Json(validations).into_response(),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.code(), e.to_string()),
    }
}

async fn list_approvals_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.list_approvals(false).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.code(), e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct DecideApprovalRequest {
    pub decision: ApprovalDecision,
}

async fn decide_approval_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<DecideApprovalRequest>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.resolve_approval(&id, req.decision).await {
        Ok(updated) => Json(updated).into_response(),
        Err(e) => err_response(StatusCode::BAD_REQUEST, e.code(), e.to_string()),
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct ListPrsQuery {
    pub project_id: Option<String>,
}

async fn list_prs_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Query(q): Query<ListPrsQuery>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state
        .orchestrator
        .list_pr_reviews(q.project_id.as_deref())
        .await
    {
        Ok(prs) => Json(prs).into_response(),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.code(), e.to_string()),
    }
}

async fn approve_pr_merge_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.approve_pr_merge(&id).await {
        Ok(pr) => Json(pr).into_response(),
        Err(e) => err_response(StatusCode::BAD_REQUEST, e.code(), e.to_string()),
    }
}

async fn get_workflow_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.get_workflow_document(None).await {
        Ok((raw_markdown, config, rendered_preview)) => Json(json!({
            "raw_markdown": raw_markdown,
            "config": config,
            "rendered_preview": rendered_preview,
            "valid": true
        }))
        .into_response(),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.code(), e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateWorkflowRequest {
    pub path: Option<String>,
    pub raw_markdown: String,
}

async fn update_workflow_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(req): Json<UpdateWorkflowRequest>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    let target_path = req
        .path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("WORKFLOW.md"));
    match state
        .orchestrator
        .save_workflow_document(&target_path, &req.raw_markdown)
        .await
    {
        Ok(cfg) => Json(json!({
            "valid": true,
            "config": cfg
        }))
        .into_response(),
        Err(e) => err_response(StatusCode::BAD_REQUEST, e.code(), e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct PreviewWorkflowRequest {
    pub raw_markdown: String,
    pub sample_identifier: Option<String>,
    pub sample_title: Option<String>,
    pub sample_description: Option<String>,
    pub attempt: Option<u32>,
}

async fn preview_workflow_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(req): Json<PreviewWorkflowRequest>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    let wf = match WorkflowDefinition::parse(&req.raw_markdown, None) {
        Ok(w) => w,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e.code(), e.to_string()),
    };
    let cfg = match ServiceConfig::from_workflow(&wf) {
        Ok(c) => c,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e.code(), e.to_string()),
    };

    let sample_issue = symphony_core::domain::Issue {
        id: "101".to_string(),
        native_ref: None,
        identifier: req
            .sample_identifier
            .unwrap_or_else(|| "SYM-101".to_string()),
        title: req
            .sample_title
            .unwrap_or_else(|| "Implement isolated Git worktree validation".to_string()),
        description: Some(req.sample_description.unwrap_or_else(|| {
            "Ensure normalized workspace paths remain inside workspace_root and run cargo test."
                .to_string()
        })),
        priority: Some(1),
        state: "Todo".to_string(),
        branch_name: Some("symphony/sym-101".to_string()),
        url: Some("https://github.com/openai/symphony/issues/101".to_string()),
        assignee_id: None,
        labels: vec!["symphony".to_string(), "rust".to_string()],
        blocked_by: vec![],
        dispatchable: true,
        created_at: Some(Utc::now()),
        updated_at: Some(Utc::now()),
    };

    match render_workflow_prompt(&wf, &sample_issue, req.attempt) {
        Ok(rendered) => Json(json!({
            "valid": true,
            "rendered_prompt": rendered,
            "config": cfg
        }))
        .into_response(),
        Err(e) => err_response(StatusCode::BAD_REQUEST, e.code(), e.to_string()),
    }
}

async fn get_settings_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.storage().get_settings().await {
        Ok(s) => Json(s).into_response(),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.code(), e.to_string()),
    }
}

async fn put_settings_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(settings): Json<AppSettings>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.storage().save_settings(&settings).await {
        Ok(()) => Json(settings).into_response(),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.code(), e.to_string()),
    }
}

async fn list_providers_handler(State(state): State<ServerState>) -> impl IntoResponse {
    let profiles = state.orchestrator.detect_providers();
    Json(profiles)
}

#[derive(Debug, Deserialize)]
pub struct SetCredentialRequest {
    pub key: String,
    pub value: String,
}

async fn set_credential_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(req): Json<SetCredentialRequest>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state
        .orchestrator
        .secret_store()
        .set_secret(&req.key, &req.value)
    {
        Ok(()) => {
            std::env::set_var(&req.key, &req.value);
            Json(json!({ "saved": true, "key": req.key })).into_response()
        }
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.code(), e.to_string()),
    }
}

async fn export_diagnostics_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.storage().export_diagnostics(&[]).await {
        Ok(bundle) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            bundle,
        )
            .into_response(),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.code(), e.to_string()),
    }
}

/// SPEC.md Section 13.7.2: `GET /api/v1/<issue_identifier>`
async fn issue_detail_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Path(issue_identifier): Path<String>,
) -> Response {
    if let Err(r) = check_remote_auth(&state, &headers) {
        return r;
    }
    match state.orchestrator.issue_detail(&issue_identifier).await {
        Ok(detail) => Json(detail).into_response(),
        Err(e) => err_response(StatusCode::NOT_FOUND, e.code(), e.to_string()),
    }
}
