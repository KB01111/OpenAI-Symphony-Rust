//! Tool-call history: one record per tool call an agent made (a shell command, a file change, an
//! MCP tool, a host-side dynamic tool such as `github_api`, a provider's built-in tool), merged
//! from the start and finish reports each provider sends.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domain::ProviderKind;
use crate::redaction::{redact_json_strings, redact_secrets};

/// Longest tool output kept per call (bytes, cut at a character boundary). Longer output is cut
/// and flagged: the full text stays in the agent's own transcript and in the worktree.
pub const MAX_TOOL_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallKind {
    /// A host-side tool Symphony executes for the agent (`github_api`, `linear_graphql`).
    Dynamic,
    /// A tool of an MCP server the agent is connected to.
    Mcp,
    /// A shell command.
    Command,
    /// A file edit or patch.
    FileChange,
    /// Any other built-in tool of the provider (Claude Code's `Read`, OpenCode's `grep`, …).
    ProviderTool,
}

impl ToolCallKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dynamic => "dynamic",
            Self::Mcp => "mcp",
            Self::Command => "command",
            Self::FileChange => "file_change",
            Self::ProviderTool => "provider_tool",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "dynamic" => Self::Dynamic,
            "mcp" => Self::Mcp,
            "command" => Self::Command,
            "file_change" => Self::FileChange,
            _ => Self::ProviderTool,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Started,
    Succeeded,
    Failed,
}

impl ToolCallStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            _ => Self::Started,
        }
    }

    pub fn is_finished(self) -> bool {
        self != Self::Started
    }
}

/// What a provider reported about one tool call: its start, its result, or both at once.
/// Fields a report does not know are `None` (or an empty `tool_name`) and leave the stored
/// value alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallUpdate {
    /// The provider's id of the call (Codex item id or `callId`, Claude `tool_use` id, OpenCode
    /// `callID`); the start and the result of a call carry the same one.
    pub call_id: String,
    pub tool_name: String,
    pub kind: ToolCallKind,
    pub arguments: Option<serde_json::Value>,
    pub output: Option<String>,
    pub status: ToolCallStatus,
    /// Duration measured by the provider, when it reports one.
    pub duration_ms: Option<u64>,
}

/// One tool call of a task, as stored and served by `GET /api/v1/tasks/{id}/tool-calls`.
/// Arguments and output are redacted before they are stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ToolCallRecord {
    /// `"{task_id}:{call_id}"`, so the start and the result of a call land on one record.
    pub id: String,
    pub task_id: String,
    pub session_id: Option<String>,
    pub provider: ProviderKind,
    pub call_id: String,
    pub tool_name: String,
    pub kind: ToolCallKind,
    pub arguments: Option<serde_json::Value>,
    pub output: Option<String>,
    /// The output was longer than [`MAX_TOOL_OUTPUT_BYTES`] and was cut.
    pub output_truncated: bool,
    pub status: ToolCallStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
}

impl ToolCallRecord {
    pub fn record_id(task_id: &str, call_id: &str) -> String {
        format!("{task_id}:{call_id}")
    }

    /// Merge a report into the stored record (`None` for the first report of a call). A finished
    /// call never goes back to `started` (reports can arrive out of order), and what a report does
    /// not know keeps its stored value.
    pub fn apply(
        existing: Option<Self>,
        task_id: &str,
        session_id: Option<&str>,
        provider: ProviderKind,
        update: ToolCallUpdate,
        at: DateTime<Utc>,
    ) -> Self {
        let mut record = existing.unwrap_or_else(|| Self {
            id: Self::record_id(task_id, &update.call_id),
            task_id: task_id.to_string(),
            session_id: None,
            provider,
            call_id: update.call_id.clone(),
            tool_name: String::new(),
            kind: update.kind,
            arguments: None,
            output: None,
            output_truncated: false,
            status: ToolCallStatus::Started,
            started_at: at,
            finished_at: None,
            duration_ms: None,
        });
        if record.session_id.is_none() {
            record.session_id = session_id.map(str::to_string);
        }
        if !update.tool_name.trim().is_empty() {
            record.tool_name = update.tool_name;
            record.kind = update.kind;
        }
        if let Some(mut arguments) = update.arguments {
            redact_json_strings(&mut arguments);
            record.arguments = Some(arguments);
        }
        if let Some(output) = update.output {
            let (output, truncated) = truncate_output(&redact_secrets(&output, &[]));
            record.output = Some(output);
            record.output_truncated = truncated;
        }
        if update.status.is_finished() {
            record.status = update.status;
            let finished_at = record.finished_at.unwrap_or(at);
            record.finished_at = Some(finished_at);
            record.duration_ms = update.duration_ms.or(record.duration_ms).or_else(|| {
                let measured = (finished_at - record.started_at).num_milliseconds();
                (measured > 0).then_some(measured as u64)
            });
        } else if update.duration_ms.is_some() {
            record.duration_ms = update.duration_ms;
        }
        record
    }
}

