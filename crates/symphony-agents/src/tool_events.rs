//! Tool calls in each provider's event stream, normalized to [`ToolCallUpdate`]s for the
//! tool-call history. Parsing never fails: a line that is not about a tool call yields nothing.

use serde_json::{json, Value};
use symphony_core::{ToolCallKind, ToolCallStatus, ToolCallUpdate};

fn str_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// Text of a result that may be a plain string, a list of content blocks (`{type: "text", text}`,
/// Codex `{type: "inputText", text}`) or any other JSON value.
fn content_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        Value::Array(blocks) => {
            let texts: Vec<&str> = blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect();
            if texts.is_empty() {
                Some(value.to_string())
            } else {
                Some(texts.join("\n"))
            }
        }
        Value::Object(map) => map
            .get("content")
            .and_then(content_text)
            .or_else(|| Some(value.to_string())),
        other => Some(other.to_string()),
    }
}

/// Codex `item/started` / `item/completed` for a command, a file change or an MCP tool call.
pub fn codex_item_tool_call(method: &str, params: &Value) -> Option<ToolCallUpdate> {
    let finished = match method {
        "item/started" => false,
        "item/completed" => true,
        _ => return None,
    };
    let item = params.get("item")?;
    let call_id = str_at(item, "id")?.to_string();
    let item_status = str_at(item, "status").unwrap_or_default();
    let mut status = if !finished {
        ToolCallStatus::Started
    } else if matches!(item_status, "failed" | "declined") {
        ToolCallStatus::Failed
    } else {
        ToolCallStatus::Succeeded
    };
    let duration_ms = item.get("durationMs").and_then(Value::as_u64);
    let update = match str_at(item, "type")? {
        "commandExecution" => {
            if finished
                && item
                    .get("exitCode")
                    .and_then(Value::as_i64)
                    .is_some_and(|c| c != 0)
            {
                status = ToolCallStatus::Failed;
            }
            ToolCallUpdate {
                call_id,
                tool_name: "shell".into(),
                kind: ToolCallKind::Command,
                arguments: Some(json!({
                    "command": item.get("command").cloned().unwrap_or(Value::Null),
                    "cwd": item.get("cwd").cloned().unwrap_or(Value::Null),
                })),
                output: finished
                    .then(|| str_at(item, "aggregatedOutput").map(str::to_string))
                    .flatten(),
                status,
                duration_ms,
            }
        }
        "fileChange" => {
            let paths: Vec<Value> = item
                .get("changes")
                .and_then(Value::as_array)
                .map(|changes| {
                    changes
                        .iter()
                        .filter_map(|c| c.get("path").cloned())
                        .collect()
                })
                .unwrap_or_default();
            ToolCallUpdate {
                call_id,
                tool_name: "apply_patch".into(),
                kind: ToolCallKind::FileChange,
                arguments: Some(json!({ "paths": paths })),
                output: None,
                status,
                duration_ms,
            }
        }
        "mcpToolCall" => {
            let error = item
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string);
            if finished && error.is_some() {
                status = ToolCallStatus::Failed;
            }
            let tool = str_at(item, "tool").unwrap_or("tool");
            let tool_name = match str_at(item, "server") {
                Some(server) => format!("{server}/{tool}"),
                None => tool.to_string(),
            };
            ToolCallUpdate {
                call_id,
                tool_name,
                kind: ToolCallKind::Mcp,
                arguments: item.get("arguments").cloned(),
                output: if finished {
                    error.or_else(|| item.get("result").and_then(content_text))
                } else {
                    None
                },
                status,
                duration_ms,
            }
        }
        _ => return None,
    };
    Some(update)
}

/// The call id of a Codex `item/tool/call` request, if it carries one.
pub fn codex_dynamic_call_id(params: &Value) -> Option<String> {
    str_at(params, "callId")
        .or_else(|| str_at(params, "call_id"))
        .map(str::to_string)
}

/// A host-side dynamic tool call (`item/tool/call`) and the result Symphony answered with.
pub fn codex_dynamic_tool_result(
    call_id: String,
    tool_name: &str,
    arguments: &Value,
    result: &Value,
    duration_ms: u64,
) -> ToolCallUpdate {
    let succeeded = result
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    ToolCallUpdate {
        call_id,
        tool_name: tool_name.to_string(),
        kind: ToolCallKind::Dynamic,
        arguments: Some(arguments.clone()),
        output: result
            .get("output")
            .and_then(content_text)
            .or_else(|| result.get("contentItems").and_then(content_text)),
        status: if succeeded {
            ToolCallStatus::Succeeded
        } else {
            ToolCallStatus::Failed
        },
        duration_ms: Some(duration_ms),
    }
}

