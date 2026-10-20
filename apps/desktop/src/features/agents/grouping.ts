/**
 * Pure grouping logic for the Agent Control Center. Combines the orchestrator snapshot (live
 * sessions, retry queue), task records and pending approvals into one row per task, assigned to
 * exactly one segment with a clear precedence:
 * live session (running / waiting for approval) > approval without session > retrying > queued >
 * recently failed > recently cancelled > recently completed.
 *
 * Rows can then be presented by status segment, provider or project (`groupRowsBy`).
 */
import {
  canCancel,
  canDispatch,
  canRetry,
  providerActions,
  type ApprovalRequest,
  type OrchestratorStateSnapshot,
  type Project,
  type ProviderCapabilities,
  type ProviderKind,
  type RunAttemptPhase,
  type Task,
  type TaskStatus
} from "@symphony/shared-types";

export type AgentSegment = "running" | "approval" | "retrying" | "queued" | "failed" | "cancelled" | "completed";

export const AGENT_SEGMENTS: ReadonlyArray<AgentSegment> = [
  "running",
  "approval",
  "retrying",
  "queued",
  "failed",
  "cancelled",
  "completed"
];

/** Segments that represent finished work (shown only within the recent window). */
export const FINISHED_SEGMENTS: ReadonlyArray<AgentSegment> = ["failed", "cancelled", "completed"];

export const RECENT_WINDOW_MS = 24 * 60 * 60 * 1000;

export interface AgentRow {
  taskId: string;
  segment: AgentSegment;
  identifier: string;
  title: string | null;
  projectId: string | null;
  provider: ProviderKind | null;
  model: string | null;
  status: TaskStatus | null;
  phase: RunAttemptPhase | null;
  /** A live agent session exists for this task. */
  live: boolean;
  sessionId: string | null;
  startedAt: string | null;
  durationSeconds: number;
  totalTokens: number;
  /** Estimated spend recorded on the task (USD); `null` when unknown. */
  costUsd: number | null;
  operation: string | null;
  lastEventAt: string | null;
  turnCount: number | null;
  workspacePath: string | null;
  pendingApprovals: number;
  attempt: number | null;
  dueAt: string | null;
  error: string | null;
  finishedAt: string | null;
}

export interface AgentGroupingInput {
  snapshot?: OrchestratorStateSnapshot | null;
  tasks?: ReadonlyArray<Task> | null;
  approvals?: ReadonlyArray<ApprovalRequest> | null;
  projects?: ReadonlyArray<Project> | null;
  /** Current time (ms since epoch); injected for determinism. */
  now: number;
  recentWindowMs?: number;
}

function pendingApprovalCounts(approvals: ReadonlyArray<ApprovalRequest>): Map<string, number> {
  const counts = new Map<string, number>();
  for (const a of approvals) {
    if (a.status !== "pending") continue;
    counts.set(a.task_id, (counts.get(a.task_id) ?? 0) + 1);
  }
  return counts;
}

function fromTask(task: Task, segment: AgentSegment, pending: number): AgentRow {
  return {
    taskId: task.id,
    segment,
    identifier: task.issue.identifier,
    title: task.issue.title,
    projectId: task.project_id,
    provider: task.provider,
    model: task.model,
    status: task.status,
    phase: task.current_phase,
    live: false,
    sessionId: task.current_session_id,
    startedAt: task.started_at,
    durationSeconds: task.duration_seconds,
    totalTokens: task.total_tokens,
    costUsd: task.estimated_cost_usd,
    operation: null,
    lastEventAt: null,
    turnCount: null,
    workspacePath: task.workspace_path,
    pendingApprovals: pending,
    attempt: task.attempt_count || null,
    dueAt: task.next_retry_at,
    error: task.last_error,
    finishedAt: finishedTime(task, segment)
  };
}

const time = (iso: string | null | undefined) => (iso ? new Date(iso).getTime() : 0);

/**
 * When a finished row ended. `completed_at` is only trusted for completed/review rows: a retried
 * task keeps the `completed_at` of an earlier run, so failures and cancellations use `updated_at`.
 */
