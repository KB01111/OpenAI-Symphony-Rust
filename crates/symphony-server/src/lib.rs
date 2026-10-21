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
//! - The optional remote companion listener ([`remote`]) uses its own token and serves only a
//!   restricted subset of operations ([`remote::REMOTE_OPERATIONS`]).

pub mod remote;

pub use remote::{
    RemoteAccessManager, RemoteAccessStatus, RemoteListenerState, RemoteTokenIssued, TokenSlot,
    REMOTE_OPERATIONS, REMOTE_TOKEN_KEY,
};

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
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use futures::stream::{self, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::info;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::openapi::{RefOr, Schema};
use utoipa::{IntoParams, Modify, OpenApi, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};

use symphony_core::{
    domain::{
        AgentEvent, AppSettings, ApprovalDecision, ApprovalRequest, LocalePreference,
        OrchestratorStateSnapshot, Project, ProviderKind, ProviderStatus, PullRequestReview,
        RepositoryValidation, RunAttempt, RuntimeInfo, Task, TaskDiff, TaskStatus, ThemePreference,
        ValidationResult, WorktreeInfo,
    },
    error::{Result, SymphonyError},
};
use symphony_orchestrator::{
    IssueDetailResponse, ProjectUpdate, RefreshResponse, SymphonyOrchestrator, WorkflowDocument,
    WorkflowValidation,
};

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

/// Generate a random 256-bit hex access token. It is registered for log redaction, so neither
/// the local nor a remote token can reach a log line.
pub fn generate_access_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    symphony_platform::logging::redact_from_logs(&token);
    token
}

/// Which clients a listener serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiScope {
    /// Local administration (desktop webview, CLI tools): every operation.
    Local,
    /// Remote companion: only [`remote::REMOTE_OPERATIONS`].
    Remote,
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
    pub scope: ApiScope,
    /// Remote listener manager exposed through the local-only `/remote-access` endpoints.
    pub remote_access: Option<Arc<RemoteAccessManager>>,
}

impl ServerConfig {
    pub fn local(port: u16, access_token: Option<String>) -> Self {
        Self {
            bind_host: "127.0.0.1".to_string(),
            port,
            access_token,
            allowed_origins: default_local_origins(),
            default_workspace_root: None,
            scope: ApiScope::Local,
            remote_access: None,
        }
    }
}

#[derive(Clone)]
pub struct ServerState {
    pub orchestrator: Arc<SymphonyOrchestrator>,
    /// Token required on protected routes; `None` disables authentication (tests only).
    pub auth: Option<TokenSlot>,
    pub scope: ApiScope,
    pub default_workspace_root: Option<PathBuf>,
    pub started_at: DateTime<Utc>,
    pub remote_access: Option<Arc<RemoteAccessManager>>,
}

