use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::{SymphonyError, SymphonyResult};

/// Blocker reference for an Issue (SPEC.md Section 4.1.1)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct BlockerRef {
    pub id: Option<String>,
    pub identifier: Option<String>,
    pub state: Option<String>,
}

/// Normalized Issue record used across orchestration, prompting, and observability (SPEC.md Section 4.1.1)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Issue {
    pub id: String,
    #[serde(default)]
    pub native_ref: Option<serde_json::Value>,
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<i32>,
    pub state: String,
    pub branch_name: Option<String>,
    pub url: Option<String>,
    pub assignee_id: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub blocked_by: Vec<BlockerRef>,
    #[serde(default = "default_true")]
    pub dispatchable: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

fn default_true() -> bool {
    true
}

impl Issue {
    /// Normalize priority for sorting per SPEC.md Section 8.2:
    /// Priority 1..4 ranks ascending before all other integers or null.
    pub fn sort_priority_rank(&self) -> u8 {
        match self.priority {
            Some(p) if (1..=4).contains(&p) => p as u8,
            _ => 5,
        }
    }

    /// Sort comparator per SPEC.md Section 8.2:
    /// 1. priority ascending (1..4 first, null/other last)
    /// 2. created_at oldest first (null last)
    /// 3. identifier lexicographic tie-breaker
    pub fn cmp_dispatch_order(&self, other: &Self) -> Ordering {
        self.sort_priority_rank()
            .cmp(&other.sort_priority_rank())
            .then_with(|| match (&self.created_at, &other.created_at) {
                (Some(a), Some(b)) => a.cmp(b),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            })
            .then_with(|| self.identifier.cmp(&other.identifier))
    }

    pub fn normalized_state(&self) -> String {
        self.state.trim().to_lowercase()
    }

    pub fn is_in_states(&self, states: &[String]) -> bool {
        let norm = self.normalized_state();
        states.iter().any(|s| s.trim().to_lowercase() == norm)
    }

    pub fn is_todo_blocked(&self, terminal_states: &[String]) -> bool {
        if self.normalized_state() != "todo" || self.blocked_by.is_empty() {
            return false;
        }
        self.blocked_by.iter().any(|b| {
            let bs = b.state.as_deref().unwrap_or("").trim().to_lowercase();
            !terminal_states
                .iter()
                .any(|ts| ts.trim().to_lowercase() == bs)
        })
    }

    pub fn has_required_labels(&self, required_labels: &[String]) -> bool {
        if required_labels.is_empty() {
            return true;
        }
        let issue_labels: Vec<String> = self
            .labels
            .iter()
            .map(|l| l.trim().to_lowercase())
            .filter(|l| !l.is_empty())
            .collect();

        required_labels.iter().all(|req| {
            let norm = req.trim().to_lowercase();
            if norm.is_empty() {
                false
            } else {
                issue_labels.contains(&norm)
            }
        })
    }
}

pub fn sort_candidate_issues(issues: &mut [Issue]) {
    issues.sort_by(|a, b| a.cmp_dispatch_order(b));
}

/// Orchestrator internal claim states (SPEC.md Section 4.1.8 & 7.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationClaimState {
    Unclaimed,
    Claimed,
    Released,
}

/// Run attempt lifecycle phases (SPEC.md Section 7.2)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunAttemptPhase {
    PreparingWorkspace,
    BuildingPrompt,
    LaunchingAgentProcess,
    InitializingSession,
    StreamingTurn,
    Finishing,
    Succeeded,
    Failed,
    TimedOut,
    Stalled,
    CanceledByReconciliation,
}

impl std::fmt::Display for RunAttemptPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::PreparingWorkspace => "preparing_workspace",
            Self::BuildingPrompt => "building_prompt",
            Self::LaunchingAgentProcess => "launching_agent_process",
            Self::InitializingSession => "initializing_session",
            Self::StreamingTurn => "streaming_turn",
            Self::Finishing => "finishing",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Stalled => "stalled",
            Self::CanceledByReconciliation => "canceled_by_reconciliation",
        };
        write!(f, "{s}")
    }
}

