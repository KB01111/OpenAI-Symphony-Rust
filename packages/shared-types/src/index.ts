export * from "./i18n.js";

export type TaskStatus =
  | "backlog"
  | "ready"
  | "queued"
  | "running"
  | "blocked"
  | "human_review"
  | "completed"
  | "failed"
  | "cancelled";

export const ALL_TASK_STATUSES: TaskStatus[] = [
  "backlog",
  "ready",
  "queued",
  "running",
  "blocked",
  "human_review",
  "completed",
  "failed",
  "cancelled"
];

export function canTransitionTaskStatus(from: TaskStatus, to: TaskStatus): boolean {
  if (from === to) return true;
  switch (from) {
    case "backlog":
      return ["ready", "queued", "blocked", "cancelled"].includes(to);
    case "ready":
      return ["backlog", "queued", "running", "blocked", "cancelled"].includes(to);
    case "queued":
      return ["ready", "running", "blocked", "failed", "cancelled"].includes(to);
    case "running":
      return ["queued", "blocked", "human_review", "completed", "failed", "cancelled"].includes(to);
    case "blocked":
      return ["backlog", "ready", "queued", "cancelled"].includes(to);
    case "human_review":
      return ["queued", "running", "completed", "failed", "cancelled"].includes(to);
    case "completed":
      return ["ready", "queued"].includes(to);
    case "failed":
      return ["ready", "queued", "cancelled"].includes(to);
    case "cancelled":
      return ["backlog", "ready", "queued"].includes(to);
  }
}

export type ProviderKind = "codex" | "claude_code" | "opencode" | "simulated";

export type RunAttemptPhase =
  | "preparing_workspace"
  | "building_prompt"
  | "launching_agent_process"
  | "initializing_session"
  | "streaming_turn"
  | "finishing"
  | "succeeded"
  | "failed"
  | "timed_out"
  | "stalled"
  | "canceled_by_reconciliation";

export interface BlockerRef {
  id: string | null;
  identifier: string | null;
  state: string | null;
}

export interface Issue {
  id: string;
  native_ref?: Record<string, unknown> | null;
  identifier: string;
  title: string;
  description: string | null;
  priority: number | null;
  state: string;
  branch_name: string | null;
  url: string | null;
  assignee_id: string | null;
  labels: string[];
  blocked_by: BlockerRef[];
  dispatchable: boolean;
  created_at: string | null;
  updated_at: string | null;
}

export interface ProviderCapabilities {
  supports_start_session: boolean;
  supports_resume_session: boolean;
  supports_cancel_execution: boolean;
  supports_pause_session: boolean;
  supports_stream_events: boolean;
  supports_approvals: boolean;
  supports_dynamic_tools: boolean;
  supports_token_usage: boolean;
  supports_model_listing: boolean;
}

export interface ProviderProfile {
  id: string;
  name: string;
  provider: ProviderKind;
  command: string;
  default_model: string | null;
  available_models: string[];
  profile_name: string | null;
  authenticated: boolean;
  installed: boolean;
  version: string | null;
  capabilities: ProviderCapabilities;
}

export interface Project {
  id: string;
  name: string;
  repo_path: string;
  default_branch: string;
  workspace_root: string;
  workflow_path: string | null;
  preferred_provider: ProviderKind;
  preferred_model: string | null;
  tracker_kind: string;
  github_repo: string | null;
  linear_project_slug: string | null;
  required_labels: string[];
  auto_dispatch: boolean;
  max_concurrent_agents: number;
  validation_commands: string[];
  created_at: string;
  updated_at: string;
}

export interface Task {
  id: string;
  project_id: string;
  issue: Issue;
  status: TaskStatus;
  claim_state: "unclaimed" | "claimed" | "released";
  assigned_provider: ProviderKind;
  assigned_model: string | null;
  preferred_provider?: ProviderKind;
  preferred_model?: string | null;
  workspace_key: string;
  workspace_path: string | null;
  branch_name: string;
  current_attempt: number;
  session_id: string | null;
  thread_id: string | null;
  turn_id: string | null;
  pr_number: number | null;
  pr_url: string | null;
  total_input_tokens: number;
  total_output_tokens: number;
  total_tokens: number;
  estimated_cost_usd: number;
  duration_seconds: number;
  last_error: string | null;
  created_at: string;
  updated_at: string;
}

export interface RunAttempt {
  id: string;
  task_id: string;
  issue_id: string;
  issue_identifier: string;
  attempt: number;
  provider: ProviderKind;
  model: string | null;
  workspace_path: string;
  phase: RunAttemptPhase;
  session_id: string | null;
  thread_id: string | null;
  turn_id: string | null;
  turn_count: number;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  error: string | null;
  started_at: string;
  finished_at: string | null;
}

export interface LiveSession {
  task_id: string;
  issue_id: string;
  issue_identifier: string;
  provider: ProviderKind;
  model: string | null;
  profile: string | null;
  session_id: string;
  thread_id: string;
  turn_id: string;
  workspace_path: string;
  codex_app_server_pid: number | null;
  phase: RunAttemptPhase;
  current_operation: string;
  turn_count: number;
  started_at: string;
  last_event: string | null;
  last_message: string | null;
  last_timestamp: string | null;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  supports_pause_session: boolean;
}