// ============================================================================
// Request / response contracts (described in the OpenAPI document)
// ============================================================================

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ApiErrorBody {
    pub error: ApiErrorDetail,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ApiErrorDetail {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
    pub version: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SchedulerStateResponse {
    pub scheduler_paused: bool,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
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
    #[schema(minimum = 1, maximum = 32)]
    pub max_concurrent_agents: Option<u32>,
    pub validation_commands: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ValidateRepositoryRequest {
    pub repo_path: String,
}

#[derive(Debug, Deserialize, Default, Serialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListTasksQuery {
    pub project_id: Option<String>,
    pub status: Option<TaskStatus>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
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

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct MoveTaskStatusRequest {
    pub status: TaskStatus,
}

#[derive(Debug, Deserialize, Default, Serialize, ToSchema)]
pub struct CancelTaskRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, Default, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LimitQuery {
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, Default, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ApprovalsQuery {
    pub only_pending: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct DecideApprovalRequest {
    pub decision: ApprovalDecision,
}

#[derive(Debug, Deserialize, Default, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ProjectQuery {
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateWorkflowRequest {
    pub raw_markdown: String,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PreviewWorkflowRequest {
    pub raw_markdown: String,
    pub sample_identifier: Option<String>,
    pub sample_title: Option<String>,
    pub sample_description: Option<String>,
    pub attempt: Option<u32>,
}

#[derive(Debug, Deserialize, Default, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ProvidersQuery {
    pub refresh: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SetCredentialRequest {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CredentialKeysResponse {
    /// Names of configured credentials. Values are never returned.
    pub configured: Vec<String>,
    pub allowed: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CredentialSavedResponse {
    pub saved: bool,
    pub key: String,
}

/// Payload of the `ready` and `resync` SSE events.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct StreamControlEvent {
    pub api_version: u32,
    pub server_time: DateTime<Utc>,
    pub reason: Option<String>,
}

// ============================================================================
// Router
// ============================================================================

/// Root of the OpenAPI document. Paths are collected from the `#[utoipa::path]` annotations of
/// the handlers registered in [`api_routes`]; [`ApiContract`] adds what annotations cannot say.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Symphony Runtime API",
        description = "Local orchestration runtime API used by Symphony Desktop, the mobile companion and local tools."
    ),
    tags(
        (name = "runtime", description = "Health, discovery and orchestrator state"),
        (name = "scheduler", description = "Pause and resume dispatching"),
        (name = "events", description = "Live event stream and event history"),
        (name = "projects", description = "Registered repositories"),
        (name = "tasks", description = "Tasks, attempts, validations and diffs"),
        (name = "issues", description = "Tracker issue detail"),
        (name = "approvals", description = "Agent approval requests"),
        (name = "reviews", description = "Pull request reviews and merge readiness"),
        (name = "worktrees", description = "Agent workspaces"),
        (name = "workflow", description = "WORKFLOW.md editing and validation"),
        (name = "settings", description = "Application settings"),
        (name = "providers", description = "Coding agent providers"),
        (name = "credentials", description = "Credential vault (key names only)"),
        (name = "diagnostics", description = "Redacted diagnostics export"),
        (name = "remote-access", description = "Remote companion listener and its access token (local administration only)"),
    ),
    // The shared error body plus types that travel over the event stream or document string
    // fields; no JSON operation references them directly.
    components(schemas(
        ApiErrorBody,
        AgentEvent,
        StreamControlEvent,
        ThemePreference,
        LocalePreference
    )),
    modifiers(&ApiContract)
)]
pub struct ApiDoc;

/// Name of the bearer-token security scheme in the OpenAPI document.
pub const BEARER_SCHEME: &str = "bearer_auth";

/// Routes that answer without a token (`health`, `runtime`).
const PUBLIC_PATHS: &[&str] = &["/api/v1/health", "/api/v1/runtime"];

/// Request bodies whose `Option` fields may be omitted. Every other schema lists all of its
/// properties as required (nullable where the Rust field is an `Option`), because the server
/// always serializes every field.
const OPTIONAL_FIELD_SCHEMAS: &[&str] = &[
    "CreateProjectRequest",
    "CreateTaskRequest",
    "CancelTaskRequest",
    "PreviewWorkflowRequest",
    "ProjectUpdate",
];

/// Individual properties added after 2.0 that older clients may omit from request bodies
/// (`#[serde(default)]` on the Rust side). Keeping them optional keeps the change non-breaking.
const OPTIONAL_PROPERTIES: &[(&str, &str)] = &[("AppSettings", "remote_allowed_origins")];

/// OpenAPI extension that marks whether an operation is available on the remote listener.
pub const REMOTE_EXTENSION: &str = "x-symphony-remote";

/// Completes the generated document: bearer security on protected operations, the shared error
/// response, response descriptions and exact `required` lists.
struct ApiContract;

impl Modify for ApiContract {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            BEARER_SCHEME,
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "Per-launch token from the desktop host or `<data_dir>/runtime.json`",
                    ))
                    .build(),
            ),
        );
        for (name, schema) in components.schemas.iter_mut() {
            if OPTIONAL_FIELD_SCHEMAS.contains(&name.as_str()) {
                continue;
            }
            if let RefOr::T(Schema::Object(object)) = schema {
                object.required = object
                    .properties
                    .keys()
                    .filter(|prop| !OPTIONAL_PROPERTIES.contains(&(name.as_str(), prop.as_str())))
                    .cloned()
                    .collect();
            }
        }

        let error = utoipa::openapi::ResponseBuilder::new()
            .description("Error (`ApiErrorBody`)")
            .content(
                "application/json",
                utoipa::openapi::ContentBuilder::new()
                    .schema(Some(utoipa::openapi::Ref::from_schema_name("ApiErrorBody")))
                    .build(),
            )
            .build();
        for (path, item) in openapi.paths.paths.iter_mut() {
            let protected = !PUBLIC_PATHS.contains(&path.as_str());
            let relative = path.strip_prefix("/api/v1").unwrap_or(path).to_string();
            for (method, operation) in [
                (Method::GET, &mut item.get),
                (Method::PUT, &mut item.put),
                (Method::POST, &mut item.post),
                (Method::DELETE, &mut item.delete),
                (Method::PATCH, &mut item.patch),
            ]
            .into_iter()
            .filter_map(|(m, op)| op.as_mut().map(|op| (m, op)))
            {
                operation.extensions = Some(
                    utoipa::openapi::extensions::ExtensionsBuilder::new()
                        .add(
                            REMOTE_EXTENSION,
                            remote::remote_operation_allowed(&method, &relative),
                        )
                        .build(),
                );
                operation
                    .responses
                    .responses
                    .entry("default".into())
                    .or_insert_with(|| RefOr::T(error.clone()));
                // OpenAPI requires a description on every response object.
                for (status, response) in operation.responses.responses.iter_mut() {
                    if let RefOr::T(response) = response {
                        if response.description.is_empty() {
                            response.description = status
                                .parse::<u16>()
                                .ok()
                                .and_then(|code| StatusCode::from_u16(code).ok())
                                .and_then(|code| code.canonical_reason())
                                .unwrap_or("Response")
                                .to_string();
                        }
                    }
                }
                if protected {
                    operation.security =
                        Some(vec![utoipa::openapi::security::SecurityRequirement::new::<
                            &str,
                            [&str; 0],
                            &str,
                        >(BEARER_SCHEME, [])]);
                }
            }
        }
    }
}

