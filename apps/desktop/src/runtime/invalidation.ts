import type { QueryClient, QueryKey } from "@tanstack/react-query";
import type { AgentEvent } from "@symphony/shared-types";
import { qk } from "../lib/queryKeys.js";

/**
 * Map a batch of runtime events to the cached resources they make stale. Streaming item events
 * (agent messages, commands…) only touch per-task timelines and the live-session snapshot, so a
 * busy agent does not trigger list refetches on every token.
 */
/**
 * Event types that can start or finish a tool call (see the drivers' tool-call reports). Message
 * deltas are not among them, so streaming text never refetches the tool-call history.
 */
const TOOL_CALL_EVENT_TYPES = new Set([
  "item/started",
  "item/completed",
  "item/tool/call",
  "tool/result",
  "item/commandExecution",
  "item/fileChange",
  "claude/assistant",
  "claude/user",
  "opencode/event",
  "codesandbox/tool"
]);

export function carriesToolCalls(eventType: string): boolean {
  return TOOL_CALL_EVENT_TYPES.has(eventType);
}

export function keysToInvalidate(batch: AgentEvent[]): QueryKey[] {
  const keys = new Map<string, QueryKey>();
  const add = (key: QueryKey) => keys.set(JSON.stringify(key), key);
  for (const e of batch) {
    const type = e.event_type;
    const taskId = e.task_id;
    if (taskId) add(qk.taskEvents(taskId));
    if (taskId && carriesToolCalls(type)) add(qk.taskToolCalls(taskId));
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
    // An automatic refresh changes one review: refresh its open detail view too.
    const reviewId = (e.payload as { review_id?: unknown } | null)?.review_id;
    if (type.startsWith("pr/") && typeof reviewId === "string") add(qk.pr(reviewId));
    if (type.startsWith("approval/") || type.endsWith("requestApproval") || type.endsWith("approvalResolved")) {
      add(qk.approvalsAll);
      add(qk.state);
    }
    if (type.startsWith("project/")) add(qk.projects);
    if (type.startsWith("scheduler/")) add(qk.state);
    if (type.startsWith("workflow/")) add(qk.workflow);
    if (type.startsWith("backup/")) add(qk.backups);
    if (type.startsWith("knowledge/")) {
      const projectId = (e.payload as { project_id?: unknown } | null)?.project_id;
      const scope = typeof projectId === "string" ? qk.knowledge(projectId) : qk.knowledgeAll;
      if (type === "knowledge/repo_changed") {
        // History and worktree state moved: the index and the AI results are unaffected.
        for (const part of ["worktrees", "commits", "commit", "diff", "overview"]) add([...scope, part]);
      } else if (type === "knowledge/analysis_completed") {
        add([...scope, "analysis"]);
      } else if (type === "knowledge/chat_completed") {
        add([...scope, "chat"]);
      } else if (type === "knowledge/index_progress") {
        // Nothing in the index has changed yet, only the status line: not every open graph view.
        add([...scope, "overview"]);
      } else {
        // A finished (or failed) run: the overview and every graph view.
        add(scope);
      }
    }
    if (type === "session_started" || type === "turn/completed" || type.startsWith("item/fileChange")) {
      add(qk.state);
      if (taskId) add(qk.taskDiff(taskId));
    }
    // A cloud agent's push (applied to the worktree by the runtime) changes the task's files.
    if (type === "codesandbox/worktree_updated" || type === "codesandbox/pushed") {
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
