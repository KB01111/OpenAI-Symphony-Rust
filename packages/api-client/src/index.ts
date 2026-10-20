import type {
  AgentEvent,
  ApprovalDecision,
  ApprovalRequest,
  AppSettings,
  IssueDetailResponse,
  OrchestratorStateSnapshot,
  Project,
  ProviderKind,
  ProviderProfile,
  PullRequestReview,
  RunAttempt,
  Task,
  TaskStatus,
  ValidationResult,
  WorkflowPreviewResponse
} from "@symphony/shared-types";

export interface SymphonyClientConfig {
  baseUrl: string;
  authToken?: string | null;
  bearerToken?: string | null;
}

export interface CreateProjectInput {
  name: string;
  repo_path: string;
  default_branch?: string;
  workspace_root?: string;
  workflow_path?: string | null;
  preferred_provider?: ProviderKind;
  preferred_model?: string | null;
  tracker_kind?: string;
  github_repo?: string | null;
  linear_project_slug?: string | null;
  required_labels?: string[];
  auto_dispatch?: boolean;
  max_concurrent_agents?: number;
  validation_commands?: string[];
}

export interface CreateManualTaskInput {
  project_id: string;
  identifier?: string | null;
  title: string;
  description?: string | null;
  priority?: number | null;
  provider?: ProviderKind | null;
  preferred_provider?: ProviderKind | null;
  model?: string | null;
  labels?: string[];
  dispatch_immediately?: boolean;
}

declare global {
  interface Window {
    __TAURI_INTERNALS__?: {
      invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
    };
  }
}

export async function invokeTauriCommand<T>(
  cmd: string,
  args?: Record<string, unknown>
): Promise<T | null> {
  if (typeof window !== "undefined" && window.__TAURI_INTERNALS__?.invoke) {
    return window.__TAURI_INTERNALS__.invoke<T>(cmd, args);
  }
  return null;
}

export class SymphonyApiClient {
  private baseUrl: string;
  private authToken: string | null;
  private initializedTauri = false;

  constructor(config?: Partial<SymphonyClientConfig>) {
    this.baseUrl = (config?.baseUrl ?? "http://127.0.0.1:47820").replace(/\/+$/, "");
    this.authToken = config?.authToken ?? config?.bearerToken ?? null;
  }

  public setConnection(baseUrl: string, authToken?: string | null): void {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this.authToken = authToken ?? null;
  }

  public getBaseUrl(): string {
    return this.baseUrl;
  }

  public async ensureTauriEndpoint(): Promise<void> {
    if (this.initializedTauri) return;
    this.initializedTauri = true;
    try {
      const cfg = await invokeTauriCommand<{ base_url: string; token: string }>(
        "get_local_api_config"
      );
      if (cfg?.base_url) {
        this.baseUrl = cfg.base_url.replace(/\/+$/, "");
        if (cfg.token) {
          this.authToken = cfg.token;
        }
      }
    } catch {
      // Fallback to default local port
    }
  }

  private async request<T>(path: string, init?: RequestInit): Promise<T> {
    await this.ensureTauriEndpoint();
    const headers = new Headers(init?.headers);
    if (!headers.has("Content-Type") && init?.body) {
      headers.set("Content-Type", "application/json");
    }
    if (this.authToken) {
      headers.set("Authorization", `Bearer ${this.authToken}`);
    }

    const response = await fetch(`${this.baseUrl}${path}`, {
      ...init,
      headers
    });

    if (!response.ok) {
      let errMsg = `HTTP ${response.status}`;
      try {
        const errJson = (await response.json()) as {
          error?: { code?: string; message?: string };
        };
        if (errJson?.error?.message) {
          errMsg = `${errJson.error.code ?? "error"}: ${errJson.error.message}`;
        }
      } catch {
        // ignore JSON parse failure
      }
      throw new Error(errMsg);
    }

    if (response.status === 204) {
      return undefined as T;
    }

    return (await response.json()) as T;
  }

  public async getState(): Promise<OrchestratorStateSnapshot> {
    return this.request<OrchestratorStateSnapshot>("/api/v1/state");
  }

  public async triggerRefresh(): Promise<{
    queued: boolean;
    coalesced: boolean;
    requested_at: string;
    operations: string[];
  }> {
    return this.request("/api/v1/refresh", { method: "POST" });
  }

  public async requestRefresh(): Promise<{
    queued: boolean;
    coalesced: boolean;
    requested_at: string;
    operations: string[];
  }> {
    return this.triggerRefresh();
  }