/// Documented API routes, split into public and token-protected groups (relative to `/api/v1`).
fn api_routes() -> (OpenApiRouter<ServerState>, OpenApiRouter<ServerState>) {
    let public = OpenApiRouter::new()
        .routes(routes!(health_handler))
        .routes(routes!(runtime_handler));
    let protected = OpenApiRouter::new()
        .routes(routes!(state_handler))
        .routes(routes!(refresh_handler))
        .routes(routes!(sse_events_handler))
        .routes(routes!(recent_events_handler))
        .routes(routes!(pause_scheduler_handler))
        .routes(routes!(resume_scheduler_handler))
        .routes(routes!(list_projects_handler, create_project_handler))
        .routes(routes!(validate_repository_handler))
        .routes(routes!(
            get_project_handler,
            update_project_handler,
            delete_project_handler
        ))
        .routes(routes!(list_tasks_handler, create_task_handler))
        .routes(routes!(get_task_by_id_handler))
        .routes(routes!(move_task_status_handler))
        .routes(routes!(dispatch_task_handler))
        .routes(routes!(cancel_task_handler))
        .routes(routes!(retry_task_handler))
        .routes(routes!(list_task_events_handler))
        .routes(routes!(list_task_attempts_handler))
        .routes(routes!(list_task_validations_handler))
        .routes(routes!(task_diff_handler))
        .routes(routes!(list_approvals_handler))
        .routes(routes!(decide_approval_handler))
        .routes(routes!(list_prs_handler))
        .routes(routes!(get_pr_handler))
        .routes(routes!(approve_pr_merge_handler))
        .routes(routes!(list_worktrees_handler))
        .routes(routes!(get_workflow_handler, update_workflow_handler))
        .routes(routes!(preview_workflow_handler))
        .routes(routes!(get_settings_handler, put_settings_handler))
        .routes(routes!(list_providers_handler))
        .routes(routes!(list_credentials_handler, set_credential_handler))
        .routes(routes!(delete_credential_handler))
        .routes(routes!(export_diagnostics_handler))
        .routes(routes!(issue_detail_handler))
        .routes(routes!(get_remote_access_handler))
        .routes(routes!(
            rotate_remote_token_handler,
            revoke_remote_token_handler
        ))
        // Undocumented compatibility routes: the legacy task alias and the SPEC.md Section
        // 13.7.2 `/api/v1/<issue_identifier>` catch-all (registered last-resort by axum).
        .route("/tasks/id/{id}", get(get_task_by_id_handler))
        .route("/{issue_identifier}", get(issue_detail_handler));
    (public, protected)
}

