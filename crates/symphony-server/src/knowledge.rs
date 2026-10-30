//! Repository Knowledge Base endpoints (`/api/v1/projects/{id}/knowledge/...`).
//!
//! Local only: they expose diffs, source structure and AI output, so none of them is in
//! `REMOTE_OPERATIONS`. Commit ids from the client are validated as hex before they reach git;
//! node ids are opaque keys into the index; no client-supplied path is ever used.

use super::*;
use symphony_core::redact_secrets;
use symphony_knowledge::model::{
    ChatMessage, CommitAnalysis, CommitDetail, CommitImpact, CommitTimeline, EngineStatus,
    GraphLevel, GraphView, IndexRunSummary, KnowledgeOverview, KnowledgeWorktree, NodeDetail,
    SearchHit, SymbolReferences,
};
use symphony_knowledge::service::{GraphQuery, KnowledgeService};
use symphony_knowledge::timeline::ProjectRef;

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct StartKnowledgeIndexRequest {
    /// Index the HEAD of this worktree instead of the default branch.
    pub worktree_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct StartKnowledgeAnalysisRequest {
    /// Run again even when an analysis for this configuration is cached.
    pub force: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AskKnowledgeRequest {
    /// 1 to 2000 characters.
    pub question: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CommitAnalysisResponse {
    /// The newest analysis of the commit, or `null` when it has never been analysed.
    pub analysis: Option<CommitAnalysis>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CommitDiffResponse {
    pub sha: String,
    /// Unified diff against the first parent, secrets redacted.
    pub diff: String,
    pub truncated: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct KnowledgeClearedResponse {
    pub cleared: bool,
}

#[derive(Debug, Deserialize, Default, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct KnowledgeGraphQuery {
    /// `modules` (default), `files` or `symbols`.
    pub level: Option<GraphLevel>,
    /// Scope the `files` level to this module.
    pub module: Option<String>,
    /// Node the `symbols` level is centred on.
    pub focus: Option<String>,
    /// Hops around the focus, 1 to 3 (default 1).
    pub depth: Option<u32>,
    /// Include inferred edges (default true).
    pub include_inferred: Option<bool>,
    /// Include external dependencies in the `modules` level (default false).
    pub include_external: Option<bool>,
}

#[derive(Debug, Deserialize, Default, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct KnowledgeSearchQuery {
    pub q: String,
    /// 1 to 100, default 25.
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, Default, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct KnowledgeNodeQuery {
    pub node_id: String,
}

#[derive(Debug, Deserialize, Default, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct KnowledgeCommitsQuery {
    /// 1 to 500, default 100.
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, Default, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct KnowledgeImpactQuery {
    /// Hops through dependents, 1 to 4 (default 2).
    pub depth: Option<u32>,
    /// Follow inferred edges too (default true).
    pub include_inferred: Option<bool>,
}

async fn context(
    state: &ServerState,
    project_id: &str,
) -> Result<(Arc<KnowledgeService>, ProjectRef)> {
    let knowledge = state.orchestrator.knowledge()?;
    let project = state.orchestrator.knowledge_project(project_id).await?;
    Ok((knowledge, project))
}

fn respond_status<T: Serialize>(status: StatusCode, result: Result<T>) -> Response {
    match result {
        Ok(value) => (status, Json(value)).into_response(),
        Err(error) => error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/knowledge/engines",
    operation_id = "getKnowledgeEngines",
    tag = "knowledge",
    responses((status = 200, description = "git, Graphify and Serena: what is installed and how to install the rest", body = Vec<EngineStatus>))
)]
pub(crate) async fn engines_handler(State(state): State<ServerState>) -> Response {
    match state.orchestrator.knowledge() {
        Ok(knowledge) => Json(knowledge.engines().await).into_response(),
        Err(error) => error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/projects/{id}/knowledge",
    operation_id = "getKnowledgeOverview",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id")),
    responses((status = 200, body = KnowledgeOverview))
)]
pub(crate) async fn overview_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    knowledge.watch_project(&project);
    let links = state
        .orchestrator
        .knowledge_task_links(&id)
        .await
        .unwrap_or_default();
    respond(knowledge.overview(&project, &links).await)
}

#[utoipa::path(
    post,
    path = "/projects/{id}/knowledge/index",
    operation_id = "startKnowledgeIndex",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id")),
    request_body = StartKnowledgeIndexRequest,
    responses((status = 202, description = "Indexing started; progress and the result arrive as `knowledge/index_*` events", body = IndexRunSummary))
)]
pub(crate) async fn start_index_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(req): Json<StartKnowledgeIndexRequest>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    respond_status(
        StatusCode::ACCEPTED,
        knowledge
            .start_index(
                &project,
                req.worktree_id.as_deref().filter(|w| !w.is_empty()),
            )
            .await,
    )
}