  public async setSchedulerPaused(paused: boolean): Promise<{ scheduler_paused: boolean }> {
    return this.request("/api/v1/scheduler/pause", {
      method: "POST",
      body: JSON.stringify({ paused })
    });
  }

  public async listProjects(): Promise<Project[]> {
    return this.request<Project[]>("/api/v1/projects");
  }

  public async createProject(input: CreateProjectInput): Promise<Project> {
    return this.request<Project>("/api/v1/projects", {
      method: "POST",
      body: JSON.stringify(input)
    });
  }

  public async deleteProject(id: string): Promise<{ deleted: boolean }> {
    return this.request(`/api/v1/projects/${encodeURIComponent(id)}`, {
      method: "DELETE"
    });
  }

  public async listTasks(projectId?: string): Promise<Task[]> {
    const query = projectId ? `?project_id=${encodeURIComponent(projectId)}` : "";
    return this.request<Task[]>(`/api/v1/tasks${query}`);
  }

  public async createManualTask(input: CreateManualTaskInput): Promise<Task> {
    return this.request<Task>("/api/v1/tasks", {
      method: "POST",
      body: JSON.stringify(input)
    });
  }

  public async getTask(id: string): Promise<Task> {
    return this.request<Task>(`/api/v1/tasks/${encodeURIComponent(id)}`);
  }

  public async updateTaskStatus(id: string, status: TaskStatus): Promise<Task> {
    return this.request<Task>(`/api/v1/tasks/${encodeURIComponent(id)}/status`, {
      method: "POST",
      body: JSON.stringify({ status })
    });
  }

  public async moveTaskStatus(id: string, status: TaskStatus): Promise<Task> {
    return this.updateTaskStatus(id, status);
  }

  public async startTask(id: string): Promise<Task> {
    return this.request<Task>(`/api/v1/tasks/${encodeURIComponent(id)}/start`, {
      method: "POST"
    });
  }

  public async dispatchTaskNow(id: string): Promise<Task> {
    return this.startTask(id);
  }

  public async cancelTask(id: string): Promise<Task> {
    return this.request<Task>(`/api/v1/tasks/${encodeURIComponent(id)}/cancel`, {
      method: "POST"
    });
  }

  public async retryTask(id: string): Promise<Task> {
    return this.request<Task>(`/api/v1/tasks/${encodeURIComponent(id)}/retry`, {
      method: "POST"
    });
  }

  public async listTaskEvents(id: string, limit = 200): Promise<AgentEvent[]> {
    return this.request<AgentEvent[]>(
      `/api/v1/tasks/${encodeURIComponent(id)}/events?limit=${limit}`
    );
  }

  public async listTaskAttempts(id: string): Promise<RunAttempt[]> {
    return this.request<RunAttempt[]>(`/api/v1/tasks/${encodeURIComponent(id)}/attempts`);
  }

  public async listTaskValidations(id: string): Promise<ValidationResult[]> {
    return this.request<ValidationResult[]>(
      `/api/v1/tasks/${encodeURIComponent(id)}/validations`
    );
  }

  public async getIssueDetail(identifier: string): Promise<IssueDetailResponse> {
    return this.request<IssueDetailResponse>(`/api/v1/${encodeURIComponent(identifier)}`);
  }

  public async listApprovals(onlyPending = true): Promise<ApprovalRequest[]> {
    return this.request<ApprovalRequest[]>(
      `/api/v1/approvals?only_pending=${onlyPending ? "true" : "false"}`
    );
  }

  public async resolveApproval(
    id: string,
    decision: ApprovalDecision
  ): Promise<{ resolved: boolean; decision: ApprovalDecision }> {
    return this.request(`/api/v1/approvals/${encodeURIComponent(id)}/resolve`, {
      method: "POST",
      body: JSON.stringify({ decision })
    });
  }

  public async listPrReviews(projectId?: string): Promise<PullRequestReview[]> {
    const query = projectId ? `?project_id=${encodeURIComponent(projectId)}` : "";
    return this.request<PullRequestReview[]>(`/api/v1/prs${query}`);
  }

  public async approvePrMerge(id: string): Promise<PullRequestReview> {
    return this.request<PullRequestReview>(`/api/v1/prs/${encodeURIComponent(id)}/approve`, {
      method: "POST"
    });
  }