/// The OpenAPI document of `/api/v1`, generated from the same route table that serves requests.
pub fn openapi() -> utoipa::openapi::OpenApi {
    let (public, protected) = api_routes();
    let (_, mut api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .nest("/api/v1", public.merge(protected))
        .split_for_parts();
    // Modifiers declared on `ApiDoc` run before the routes are merged; apply the contract again
    // so it covers every collected path and schema.
    ApiContract.modify(&mut api);
    api
}

pub fn build_router(orchestrator: Arc<SymphonyOrchestrator>, config: &ServerConfig) -> Router {
    build_router_inner(orchestrator, config, None)
}

/// Build a router; `token` overrides `config.access_token` with a live, replaceable token (the
/// remote listener's). A remote-scope router always requires a token, even an absent one.
pub(crate) fn build_router_inner(
    orchestrator: Arc<SymphonyOrchestrator>,
    config: &ServerConfig,
    token: Option<TokenSlot>,
) -> Router {
    let auth = match (token, config.scope) {
        (Some(slot), _) => Some(slot),
        (None, ApiScope::Remote) => Some(TokenSlot::new(None)),
        (None, ApiScope::Local) => config
            .access_token
            .clone()
            .filter(|t| !t.trim().is_empty())
            .map(|t| TokenSlot::new(Some(t))),
    };
    let state = ServerState {
        orchestrator,
        auth,
        scope: config.scope,
        default_workspace_root: config.default_workspace_root.clone(),
        started_at: Utc::now(),
        remote_access: match config.scope {
            ApiScope::Local => config.remote_access.clone(),
            ApiScope::Remote => None,
        },
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

    let (public, protected) = api_routes();
    // Route layers run last-added first: authentication, then the remote scope check.
    let protected = match config.scope {
        ApiScope::Local => protected,
        ApiScope::Remote => {
            protected.route_layer(middleware::from_fn(remote::enforce_remote_scope))
        }
    };
    let protected = protected.route_layer(middleware::from_fn_with_state(
        state.clone(),
        require_bearer_token,
    ));
    let (api, _) = public.merge(protected).split_for_parts();

    let router = Router::new()
        .route("/", get(root_handler))
        .nest("/api/v1", api);
    let router = match config.scope {
        ApiScope::Local => router,
        ApiScope::Remote => router.layer(middleware::from_fn(remote::reject_cleartext_forwarding)),
    };
    router.layer(cors).with_state(state)
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
    if let Some(slot) = &state.auth {
        let provided = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .unwrap_or("");
        // A revoked (absent) token rejects every request.
        let valid = slot
            .current()
            .is_some_and(|expected| token_matches(&expected, provided.trim()));
        if !valid {
            return api_error(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "A valid Bearer token is required",
            );
        }
    }
    next.run(request).await
}

pub(crate) fn api_error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
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

#[utoipa::path(
    get,
    path = "/health",
    operation_id = "health",
    tag = "runtime",
    responses((status = 200, body = HealthResponse))
)]
async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".into(),
        service: "symphony-orchestrator".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        timestamp: Utc::now(),
    })
}

#[utoipa::path(
    get,
    path = "/runtime",
    operation_id = "getRuntime",
    tag = "runtime",
    responses((status = 200, body = RuntimeInfo))
)]
async fn runtime_handler(State(state): State<ServerState>) -> Json<RuntimeInfo> {
    Json(RuntimeInfo {
        service: "symphony-orchestrator".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        api_version: API_VERSION,
        auth_required: state.auth.is_some(),
        started_at: state.started_at,
        event_stream: "/api/v1/events".into(),
    })
}

// ============================================================================
// State & scheduler
// ============================================================================

#[utoipa::path(
    get,
    path = "/state",
    operation_id = "getState",
    tag = "runtime",
    responses((status = 200, body = OrchestratorStateSnapshot))
)]
async fn state_handler(State(state): State<ServerState>) -> Response {
    respond(state.orchestrator.state_snapshot().await)
}