fn truncate_output(output: &str) -> (String, bool) {
    if output.len() <= MAX_TOOL_OUTPUT_BYTES {
        return (output.to_string(), false);
    }
    let mut cut = MAX_TOOL_OUTPUT_BYTES;
    while !output.is_char_boundary(cut) {
        cut -= 1;
    }
    (output[..cut].to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn update(status: ToolCallStatus) -> ToolCallUpdate {
        ToolCallUpdate {
            call_id: "call-1".into(),
            tool_name: "github_api".into(),
            kind: ToolCallKind::Dynamic,
            arguments: Some(
                json!({ "path": "/repos/o/r/issues", "token": "ghp_abcdefghijklmnopqrstuvwxyz0123" }),
            ),
            output: None,
            status,
            duration_ms: None,
        }
    }

    #[test]
    fn start_and_result_merge_into_one_redacted_record() {
        let t0 = Utc::now();
        let started = ToolCallRecord::apply(
            None,
            "task-1",
            Some("thread-turn"),
            ProviderKind::Codex,
            update(ToolCallStatus::Started),
            t0,
        );
        assert_eq!(started.id, "task-1:call-1");
        assert_eq!(started.status, ToolCallStatus::Started);
        assert_eq!(started.arguments.as_ref().unwrap()["token"], "[REDACTED]");

        // The result knows neither the name nor the arguments.
        let result = ToolCallUpdate {
            tool_name: String::new(),
            arguments: None,
            output: Some("created ghp_abcdefghijklmnopqrstuvwxyz0123".into()),
            ..update(ToolCallStatus::Succeeded)
        };
        let done = ToolCallRecord::apply(
            Some(started),
            "task-1",
            None,
            ProviderKind::Codex,
            result,
            t0 + chrono::Duration::milliseconds(250),
        );
        assert_eq!(done.tool_name, "github_api");
        assert_eq!(done.session_id.as_deref(), Some("thread-turn"));
        assert!(done.arguments.is_some());
        assert_eq!(done.output.as_deref(), Some("created [REDACTED]"));
        assert_eq!(done.status, ToolCallStatus::Succeeded);
        assert_eq!(done.duration_ms, Some(250));

        // A late start report never reopens a finished call.
        let late = ToolCallRecord::apply(
            Some(done),
            "task-1",
            None,
            ProviderKind::Codex,
            update(ToolCallStatus::Started),
            t0 + chrono::Duration::seconds(1),
        );
        assert_eq!(late.status, ToolCallStatus::Succeeded);
        assert_eq!(late.duration_ms, Some(250));
    }

    #[test]
    fn long_output_is_cut_at_a_character_boundary_and_flagged() {
        let output = "é".repeat(MAX_TOOL_OUTPUT_BYTES);
        let record = ToolCallRecord::apply(
            None,
            "t",
            None,
            ProviderKind::ClaudeCode,
            ToolCallUpdate {
                output: Some(output),
                duration_ms: Some(7),
                ..update(ToolCallStatus::Failed)
            },
            Utc::now(),
        );
        let kept = record.output.unwrap();
        assert!(record.output_truncated);
        assert!(kept.len() <= MAX_TOOL_OUTPUT_BYTES);
        assert!(kept.chars().all(|c| c == 'é'));
        assert_eq!(record.status, ToolCallStatus::Failed);
        assert_eq!(record.duration_ms, Some(7));
    }

    #[test]
    fn kinds_and_statuses_round_trip_through_their_storage_names() {
        for kind in [
            ToolCallKind::Dynamic,
            ToolCallKind::Mcp,
            ToolCallKind::Command,
            ToolCallKind::FileChange,
            ToolCallKind::ProviderTool,
        ] {
            assert_eq!(ToolCallKind::parse(kind.as_str()), kind);
        }
        for status in [
            ToolCallStatus::Started,
            ToolCallStatus::Succeeded,
            ToolCallStatus::Failed,
        ] {
            assert_eq!(ToolCallStatus::parse(status.as_str()), status);
        }
    }
}