/// Symphony Desktop Kanban & Orchestration Task Statuses (Section 4.C & 5)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Backlog,
    Ready,
    Queued,
    Running,
    Blocked,
    HumanReview,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Ready => "ready",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::HumanReview => "human_review",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_tracker_state(state: &str) -> Self {
        match state.trim().to_lowercase().as_str() {
            "backlog" | "triage" | "icebox" => Self::Backlog,
            "todo" | "ready" | "open" => Self::Ready,
            "queued" => Self::Queued,
            "in progress" | "in_progress" | "running" | "started" => Self::Running,
            "blocked" => Self::Blocked,
            "in review" | "in_review" | "human_review" | "review" => Self::HumanReview,
            "done" | "closed" | "completed" | "merged" => Self::Completed,
            "failed" | "error" => Self::Failed,
            "cancelled" | "canceled" | "duplicate" => Self::Cancelled,
            _ => Self::Ready,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }

    pub fn is_dispatch_candidate(&self) -> bool {
        matches!(self, Self::Ready | Self::Queued)
    }

    /// Validated state machine transitions
    pub fn allowed_transitions(&self) -> &'static [TaskStatus] {
        use TaskStatus::*;
        match self {
            Backlog => &[Backlog, Ready, Queued, Blocked, Cancelled],
            Ready => &[Ready, Backlog, Queued, Running, Blocked, Cancelled],
            Queued => &[Queued, Ready, Running, Blocked, Failed, Cancelled],
            Running => &[
                Running,
                Queued,
                Blocked,
                HumanReview,
                Completed,
                Failed,
                Cancelled,
            ],
            Blocked => &[Blocked, Backlog, Ready, Queued, Cancelled],
            HumanReview => &[
                HumanReview,
                Ready,
                Queued,
                Running,
                Completed,
                Failed,
                Cancelled,
            ],
            Completed => &[Completed, Ready, Queued],
            Failed => &[Failed, Ready, Queued, Running, Cancelled],
            Cancelled => &[Cancelled, Backlog, Ready, Queued],
        }
    }

    pub fn can_transition_to(&self, next: TaskStatus) -> bool {
        self.allowed_transitions().contains(&next)
    }

    pub fn validate_transition(&self, next: TaskStatus) -> SymphonyResult<()> {
        if self.can_transition_to(next) {
            Ok(())
        } else {
            Err(SymphonyError::InvalidStateTransition {
                from: self.as_str().to_string(),
                to: next.as_str().to_string(),
            })
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = SymphonyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "backlog" => Ok(Self::Backlog),
            "ready" => Ok(Self::Ready),
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "blocked" => Ok(Self::Blocked),
            "human_review" | "human review" | "in_review" => Ok(Self::HumanReview),
            "completed" | "done" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" | "canceled" => Ok(Self::Cancelled),
            other => Err(SymphonyError::InvalidConfig(format!(
                "unknown task status: {other}"
            ))),
        }
    }
}