#[utoipa::path(
    get,
    path = "/projects/{id}/knowledge/graph",
    operation_id = "getKnowledgeGraph",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id"), KnowledgeGraphQuery),
    responses((status = 200, description = "A bounded view of the code graph; `truncated` says when it was cut", body = GraphView))
)]
pub(crate) async fn graph_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Query(q): Query<KnowledgeGraphQuery>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    let query = GraphQuery {
        level: q.level.unwrap_or(GraphLevel::Modules),
        module: q.module.filter(|m| !m.is_empty()),
        focus: q.focus.filter(|f| !f.is_empty()),
        depth: q.depth.unwrap_or(1),
        include_inferred: q.include_inferred.unwrap_or(true),
        include_external: q.include_external.unwrap_or(false),
    };
    respond(knowledge.graph(&project, &query).await)
}

#[utoipa::path(
    get,
    path = "/projects/{id}/knowledge/search",
    operation_id = "searchKnowledge",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id"), KnowledgeSearchQuery),
    responses((status = 200, description = "Files and symbols whose name or path matches, best first", body = Vec<SearchHit>))
)]
pub(crate) async fn search_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Query(q): Query<KnowledgeSearchQuery>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    if q.q.trim().is_empty() || q.q.len() > 200 {
        return bad_request("`q` must be 1 to 200 characters");
    }
    respond(
        knowledge
            .search(&project, &q.q, q.limit.unwrap_or(25) as usize)
            .await,
    )
}

#[utoipa::path(
    get,
    path = "/projects/{id}/knowledge/node",
    operation_id = "getKnowledgeNode",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id"), KnowledgeNodeQuery),
    responses((status = 200, description = "What the node uses, what uses it, what it contains and its recent commits", body = NodeDetail))
)]
pub(crate) async fn node_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Query(q): Query<KnowledgeNodeQuery>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    respond(knowledge.node(&project, &q.node_id).await)
}

#[utoipa::path(
    get,
    path = "/projects/{id}/knowledge/node/references",
    operation_id = "getKnowledgeNodeReferences",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id"), KnowledgeNodeQuery),
    responses((status = 200, description = "Precise references from the optional language-server navigator; `available` is false, with a reason, when it cannot answer. The first request per indexed commit can take a minute.", body = SymbolReferences))
)]
pub(crate) async fn node_references_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Query(q): Query<KnowledgeNodeQuery>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    respond(knowledge.node_references(&project, &q.node_id).await)
}

#[utoipa::path(
    get,
    path = "/projects/{id}/knowledge/worktrees",
    operation_id = "listKnowledgeWorktrees",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id")),
    responses((status = 200, description = "Every git worktree of the repository, with its HEAD, uncommitted changes and owning task", body = Vec<KnowledgeWorktree>))
)]
pub(crate) async fn worktrees_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    knowledge.watch_project(&project);
    let links = state
        .orchestrator
        .knowledge_task_links(&id)
        .await
        .unwrap_or_default();
    respond(knowledge.worktrees(&project, &links).await)
}

#[utoipa::path(
    get,
    path = "/projects/{id}/knowledge/commits",
    operation_id = "listKnowledgeCommits",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id"), KnowledgeCommitsQuery),
    responses((status = 200, description = "One row per commit across all worktrees, newest first", body = CommitTimeline))
)]
pub(crate) async fn commits_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Query(q): Query<KnowledgeCommitsQuery>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    knowledge.watch_project(&project);
    respond(
        knowledge
            .timeline(&project, q.limit.unwrap_or(100) as usize)
            .await,
    )
}

#[utoipa::path(
    get,
    path = "/projects/{id}/knowledge/commits/{sha}",
    operation_id = "getKnowledgeCommit",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id"), ("sha" = String, Path, description = "Commit id (7 to 64 lowercase hex digits)")),
    responses((status = 200, body = CommitDetail))
)]
pub(crate) async fn commit_handler(
    State(state): State<ServerState>,
    Path((id, sha)): Path<(String, String)>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    respond(knowledge.commit_detail(&project, &sha).await)
}