function finishedTime(task: Task, segment: AgentSegment): string | null {
  if (segment === "completed") return task.completed_at ?? task.updated_at;
  return FINISHED_SEGMENTS.includes(segment) ? task.updated_at : null;
}

/** Build one row per task across all segments. */
export function buildAgentRows(input: AgentGroupingInput): AgentRow[] {
  const { snapshot, now } = input;
  const tasks = input.tasks ?? [];
  const window = input.recentWindowMs ?? RECENT_WINDOW_MS;
  const taskById = new Map(tasks.map((t) => [t.id, t]));
  const autoProjects = new Set((input.projects ?? []).filter((p) => p.auto_dispatch).map((p) => p.id));
  const pending = pendingApprovalCounts(input.approvals ?? []);
  const seen = new Set<string>();
  const rows: AgentRow[] = [];
  const push = (row: AgentRow) => {
    seen.add(row.taskId);
    rows.push(row);
  };

  // 1. Live sessions.
  for (const s of snapshot?.running ?? []) {
    if (seen.has(s.task_id)) continue;
    const task = taskById.get(s.task_id);
    const count = pending.get(s.task_id) ?? 0;
    push({
      taskId: s.task_id,
      segment: count > 0 ? "approval" : "running",
      identifier: s.issue_identifier,
      title: task?.issue.title ?? null,
      projectId: s.project_id,
      provider: s.provider,
      model: s.model ?? task?.model ?? null,
      status: task?.status ?? "running",
      phase: s.phase,
      live: true,
      sessionId: s.session_id,
      startedAt: s.started_at,
      durationSeconds: Math.max(0, (now - time(s.started_at)) / 1000),
      totalTokens: s.tokens.total_tokens,
      costUsd: task?.estimated_cost_usd ?? null,
      operation: s.last_message || s.last_event || null,
      lastEventAt: s.last_event_at,
      turnCount: s.turn_count,
      workspacePath: s.workspace_path || task?.workspace_path || null,
      pendingApprovals: count,
      attempt: task?.attempt_count ?? null,
      dueAt: null,
      error: null,
      finishedAt: null
    });
  }

  // 2. Pending approvals whose session is not (or no longer) in the snapshot.
  for (const [taskId, count] of pending) {
    if (seen.has(taskId)) continue;
    const task = taskById.get(taskId);
    if (task) push(fromTask(task, "approval", count));
  }

  // 3. Retry queue.
  for (const r of snapshot?.retrying ?? []) {
    if (seen.has(r.task_id)) continue;
    const task = taskById.get(r.task_id);
    const base = task ? fromTask(task, "retrying", pending.get(r.task_id) ?? 0) : null;
    push({
      ...(base ?? {
        taskId: r.task_id,
        segment: "retrying" as const,
        identifier: r.identifier,
        title: null,
        projectId: null,
        provider: null,
        model: null,
        status: null,
        phase: null,
        live: false,
        sessionId: null,
        startedAt: null,
        durationSeconds: 0,
        totalTokens: 0,
        costUsd: null,
        operation: null,
        lastEventAt: null,
        turnCount: null,
        workspacePath: null,
        pendingApprovals: 0,
        finishedAt: null
      }),
      identifier: r.identifier,
      attempt: r.attempt,
      dueAt: r.due_at,
      error: r.error ?? base?.error ?? null
    });
  }

  // 4. Queued (and ready tasks in auto-dispatch projects, which the scheduler will pick up).
  for (const t of tasks) {
    if (seen.has(t.id)) continue;
    if (t.status === "queued" || (t.status === "ready" && autoProjects.has(t.project_id))) {
      push(fromTask(t, "queued", pending.get(t.id) ?? 0));
    }
  }

  // 5-7. Recently failed / cancelled / completed.
  for (const t of tasks) {
    if (seen.has(t.id)) continue;
    const done = t.status === "completed" || t.status === "human_review";
    const finished = time(done ? (t.completed_at ?? t.updated_at) : t.updated_at);
    if (now - finished > window) continue;
    if (t.status === "failed") push(fromTask(t, "failed", 0));
    else if (t.status === "cancelled") push(fromTask(t, "cancelled", 0));
    else if (t.status === "completed" || t.status === "human_review") push(fromTask(t, "completed", 0));
  }
  return rows;
}