/// Multi-provider coding agent kinds
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Codex,
    ClaudeCode,
    /// Serialized as `opencode` to match `ProviderKind::as_str` and persisted SQLite values;
    /// `open_code` is still accepted for payloads written by earlier builds.
    #[serde(rename = "opencode", alias = "open_code")]
    OpenCode,
    Simulated,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
            Self::OpenCode => "opencode",
            Self::Simulated => "simulated",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Codex => "OpenAI Codex",
            Self::ClaudeCode => "Claude Code",
            Self::OpenCode => "OpenCode",
            Self::Simulated => "Deterministic Test Agent",
        }
    }

    pub fn default_capabilities(&self) -> ProviderCapabilities {
        match self {
            Self::Codex => ProviderCapabilities {
                provider: Self::Codex,
                available: false,
                binary_path: None,
                version: None,
                supports_session_resume: true,
                supports_mid_turn_pause: false,
                supports_cancel: true,
                supports_streaming_events: true,
                supports_approvals: true,
                supports_dynamic_tools: true,
                supports_token_usage: true,
                models: vec![
                    "gpt-5.3-codex".to_string(),
                    "gpt-5.2-codex".to_string(),
                    "gpt-5-codex".to_string(),
                    "o4-mini".to_string(),
                ],
                status_message: "OpenAI Codex App Server (JSON-RPC 2.0 over stdio)".to_string(),
            },
            Self::ClaudeCode => ProviderCapabilities {
                provider: Self::ClaudeCode,
                available: false,
                binary_path: None,
                version: None,
                supports_session_resume: true,
                supports_mid_turn_pause: false,
                supports_cancel: true,
                supports_streaming_events: true,
                supports_approvals: false,
                supports_dynamic_tools: false,
                supports_token_usage: true,
                models: vec![
                    "claude-sonnet-4-5".to_string(),
                    "claude-opus-4-1".to_string(),
                ],
                status_message: "Claude Code CLI (`claude -p --output-format stream-json`)"
                    .to_string(),
            },
            Self::OpenCode => ProviderCapabilities {
                provider: Self::OpenCode,
                available: false,
                binary_path: None,
                version: None,
                supports_session_resume: true,
                supports_mid_turn_pause: false,
                supports_cancel: true,
                supports_streaming_events: true,
                supports_approvals: false,
                supports_dynamic_tools: false,
                supports_token_usage: true,
                models: vec![
                    "openai/gpt-5-codex".to_string(),
                    "anthropic/claude-sonnet-4-5".to_string(),
                ],
                status_message: "OpenCode CLI (`opencode run --format json`)".to_string(),
            },
            Self::Simulated => ProviderCapabilities {
                provider: Self::Simulated,
                available: true,
                binary_path: Some("builtin://simulated-agent".to_string()),
                version: Some("1.0.0".to_string()),
                supports_session_resume: true,
                supports_mid_turn_pause: false,
                supports_cancel: true,
                supports_streaming_events: true,
                supports_approvals: true,
                supports_dynamic_tools: true,
                supports_token_usage: true,
                models: vec!["simulated-deterministic-v1".to_string()],
                status_message: "Built-in deterministic test provider".to_string(),
            },
        }
    }
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ProviderKind {
    type Err = SymphonyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "codex" | "openai_codex" | "openai" => Ok(Self::Codex),
            "claude_code" | "claude-code" | "claude" => Ok(Self::ClaudeCode),
            "opencode" | "open_code" => Ok(Self::OpenCode),
            "simulated" | "fake" | "test" => Ok(Self::Simulated),
            other => Err(SymphonyError::InvalidConfig(format!(
                "unknown provider kind: {other}"
            ))),
        }
    }
}

/// Typed capabilities exposed by an agent provider (Section 6)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProviderCapabilities {
    pub provider: ProviderKind,
    pub available: bool,
    pub binary_path: Option<String>,
    pub version: Option<String>,
    pub supports_session_resume: bool,
    pub supports_mid_turn_pause: bool,
    pub supports_cancel: bool,
    pub supports_streaming_events: bool,
    pub supports_approvals: bool,
    pub supports_dynamic_tools: bool,
    pub supports_token_usage: bool,
    pub models: Vec<String>,
    pub status_message: String,
}

/// Non-secret provider profile configuration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProviderProfile {
    pub id: String,
    pub name: String,
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub command_override: Option<String>,
    pub approval_policy: String,
    pub sandbox_mode: String,
    pub credential_ref: Option<String>,
    pub is_default: bool,
}

