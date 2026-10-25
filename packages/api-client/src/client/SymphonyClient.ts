import * as api from "@symphony/shared-types/api";
import { createClient, type Client } from "@symphony/shared-types/api/client";
import type {
  AppSettings,
  ApprovalDecision,
  CreateProjectRequest,
  CreateTaskRequest,
  PreviewWorkflowRequest,
  ProjectUpdate,
  TaskStatus
} from "@symphony/shared-types";
import {
  API_BASE_PATH,
  DEFAULT_TIMEOUT_MS,
  configuredApiConfig,
  defaultRuntimeTarget,
  normalizeBaseUrl,
  resolveApiConfig,
  type ApiConfig,
  type ApiConfigOptions,
  type RuntimeTarget
} from "../config/apiConfig.js";
import { SymphonyApiError } from "../errors/index.js";
import type { RuntimeEndpoint } from "../transport/discovery.js";
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

/** Options every generated SDK function accepts from this client. */
interface CallOptions {
  client: Client;
  signal: AbortSignal;
  throwOnError: false;
}

/** The shape generated SDK functions resolve to when `throwOnError` is `false`. */
type CallResult<T> = { data?: T; error?: unknown; response?: Response };

/** Query values: empty strings, `null` and `undefined` are omitted. */
function q<T>(value: T | null | undefined | ""): T | undefined {
  return value === null || value === "" ? undefined : value;
}

/**
 * Typed client for the Symphony runtime API (`/api/v1/*`).
 *
 * Every endpoint call goes through the SDK that Hey API generates from the Rust OpenAPI document
 * (`@symphony/shared-types/api`), so paths, parameters and payloads cannot drift from the server.
 * This class adds what the generated SDK does not provide: one connection configuration for
 * desktop, local and remote runtimes, bearer authentication, request deadlines, normalized
 * errors and the authenticated event stream.
 */
export class SymphonyClient {
  private config: ApiConfig;
  private readonly http: Client;

  constructor(options: SymphonyClientOptions | ApiConfig) {
    this.config = {
      baseUrl: normalizeBaseUrl(options.baseUrl),
      token: options.token ?? null,
      endpoint: options.endpoint ?? { source: "configured", mode: "remote" },
      timeoutMs: options.timeoutMs ?? DEFAULT_TIMEOUT_MS,
      fetchImpl: options.fetchImpl
    };
    const fetchImpl = this.config.fetchImpl;
    this.http = createClient({
      baseUrl: this.config.baseUrl,
      // Resolved per request so `setConnection` takes effect immediately. Only operations the
      // OpenAPI document marks as bearer-protected receive it.
      auth: () => this.config.token ?? undefined,
      fetch: (request) => (fetchImpl ? fetchImpl(request) : fetch(request))
    });
  }

  /** Build a client from a resolved connection configuration. */
  static fromConfig(config: ApiConfig): SymphonyClient {
    return new SymphonyClient(config);
  }

  /** Resolve `target` (desktop IPC, local discovery or a remote endpoint) and build a client. */
  static async connect(target: RuntimeTarget, options: ApiConfigOptions = {}): Promise<SymphonyClient> {
    return SymphonyClient.fromConfig(await resolveApiConfig(target, options));
  }

  /** Create a client for the local runtime using desktop IPC or dev-server discovery. */
  static discover(fetchImpl?: typeof fetch): Promise<SymphonyClient> {
    return SymphonyClient.connect(defaultRuntimeTarget(), { fetchImpl });
  }

  get connection(): { baseUrl: string; source: RuntimeEndpoint["source"]; mode: RuntimeEndpoint["mode"] } {
    return { baseUrl: this.config.baseUrl, ...this.config.endpoint };
  }

  /** Point this client at an explicitly configured (remote) runtime. */
  setConnection(baseUrl: string, token: string | null): void {
    const next = configuredApiConfig(baseUrl, token, "remote", {
      fetchImpl: this.config.fetchImpl,
      timeoutMs: this.config.timeoutMs
    });
    this.config = next;
    this.http.setConfig({ baseUrl: next.baseUrl });
  }

  // --------------------------------------------------------------------------------------------
  // Transport
  // --------------------------------------------------------------------------------------------

  /**
   * Run one generated SDK call with the client's deadline and the caller's signal. Both stay
   * armed until the response body has been read, so a stalled body still times out and can be
   * cancelled. Failures become `SymphonyApiError`.
   */
  private async call<T>(operation: (options: CallOptions) => Promise<CallResult<T>>, signal?: AbortSignal): Promise<T> {
    const deadline = new AbortController();
    const timer = setTimeout(() => deadline.abort(), this.config.timeoutMs);
    const onAbort = () => deadline.abort();
    signal?.addEventListener("abort", onAbort, { once: true });
    try {
      let result: CallResult<T>;
      try {
        result = await operation({ client: this.http, signal: deadline.signal, throwOnError: false });
      } catch (error) {
        throw this.failure(error, signal);
      }
      if (result.error !== undefined) {
        // An HTTP error carries the response; a network failure or abort does not.
        if (result.response && !result.response.ok) {
          throw SymphonyApiError.fromBody(result.response.status, result.error);
        }
        throw this.failure(result.error, signal);
      }
      return (result.response?.status === 204 ? undefined : result.data) as T;
    } finally {
      clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
    }
  }

