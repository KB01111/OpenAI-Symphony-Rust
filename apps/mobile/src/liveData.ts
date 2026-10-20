/**
 * Pure data helpers for the companion: query keys, event → cache invalidation mapping, task list
 * filtering/sorting. Type-only imports so it runs under `node --test`.
 */
import type { AgentEvent, Task, TaskStatus } from "@symphony/shared-types";

export const mobileKeys = {
  all: ["symphony"] as const,
  state: ["symphony", "state"] as const,
  tasks: ["symphony", "tasks"] as const,
  approvals: ["symphony", "approvals"] as const,
  taskEvents: (taskId: string) => ["symphony", "task-events", taskId] as const
};

export type MobileQueryKey = readonly string[];

/**
 * Map a batch of runtime events to the cached resources they make stale. Streaming item events
 * only touch the per-task timeline (and the live snapshot when a turn starts/ends), so a busy
 * agent does not refetch the task list on every token.
 */
export function keysForEvents(batch: ReadonlyArray<Pick<AgentEvent, "event_type" | "task_id">>): MobileQueryKey[] {
  const keys = new Map<string, MobileQueryKey>();
  const add = (key: MobileQueryKey) => keys.set(key.join("\u0000"), key);
  for (const event of batch) {
    const type = event.event_type;
    if (event.task_id) add(mobileKeys.taskEvents(event.task_id));
    if (type.startsWith("task/") || type.startsWith("attempt/") || type.startsWith("workspace/")) {
      add(mobileKeys.tasks);
      add(mobileKeys.state);
    }
    if (type.startsWith("approval/") || type.endsWith("requestApproval") || type.endsWith("approvalResolved")) {
      add(mobileKeys.approvals);
      add(mobileKeys.state);
    }
    if (type.startsWith("scheduler/") || type === "session_started" || type === "turn/completed") {
      add(mobileKeys.state);
    }
  }
  return Array.from(keys.values());
}

export type TaskFilter = "all" | "active" | "attention" | "done";

export const TASK_FILTERS: ReadonlyArray<TaskFilter> = ["all", "active", "attention", "done"];

const FILTER_STATUSES: Record<Exclude<TaskFilter, "all">, ReadonlyArray<TaskStatus>> = {
  active: ["ready", "queued", "running"],
  attention: ["blocked", "human_review", "failed"],
  done: ["completed", "cancelled"]
};

export function matchesFilter(status: TaskStatus, filter: TaskFilter): boolean {
  return filter === "all" || FILTER_STATUSES[filter].includes(status);
}

/** Filter by group and sort newest-updated first (stable for equal timestamps). */
export function visibleTasks(tasks: ReadonlyArray<Task>, filter: TaskFilter): Task[] {
  return tasks
    .filter((task) => matchesFilter(task.status, filter))
    .map((task, index) => ({ task, index, time: Date.parse(task.updated_at) || 0 }))
    .sort((a, b) => b.time - a.time || a.index - b.index)
    .map((entry) => entry.task);
}

export function countByFilter(tasks: ReadonlyArray<Task>): Record<TaskFilter, number> {
  const counts: Record<TaskFilter, number> = { all: tasks.length, active: 0, attention: 0, done: 0 };
  for (const task of tasks) {
    for (const filter of ["active", "attention", "done"] as const) {
      if (matchesFilter(task.status, filter)) counts[filter] += 1;
    }
  }
  return counts;
}