/// Project entity representing a connected local + tracker repository
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub repo_path: String,
    pub default_branch: String,
    pub workspace_root: String,
    pub workflow_path: Option<String>,
    pub preferred_provider: ProviderKind,
    pub preferred_model: Option<String>,
    pub tracker_kind: String,
    pub github_repo: Option<String>,
    pub linear_project_slug: Option<String>,
    pub required_labels: Vec<String>,
    pub auto_dispatch: bool,
    pub max_concurrent_agents: u32,
    pub validation_commands: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Task record persisted in SQLite and orchestrated by Symphony
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub issue: Issue,
    pub status: TaskStatus,
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub workspace_path: Option<String>,
    pub branch_name: Option<String>,
    pub attempt_count: u32,
    pub current_session_id: Option<String>,
    pub current_thread_id: Option<String>,
    pub current_turn_id: Option<String>,
    pub current_phase: Option<RunAttemptPhase>,
    pub pr_number: Option<u64>,
    pub pr_url: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub estimated_cost_usd: f64,
    pub duration_seconds: f64,
    pub last_error: Option<String>,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl Task {
    #[allow(clippy::too_many_arguments)]
    pub fn new_manual(
        project_id: String,
        identifier: String,
        title: String,
        description: Option<String>,
        priority: Option<i32>,
        provider: ProviderKind,
        model: Option<String>,
        labels: Vec<String>,
    ) -> Self {
        let now = Utc::now();
        let id = Uuid::new_v4().to_string();
        let issue = Issue {
            id: id.clone(),
            native_ref: None,
            identifier,
            title,
            description,
            priority,
            state: "Todo".to_string(),
            branch_name: None,
            url: None,
            assignee_id: None,
            labels,
            blocked_by: Vec::new(),
            dispatchable: true,
            created_at: Some(now),
            updated_at: Some(now),
        };
        Self {
            id,
            project_id,
            issue,
            status: TaskStatus::Ready,
            provider,
            model,
            workspace_path: None,
            branch_name: None,
            attempt_count: 0,
            current_session_id: None,
            current_thread_id: None,
            current_turn_id: None,
            current_phase: None,
            pr_number: None,
            pr_url: None,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            duration_seconds: 0.0,
            last_error: None,
            next_retry_at: None,
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
        }
    }

    pub fn transition_to(&mut self, next: TaskStatus) -> SymphonyResult<()> {
        self.status.validate_transition(next)?;
        self.status = next;
        self.updated_at = Utc::now();
        if next.is_terminal() {
            self.completed_at = Some(self.updated_at);
            self.current_session_id = None;
        }
        Ok(())
    }
}

/// Run attempt record (SPEC.md Section 4.1.5)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RunAttempt {
    pub id: String,
    pub task_id: String,
    pub issue_id: String,
    pub issue_identifier: String,
    pub attempt_number: u32,
    pub provider: ProviderKind,
    pub workspace_path: String,
    pub branch_name: String,
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub turn_count: u32,
    pub phase: RunAttemptPhase,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

/// Live session metadata tracked while an agent subprocess is active (SPEC.md Section 4.1.6)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct LiveSession {
    pub session_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub task_id: String,
    pub issue_id: String,
    pub issue_identifier: String,
    pub issue_title: String,
    pub project_id: String,
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub workspace_path: String,
    pub branch_name: String,
    pub pid: Option<u32>,
    pub phase: RunAttemptPhase,
    pub attempt_number: u32,
    pub turn_count: u32,
    pub started_at: DateTime<Utc>,
    pub last_event: Option<String>,
    pub last_message: Option<String>,
    pub last_event_at: Option<DateTime<Utc>>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

/// Retry Entry in orchestrator state (SPEC.md Section 4.1.7)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RetryEntry {
    pub task_id: String,
    pub issue_id: String,
    pub identifier: String,
    pub attempt: u32,
    pub due_at: DateTime<Utc>,
    pub error: Option<String>,
}

/// Token usage report
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

impl TokenUsage {
    pub fn add(&mut self, other: &Self) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
    }

    pub fn estimate_cost_usd(&self, model: Option<&str>) -> f64 {
        // Approximate per-1M token pricing for estimation
        let (in_rate, out_rate) = match model.unwrap_or("gpt-5-codex") {
            m if m.contains("mini") => (0.40, 1.60),
            m if m.contains("claude") => (3.00, 15.00),
            _ => (1.50, 6.00),
        };
        (self.input_tokens as f64 / 1_000_000.0) * in_rate
            + (self.output_tokens as f64 / 1_000_000.0) * out_rate
    }
}

