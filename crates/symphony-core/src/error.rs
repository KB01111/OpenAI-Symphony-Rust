use thiserror::Error;

#[derive(Debug, Error)]
pub enum SymphonyError {
    #[error("missing_workflow_file: {0}")]
    MissingWorkflowFile(String),

    #[error("workflow_parse_error: {0}")]
    WorkflowParseError(String),

    #[error("workflow_front_matter_not_a_map")]
    WorkflowFrontMatterNotAMap,

    #[error("template_parse_error: {0}")]
    TemplateParseError(String),

    #[error("template_render_error: {0}")]
    TemplateRenderError(String),

    #[error("invalid_config: {0}")]
    InvalidConfig(String),

    #[error("invalid_workspace_cwd: {0}")]
    InvalidWorkspaceCwd(String),

    #[error("workspace_equals_root: {0}")]
    WorkspaceEqualsRoot(String),

    #[error("workspace_outside_root: workspace={workspace}, root={root}")]
    WorkspaceOutsideRoot { workspace: String, root: String },

    #[error("symlink_escape: workspace={workspace}, root={root}")]
    SymlinkEscape { workspace: String, root: String },

    #[error("git_worktree_error: {0}")]
    GitWorktreeError(String),

    #[error("hook_failed: hook={hook}, reason={reason}")]
    HookFailed { hook: String, reason: String },

    #[error("hook_timeout: hook={hook}, timeout_ms={timeout_ms}")]
    HookTimeout { hook: String, timeout_ms: u64 },

    #[error("invalid_state_transition: cannot transition from {from} to {to}")]
    InvalidStateTransition { from: String, to: String },

    #[error("issue_not_found: {0}")]
    IssueNotFound(String),

    #[error("project_not_found: {0}")]
    ProjectNotFound(String),

    #[error("task_not_found: {0}")]
    TaskNotFound(String),

    #[error("tracker_error: {code}: {message}")]
    TrackerError { code: String, message: String },

    #[error("codex_not_found: {0}")]
    CodexNotFound(String),

    #[error("agent_protocol_error: {code}: {message}")]
    AgentProtocolError { code: String, message: String },

    #[error("turn_failed: {0}")]
    TurnFailed(String),

    #[error("turn_cancelled: {0}")]
    TurnCancelled(String),

    #[error("turn_timeout: exceeded {0} ms")]
    TurnTimeout(u64),

    #[error("stall_timeout: exceeded {0} ms")]
    StallTimeout(u64),

    #[error("unsupported_provider_capability: provider={provider}, capability={capability}")]
    UnsupportedProviderCapability {
        provider: String,
        capability: String,
    },

    #[error("not_found: {0}")]
    NotFound(String),

    #[error("json_error: {0}")]
    JsonError(String),

    #[error("storage_error: {0}")]
    StorageError(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("io_error: {0}")]
    IoError(String),

    #[error("conflict: {0}")]
    Conflict(String),
}

impl SymphonyError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingWorkflowFile(_) => "missing_workflow_file",
            Self::WorkflowParseError(_) => "workflow_parse_error",
            Self::WorkflowFrontMatterNotAMap => "workflow_front_matter_not_a_map",
            Self::TemplateParseError(_) => "template_parse_error",
            Self::TemplateRenderError(_) => "template_render_error",
            Self::InvalidConfig(_) => "invalid_config",
            Self::InvalidWorkspaceCwd(_) => "invalid_workspace_cwd",
            Self::WorkspaceEqualsRoot(_) => "workspace_equals_root",
            Self::WorkspaceOutsideRoot { .. } => "workspace_outside_root",
            Self::SymlinkEscape { .. } => "symlink_escape",
            Self::GitWorktreeError(_) => "git_worktree_error",
            Self::HookFailed { .. } => "hook_failed",
            Self::HookTimeout { .. } => "hook_timeout",
            Self::InvalidStateTransition { .. } => "invalid_state_transition",
            Self::IssueNotFound(_) => "issue_not_found",
            Self::ProjectNotFound(_) => "project_not_found",
            Self::TaskNotFound(_) => "task_not_found",
            Self::TrackerError { .. } => "tracker_error",
            Self::CodexNotFound(_) => "codex_not_found",
            Self::AgentProtocolError { .. } => "agent_protocol_error",
            Self::TurnFailed(_) => "turn_failed",
            Self::TurnCancelled(_) => "turn_cancelled",
            Self::TurnTimeout(_) => "turn_timeout",
            Self::StallTimeout(_) => "stall_timeout",
            Self::UnsupportedProviderCapability { .. } => "unsupported_provider_capability",
            Self::NotFound(_) => "not_found",
            Self::JsonError(_) => "json_error",
            Self::StorageError(_) => "storage_error",
            Self::Unauthorized(_) => "unauthorized",
            Self::IoError(_) => "io_error",
            Self::Conflict(_) => "conflict",
        }
    }
}

impl From<std::io::Error> for SymphonyError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err.to_string())
    }
}

impl From<serde_json::Error> for SymphonyError {
    fn from(err: serde_json::Error) -> Self {
        Self::JsonError(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, SymphonyError>;
pub type SymphonyResult<T> = std::result::Result<T, SymphonyError>;
