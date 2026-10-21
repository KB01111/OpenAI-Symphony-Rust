//! Process lifecycle of the one-shot CLI drivers (Claude Code, OpenCode), exercised with fake agent
//! executables: an abandoned (cancelled or stalled) turn must not leave the agent or the tools it
//! started running in the workspace, a chatty stderr must not block the agent, and the turn
//! timeout applies.
#![cfg(unix)]

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use symphony_agents::{start_agent_session, AgentLaunchConfig, AgentStreamEvent, AutoApprovalGate};
use symphony_core::domain::ProviderKind;
use symphony_core::error::SymphonyError;
use tokio::sync::mpsc;

fn fake_agent(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("fake-agent.sh");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn config(
    provider: ProviderKind,
    command: &Path,
    workspace: &Path,
    turn_timeout_ms: u64,
) -> AgentLaunchConfig {
    AgentLaunchConfig {
        provider,
        command: command.display().to_string(),
        model: None,
        workspace_path: workspace.to_path_buf(),
        task_id: "task-1".into(),
        issue_identifier: "ISSUE-1".into(),
        issue_title: "Lifecycle".into(),
        approval_policy: json!("never"),
        thread_sandbox: json!("workspace-write"),
        turn_sandbox_policy: None,
        read_timeout_ms: 5_000,
        turn_timeout_ms,
        extra_env: HashMap::new(),
        dynamic_tools: Vec::new(),
        require_human_approval_for_requests: false,
    }
}

/// Alive and not a zombie (a killed process stays a zombie until its parent reaps it).
fn running(pid: i32) -> bool {
    // SAFETY: signal 0 only checks for existence.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    !stat.is_empty() && !stat.contains(") Z ")
}

async fn read_pid(path: &Path) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(raw) = std::fs::read_to_string(path) {
            if let Ok(pid) = raw.trim().parse() {
                return pid;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the fake agent never started its tool"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn assert_terminated(pid: i32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while running(pid) {
        assert!(Instant::now() < deadline, "process {pid} is still running");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn abandoned_turn_kills_the_agent_tree(provider: ProviderKind) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("ws");
    std::fs::create_dir(&workspace).unwrap();
    let pid_file = dir.path().join("tool.pid");
    // The "agent" starts a long-running tool, reports progress and keeps working.
    let agent = fake_agent(
        dir.path(),
        &format!(
            "sleep 300 &\necho $! > '{}'\necho '{{\"type\":\"assistant\",\"session_id\":\"s-1\"}}'\nsleep 300",
            pid_file.display()
        ),
    );
    let (tx, mut rx) = mpsc::channel::<AgentStreamEvent>(64);
    let mut session = start_agent_session(
        config(provider, &agent, &workspace, 600_000),
        None,
        None,
        Arc::new(AutoApprovalGate),
        tx.clone(),
    )
    .await
    .unwrap();

    // Same shape as the orchestrator: the turn future is dropped when cancellation wins.
    let tool_pid = {
        let turn = session.run_turn("work", tx);
        tokio::pin!(turn);
        let tool_pid = tokio::select! {
            _ = &mut turn => panic!("the turn must still be running"),
            pid = read_pid(&pid_file) => pid,
        };
        // Wait until the agent produced output, so it is definitely mid-turn.
        let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        assert!(running(tool_pid));
        tool_pid
    };
    let agent_pid = session.pid().expect("agent pid") as i32;
    session.cancel().await.unwrap();

    assert_terminated(tool_pid).await;
    assert_terminated(agent_pid).await;
}

#[tokio::test]
async fn cancelled_claude_code_turn_kills_the_agent_and_its_tools() {
    abandoned_turn_kills_the_agent_tree(ProviderKind::ClaudeCode).await;
}

#[tokio::test]
async fn cancelled_opencode_turn_kills_the_agent_and_its_tools() {
    abandoned_turn_kills_the_agent_tree(ProviderKind::OpenCode).await;
}

#[tokio::test]
async fn a_chatty_stderr_does_not_block_the_agent() {
    let dir = tempfile::tempdir().unwrap();
    // ~2 MiB on stderr: far beyond a pipe buffer, so an undrained pipe would block the agent.
    let agent = fake_agent(
        dir.path(),
        "i=0; while [ $i -lt 2048 ]; do head -c 1023 /dev/zero | tr '\\0' x >&2; echo >&2; i=$((i+1)); done\necho '{\"type\":\"result\",\"result\":\"done\",\"session_id\":\"s-2\"}'",
    );
    let (tx, mut rx) = mpsc::channel::<AgentStreamEvent>(16);
    // Keep the event channel moving, like the orchestrator's event pump.
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let mut session = start_agent_session(
        config(ProviderKind::ClaudeCode, &agent, dir.path(), 60_000),
        None,
        None,
        Arc::new(AutoApprovalGate),
        tx.clone(),
    )
    .await
    .unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(30), session.run_turn("work", tx))
        .await
        .expect("the agent must not block on a full stderr pipe")
        .unwrap();
    assert!(outcome.succeeded);
    assert_eq!(outcome.assistant_messages, vec!["done".to_string()]);
}

#[tokio::test]
async fn the_turn_timeout_applies_to_cli_agents() {
    let dir = tempfile::tempdir().unwrap();
    let agent = fake_agent(dir.path(), "sleep 300");
    let (tx, _rx) = mpsc::channel::<AgentStreamEvent>(16);
    let mut session = start_agent_session(
        // The drivers enforce a 5 s floor.
        config(ProviderKind::OpenCode, &agent, dir.path(), 1),
        None,
        None,
        Arc::new(AutoApprovalGate),
        tx.clone(),
    )
    .await
    .unwrap();
    let started = Instant::now();
    let err = session.run_turn("work", tx).await.unwrap_err();
    assert!(matches!(err, SymphonyError::TurnTimeout(1)), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(20));
    let agent_pid = session.pid().expect("agent pid") as i32;
    assert_terminated(agent_pid).await;
}