/// Structured agent/orchestrator event emitted in real time and stored in SQLite
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AgentEvent {
    pub id: String,
    pub task_id: Option<String>,
    pub issue_identifier: Option<String>,
    pub session_id: Option<String>,
    pub provider: Option<ProviderKind>,
    pub event_type: String,
    pub phase: Option<RunAttemptPhase>,
    pub message: String,
    pub payload: Option<serde_json::Value>,
    pub timestamp: DateTime<Utc>,
}

/// Approval request raised by an agent session (e.g. command execution, file patch, PR creation, merge)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ApprovalRequest {
    pub id: String,
    pub task_id: String,
    pub issue_identifier: String,
    pub session_id: String,
    pub provider: ProviderKind,
    pub kind: String,
    pub title: String,
    pub description: String,
    pub command_or_diff: Option<String>,
    pub status: ApprovalStatus,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub decision: Option<ApprovalDecision>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Accept,
    AcceptForSession,
    Decline,
    Cancel,
}

/// Validation command execution record (Section 10)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ValidationResult {
    pub id: String,
    pub task_id: String,
    pub attempt_number: u32,
    pub command: String,
    pub exit_code: i32,
    pub passed: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub executed_at: DateTime<Utc>,
}

/// Structured Pull Request Review & Merge-Readiness Rating (1 to 5 backed by evidence, Section 4.F)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PullRequestReview {
    pub id: String,
    pub task_id: String,
    pub project_id: String,
    pub pr_number: u64,
    pub title: String,
    pub description: String,
    pub url: String,
    pub branch_name: String,
    pub base_branch: String,
    pub state: String,
    pub ci_status: String,
    pub mergeable: bool,
    pub has_conflicts: bool,
    pub changed_files: Vec<ChangedFileSummary>,
    pub diff_preview: String,
    pub validation_results: Vec<ValidationResult>,
    pub review_comments: Vec<ReviewCommentEvaluation>,
    pub security_warnings: Vec<String>,
    pub merge_readiness: MergeReadinessReport,
    pub human_approved: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ChangedFileSummary {
    pub path: String,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ReviewCommentEvaluation {
    pub author: String,
    pub is_agent: bool,
    pub body: String,
    pub file_path: Option<String>,
    pub verified_against_diff: bool,
    pub evaluation_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MergeReadinessReport {
    /// Structured rating from 1 (blocked/critical issues) to 5 (ready to merge with full verification)
    pub score: u8,
    pub label: String,
    pub evidence: Vec<String>,
    pub blockers: Vec<String>,
}

impl MergeReadinessReport {
    pub fn evaluate(
        has_conflicts: bool,
        ci_status: &str,
        validations: &[ValidationResult],
        security_warnings: &[String],
        changed_files_count: usize,
    ) -> Self {
        let mut evidence = Vec::new();
        let mut blockers = Vec::new();
        let mut score: i32 = 5;

        if has_conflicts {
            score -= 3;
            blockers.push("Branch has unresolved Git merge conflicts.".to_string());
        } else {
            evidence.push("Clean Git merge base without conflicts.".to_string());
        }

        if !security_warnings.is_empty() {
            score -= 2;
            for w in security_warnings {
                blockers.push(format!("Security warning: {w}"));
            }
        } else {
            evidence.push("Zero security warnings or secret leaks detected in diff.".to_string());
        }

        if validations.is_empty() {
            score -= 1;
            blockers.push("No local validation commands have been executed yet.".to_string());
        } else {
            let passed_count = validations.iter().filter(|v| v.passed).count();
            if passed_count == validations.len() {
                evidence.push(format!(
                    "All {passed_count}/{} validation commands passed.",
                    validations.len()
                ));
            } else {
                score -= 2;
                blockers.push(format!(
                    "{}/{} validation commands failed.",
                    validations.len() - passed_count,
                    validations.len()
                ));
            }
        }

        match ci_status.to_lowercase().as_str() {
            "success" | "passed" => {
                evidence.push("Remote CI checks passed.".to_string());
            }
            "failure" | "failed" => {
                score -= 2;
                blockers.push("Remote CI checks reported failure.".to_string());
            }
            "pending" | "running" => {
                score -= 1;
                evidence.push("Remote CI checks are still in progress.".to_string());
            }
            _ => {
                evidence.push(format!("CI status: {ci_status}."));
            }
        }

        if changed_files_count == 0 {
            score -= 2;
            blockers.push("No file changes detected in branch.".to_string());
        } else {
            evidence.push(format!(
                "{changed_files_count} file(s) modified in worktree."
            ));
        }

        let clamped = score.clamp(1, 5) as u8;
        let label = match clamped {
            5 => "Ready for Human Approval & Merge".to_string(),
            4 => "Strong Candidate (Minor Pending Items)".to_string(),
            3 => "Needs Review Attention".to_string(),
            2 => "Not Ready (Validation or CI Issues)".to_string(),
            _ => "Blocked (Conflicts or Critical Failures)".to_string(),
        };

        Self {
            score: clamped,
            label,
            evidence,
            blockers,
        }
    }
}

/// Hardware & power telemetry snapshot (Section 13: ASUS Zephyrus G16 / Windows 11 aware)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct HardwareSnapshot {
    pub cpu_brand: String,
    pub cpu_usage_percent: f32,
    pub total_memory_mb: u64,
    pub used_memory_mb: u64,
    pub available_memory_mb: u64,
    pub memory_usage_percent: f32,
    pub on_battery: bool,
    pub battery_percent: Option<u8>,
    pub power_mode: String,
    pub thermal_throttling: bool,
    pub recommended_max_concurrency: u32,
    pub effective_max_concurrency: u32,
    pub dispatch_delayed_reason: Option<String>,
    pub captured_at: DateTime<Utc>,
}

/// Configurable resource-aware scheduling policy (Section 13)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct ResourcePolicy {
    pub max_concurrent_on_ac: u32,
    pub max_concurrent_on_battery: u32,
    pub high_memory_threshold_percent: f32,
    pub high_cpu_threshold_percent: f32,
    pub reduce_on_thermal_throttle: bool,
}

