/** Typed fixtures matching the generated Rust contracts. */
import type {
  AppSettings,
  ApprovalRequest,
  OrchestratorStateSnapshot,
  Project,
  ProviderStatus,
  PullRequestReview,
  Task,
  WorktreeInfo
} from "@symphony/shared-types";

const now = "2026-10-20T10:00:00Z";

export function project(overrides: Partial<Project> = {}): Project {
  return {
    id: "p1",
    name: "symphony",
    repo_path: "C:\\src\\symphony",
    default_branch: "main",
    workspace_root: "C:\\ws",
    workflow_path: null,
    preferred_provider: "simulated",
    preferred_model: null,
    tracker_kind: "manual",
    github_repo: null,
    linear_project_slug: null,
    required_labels: ["symphony"],
    auto_dispatch: false,
    max_concurrent_agents: 2,
    validation_commands: [],
    created_at: now,
    updated_at: now,
    ...overrides
  };
}

export function task(overrides: Partial<Task> & { identifier?: string; title?: string } = {}): Task {
  const { identifier = "SYM-1", title = "Add retry backoff", ...rest } = overrides;
  return {
    id: "t1",
    project_id: "p1",
    issue: {
      id: "i1",
      native_ref: null,
      identifier,
      title,
      description: "Implement it",
      priority: 2,
      state: "Todo",
      branch_name: null,
      url: null,
      assignee_id: null,
      labels: [],
      blocked_by: [],
      dispatchable: true,
      created_at: now,
      updated_at: now
    },
    status: "ready",
    provider: "simulated",
    model: null,
    workspace_path: null,
    branch_name: null,
    attempt_count: 0,
    current_session_id: null,
    current_thread_id: null,
    current_turn_id: null,
    current_phase: null,
    pr_number: null,
    pr_url: null,
    input_tokens: 0,
    output_tokens: 0,
    total_tokens: 0,
    estimated_cost_usd: 0,
    thread_usage: null,
    usage_estimated: false,
    duration_seconds: 0,
    last_error: null,
    next_retry_at: null,
    created_at: now,
    updated_at: now,
    started_at: null,
    completed_at: null,
    ...rest
  };
}

export function snapshot(overrides: Partial<OrchestratorStateSnapshot> = {}): OrchestratorStateSnapshot {
  return {
    generated_at: now,
    scheduler_paused: false,
    counts: { running: 0, retrying: 0, queued: 0, blocked: 0, human_review: 0, completed: 0, failed: 0 },
    running: [],
    retrying: [],
    codex_totals: { input_tokens: 0, output_tokens: 0, total_tokens: 0, seconds_running: 0, estimated_cost_usd: 0 },
    rate_limits: null,
    hardware: {
      cpu_brand: "Test CPU",
      cpu_usage_percent: 12,
      total_memory_mb: 32768,
      used_memory_mb: 8192,
      available_memory_mb: 24576,
      memory_usage_percent: 25,
      on_battery: false,
      battery_percent: null,
      power_mode: "ac",
      thermal_throttling: false,
      recommended_max_concurrency: 2,
      effective_max_concurrency: 2,
      dispatch_delayed_reason: null,
      captured_at: now
    },
    active_workflow_path: null,
    workflow_error: null,
    ...overrides
  };
}

export function settings(overrides: Partial<AppSettings> = {}): AppSettings {
  return {
    theme: "dark",
    locale: "en",
    minimize_to_tray_on_close: true,
    start_background_runtime_on_login: false,
    default_provider: "codex",
    default_model: "gpt-5-codex",
    polling_interval_ms: 30000,
    resource_policy: {
      max_concurrent_on_ac: 2,
      max_concurrent_on_battery: 1,
      high_memory_threshold_percent: 88,
      high_cpu_threshold_percent: 92,
      reduce_on_thermal_throttle: true
    },
    remote_api_enabled: false,
    remote_bind_address: "127.0.0.1",
    remote_port: 47821,
    onboarding_completed: true,
    extra_env_vars: {},
    provider_default_models: {},
    ...overrides
  };
}

export function approval(overrides: Partial<ApprovalRequest> = {}): ApprovalRequest {
  return {
    id: "a1",
    task_id: "t1",
    issue_identifier: "SYM-1",
    session_id: "s1",
    provider: "simulated",
    kind: "command_execution",
    title: "Run validation command",
    description: "Run tests",
    command_or_diff: "cargo test --workspace",
    status: "pending",
    created_at: now,
    resolved_at: null,
    decision: null,
    ...overrides
  };
}

export function providerStatus(overrides: Partial<ProviderStatus> = {}): ProviderStatus {
  return {
    provider: "simulated",
    display_name: "Deterministic Test Agent",
    installed: true,
    binary_path: "builtin://simulated-agent",
    version: "1.0.0",
    auth_state: "not_required",
    auth_detail: null,
    default_model: "simulated-deterministic-v1",
    capabilities: {
      provider: "simulated",
      available: true,
      binary_path: null,
      version: null,
      supports_session_resume: true,
      supports_mid_turn_pause: false,
      supports_cancel: true,
      supports_streaming_events: true,
      supports_approvals: true,
      supports_dynamic_tools: true,
      supports_token_usage: true,
      models: ["simulated-deterministic-v1"],
      status_message: "Built-in deterministic test provider"
    },
    detected_at: now,
    ...overrides
  };
}

export function pullRequest(overrides: Partial<PullRequestReview> = {}): PullRequestReview {
  return {
    id: "pr-t1",
    task_id: "t1",
    project_id: "p1",
    pr_number: 1,
    title: "SYM-1: Add retry backoff",
    description: "Implemented",
    url: "local://worktree/symphony/sym-1",
    branch_name: "symphony/sym-1",
    base_branch: "main",
    state: "open",
    ci_status: "passed",
    mergeable: true,
    has_conflicts: false,
    changed_files: [{ path: "SYMPHONY_TASK_NOTE.md", status: "added", additions: 4, deletions: 0 }],
    diff_preview: "diff --git a/SYMPHONY_TASK_NOTE.md b/SYMPHONY_TASK_NOTE.md\n+++ b/SYMPHONY_TASK_NOTE.md\n@@ -0,0 +1 @@\n+# Task",
    validation_results: [],
    review_comments: [],
    security_warnings: [],
    merge_readiness: { score: 4, label: "Strong Candidate", evidence: ["Clean merge"], blockers: [] },
    reviewed_commit: null,
    unpushed_changes: false,
    human_approved: false,
    updated_at: now,
    ...overrides
  };
}

export function worktree(overrides: Partial<WorktreeInfo> = {}): WorktreeInfo {
  return {
    task_id: "t1",
    project_id: "p1",
    project_name: "symphony",
    repo_path: "C:\\src\\symphony",
    issue_identifier: "SYM-1",
    issue_title: "Add retry backoff",
    task_status: "human_review",
    provider: "simulated",
    workspace_path: "C:\\ws\\SYM-1",
    branch_name: "symphony/sym-1",
    exists: true,
    dirty: true,
    active_session_id: null,
    last_activity_at: now,
    ...overrides
  };
}