#[utoipa::path(
    post,
    path = "/refresh",
    operation_id = "refresh",
    tag = "runtime",
    responses((status = 202, body = RefreshResponse))
)]
async fn refresh_handler(State(state): State<ServerState>) -> Response {
    let resp = state.orchestrator.trigger_refresh().await;
    (StatusCode::ACCEPTED, Json(resp)).into_response()
}

#[utoipa::path(
    post,
    path = "/scheduler/pause",
    operation_id = "pauseScheduler",
    tag = "scheduler",
    responses((status = 200, body = SchedulerStateResponse))
)]
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

#[utoipa::path(
    post,
    path = "/scheduler/resume",
    operation_id = "resumeScheduler",
    tag = "scheduler",
    responses((status = 200, body = SchedulerStateResponse))
)]
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
#[utoipa::path(
    get,
    path = "/events",
    operation_id = "streamEvents",
    tag = "events",
    params(("Last-Event-ID" = Option<String>, Header, description = "Resume after this event id; persisted events newer than it are replayed first.")),
    responses((status = 200, description = "Server-Sent Events. `ready`/`resync` carry a `StreamControlEvent`; every other event is an `AgentEvent` named by its `event_type` with `id` = event id.", content_type = "text/event-stream", body = String))
)]
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
    // End the stream when the token it was opened with is rotated or revoked. Subscribe first,
    // then confirm the presented token is still current: a rotation between the auth middleware
    // and this point is either seen here (401) or by the subscription (stream ends).
    let token_changed = {
        let mut rx = state.auth.as_ref().map(TokenSlot::subscribe);
        if let (Some(rx), Some(slot)) = (rx.as_mut(), state.auth.as_ref()) {
            rx.mark_unchanged();
            let presented = headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer "))
                .unwrap_or("")
                .trim();
            if !slot
                .current()
                .is_some_and(|current| token_matches(&current, presented))
            {
                return api_error(
                    StatusCode::UNAUTHORIZED,
                    "unauthorized",
                    "A valid Bearer token is required",
                );
            }
        }
        async move {
            match rx {
                Some(mut rx) => {
                    let _ = rx.changed().await;
                }
                None => futures::future::pending::<()>().await,
            }
        }
    };
    let combined: std::pin::Pin<
        Box<dyn Stream<Item = std::result::Result<SseEvent, Infallible>> + Send>,
    > = Box::pin(
        stream::iter(initial)
            .chain(live)
            .take_until(Box::pin(token_changed))
            .map(Ok),
    );

    Sse::new(combined)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

/// `GET /api/v1/events/recent?limit=N` — persisted events across all tasks, newest first.
#[utoipa::path(
    get,
    path = "/events/recent",
    operation_id = "listRecentEvents",
    tag = "events",
    params(LimitQuery),
    responses((status = 200, description = "Newest first", body = Vec<AgentEvent>))
)]
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

#[utoipa::path(
    get,
    path = "/projects",
    operation_id = "listProjects",
    tag = "projects",
    responses((status = 200, body = Vec<Project>))
)]
async fn list_projects_handler(State(state): State<ServerState>) -> Response {
    respond(state.orchestrator.list_projects().await)
}

#[utoipa::path(
    get,
    path = "/projects/{id}",
    operation_id = "getProject",
    tag = "projects",
    params(("id" = String, Path, description = "Project id")),
    responses((status = 200, body = Project))
)]
async fn get_project_handler(State(state): State<ServerState>, Path(id): Path<String>) -> Response {
    match state.orchestrator.list_projects().await {
        Ok(list) => match list.into_iter().find(|p| p.id == id) {
            Some(p) => Json(p).into_response(),
            None => error_response(SymphonyError::ProjectNotFound(id)),
        },
        Err(e) => error_response(e),
    }
}

#[utoipa::path(
    post,
    path = "/projects/validate",
    operation_id = "validateRepository",
    tag = "projects",
    request_body = ValidateRepositoryRequest,
    responses((status = 200, body = RepositoryValidation))
)]
async fn validate_repository_handler(
    State(state): State<ServerState>,
    Json(req): Json<ValidateRepositoryRequest>,
) -> Response {
    Json(state.orchestrator.validate_repository(&req.repo_path).await).into_response()
}

