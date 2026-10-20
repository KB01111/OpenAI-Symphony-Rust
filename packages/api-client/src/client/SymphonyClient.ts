import type {
  AgentEvent,
  AppSettings,
  ApprovalDecision,
  ApprovalRequest,
  CreateProjectRequest,
  CreateTaskRequest,
  CredentialKeysResponse,
  CredentialSavedResponse,
  HealthResponse,
  IssueDetailResponse,
  OrchestratorStateSnapshot,
  PreviewWorkflowRequest,
  Project,
  ProjectUpdate,
  ProviderStatus,
  PullRequestReview,
  RefreshResponse,
  RepositoryValidation,
  RunAttempt,
  RuntimeInfo,
  SchedulerStateResponse,
  Task,
  TaskDiff,
  TaskStatus,
  ValidationResult,
  WorkflowDocument,
  WorkflowValidation,
  WorktreeInfo
} from "@symphony/shared-types";
import { SymphonyApiError } from "../errors/index.js";
import { discoverRuntimeEndpoint, type RuntimeEndpoint } from "../transport/discovery.js";
import { NativeEventStream } from "../streaming/native.js";
import { AuthenticatedSseStream, type EventStream, type EventStreamHandlers } from "../streaming/sse.js";

export interface SymphonyClientOptions {
  baseUrl: string;
  token?: string | null;
  fetchImpl?: typeof fetch;
  /** Per-request timeout. Streaming requests are not subject to it. */
  timeoutMs?: number;
  /** How the endpoint was obtained; selects the event transport. */
  endpoint?: Pick<RuntimeEndpoint, "source" | "mode">;
}

interface RequestOptions {
  body?: unknown;
  query?: Record<string, string | number | boolean | null | undefined>;
  signal?: AbortSignal;
}

/**
 * Typed client for the Symphony runtime API (`/api/v1/*`). Every route here exists in
 * `crates/symphony-server/src/lib.rs`; payload types are generated from the Rust structs.
 */
export class SymphonyClient {
  private baseUrl: string;
  private token: string | null;
  private readonly fetchImpl: typeof fetch;
  private readonly timeoutMs: number;
  private endpoint: Pick<RuntimeEndpoint, "source" | "mode">;

  constructor(options: SymphonyClientOptions) {
    this.baseUrl = options.baseUrl.replace(/\/+$/, "");
    this.token = options.token ?? null;
    this.fetchImpl = options.fetchImpl ?? ((input, init) => fetch(input, init));
    this.timeoutMs = options.timeoutMs ?? 20_000;
    this.endpoint = options.endpoint ?? { source: "configured", mode: "remote" };
  }

  /** Create a client for the local runtime using desktop IPC or dev-server discovery. */
  static async discover(fetchImpl?: typeof fetch): Promise<SymphonyClient> {
    const endpoint = await discoverRuntimeEndpoint(fetchImpl);
    return new SymphonyClient({
      baseUrl: endpoint.baseUrl,
      token: endpoint.token,
      fetchImpl,
      endpoint
    });
  }

  get connection(): { baseUrl: string; source: RuntimeEndpoint["source"]; mode: RuntimeEndpoint["mode"] } {
    return { baseUrl: this.baseUrl, ...this.endpoint };
  }

  setConnection(baseUrl: string, token: string | null): void {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this.token = token;
    this.endpoint = { source: "configured", mode: "remote" };
  }

  // --------------------------------------------------------------------------------------------
  // Transport
  // --------------------------------------------------------------------------------------------

  private url(path: string, query?: RequestOptions["query"]): string {
    const url = `${this.baseUrl}/api/v1${path}`;
    if (!query) return url;
    const params = new URLSearchParams();
    for (const [key, value] of Object.entries(query)) {
      if (value !== undefined && value !== null && value !== "") params.set(key, String(value));
    }
    const qs = params.toString();
    return qs ? `${url}?${qs}` : url;
  }

