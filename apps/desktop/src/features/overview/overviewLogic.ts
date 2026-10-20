import type { AgentEvent, OrchestratorStateSnapshot, Task } from "@symphony/shared-types";

export interface OverviewMetrics {
  activeAgents: number;
  queued: number;
  failed: number;
  completed: number;
  humanReview: number;
  retrying: number;
  activeRepositories: number;
  totalTokens: number;
  estimatedCost: number;
  secondsRunning: number;
  health: "healthy" | "degraded" | "paused";
}

/**
 * Derive dashboard metrics from runtime data only. A repository is "active" when it has a task
 * that is queued, running, blocked or awaiting review.
 */
export function computeOverviewMetrics(
  state: OrchestratorStateSnapshot,
  tasks: ReadonlyArray<Task>,
  streamLive: boolean
): OverviewMetrics {
  const activeStatuses = new Set(["queued", "running", "blocked", "human_review"]);
  const activeRepositories = new Set(tasks.filter((t) => activeStatuses.has(t.status)).map((t) => t.project_id)).size;
  const degraded = !streamLive || state.workflow_error !== null || state.hardware.effective_max_concurrency === 0;
  return {
    activeAgents: state.counts.running,
    queued: state.counts.queued,
    failed: state.counts.failed,
    completed: state.counts.completed,
    humanReview: state.counts.human_review,
    retrying: state.counts.retrying,
    activeRepositories,
    totalTokens: state.codex_totals.total_tokens,
    estimatedCost: state.codex_totals.estimated_cost_usd,
    secondsRunning: state.codex_totals.seconds_running,
    health: state.scheduler_paused ? "paused" : degraded ? "degraded" : "healthy"
  };
}

/** Merge persisted recent events with live ones; newest first, deduped, capped. */
export function recentTimeline(history: ReadonlyArray<AgentEvent> | undefined, live: ReadonlyArray<AgentEvent>, limit = 80): AgentEvent[] {
  const map = new Map<string, AgentEvent>();
  for (const e of history ?? []) map.set(e.id, e);
  for (const e of live) map.set(e.id, e);
  return Array.from(map.values())
    .sort((a, b) => (a.timestamp < b.timestamp ? 1 : a.timestamp > b.timestamp ? -1 : 0))
    .slice(0, limit);
}
