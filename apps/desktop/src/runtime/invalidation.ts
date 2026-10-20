import type { QueryClient, QueryKey } from "@tanstack/react-query";
import type { AgentEvent } from "@symphony/shared-types";
import { qk } from "../lib/queryKeys.js";

/**
 * Map a batch of runtime events to the cached resources they make stale. Streaming item events
 * (agent messages, commands…) only touch per-task timelines and the live-session snapshot, so a
 * busy agent does not trigger list refetches on every token.
 */
export function keysToInvalidate(batch: AgentEvent[]): QueryKey[] {
  const keys = new Map<string, QueryKey>();
  const add = (key: QueryKey) => keys.set(JSON.stringify(key), key);
  for (const e of batch) {
    const type = e.event_type;
    const taskId = e.task_id;
    if (taskId) add(qk.taskEvents(taskId));
    if (type.startsWith("task/") || type.startsWith("attempt/") || type.startsWith("workspace/") || type === "pr/human_approved") {
      add(qk.tasksAll);
      add(qk.state);
      add(qk.worktrees);
      if (taskId) {
        add(qk.task(taskId));
        add(qk.taskAttempts(taskId));
        add(qk.taskValidations(taskId));
        add(qk.taskDiff(taskId));
      }
    }
    if (type.startsWith("attempt/") || type.startsWith("pr/")) add(qk.prsAll);
    if (type.startsWith("approval/") || type.endsWith("requestApproval") || type.endsWith("approvalResolved")) {
      add(qk.approvalsAll);
      add(qk.state);
    }
    if (type.startsWith("project/")) add(qk.projects);
    if (type.startsWith("scheduler/")) add(qk.state);
    if (type.startsWith("workflow/")) add(qk.workflow);
    if (type === "session_started" || type === "turn/completed" || type.startsWith("item/fileChange")) {
      add(qk.state);
      if (taskId) add(qk.taskDiff(taskId));
    }
  }
  return Array.from(keys.values());
}

export function invalidateForEvents(client: QueryClient, batch: AgentEvent[]): void {
  for (const key of keysToInvalidate(batch)) {
    void client.invalidateQueries({ queryKey: key });
  }
}