  private failure(error: unknown, signal?: AbortSignal): SymphonyApiError {
    if (error instanceof SymphonyApiError) return error;
    return signal?.aborted ? new SymphonyApiError(0, "aborted", "Request aborted") : SymphonyApiError.network(error);
  }

  // --------------------------------------------------------------------------------------------
  // Runtime
  // --------------------------------------------------------------------------------------------

  health(signal?: AbortSignal) {
    return this.call((o) => api.health(o), signal);
  }
  runtime(signal?: AbortSignal) {
    return this.call((o) => api.getRuntime(o), signal);
  }
  getState(signal?: AbortSignal) {
    return this.call((o) => api.getState(o), signal);
  }
  /** Persisted runtime events across all tasks, newest first. */
  listRecentEvents(limit = 100, signal?: AbortSignal) {
    return this.call((o) => api.listRecentEvents({ ...o, query: { limit } }), signal);
  }
  refresh() {
    return this.call((o) => api.refresh(o));
  }
  pauseScheduler() {
    return this.call((o) => api.pauseScheduler(o));
  }
  resumeScheduler() {
    return this.call((o) => api.resumeScheduler(o));
  }
  setSchedulerPaused(paused: boolean) {
    return paused ? this.pauseScheduler() : this.resumeScheduler();
  }

  // --------------------------------------------------------------------------------------------
  // Projects
  // --------------------------------------------------------------------------------------------

  listProjects(signal?: AbortSignal) {
    return this.call((o) => api.listProjects(o), signal);
  }
  getProject(id: string, signal?: AbortSignal) {
    return this.call((o) => api.getProject({ ...o, path: { id } }), signal);
  }
  validateRepository(repoPath: string) {
    return this.call((o) => api.validateRepository({ ...o, body: { repo_path: repoPath } }));
  }
  createProject(input: CreateProjectRequest) {
    return this.call((o) => api.createProject({ ...o, body: input }));
  }
  updateProject(id: string, update: ProjectUpdate) {
    return this.call((o) => api.updateProject({ ...o, path: { id }, body: update }));
  }
  deleteProject(id: string): Promise<void> {
    return this.call((o) => api.deleteProject({ ...o, path: { id } })).then(() => undefined);
  }

  // --------------------------------------------------------------------------------------------
  // Tasks
  // --------------------------------------------------------------------------------------------

  listTasks(filter: { projectId?: string | null; status?: TaskStatus | null } = {}, signal?: AbortSignal) {
    return this.call(
      (o) => api.listTasks({ ...o, query: { project_id: q(filter.projectId), status: q(filter.status) } }),
      signal
    );
  }
  getTask(id: string, signal?: AbortSignal) {
    return this.call((o) => api.getTask({ ...o, path: { id } }), signal);
  }
  /** Create a manual task. `auto_start: true` dispatches it immediately. */
  createTask(input: CreateTaskRequest) {
    return this.call((o) => api.createTask({ ...o, body: input }));
  }
  moveTaskStatus(id: string, status: TaskStatus) {
    return this.call((o) => api.moveTaskStatus({ ...o, path: { id }, body: { status } }));
  }
  dispatchTask(id: string) {
    return this.call((o) => api.dispatchTask({ ...o, path: { id } }));
  }
  cancelTask(id: string, reason?: string) {
    return this.call((o) => api.cancelTask({ ...o, path: { id }, body: { reason: reason ?? null } }));
  }
  retryTask(id: string) {
    return this.call((o) => api.retryTask({ ...o, path: { id } }));
  }
  /** Newest first. With `before` (an event id of this task), the page of events older than it. */
  listTaskEvents(id: string, limit = 500, signal?: AbortSignal, before?: string) {
    return this.call((o) => api.listTaskEvents({ ...o, path: { id }, query: before ? { limit, before } : { limit } }), signal);
  }
  listTaskAttempts(id: string, signal?: AbortSignal) {
    return this.call((o) => api.listTaskAttempts({ ...o, path: { id } }), signal);
  }
  listTaskValidations(id: string, signal?: AbortSignal) {
    return this.call((o) => api.listTaskValidations({ ...o, path: { id } }), signal);
  }
  getTaskDiff(id: string, signal?: AbortSignal) {
    return this.call((o) => api.getTaskDiff({ ...o, path: { id } }), signal);
  }
  getIssueDetail(identifier: string, signal?: AbortSignal) {
    return this.call((o) => api.getIssueDetail({ ...o, path: { issue_identifier: identifier } }), signal);
  }

  // --------------------------------------------------------------------------------------------
  // Approvals, reviews, worktrees
  // --------------------------------------------------------------------------------------------