  private async request<T>(method: string, path: string, options: RequestOptions = {}): Promise<T> {
    const headers: Record<string, string> = { Accept: "application/json" };
    if (this.token) headers.Authorization = `Bearer ${this.token}`;
    if (options.body !== undefined) headers["Content-Type"] = "application/json";

    // The deadline and the caller's signal stay armed until the body has been read, so a stalled
    // body still times out and can be cancelled.
    const timeout = new AbortController();
    const timer = setTimeout(() => timeout.abort(), this.timeoutMs);
    const onAbort = () => timeout.abort();
    options.signal?.addEventListener("abort", onAbort, { once: true });
    const failure = (error: unknown) =>
      options.signal?.aborted ? new SymphonyApiError(0, "aborted", "Request aborted") : SymphonyApiError.network(error);

    try {
      let response: Response;
      try {
        response = await this.fetchImpl(this.url(path, options.query), {
          method,
          headers,
          body: options.body !== undefined ? JSON.stringify(options.body) : undefined,
          signal: timeout.signal
        });
      } catch (error) {
        throw failure(error);
      }

      if (!response.ok) {
        let body: unknown = null;
        try {
          body = await response.json();
        } catch {
          /* non-JSON or interrupted error body */
        }
        throw SymphonyApiError.fromBody(response.status, body);
      }
      if (response.status === 204) return undefined as T;
      let text: string;
      try {
        text = await response.text();
      } catch (error) {
        throw failure(error);
      }
      return (text ? JSON.parse(text) : undefined) as T;
    } finally {
      clearTimeout(timer);
      options.signal?.removeEventListener("abort", onAbort);
    }
  }

  // --------------------------------------------------------------------------------------------
  // Runtime
  // --------------------------------------------------------------------------------------------

  health(signal?: AbortSignal) {
    return this.request<HealthResponse>("GET", "/health", { signal });
  }
  runtime(signal?: AbortSignal) {
    return this.request<RuntimeInfo>("GET", "/runtime", { signal });
  }
  getState(signal?: AbortSignal) {
    return this.request<OrchestratorStateSnapshot>("GET", "/state", { signal });
  }
  /** Persisted runtime events across all tasks, newest first. */
  listRecentEvents(limit = 100, signal?: AbortSignal) {
    return this.request<AgentEvent[]>("GET", "/events/recent", { query: { limit }, signal });
  }
  refresh() {
    return this.request<RefreshResponse>("POST", "/refresh");
  }
  pauseScheduler() {
    return this.request<SchedulerStateResponse>("POST", "/scheduler/pause");
  }
  resumeScheduler() {
    return this.request<SchedulerStateResponse>("POST", "/scheduler/resume");
  }
  setSchedulerPaused(paused: boolean) {
    return paused ? this.pauseScheduler() : this.resumeScheduler();
  }

  // --------------------------------------------------------------------------------------------
  // Projects
  // --------------------------------------------------------------------------------------------

  listProjects(signal?: AbortSignal) {
    return this.request<Project[]>("GET", "/projects", { signal });
  }
  getProject(id: string, signal?: AbortSignal) {
    return this.request<Project>("GET", `/projects/${encodeURIComponent(id)}`, { signal });
  }
  validateRepository(repoPath: string) {
    return this.request<RepositoryValidation>("POST", "/projects/validate", {
      body: { repo_path: repoPath }
    });
  }
  createProject(input: CreateProjectRequest) {
    return this.request<Project>("POST", "/projects", { body: input });
  }
  updateProject(id: string, update: ProjectUpdate) {
    return this.request<Project>("PATCH", `/projects/${encodeURIComponent(id)}`, { body: update });
  }
  deleteProject(id: string) {
    return this.request<void>("DELETE", `/projects/${encodeURIComponent(id)}`);
  }

  // --------------------------------------------------------------------------------------------
  // Tasks
  // --------------------------------------------------------------------------------------------

  listTasks(filter: { projectId?: string | null; status?: TaskStatus | null } = {}, signal?: AbortSignal) {
    return this.request<Task[]>("GET", "/tasks", {
      query: { project_id: filter.projectId, status: filter.status },
      signal
    });
  }
  getTask(id: string, signal?: AbortSignal) {
    return this.request<Task>("GET", `/tasks/${encodeURIComponent(id)}`, { signal });
  }
  /** Create a manual task. `auto_start: true` dispatches it immediately. */
  createTask(input: CreateTaskRequest) {
    return this.request<Task>("POST", "/tasks", { body: input });
  }
  moveTaskStatus(id: string, status: TaskStatus) {
    return this.request<Task>("PATCH", `/tasks/${encodeURIComponent(id)}/status`, { body: { status } });
  }
  dispatchTask(id: string) {
    return this.request<Task>("POST", `/tasks/${encodeURIComponent(id)}/dispatch`);
  }
  cancelTask(id: string, reason?: string) {
    return this.request<Task>("POST", `/tasks/${encodeURIComponent(id)}/cancel`, {
      body: { reason: reason ?? null }
    });
  }
  retryTask(id: string) {
    return this.request<Task>("POST", `/tasks/${encodeURIComponent(id)}/retry`);
  }
  listTaskEvents(id: string, limit = 500, signal?: AbortSignal) {
    return this.request<AgentEvent[]>("GET", `/tasks/${encodeURIComponent(id)}/events`, {
      query: { limit },
      signal
    });
  }
  listTaskAttempts(id: string, signal?: AbortSignal) {
    return this.request<RunAttempt[]>("GET", `/tasks/${encodeURIComponent(id)}/attempts`, { signal });
  }
  listTaskValidations(id: string, signal?: AbortSignal) {
    return this.request<ValidationResult[]>("GET", `/tasks/${encodeURIComponent(id)}/validations`, {
      signal
    });
  }
  getTaskDiff(id: string, signal?: AbortSignal) {
    return this.request<TaskDiff>("GET", `/tasks/${encodeURIComponent(id)}/diff`, { signal });
  }
  getIssueDetail(identifier: string, signal?: AbortSignal) {
    return this.request<IssueDetailResponse>("GET", `/issues/${encodeURIComponent(identifier)}`, {
      signal
    });
  }