#[utoipa::path(
    post,
    path = "/projects",
    operation_id = "createProject",
    tag = "projects",
    request_body = CreateProjectRequest,
    responses((status = 201, body = Project))
)]
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

#[utoipa::path(
    patch,
    path = "/projects/{id}",
    operation_id = "updateProject",
    tag = "projects",
    params(("id" = String, Path, description = "Project id")),
    request_body = ProjectUpdate,
    responses((status = 200, body = Project))
)]
async fn update_project_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(update): Json<ProjectUpdate>,
) -> Response {
    respond(state.orchestrator.update_project(&id, update).await)
}

#[utoipa::path(
    delete,
    path = "/projects/{id}",
    operation_id = "deleteProject",
    tag = "projects",
    params(("id" = String, Path, description = "Project id")),
    responses((status = 204, description = "Deleted"))
)]
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

#[utoipa::path(
    get,
    path = "/tasks",
    operation_id = "listTasks",
    tag = "tasks",
    params(ListTasksQuery),
    responses((status = 200, body = Vec<Task>))
)]
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

#[utoipa::path(
    post,
    path = "/tasks",
    operation_id = "createTask",
    tag = "tasks",
    request_body = CreateTaskRequest,
    responses((status = 201, body = Task))
)]
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

#[utoipa::path(
    get,
    path = "/tasks/{id}",
    operation_id = "getTask",
    tag = "tasks",
    params(("id" = String, Path, description = "Task id")),
    responses((status = 200, body = Task))
)]
async fn get_task_by_id_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Response {
    respond(state.orchestrator.get_task(&id).await)
}

#[utoipa::path(
    patch,
    path = "/tasks/{id}/status",
    operation_id = "moveTaskStatus",
    tag = "tasks",
    params(("id" = String, Path, description = "Task id")),
    request_body = MoveTaskStatusRequest,
    responses((status = 200, body = Task))
)]
async fn move_task_status_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(req): Json<MoveTaskStatusRequest>,
) -> Response {
    respond(state.orchestrator.transition_task(&id, req.status).await)
}

#[utoipa::path(
    post,
    path = "/tasks/{id}/dispatch",
    operation_id = "dispatchTask",
    tag = "tasks",
    params(("id" = String, Path, description = "Task id")),
    responses((status = 200, body = Task))
)]
async fn dispatch_task_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Response {
    respond(state.orchestrator.dispatch_task_now(&id).await)
}

#[utoipa::path(
    post,
    path = "/tasks/{id}/cancel",
    operation_id = "cancelTask",
    tag = "tasks",
    params(("id" = String, Path, description = "Task id")),
    request_body(content = Option<CancelTaskRequest>, description = "Optional cancellation reason"),
    responses((status = 200, body = Task))
)]
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

#[utoipa::path(
    post,
    path = "/tasks/{id}/retry",
    operation_id = "retryTask",
    tag = "tasks",
    params(("id" = String, Path, description = "Task id")),
    responses((status = 200, body = Task))
)]
async fn retry_task_handler(State(state): State<ServerState>, Path(id): Path<String>) -> Response {
    respond(state.orchestrator.retry_task(&id).await)
}

#[utoipa::path(
    get,
    path = "/tasks/{id}/events",
    operation_id = "listTaskEvents",
    tag = "tasks",
    params(("id" = String, Path, description = "Task id"), LimitQuery),
    responses((status = 200, description = "Newest first", body = Vec<AgentEvent>))
)]
async fn list_task_events_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Query(q): Query<LimitQuery>,
) -> Response {
    let limit = q.limit.unwrap_or(500).clamp(1, 500);
    respond(state.orchestrator.list_events(Some(&id), limit).await)
}

#[utoipa::path(
    get,
    path = "/tasks/{id}/attempts",
    operation_id = "listTaskAttempts",
    tag = "tasks",
    params(("id" = String, Path, description = "Task id")),
    responses((status = 200, body = Vec<RunAttempt>))
)]
async fn list_task_attempts_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Response {
    respond(state.orchestrator.list_run_attempts(&id).await)
}

