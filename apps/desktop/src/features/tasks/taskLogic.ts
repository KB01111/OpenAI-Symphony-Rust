/**
 * Pure task-management logic (filtering, ordering, grouping and manual-move validation). Kept
 * free of React so it can be unit tested and reused by the board, table and menus.
 */
import {
  ALL_TASK_STATUSES,
  explainManualTransition,
  formatMessage,
  type OrchestratorStateSnapshot,
  type Task,
  type TaskStatus,
  type TransitionRejection,
  type TranslationDictionary
} from "@symphony/shared-types";
import type { TaskFilters } from "../../store/uiStore.js";

/** Priority filter value meaning "tasks without a priority". */
export const NO_PRIORITY = 0;
export const PRIORITY_VALUES: ReadonlyArray<number> = [1, 2, 3, 4];

/** Normalized priority: 1..4, or `null` when unset/out of range (0 = "no priority" in trackers). */
export function normalizePriority(priority: number | null | undefined): number | null {
  return priority != null && priority >= 1 && priority <= 4 ? priority : null;
}

export function matchesSearch(task: Task, search: string): boolean {
  const q = search.trim().toLowerCase();
  if (!q) return true;
  return (
    task.issue.identifier.toLowerCase().includes(q) ||
    task.issue.title.toLowerCase().includes(q) ||
    (task.issue.description ?? "").toLowerCase().includes(q)
  );
}

export function matchesFilters(task: Task, filters: TaskFilters): boolean {
  if (filters.statuses.length > 0 && !filters.statuses.includes(task.status)) return false;
  if (filters.providers.length > 0 && !filters.providers.includes(task.provider)) return false;
  if (filters.priority != null) {
    const p = normalizePriority(task.issue.priority);
    if (filters.priority === NO_PRIORITY ? p !== null : p !== filters.priority) return false;
  }
  return matchesSearch(task, filters.search);
}

export function filterTasks(tasks: ReadonlyArray<Task>, filters: TaskFilters): Task[] {
  return tasks.filter((t) => matchesFilters(t, filters));
}

export function hasActiveFilters(filters: TaskFilters): boolean {
  return filters.search.trim() !== "" || filters.statuses.length > 0 || filters.providers.length > 0 || filters.priority != null;
}

/** Board order: priority ascending (1..4 first, no priority last), then oldest first. */
export function compareBoardOrder(a: Task, b: Task): number {
  const pa = normalizePriority(a.issue.priority) ?? Number.POSITIVE_INFINITY;
  const pb = normalizePriority(b.issue.priority) ?? Number.POSITIVE_INFINITY;
  if (pa !== pb) return pa < pb ? -1 : 1;
  const ca = Date.parse(a.created_at);
  const cb = Date.parse(b.created_at);
  if (ca !== cb) return ca - cb;
  return a.issue.identifier.localeCompare(b.issue.identifier);
}

export function sortForBoard(tasks: ReadonlyArray<Task>): Task[] {
  return [...tasks].sort(compareBoardOrder);
}

export type TasksByStatus = Record<TaskStatus, Task[]>;

/** Group into all nine board columns (in `ALL_TASK_STATUSES` order), each sorted for the board. */
export function groupByStatus(tasks: ReadonlyArray<Task>): TasksByStatus {
  const groups = Object.fromEntries(ALL_TASK_STATUSES.map((s) => [s, [] as Task[]])) as TasksByStatus;
  for (const task of tasks) groups[task.status]?.push(task);
  for (const status of ALL_TASK_STATUSES) groups[status].sort(compareBoardOrder);
  return groups;
}

/** Task ids with a live agent session in the orchestrator snapshot. */
export function activeTaskIds(state: Pick<OrchestratorStateSnapshot, "running"> | undefined): Set<string> {
  return new Set(state?.running.map((r) => r.task_id) ?? []);
}

/** Statuses a task may be moved to manually right now (excludes its own status and `running`). */
export function allowedMoveTargets(from: TaskStatus, hasActiveSession: boolean): TaskStatus[] {
  return ALL_TASK_STATUSES.filter((to) => explainManualTransition(from, to, hasActiveSession) === null);
}

export type DropDecision =
  | { kind: "noop" }
  | { kind: "reject"; reason: TransitionRejection }
  | { kind: "move"; status: TaskStatus };

/** Decide what a drop of `task` on column `target` should do, without calling the API. */
export function resolveDrop(task: Pick<Task, "id" | "status">, target: TaskStatus | null, activeIds: ReadonlySet<string>): DropDecision {
  if (!target) return { kind: "noop" };
  const reason = explainManualTransition(task.status, target, activeIds.has(task.id));
  if (reason === "same_status") return { kind: "noop" };
  if (reason) return { kind: "reject", reason };
  return { kind: "move", status: target };
}

/** Localized explanation for a rejected manual transition. */
export function rejectionMessage(t: TranslationDictionary, reason: TransitionRejection, from: TaskStatus, to: TaskStatus): string {
  return formatMessage(t.transitionRejection[reason], { from: t.status[from], to: t.status[to] });
}

export interface DropHandlers {
  move: (id: string, status: TaskStatus) => void;
  warn: (title: string, body: string) => void;
}

/**
 * Apply a drop: valid moves call `move` (the optimistic PATCH); invalid ones only show the
 * localized reason and never reach the runtime.
 */
export function applyDrop(
  task: Pick<Task, "id" | "status">,
  target: TaskStatus | null,
  activeIds: ReadonlySet<string>,
  t: TranslationDictionary,
  rejectTitle: string,
  handlers: DropHandlers
): DropDecision {
  const decision = resolveDrop(task, target, activeIds);
  if (decision.kind === "reject" && target) handlers.warn(rejectTitle, rejectionMessage(t, decision.reason, task.status, target));
  if (decision.kind === "move") handlers.move(task.id, decision.status);
  return decision;
}

/** Comma-separated labels → trimmed, de-duplicated list. */
export function parseLabels(input: string): string[] {
  const seen = new Set<string>();
  for (const part of input.split(",")) {
    const label = part.trim();
    if (label) seen.add(label);
  }
  return [...seen];
}

/** Index of a status in board order (used to sort the table's status column). */
export function statusOrder(status: TaskStatus): number {
  return ALL_TASK_STATUSES.indexOf(status);
}