  // --------------------------------------------------------------------------------------------
  // Approvals, reviews, worktrees
  // --------------------------------------------------------------------------------------------

  listApprovals(onlyPending = true, signal?: AbortSignal) {
    return this.request<ApprovalRequest[]>("GET", "/approvals", {
      query: { only_pending: onlyPending },
      signal
    });
  }
  decideApproval(id: string, decision: ApprovalDecision) {
    return this.request<ApprovalRequest>("POST", `/approvals/${encodeURIComponent(id)}/decide`, {
      body: { decision }
    });
  }
  listPullRequests(projectId?: string | null, signal?: AbortSignal) {
    return this.request<PullRequestReview[]>("GET", "/prs", { query: { project_id: projectId }, signal });
  }
  getPullRequest(id: string, signal?: AbortSignal) {
    return this.request<PullRequestReview>("GET", `/prs/${encodeURIComponent(id)}`, { signal });
  }
  /** Records explicit human approval. This never merges anything. */
  approvePullRequest(id: string) {
    return this.request<PullRequestReview>("POST", `/prs/${encodeURIComponent(id)}/approve-merge`);
  }
  listWorktrees(signal?: AbortSignal) {
    return this.request<WorktreeInfo[]>("GET", "/worktrees", { signal });
  }

  // --------------------------------------------------------------------------------------------
  // Workflow
  // --------------------------------------------------------------------------------------------

  getWorkflow(signal?: AbortSignal) {
    return this.request<WorkflowDocument>("GET", "/workflow", { signal });
  }
  /** Validate and render unsaved workflow text with the runtime's real parser. */
  previewWorkflow(input: PreviewWorkflowRequest, signal?: AbortSignal) {
    return this.request<WorkflowValidation>("POST", "/workflow/preview", { body: input, signal });
  }
  saveWorkflow(rawMarkdown: string) {
    return this.request<WorkflowValidation>("PUT", "/workflow", { body: { raw_markdown: rawMarkdown } });
  }

  // --------------------------------------------------------------------------------------------
  // Settings, providers, credentials, diagnostics
  // --------------------------------------------------------------------------------------------

  getSettings(signal?: AbortSignal) {
    return this.request<AppSettings>("GET", "/settings", { signal });
  }
  saveSettings(settings: AppSettings) {
    return this.request<AppSettings>("PUT", "/settings", { body: settings });
  }
  listProviders(refresh = false, signal?: AbortSignal) {
    return this.request<ProviderStatus[]>("GET", "/providers", { query: { refresh }, signal });
  }
  listCredentials(signal?: AbortSignal) {
    return this.request<CredentialKeysResponse>("GET", "/credentials", { signal });
  }
  saveCredential(key: string, value: string) {
    return this.request<CredentialSavedResponse>("POST", "/credentials", { body: { key, value } });
  }
  deleteCredential(key: string) {
    return this.request<void>("DELETE", `/credentials/${encodeURIComponent(key)}`);
  }
  exportDiagnostics() {
    return this.request<Record<string, unknown>>("GET", "/diagnostics/export");
  }

  // --------------------------------------------------------------------------------------------
  // Streaming
  // --------------------------------------------------------------------------------------------

  /**
   * Create (but do not start) the runtime event stream. Uses the in-process Tauri channel when the
   * runtime is embedded in the desktop host, otherwise authenticated SSE.
   */
  createEventStream(handlers: EventStreamHandlers): EventStream {
    if (this.endpoint.source === "tauri" && this.endpoint.mode === "embedded") {
      return new NativeEventStream(handlers);
    }
    return new AuthenticatedSseStream({
      ...handlers,
      url: this.url("/events"),
      token: this.token,
      fetchImpl: this.fetchImpl
    });
  }
}