export interface RetryEntry {
  task_id: string;
  issue_id: string;
  identifier: string;
  attempt: number;
  due_at: string;
  error: string | null;
}

export interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
}

export interface CodexTotals {
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  seconds_running: number;
  estimated_cost_usd: number;
}

export interface AgentEvent {
  id: string;
  task_id: string;
  issue_identifier: string;
  attempt_id?: string | null;
  session_id: string | null;
  provider: ProviderKind;
  event_type: string;
  phase?: RunAttemptPhase | null;
  turn_number?: number | null;
  summary?: string;
  message?: string;
  payload?: Record<string, unknown> | string | null;
  raw_payload?: Record<string, unknown> | string | null;
  usage: TokenUsage | null;
  timestamp: string;
}

export type ApprovalStatus = "pending" | "approved" | "approved_for_session" | "denied" | "aborted";

export interface ApprovalRequest {
  id: string;
  task_id: string;
  issue_identifier: string;
  session_id: string;
  provider: ProviderKind;
  request_id: string;
  kind: string;
  command: string | null;
  cwd: string | null;
  file_paths: string[];
  reason: string | null;
  status: ApprovalStatus;
  created_at: string;
  resolved_at: string | null;
}

export type ApprovalDecision = "approve" | "approve_for_session" | "deny" | "abort";

export interface ValidationResult {
  id: string;
  task_id: string;
  attempt: number;
  command: string;
  exit_code: number;
  passed: boolean;
  stdout: string;
  stderr: string;
  duration_ms: number;
  executed_at: string;
}

export interface ReviewCommentEvaluation {
  author: string;
  is_agent: boolean;
  body: string;
  file_path: string | null;
  line: number | null;
  verified_against_diff: boolean;
  evaluation_summary: string;
}

export interface MergeReadinessReport {
  score: number;
  ci_passing: boolean;
  validation_passing: boolean;
  no_merge_conflicts: boolean;
  security_clean: boolean;
  reviewed_agent_comments: number;
  rationale: string[];
  warnings: string[];
}

export interface PullRequestReview {
  id: string;
  task_id: string;
  project_id: string;
  pr_number: number;
  title: string;
  description: string;
  url: string;
  head_branch: string;
  base_branch: string;
  changed_files: string[];
  diff_preview: string;
  ci_status: string;
  mergeable: boolean;
  merge_readiness: MergeReadinessReport;
  comment_evaluations: ReviewCommentEvaluation[];
  human_approved: boolean;
  created_at: string;
  updated_at: string;
}

export interface HardwareSnapshot {
  on_ac_power: boolean;
  battery_percent: number | null;
  cpu_usage_percent: number;
  total_memory_mb: number;
  available_memory_mb: number;
  memory_usage_percent: number;
  memory_pressure_high: boolean;
  thermal_throttled: boolean;
  recommended_max_concurrency: number;
  captured_at: string;
}

export interface ResourcePolicy {
  max_agents_ac: number;
  max_agents_battery: number;
  memory_pressure_threshold_percent: number;
  reduce_on_thermal_throttle: boolean;
}

export interface OrchestratorCounts {
  running: number;
  retrying: number;
  queued: number;
  blocked: number;
  completed: number;
  failed: number;
}

export interface OrchestratorStateSnapshot {
  generated_at: string;
  scheduler_paused: boolean;
  counts: OrchestratorCounts;
  running: LiveSession[];
  retrying: RetryEntry[];
  codex_totals: CodexTotals;
  rate_limits: Record<string, unknown> | null;
  hardware: HardwareSnapshot;
  recent_events: AgentEvent[];
}

export interface IssueDetailResponse {
  issue_identifier: string;
  issue_id: string;
  status: TaskStatus;
  claim_state: "unclaimed" | "claimed" | "released";
  workspace: {
    key: string;
    path: string | null;
    branch_name: string;
  };
  attempts: {
    restart_count: number;
    current_retry_attempt: number;
    history: RunAttempt[];
  };
  running: LiveSession | null;
  retry: RetryEntry | null;
  recent_events: AgentEvent[];
  validation_results: ValidationResult[];
  pull_request: PullRequestReview | null;
  last_error: string | null;
}

export interface WorkflowPreviewResponse {
  path: string;
  raw_content: string;
  valid: boolean;
  validation_error: string | null;
  rendered_prompt_preview: string | null;
  parsed_config: Record<string, unknown> | null;
}

export interface AppSettings {
  theme: "system" | "dark" | "light";
  language: "en" | "sv";
  close_action: "minimize_to_tray" | "exit_application";
  start_on_login: boolean;
  local_api_port: number;
  remote_api_enabled: boolean;
  remote_api_bind: string;
  resource_policy: ResourcePolicy;
  default_provider: ProviderKind;
  onboarding_completed: boolean;
}