  listApprovals(onlyPending = true, signal?: AbortSignal) {
    return this.call((o) => api.listApprovals({ ...o, query: { only_pending: onlyPending } }), signal);
  }
  decideApproval(id: string, decision: ApprovalDecision) {
    return this.call((o) => api.decideApproval({ ...o, path: { id }, body: { decision } }));
  }
  listPullRequests(projectId?: string | null, signal?: AbortSignal) {
    return this.call((o) => api.listPrReviews({ ...o, query: { project_id: q(projectId) } }), signal);
  }
  getPullRequest(id: string, signal?: AbortSignal) {
    return this.call((o) => api.getPrReview({ ...o, path: { id } }), signal);
  }
  /** Records explicit human approval. This never merges anything. */
  approvePullRequest(id: string) {
    return this.call((o) => api.approvePrMerge({ ...o, path: { id } }));
  }
  /**
   * Publish the reviewed branch as a GitHub pull request (commit, push, open). An explicit human
   * action; it never merges anything. Repeating it returns the existing pull request.
   */
  createPullRequest(id: string) {
    return this.call((o) => api.createPullRequest({ ...o, path: { id } }));
  }
  /** Re-read the pull request's CI result, merge state and comments from GitHub and re-rate it. */
  refreshPullRequest(id: string) {
    return this.call((o) => api.refreshPullRequest({ ...o, path: { id } }));
  }
  listWorktrees(signal?: AbortSignal) {
    return this.call((o) => api.listWorktrees(o), signal);
  }

  // --------------------------------------------------------------------------------------------
  // Workflow
  // --------------------------------------------------------------------------------------------

  getWorkflow(signal?: AbortSignal) {
    return this.call((o) => api.getWorkflow(o), signal);
  }
  /** Validate and render unsaved workflow text with the runtime's real parser. */
  previewWorkflow(input: PreviewWorkflowRequest, signal?: AbortSignal) {
    return this.call((o) => api.previewWorkflow({ ...o, body: input }), signal);
  }
  saveWorkflow(rawMarkdown: string) {
    return this.call((o) => api.updateWorkflow({ ...o, body: { raw_markdown: rawMarkdown } }));
  }

  // --------------------------------------------------------------------------------------------
  // Settings, providers, credentials, diagnostics
  // --------------------------------------------------------------------------------------------

  getSettings(signal?: AbortSignal) {
    return this.call((o) => api.getSettings(o), signal);
  }
  saveSettings(settings: AppSettings) {
    return this.call((o) => api.updateSettings({ ...o, body: settings }));
  }
  listProviders(refresh = false, signal?: AbortSignal) {
    return this.call((o) => api.listProviders({ ...o, query: { refresh } }), signal);
  }
  listCredentials(signal?: AbortSignal) {
    return this.call((o) => api.listCredentials(o), signal);
  }
  saveCredential(key: string, value: string) {
    return this.call((o) => api.setCredential({ ...o, body: { key, value } }));
  }
  deleteCredential(key: string): Promise<void> {
    return this.call((o) => api.deleteCredential({ ...o, path: { key } })).then(() => undefined);
  }
  exportDiagnostics() {
    return this.call((o) => api.exportDiagnostics(o)) as Promise<Record<string, unknown>>;
  }

  // --------------------------------------------------------------------------------------------
  // Backups (local administration only)
  // --------------------------------------------------------------------------------------------

  /** Existing local backups, newest first, and the directory they are stored in. */
  listBackups(signal?: AbortSignal) {
    return this.call((o) => api.listBackups(o), signal);
  }
  /** Create a SQLite-consistent backup now. Credentials are never included. */
  createBackup() {
    return this.call((o) => api.createBackup(o));
  }

  // --------------------------------------------------------------------------------------------
  // Remote companion access (local administration only)
  // --------------------------------------------------------------------------------------------

  /** Persisted remote-access settings plus the listener's effective state. */
  getRemoteAccess(signal?: AbortSignal) {
    return this.call((o) => api.getRemoteAccess(o), signal);
  }
  /** Generate a new remote access token; the response is the only time it is returned. */
  rotateRemoteAccessToken() {
    return this.call((o) => api.rotateRemoteAccessToken(o));
  }
  /** Revoke the remote access token and stop the remote listener. */
  revokeRemoteAccessToken() {
    return this.call((o) => api.revokeRemoteAccessToken(o));
  }

  // --------------------------------------------------------------------------------------------
  // Streaming
  // --------------------------------------------------------------------------------------------

  /**
   * Create (but do not start) the runtime event stream. Uses the in-process Tauri channel when the
   * runtime is embedded in the desktop host, otherwise authenticated SSE with `Last-Event-ID`
   * replay (the generated SDK's SSE helper is deliberately not used).
   */
  createEventStream(handlers: EventStreamHandlers): EventStream {
    const { endpoint, baseUrl, token, fetchImpl } = this.config;
    if (endpoint.source === "tauri" && endpoint.mode === "embedded") {
      return new NativeEventStream(handlers);
    }
    return new AuthenticatedSseStream({
      ...handlers,
      url: `${baseUrl}${API_BASE_PATH}/events`,
      token,
      fetchImpl: fetchImpl ?? ((input, init) => fetch(input, init))
    });
  }
}
