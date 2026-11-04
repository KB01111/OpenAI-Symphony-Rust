//! Claude Code's host permission protocol (`--permission-prompt-tool stdio`), exercised with a
//! scripted `claude`: each `can_use_tool` request goes through Symphony's approval gate and the
//! decision is answered on stdin. The message shapes match what claude 2.1.310 sends and accepts
//! (checked against the real CLI with a scripted Messages API, see docs/PRODUCTION_GAP_AUDIT.md, AG-04).
#![cfg(unix)]

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use symphony_agents::{start_agent_session, AgentLaunchConfig, AgentStreamEvent, ApprovalGate};
use symphony_core::domain::{ApprovalDecision, ApprovalRequest, ProviderKind};
use symphony_core::error::Result;
use tokio::sync::mpsc;

/// Written by `cp` so a concurrently forking test cannot hold it open for writing (ETXTBSY).
fn fake_claude(dir: &Path) -> PathBuf {
    let source = dir.join("claude.src");
    // Records its arguments and the user message, asks for one Bash command, records the answer,
    // then reports the result and waits for the end of its input like the real CLI.
    std::fs::write(
        &source,
        r#"#!/bin/sh
printf '%s\n' "$@" > args.txt
IFS= read -r user; printf '%s\n' "$user" > user.txt
echo '{"type":"system","subtype":"init","session_id":"sess-1"}'
echo '{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"cargo test","description":"Run the tests"},"description":"Run the tests","permission_suggestions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"cargo test *"}],"behavior":"allow","destination":"localSettings"},{"type":"addDirectories","directories":["/elsewhere"],"destination":"session"}]}}'
IFS= read -r answer; printf '%s\n' "$answer" >> answers.txt
echo '{"type":"control_request","request_id":"req-2","request":{"subtype":"mcp_message","server_name":"x"}}'
IFS= read -r answer; printf '%s\n' "$answer" >> answers.txt
echo '{"type":"result","subtype":"success","result":"done","session_id":"sess-1","usage":{"input_tokens":3,"output_tokens":2}}'
cat > /dev/null
"#,
    )
    .unwrap();
    let path = dir.join("claude");
    assert!(std::process::Command::new("cp")
        .arg(&source)
        .arg(&path)
        .status()
        .unwrap()
        .success());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn config(command: &Path, workspace: &Path, policy: Value) -> AgentLaunchConfig {
    AgentLaunchConfig {
        provider: ProviderKind::ClaudeCode,
        command: command.display().to_string(),
        model: None,
        workspace_path: workspace.to_path_buf(),
        task_id: "task-1".into(),
        issue_identifier: "SYM-1".into(),
        issue_title: "Permissions".into(),
        approval_policy: policy,
        thread_sandbox: json!("workspace-write"),
        turn_sandbox_policy: None,
        read_timeout_ms: 5_000,
        turn_timeout_ms: 60_000,
        extra_env: HashMap::new(),
        dynamic_tools: Vec::new(),
        require_human_approval_for_requests: false,
        resume_usage_baseline: None,
        cloud: None,
    }
}

struct RecordingGate {
    decision: ApprovalDecision,
    seen: Mutex<Vec<ApprovalRequest>>,
}

#[async_trait]
impl ApprovalGate for RecordingGate {
    async fn request_approval(&self, request: ApprovalRequest) -> Result<ApprovalDecision> {
        self.seen.lock().unwrap().push(request);
        Ok(self.decision)
    }
}

struct Run {
    dir: tempfile::TempDir,
    gate: Arc<RecordingGate>,
    outcomes: Vec<symphony_agents::TurnExecutionOutcome>,
}

impl Run {
    fn answers(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.path().join("answers.txt"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

async fn run(decision: ApprovalDecision, policy: Value, turns: usize) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let claude = fake_claude(dir.path());
    let gate = Arc::new(RecordingGate {
        decision,
        seen: Mutex::new(Vec::new()),
    });
    let (tx, mut rx) = mpsc::channel::<AgentStreamEvent>(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let mut session = start_agent_session(
        config(&claude, dir.path(), policy),
        None,
        None,
        gate.clone(),
        tx.clone(),
    )
    .await
    .unwrap();
    let mut outcomes = Vec::new();
    for _ in 0..turns {
        outcomes.push(
            tokio::time::timeout(
                std::time::Duration::from_secs(30),
                session.run_turn("Fix the bug", tx.clone()),
            )
            .await
            .expect("the turn ends once the result is reported")
            .unwrap(),
        );
    }
    Run {
        dir,
        gate,
        outcomes,
    }
}

#[tokio::test]
async fn a_tool_use_is_decided_by_the_approval_gate_and_answered_on_stdin() {
    let r = run(ApprovalDecision::Accept, json!("on-request"), 1).await;
    let args = std::fs::read_to_string(r.dir.path().join("args.txt")).unwrap();
    for flag in [
        "--input-format",
        "stream-json",
        "--permission-prompt-tool",
        "stdio",
    ] {
        assert!(args.lines().any(|a| a == flag), "{args}");
    }
    // The prompt travels as a stream-json user message, not as an argument.
    let user: Value =
        serde_json::from_str(&std::fs::read_to_string(r.dir.path().join("user.txt")).unwrap())
            .unwrap();
    assert_eq!(
        user,
        json!({ "type": "user", "message": { "role": "user", "content": "Fix the bug" } })
    );
    assert!(!args.contains("Fix the bug"));

    let seen = r.gate.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].provider, ProviderKind::ClaudeCode);
    assert_eq!(seen[0].kind, "command_execution");
    assert_eq!(seen[0].command_or_diff.as_deref(), Some("cargo test"));
    assert_eq!(seen[0].description, "Run the tests");
    assert_eq!(seen[0].task_id, "task-1");

    let answers = r.answers();
    assert_eq!(
        answers[0],
        json!({ "type": "control_response", "response": { "subtype": "success", "request_id": "req-1",
            "response": { "behavior": "allow", "updatedInput": { "command": "cargo test", "description": "Run the tests" } } } })
    );
    // Requests Symphony does not handle are refused, never left unanswered.
    assert_eq!(answers[1]["response"]["subtype"], "error");
    assert_eq!(answers[1]["response"]["request_id"], "req-2");
    assert!(r.outcomes[0].succeeded);
}

#[tokio::test]
async fn a_declined_tool_use_is_denied() {
    let r = run(ApprovalDecision::Decline, json!("on-request"), 1).await;
    let result = &r.answers()[0]["response"]["response"];
    assert_eq!(result["behavior"], "deny");
    assert!(result.get("interrupt").is_none());
    assert!(!r.outcomes[0].cancelled);
}

#[tokio::test]
async fn aborting_interrupts_claude_and_cancels_the_turn() {
    let r = run(ApprovalDecision::Cancel, json!("on-request"), 1).await;
    let result = &r.answers()[0]["response"]["response"];
    assert_eq!(result["behavior"], "deny");
    assert_eq!(result["interrupt"], true);
    assert!(r.outcomes[0].cancelled && !r.outcomes[0].succeeded);
}

#[tokio::test]
async fn approval_for_the_session_is_remembered_and_never_written_to_settings() {
    let r = run(ApprovalDecision::AcceptForSession, json!("on-request"), 2).await;
    // Asked once; the identical request in the next turn is allowed without asking.
    assert_eq!(r.gate.seen.lock().unwrap().len(), 1);
    let answers = r.answers();
    let first = &answers[0]["response"]["response"];
    assert_eq!(first["behavior"], "allow");
    // Claude Code's own rule suggestion is kept, but only for this session; a suggestion that is
    // not a rule (another directory) is not passed on.
    assert_eq!(
        first["updatedPermissions"],
        json!([{ "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "cargo test *" }], "behavior": "allow", "destination": "session" }])
    );
    assert_eq!(answers[2]["response"]["response"]["behavior"], "allow");
}

/// Claude Code has no sandbox: with nobody to ask, what its own rules did not allow stays denied.
#[tokio::test]
async fn a_never_asking_policy_denies_without_the_gate() {
    let r = run(ApprovalDecision::Accept, json!("never"), 1).await;
    assert!(r.gate.seen.lock().unwrap().is_empty());
    let result = &r.answers()[0]["response"]["response"];
    assert_eq!(result["behavior"], "deny");
    assert!(result["message"].as_str().unwrap().contains("never"));
}