  public async getWorkflow(path?: string): Promise<WorkflowPreviewResponse> {
    const query = path ? `?path=${encodeURIComponent(path)}` : "";
    return this.request<WorkflowPreviewResponse>(`/api/v1/workflow${query}`);
  }

  public async getWorkflowDocument(path?: string): Promise<WorkflowPreviewResponse> {
    return this.getWorkflow(path);
  }

  public async saveWorkflow(
    rawContent: string,
    path?: string
  ): Promise<WorkflowPreviewResponse> {
    return this.request<WorkflowPreviewResponse>("/api/v1/workflow", {
      method: "POST",
      body: JSON.stringify({ path: path ?? null, raw_content: rawContent })
    });
  }

  public async saveWorkflowDocument(input: {
    path?: string | null;
    raw_content: string;
  }): Promise<WorkflowPreviewResponse> {
    return this.saveWorkflow(input.raw_content, input.path ?? undefined);
  }

  public async getSettings(): Promise<AppSettings> {
    return this.request<AppSettings>("/api/v1/settings");
  }

  public async saveSettings(settings: AppSettings): Promise<AppSettings> {
    const saved = await this.request<AppSettings>("/api/v1/settings", {
      method: "PUT",
      body: JSON.stringify(settings)
    });
    await invokeTauriCommand("set_close_behavior", {
      minimizeToTray: settings.close_action === "minimize_to_tray"
    });
    return saved;
  }

  public async listProviders(): Promise<ProviderProfile[]> {
    return this.request<ProviderProfile[]>("/api/v1/providers");
  }

  public async detectProviders(): Promise<ProviderProfile[]> {
    return this.listProviders();
  }

  public async saveCredential(key: string, value: string): Promise<{ saved: boolean }> {
    return this.request("/api/v1/credentials", {
      method: "POST",
      body: JSON.stringify({ key, value })
    });
  }

  public async setCredential(key: string, value: string): Promise<{ saved: boolean }> {
    return this.saveCredential(key, value);
  }

  public async exportDiagnostics(): Promise<Record<string, unknown>> {
    return this.request<Record<string, unknown>>("/api/v1/diagnostics/export");
  }

  public async openWorkspaceInExplorer(path: string): Promise<void> {
    await invokeTauriCommand("open_workspace_in_explorer", { path });
  }

  public async stopBackgroundOrchestrator(): Promise<void> {
    await invokeTauriCommand("stop_background_orchestrator");
  }

  public async exitApplicationCompletely(): Promise<void> {
    await invokeTauriCommand("exit_application_completely");
  }

  public subscribeEvents(onBatch: (events: AgentEvent[]) => void): () => void {
    if (typeof EventSource === "undefined") {
      return () => {};
    }
    let buffer: AgentEvent[] = [];
    let flushTimer: ReturnType<typeof setTimeout> | null = null;
    const url = `${this.baseUrl}/api/v1/events/stream`;
    let es: EventSource | null = null;

    try {
      es = new EventSource(url);
      es.onmessage = (ev) => {
        try {
          const parsed = JSON.parse(ev.data) as AgentEvent;
          buffer.push(parsed);
          if (!flushTimer) {
            flushTimer = setTimeout(() => {
              const items = buffer;
              buffer = [];
              flushTimer = null;
              if (items.length > 0) {
                onBatch(items);
              }
            }, 60);
          }
        } catch {
          // ignore malformed event
        }
      };
    } catch {
      // EventSource connection failed
    }

    return () => {
      if (flushTimer) clearTimeout(flushTimer);
      es?.close();
    };
  }

  public subscribeToEventStream(onBatch: (events: AgentEvent[]) => void): () => void {
    return this.subscribeEvents(onBatch);
  }
}

export const defaultSymphonyClient = new SymphonyApiClient();

export async function createSymphonyClientFromTauri(): Promise<SymphonyApiClient> {
  const client = new SymphonyApiClient();
  await client.ensureTauriEndpoint();
  return client;
}

export async function setCloseBehavior(minimizeToTray: boolean): Promise<void> {
  await invokeTauriCommand("set_close_behavior", { minimizeToTray });
}

export async function stopBackgroundOrchestrator(): Promise<void> {
  await invokeTauriCommand("stop_background_orchestrator");
}

export async function exitApplicationCompletely(): Promise<void> {
  await invokeTauriCommand("exit_application_completely");
}

export async function openWorkspaceInExplorer(path: string): Promise<void> {
  await invokeTauriCommand("open_workspace_in_explorer", { path });
}