fn claude_tool_kind(name: &str) -> ToolCallKind {
    match name {
        "Bash" => ToolCallKind::Command,
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => ToolCallKind::FileChange,
        name if name.starts_with("mcp__") => ToolCallKind::Mcp,
        _ => ToolCallKind::ProviderTool,
    }
}

/// Claude Code stream-json: `tool_use` blocks of an `assistant` line start calls, `tool_result`
/// blocks of a `user` line finish them.
pub fn claude_tool_calls(line: &Value) -> Vec<ToolCallUpdate> {
    let line_type = str_at(line, "type").unwrap_or_default();
    if !matches!(line_type, "assistant" | "user") {
        return Vec::new();
    }
    let Some(blocks) = line.pointer("/message/content").and_then(Value::as_array) else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter_map(|block| match (line_type, str_at(block, "type")?) {
            ("assistant", "tool_use") => {
                let name = str_at(block, "name").unwrap_or_default();
                Some(ToolCallUpdate {
                    call_id: str_at(block, "id")?.to_string(),
                    tool_name: name.to_string(),
                    kind: claude_tool_kind(name),
                    arguments: block.get("input").cloned(),
                    output: None,
                    status: ToolCallStatus::Started,
                    duration_ms: None,
                })
            }
            ("user", "tool_result") => Some(ToolCallUpdate {
                call_id: str_at(block, "tool_use_id")?.to_string(),
                // The result does not repeat the name; the stored start keeps it.
                tool_name: String::new(),
                kind: ToolCallKind::ProviderTool,
                arguments: None,
                output: block.get("content").and_then(content_text),
                status: if block.get("is_error").and_then(Value::as_bool) == Some(true) {
                    ToolCallStatus::Failed
                } else {
                    ToolCallStatus::Succeeded
                },
                duration_ms: None,
            }),
            _ => None,
        })
        .collect()
}

fn opencode_tool_kind(name: &str) -> ToolCallKind {
    match name {
        "bash" => ToolCallKind::Command,
        "edit" | "write" | "patch" | "multiedit" => ToolCallKind::FileChange,
        _ => ToolCallKind::ProviderTool,
    }
}