#[utoipa::path(
    get,
    path = "/projects/{id}/knowledge/commits/{sha}/diff",
    operation_id = "getKnowledgeCommitDiff",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id"), ("sha" = String, Path, description = "Commit id")),
    responses((status = 200, body = CommitDiffResponse))
)]
pub(crate) async fn commit_diff_handler(
    State(state): State<ServerState>,
    Path((id, sha)): Path<(String, String)>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    respond(
        knowledge
            .commit_diff(&project, &sha)
            .await
            .map(|(diff, truncated)| CommitDiffResponse {
                sha,
                diff: redact_secrets(&diff, &[]),
                truncated,
            }),
    )
}

#[utoipa::path(
    get,
    path = "/projects/{id}/knowledge/commits/{sha}/impact",
    operation_id = "getKnowledgeCommitImpact",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id"), ("sha" = String, Path, description = "Commit id"), KnowledgeImpactQuery),
    responses((status = 200, description = "The symbols the commit touched and the code that depends on them", body = CommitImpact))
)]
pub(crate) async fn commit_impact_handler(
    State(state): State<ServerState>,
    Path((id, sha)): Path<(String, String)>,
    Query(q): Query<KnowledgeImpactQuery>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    respond(
        knowledge
            .commit_impact(&project, &sha, q.depth, q.include_inferred.unwrap_or(true))
            .await,
    )
}

#[utoipa::path(
    get,
    path = "/projects/{id}/knowledge/commits/{sha}/analysis",
    operation_id = "getKnowledgeCommitAnalysis",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id"), ("sha" = String, Path, description = "Commit id")),
    responses((status = 200, body = CommitAnalysisResponse))
)]
pub(crate) async fn commit_analysis_handler(
    State(state): State<ServerState>,
    Path((id, sha)): Path<(String, String)>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    respond(
        knowledge
            .commit_analysis(&project, &sha)
            .await
            .map(|analysis| CommitAnalysisResponse { analysis }),
    )
}

#[utoipa::path(
    post,
    path = "/projects/{id}/knowledge/commits/{sha}/analysis",
    operation_id = "startKnowledgeCommitAnalysis",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id"), ("sha" = String, Path, description = "Commit id")),
    request_body = StartKnowledgeAnalysisRequest,
    responses((status = 202, description = "The cached analysis, or a running one that finishes with a `knowledge/analysis_completed` event", body = CommitAnalysis))
)]
pub(crate) async fn start_analysis_handler(
    State(state): State<ServerState>,
    Path((id, sha)): Path<(String, String)>,
    Json(req): Json<StartKnowledgeAnalysisRequest>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    respond_status(
        StatusCode::ACCEPTED,
        knowledge
            .start_commit_analysis(&project, &sha, req.force.unwrap_or(false))
            .await,
    )
}

#[utoipa::path(
    get,
    path = "/projects/{id}/knowledge/chat",
    operation_id = "listKnowledgeChat",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id")),
    responses((status = 200, description = "The recent conversation, oldest first", body = Vec<ChatMessage>))
)]
pub(crate) async fn chat_list_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    respond(knowledge.chat_messages(&project).await)
}

#[utoipa::path(
    post,
    path = "/projects/{id}/knowledge/chat",
    operation_id = "askKnowledge",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id")),
    request_body = AskKnowledgeRequest,
    responses((status = 202, description = "The question and the answer being produced (`status = running`); it completes with a `knowledge/chat_completed` event", body = Vec<ChatMessage>))
)]
pub(crate) async fn ask_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(req): Json<AskKnowledgeRequest>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    respond_status(
        StatusCode::ACCEPTED,
        knowledge.ask(&project, &req.question).await,
    )
}

#[utoipa::path(
    delete,
    path = "/projects/{id}/knowledge/chat",
    operation_id = "clearKnowledgeChat",
    tag = "knowledge",
    params(("id" = String, Path, description = "Project id")),
    responses((status = 200, body = KnowledgeClearedResponse))
)]
pub(crate) async fn chat_clear_handler(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Response {
    let (knowledge, project) = match context(&state, &id).await {
        Ok(found) => found,
        Err(error) => return error_response(error),
    };
    respond(
        knowledge
            .clear_chat(&project)
            .await
            .map(|()| KnowledgeClearedResponse { cleared: true }),
    )
}