impl Default for ResourcePolicy {
    fn default() -> Self {
        Self {
            max_concurrent_on_ac: 2,
            max_concurrent_on_battery: 1,
            high_memory_threshold_percent: 88.0,
            high_cpu_threshold_percent: 92.0,
            reduce_on_thermal_throttle: true,
        }
    }
}

/// SPEC.md Section 13.7.2 `/api/v1/state` response contract + Symphony Desktop extensions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct OrchestratorStateSnapshot {
    pub generated_at: DateTime<Utc>,
    pub scheduler_paused: bool,
    pub counts: StateCounts,
    pub running: Vec<RunningSessionSummary>,
    pub retrying: Vec<RetryEntry>,
    pub codex_totals: CodexTotals,
    pub rate_limits: Option<serde_json::Value>,
    pub hardware: HardwareSnapshot,
    pub active_workflow_path: Option<String>,
    pub workflow_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct StateCounts {
    pub running: usize,
    pub retrying: usize,
    pub queued: usize,
    pub blocked: usize,
    pub human_review: usize,
    pub completed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RunningSessionSummary {
    pub issue_id: String,
    pub issue_identifier: String,
    pub task_id: String,
    pub project_id: String,
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub state: String,
    pub phase: RunAttemptPhase,
    pub session_id: String,
    pub workspace_path: String,
    pub turn_count: u32,
    pub last_event: Option<String>,
    pub last_message: String,
    pub started_at: DateTime<Utc>,
    pub last_event_at: Option<DateTime<Utc>>,
    pub tokens: TokenUsage,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CodexTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub seconds_running: f64,
    pub estimated_cost_usd: f64,
}

/// Allowed values of [`AppSettings::theme`]. Documents the API contract only; the setting is
/// stored as a string and validated by the API layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    System,
    Dark,
    Light,
}

/// Allowed values of [`AppSettings::locale`]. Documents the API contract only; the setting is
/// stored as a string and validated by the API layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LocalePreference {
    En,
    Sv,
}