/// An OpenCode `tool` message part (`opencode run --format json` and `opencode serve` events).
pub fn opencode_tool_call(part: &Value) -> Option<ToolCallUpdate> {
    if str_at(part, "type") != Some("tool") {
        return None;
    }
    let call_id = str_at(part, "callID")
        .or_else(|| str_at(part, "id"))?
        .to_string();
    let name = str_at(part, "tool").unwrap_or_default();
    let state = part.get("state").cloned().unwrap_or(Value::Null);
    let status = match str_at(&state, "status") {
        Some("completed") => ToolCallStatus::Succeeded,
        Some("error") => ToolCallStatus::Failed,
        _ => ToolCallStatus::Started,
    };
    let duration_ms = match (
        state.pointer("/time/start").and_then(Value::as_u64),
        state.pointer("/time/end").and_then(Value::as_u64),
    ) {
        (Some(start), Some(end)) if end >= start => Some(end - start),
        _ => None,
    };
    Some(ToolCallUpdate {
        call_id,
        tool_name: name.to_string(),
        kind: opencode_tool_kind(name),
        arguments: state.get("input").cloned(),
        output: match status {
            ToolCallStatus::Succeeded => str_at(&state, "output").map(str::to_string),
            ToolCallStatus::Failed => str_at(&state, "error").map(str::to_string),
            ToolCallStatus::Started => None,
        },
        status,
        duration_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_items_become_start_and_finish_reports() {
        let started = json!({ "item": { "type": "commandExecution", "id": "item-1",
            "command": "cargo test", "cwd": "/ws", "status": "inProgress" } });
        let update = codex_item_tool_call("item/started", &started).unwrap();
        assert_eq!(update.status, ToolCallStatus::Started);
        assert_eq!(update.kind, ToolCallKind::Command);
        assert_eq!(update.output, None);

        let failed = json!({ "item": { "type": "commandExecution", "id": "item-1",
            "command": "cargo test", "status": "completed", "exitCode": 101,
            "aggregatedOutput": "1 failed", "durationMs": 900 } });
        let update = codex_item_tool_call("item/completed", &failed).unwrap();
        assert_eq!(update.status, ToolCallStatus::Failed);
        assert_eq!(update.output.as_deref(), Some("1 failed"));
        assert_eq!(update.duration_ms, Some(900));

        let mcp = json!({ "item": { "type": "mcpToolCall", "id": "item-2", "server": "docs",
            "tool": "search", "arguments": { "q": "x" }, "status": "completed",
            "result": { "content": [{ "type": "text", "text": "found" }] } } });
        let update = codex_item_tool_call("item/completed", &mcp).unwrap();
        assert_eq!(update.tool_name, "docs/search");
        assert_eq!(update.output.as_deref(), Some("found"));
        assert_eq!(update.status, ToolCallStatus::Succeeded);

        let patch = json!({ "item": { "type": "fileChange", "id": "item-3", "status": "declined",
            "changes": [{ "path": "src/a.rs", "kind": "update" }] } });
        let update = codex_item_tool_call("item/completed", &patch).unwrap();
        assert_eq!(update.status, ToolCallStatus::Failed);
        assert_eq!(update.arguments, Some(json!({ "paths": ["src/a.rs"] })));

        let message = json!({ "item": { "type": "agentMessage", "id": "item-4", "text": "hi" } });
        assert!(codex_item_tool_call("item/completed", &message).is_none());
        assert!(codex_item_tool_call("item/agentMessage/delta", &started).is_none());
    }

    #[test]
    fn a_dynamic_tool_result_records_what_symphony_answered() {
        let result = json!({ "success": false, "output": "{\"error\":\"refused\"}",
            "contentItems": [{ "type": "inputText", "text": "{\"error\":\"refused\"}" }] });
        let update = codex_dynamic_tool_result(
            "call-9".into(),
            "github_api",
            &json!({ "method": "PUT" }),
            &result,
            42,
        );
        assert_eq!(update.kind, ToolCallKind::Dynamic);
        assert_eq!(update.status, ToolCallStatus::Failed);
        assert_eq!(update.output.as_deref(), Some("{\"error\":\"refused\"}"));
        assert_eq!(update.duration_ms, Some(42));
        assert_eq!(
            codex_dynamic_call_id(&json!({ "callId": "c1" })).as_deref(),
            Some("c1")
        );
    }

    #[test]
    fn claude_tool_use_and_tool_result_blocks_are_read() {
        let assistant = json!({ "type": "assistant", "message": { "content": [
            { "type": "text", "text": "Running tests" },
            { "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": { "command": "ls" } },
            { "type": "tool_use", "id": "toolu_2", "name": "mcp__docs__search", "input": {} }
        ] } });
        let calls = claude_tool_calls(&assistant);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].kind, ToolCallKind::Command);
        assert_eq!(calls[1].kind, ToolCallKind::Mcp);

        let user = json!({ "type": "user", "message": { "content": [
            { "type": "tool_result", "tool_use_id": "toolu_1", "is_error": true,
              "content": [{ "type": "text", "text": "permission denied" }] }
        ] } });
        let calls = claude_tool_calls(&user);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].call_id, "toolu_1");
        assert_eq!(calls[0].status, ToolCallStatus::Failed);
        assert_eq!(calls[0].output.as_deref(), Some("permission denied"));
        assert!(calls[0].tool_name.is_empty());

        assert!(claude_tool_calls(&json!({ "type": "result", "result": "done" })).is_empty());
    }

    #[test]
    fn opencode_tool_parts_are_read_with_their_duration() {
        let part = json!({ "type": "tool", "callID": "call_1", "tool": "bash",
            "state": { "status": "completed", "input": { "command": "ls" }, "output": "a\nb",
                       "time": { "start": 1000, "end": 1250 } } });
        let update = opencode_tool_call(&part).unwrap();
        assert_eq!(update.kind, ToolCallKind::Command);
        assert_eq!(update.status, ToolCallStatus::Succeeded);
        assert_eq!(update.output.as_deref(), Some("a\nb"));
        assert_eq!(update.duration_ms, Some(250));

        let failed = json!({ "type": "tool", "callID": "call_2", "tool": "edit",
            "state": { "status": "error", "error": "file not found" } });
        let update = opencode_tool_call(&failed).unwrap();
        assert_eq!(update.kind, ToolCallKind::FileChange);
        assert_eq!(update.status, ToolCallStatus::Failed);
        assert_eq!(update.output.as_deref(), Some("file not found"));

        assert!(opencode_tool_call(&json!({ "type": "text", "text": "hi" })).is_none());
    }
}