#[utoipa::path(
    get,
    path = "/tasks/{id}/validations",
    operation_id = "listTaskValidations",
    tag = "tasks",
    params(("id" = String, Path, description = "Task id")),
    responses((status = 200, body = Vec<ValidationResult>))
)]
async fn list_task_validations_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Response {
    respond(state.orchestrator.list_validation_results(&id).await)
}

#[utoipa::path(
    get,
    path = "/tasks/{id}/diff",
    operation_id = "getTaskDiff",
    tag = "tasks",
    params(("id" = String, Path, description = "Task id")),
    responses((status = 200, body = TaskDiff))
)]
async fn task_diff_handler(State(state): State<ServerState>, Path(id): Path<String>) -> Response {
    respond(state.orchestrator.task_diff(&id).await)
}

#[utoipa::path(
    get,
    path = "/issues/{issue_identifier}",
    operation_id = "getIssueDetail",
    tag = "issues",
    params(("issue_identifier" = String, Path, description = "Tracker issue identifier, e.g. `SYM-12`")),
    responses((status = 200, body = IssueDetailResponse))
)]
async fn issue_detail_handler(
    State(state): State<ServerState>,
    Path(issue_identifier): Path<String>,
) -> Response {
    respond(state.orchestrator.issue_detail(&issue_identifier).await)
}

// ============================================================================
// Approvals & pull requests
// ============================================================================

#[utoipa::path(
    get,
    path = "/approvals",
    operation_id = "listApprovals",
    tag = "approvals",
    params(ApprovalsQuery),
    responses((status = 200, body = Vec<ApprovalRequest>))
)]
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

#[utoipa::path(
    post,
    path = "/approvals/{id}/decide",
    operation_id = "decideApproval",
    tag = "approvals",
    params(("id" = String, Path, description = "Approval id")),
    request_body = DecideApprovalRequest,
    responses((status = 200, body = ApprovalRequest))
)]
async fn decide_approval_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(req): Json<DecideApprovalRequest>,
) -> Response {
    respond(state.orchestrator.resolve_approval(&id, req.decision).await)
}

#[utoipa::path(
    get,
    path = "/prs",
    operation_id = "listPrReviews",
    tag = "reviews",
    params(ProjectQuery),
    responses((status = 200, body = Vec<PullRequestReview>))
)]
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

#[utoipa::path(
    get,
    path = "/prs/{id}",
    operation_id = "getPrReview",
    tag = "reviews",
    params(("id" = String, Path, description = "Review id")),
    responses((status = 200, body = PullRequestReview))
)]
async fn get_pr_handler(State(state): State<ServerState>, Path(id): Path<String>) -> Response {
    respond(state.orchestrator.get_pr_review(&id).await)
}

#[utoipa::path(
    post,
    path = "/prs/{id}/approve-merge",
    operation_id = "approvePrMerge",
    tag = "reviews",
    params(("id" = String, Path, description = "Review id")),
    responses((status = 200, body = PullRequestReview))
)]
async fn approve_pr_merge_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Response {
    respond(state.orchestrator.approve_pr_merge(&id).await)
}

#[utoipa::path(
    get,
    path = "/worktrees",
    operation_id = "listWorktrees",
    tag = "worktrees",
    responses((status = 200, body = Vec<WorktreeInfo>))
)]
async fn list_worktrees_handler(State(state): State<ServerState>) -> Response {
    respond(state.orchestrator.list_worktrees().await)
}

// ============================================================================
// Workflow
// ============================================================================

#[utoipa::path(
    get,
    path = "/workflow",
    operation_id = "getWorkflow",
    tag = "workflow",
    responses((status = 200, body = WorkflowDocument))
)]
async fn get_workflow_handler(State(state): State<ServerState>) -> Response {
    Json(state.orchestrator.get_workflow_document().await).into_response()
}

#[utoipa::path(
    put,
    path = "/workflow",
    operation_id = "updateWorkflow",
    tag = "workflow",
    request_body = UpdateWorkflowRequest,
    responses((status = 200, body = WorkflowValidation))
)]
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
#[utoipa::path(
    post,
    path = "/workflow/preview",
    operation_id = "previewWorkflow",
    tag = "workflow",
    request_body = PreviewWorkflowRequest,
    responses((status = 200, description = "Always 200; invalid documents report `valid: false`", body = WorkflowValidation))
)]
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