/// Application settings persisted in SQLite (non-secret).
///
/// `#[serde(default)]` keeps previously persisted JSON readable when fields are added:
/// missing keys fall back to `AppSettings::default()` instead of failing deserialization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct AppSettings {
    /// `system` | `dark` | `light` (validated by the API layer).
    #[schema(value_type = ThemePreference)]
    pub theme: String,
    /// `en` | `sv` (validated by the API layer).
    #[schema(value_type = LocalePreference)]
    pub locale: String,
    pub minimize_to_tray_on_close: bool,
    pub start_background_runtime_on_login: bool,
    pub default_provider: ProviderKind,
    pub default_model: String,
    pub polling_interval_ms: u64,
    pub resource_policy: ResourcePolicy,
    pub remote_api_enabled: bool,
    pub remote_bind_address: String,
    pub remote_port: u16,
    pub onboarding_completed: bool,
    pub extra_env_vars: HashMap<String, String>,
    /// Per-provider default model preference (`ProviderKind` -> model id).
    pub provider_default_models: HashMap<ProviderKind, String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: "system".to_string(),
            locale: "en".to_string(),
            minimize_to_tray_on_close: true,
            start_background_runtime_on_login: false,
            default_provider: ProviderKind::Codex,
            default_model: "gpt-5-codex".to_string(),
            polling_interval_ms: 30_000,
            resource_policy: ResourcePolicy::default(),
            remote_api_enabled: false,
            remote_bind_address: "127.0.0.1".to_string(),
            remote_port: 47821,
            onboarding_completed: false,
            extra_env_vars: HashMap::new(),
            provider_default_models: HashMap::new(),
        }
    }
}

/// Authentication state of a provider CLI as observed by the runtime.
///
/// Detection only inspects the presence of credential files or environment variables; it never
/// reads or reports secret values. `Unknown` is used when credentials may live in an OS keychain
/// that the runtime does not probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthState {
    Authenticated,
    Unauthenticated,
    Unknown,
    NotRequired,
}

/// Runtime-detected status of a coding agent provider (`GET /api/v1/providers`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ProviderStatus {
    pub provider: ProviderKind,
    pub display_name: String,
    pub installed: bool,
    pub binary_path: Option<String>,
    pub version: Option<String>,
    pub auth_state: ProviderAuthState,
    /// Non-secret description of the credential source (e.g. "OPENAI_API_KEY environment variable").
    pub auth_detail: Option<String>,
    pub default_model: Option<String>,
    pub capabilities: ProviderCapabilities,
    pub detected_at: DateTime<Utc>,
}

/// Git worktree allocated for a task (`GET /api/v1/worktrees`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct WorktreeInfo {
    pub task_id: String,
    pub project_id: String,
    pub project_name: Option<String>,
    pub repo_path: Option<String>,
    pub issue_identifier: String,
    pub issue_title: String,
    pub task_status: TaskStatus,
    pub provider: ProviderKind,
    pub workspace_path: String,
    pub branch_name: Option<String>,
    pub exists: bool,
    pub dirty: bool,
    pub active_session_id: Option<String>,
    pub last_activity_at: DateTime<Utc>,
}

/// Changed files and unified diff for a task workspace (`GET /api/v1/tasks/{id}/diff`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TaskDiff {
    pub task_id: String,
    pub workspace_path: Option<String>,
    pub branch_name: Option<String>,
    pub exists: bool,
    pub changed_files: Vec<ChangedFileSummary>,
    pub diff: String,
    pub truncated: bool,
}

/// Result of validating a local repository before project registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RepositoryValidation {
    pub repo_path: String,
    pub exists: bool,
    pub is_directory: bool,
    pub is_git_repository: bool,
    pub current_branch: Option<String>,
    pub has_workflow_file: bool,
    pub suggested_name: Option<String>,
    pub errors: Vec<String>,
}

/// Runtime identity used by clients to discover capabilities (`GET /api/v1/runtime`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RuntimeInfo {
    pub service: String,
    pub version: String,
    pub api_version: u32,
    pub auth_required: bool,
    pub started_at: DateTime<Utc>,
    pub event_stream: String,
}