export type AgentGroups = Record<AgentSegment, AgentRow[]>;

/** Split rows into segments with a stable, operator-friendly order inside each. */
export function groupAgentRows(rows: ReadonlyArray<AgentRow>): AgentGroups {
  const groups = Object.fromEntries(AGENT_SEGMENTS.map((s) => [s, [] as AgentRow[]])) as AgentGroups;
  for (const row of rows) groups[row.segment].push(row);
  const byStart = (a: AgentRow, b: AgentRow) => time(a.startedAt) - time(b.startedAt);
  groups.running.sort(byStart);
  groups.approval.sort(byStart);
  groups.retrying.sort((a, b) => time(a.dueAt) - time(b.dueAt));
  // Queued rows keep the runtime's task order (the scheduler's own ordering).
  const byFinishedDesc = (a: AgentRow, b: AgentRow) => time(b.finishedAt) - time(a.finishedAt);
  for (const segment of FINISHED_SEGMENTS) groups[segment].sort(byFinishedDesc);
  return groups;
}

export type AgentGroupBy = "status" | "provider" | "project";

export const AGENT_GROUP_BY: ReadonlyArray<AgentGroupBy> = ["status", "provider", "project"];

export interface AgentRowGroup {
  /** Segment, provider kind or project id; `null` collects rows without a known provider/project. */
  key: string | null;
  rows: AgentRow[];
}

/**
 * Present rows grouped by status segment, provider or project. Rows keep the segment ordering
 * (live work first) inside every group; empty status segments are omitted.
 */
export function groupRowsBy(
  rows: ReadonlyArray<AgentRow>,
  by: AgentGroupBy,
  options: { providerOrder?: ReadonlyArray<ProviderKind>; projectNames?: ReadonlyMap<string, string> } = {}
): AgentRowGroup[] {
  const segments = groupAgentRows(rows);
  if (by === "status") {
    return AGENT_SEGMENTS.filter((s) => segments[s].length > 0).map((s) => ({ key: s, rows: segments[s] }));
  }
  const ordered = AGENT_SEGMENTS.flatMap((s) => segments[s]);
  const buckets = new Map<string | null, AgentRow[]>();
  for (const row of ordered) {
    const key = by === "provider" ? row.provider : row.projectId;
    const list = buckets.get(key);
    if (list) list.push(row);
    else buckets.set(key, [row]);
  }
  const groups = [...buckets].map(([key, list]) => ({ key, rows: list }));
  const rank = (key: string): string => {
    if (by === "provider") {
      const index = options.providerOrder?.indexOf(key as ProviderKind) ?? -1;
      return index >= 0 ? String(index).padStart(4, "0") : `~${key}`;
    }
    return (options.projectNames?.get(key) ?? `~${key}`).toLocaleLowerCase();
  };
  // Rows without a known provider/project always come last.
  return groups.sort((a, b) =>
    a.key === null ? (b.key === null ? 0 : 1) : b.key === null ? -1 : rank(a.key).localeCompare(rank(b.key))
  );
}

export interface AgentRowActions {
  dispatch: boolean;
  retry: boolean;
  cancel: boolean;
}

/**
 * Actions a row may offer, from the shared state machine and runtime-reported provider
 * capabilities. Mid-turn pause is never offered here: the runtime exposes no per-run pause
 * endpoint and no provider currently reports `supports_mid_turn_pause`.
 */
export function rowActions(row: AgentRow, capabilities: ProviderCapabilities | null | undefined): AgentRowActions {
  const status = row.status;
  return {
    dispatch: status !== null && row.segment === "queued" && canDispatch(status),
    retry: status !== null && !row.live && canRetry(status),
    // Stopping a live session requires provider support; queued/retrying work can always be cancelled.
    cancel: status !== null && canCancel(status) && (!row.live || providerActions(capabilities).cancel)
  };
}