#[utoipa::path(
    get,
    path = "/settings",
    operation_id = "getSettings",
    tag = "settings",
    responses((status = 200, body = AppSettings))
)]
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
        .any(|k| ALLOWED_CREDENTIAL_KEYS.contains(&k.as_str()) || k == REMOTE_TOKEN_KEY)
    {
        return Err("Secrets must be stored through the credential vault, not settings".into());
    }
    remote::validate_remote_settings(settings)?;
    Ok(())
}

#[utoipa::path(
    put,
    path = "/settings",
    operation_id = "updateSettings",
    tag = "settings",
    request_body = AppSettings,
    responses((status = 200, body = AppSettings))
)]
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
            // Apply remote-access changes to the running listener right away; the effective
            // state (including bind failures) is reported by `GET /remote-access`.
            if let Some(remote) = &state.remote_access {
                remote.apply_persisted(&settings).await;
            }
            Json(settings).into_response()
        }
        Err(e) => error_response(e),
    }
}

#[utoipa::path(
    get,
    path = "/providers",
    operation_id = "listProviders",
    tag = "providers",
    params(ProvidersQuery),
    responses((status = 200, body = Vec<ProviderStatus>))
)]
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

#[utoipa::path(
    get,
    path = "/credentials",
    operation_id = "listCredentials",
    tag = "credentials",
    responses((status = 200, body = CredentialKeysResponse))
)]
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

#[utoipa::path(
    post,
    path = "/credentials",
    operation_id = "setCredential",
    tag = "credentials",
    request_body = SetCredentialRequest,
    responses((status = 200, body = CredentialSavedResponse))
)]
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

#[utoipa::path(
    delete,
    path = "/credentials/{key}",
    operation_id = "deleteCredential",
    tag = "credentials",
    params(("key" = String, Path, description = "Credential key from the allowlist")),
    responses((status = 204, description = "Deleted"))
)]
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

#[utoipa::path(
    get,
    path = "/diagnostics/export",
    operation_id = "exportDiagnostics",
    tag = "diagnostics",
    responses((status = 200, description = "Redacted diagnostics bundle", body = Object))
)]
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

// ============================================================================
// Remote access (local administration only)
// ============================================================================

fn remote_unavailable() -> Response {
    api_error(
        StatusCode::NOT_IMPLEMENTED,
        "remote_access_unavailable",
        "This runtime host does not manage a remote listener",
    )
}

/// Persisted remote-access settings together with the listener's effective state.
#[utoipa::path(
    get,
    path = "/remote-access",
    operation_id = "getRemoteAccess",
    tag = "remote-access",
    responses((status = 200, body = RemoteAccessStatus))
)]
async fn get_remote_access_handler(State(state): State<ServerState>) -> Response {
    match &state.remote_access {
        Some(remote) => Json(remote.status().await).into_response(),
        None => remote_unavailable(),
    }
}

/// Generate a new remote access token (256 random bits), replacing any previous one. The token
/// is returned once and cannot be read back; open remote sessions using the old token end.
#[utoipa::path(
    post,
    path = "/remote-access/token",
    operation_id = "rotateRemoteAccessToken",
    tag = "remote-access",
    responses((status = 200, body = RemoteTokenIssued))
)]
async fn rotate_remote_token_handler(State(state): State<ServerState>) -> Response {
    match &state.remote_access {
        Some(remote) => match remote.rotate_token().await {
            Ok(issued) => ([(header::CACHE_CONTROL, "no-store")], Json(issued)).into_response(),
            Err(e) => error_response(e),
        },
        None => remote_unavailable(),
    }
}

/// Revoke the remote access token and stop the remote listener.
#[utoipa::path(
    delete,
    path = "/remote-access/token",
    operation_id = "revokeRemoteAccessToken",
    tag = "remote-access",
    responses((status = 200, body = RemoteAccessStatus))
)]
async fn revoke_remote_token_handler(State(state): State<ServerState>) -> Response {
    match &state.remote_access {
        Some(remote) => respond(remote.revoke_token().await),
        None => remote_unavailable(),
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
