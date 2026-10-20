import type { ProviderCapabilities, ProviderKind, RunAttemptPhase, TaskStatus } from "./generated/openapi/types.gen.js";
import { TASK_STATUS_TRANSITIONS } from "./generated/stateMachine.js";

export const ALL_PROVIDERS: ReadonlyArray<ProviderKind> = [
  "codex",
  "claude_code",
  "opencode",
  "simulated"
];

/** Mirrors `TaskStatus::can_transition_to` using the generated Rust transition table. */
export function canTransitionTaskStatus(from: TaskStatus, to: TaskStatus): boolean {
  return TASK_STATUS_TRANSITIONS[from].includes(to);
}

export type TransitionRejection =
  | "same_status"
  | "use_dispatch"
  | "active_session"
  | "not_allowed";

/**
 * Explains why a manual status move would be rejected by the runtime, or returns `null` when the
 * move is allowed. Matches the checks in `SymphonyOrchestrator::transition_task`; the server
 * remains authoritative and re-validates every request.
 */
export function explainManualTransition(
  from: TaskStatus,
  to: TaskStatus,
  hasActiveSession: boolean
): TransitionRejection | null {
  if (from === to) return "same_status";
  if (to === "running") return "use_dispatch";
  if (hasActiveSession) return "active_session";
  if (!canTransitionTaskStatus(from, to)) return "not_allowed";
  return null;
}

/** Statuses from which the runtime accepts `POST /tasks/{id}/dispatch`. */
export function canDispatch(status: TaskStatus): boolean {
  return status !== "running" && canTransitionTaskStatus(status, "running");
}

/** Statuses from which `POST /tasks/{id}/retry` (move to queued + dispatch) is accepted. */
export function canRetry(status: TaskStatus): boolean {
  return status !== "queued" && status !== "running" && canTransitionTaskStatus(status, "queued");
}

export function canCancel(status: TaskStatus): boolean {
  return status !== "cancelled" && canTransitionTaskStatus(status, "cancelled");
}

export const ACTIVE_PHASES: ReadonlyArray<RunAttemptPhase> = [
  "preparing_workspace",
  "building_prompt",
  "launching_agent_process",
  "initializing_session",
  "streaming_turn",
  "finishing"
];

export function isActivePhase(phase: RunAttemptPhase | null | undefined): boolean {
  return phase != null && ACTIVE_PHASES.includes(phase);
}

/** Provider actions the UI may offer, derived only from runtime-reported capabilities. */
export interface ProviderActionAvailability {
  cancel: boolean;
  pause: boolean;
  approvals: boolean;
  resume: boolean;
}

export function providerActions(
  capabilities: ProviderCapabilities | null | undefined
): ProviderActionAvailability {
  return {
    cancel: capabilities?.supports_cancel ?? false,
    pause: capabilities?.supports_mid_turn_pause ?? false,
    approvals: capabilities?.supports_approvals ?? false,
    resume: capabilities?.supports_session_resume ?? false
  };
}
