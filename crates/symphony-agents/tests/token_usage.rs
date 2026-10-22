//! Token usage reported by the drivers: stream events carry the cumulative usage of the agent
//! session (so a repeated report can never be counted twice), turn outcomes the usage of that
//! turn alone (Codex reports absolute thread totals that also cover the earlier turns).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::json;
use symphony_agents::{start_agent_session, AgentLaunchConfig, AgentStreamEvent, AutoApprovalGate};
use symphony_core::domain::{ProviderKind, TokenUsage};
use tokio::sync::mpsc;

fn config(provider: ProviderKind, command: String, workspace: &Path) -> AgentLaunchConfig {
    AgentLaunchConfig {
        provider,
        command,
        model: None,
        workspace_path: workspace.to_path_buf(),
        task_id: "task-1".into(),
        issue_identifier: "ISSUE-1".into(),
        issue_title: "Token usage".into(),
        approval_policy: json!("never"),
        thread_sandbox: json!("workspace-write"),
        turn_sandbox_policy: None,
        // A cold `node` start can take seconds on a loaded runner.
        read_timeout_ms: 30_000,
        turn_timeout_ms: 30_000,
        extra_env: HashMap::new(),
        dynamic_tools: Vec::new(),
        require_human_approval_for_requests: false,
        resume_usage_baseline: None,
    }
}

fn usage(input: u64, output: u64) -> TokenUsage {
    TokenUsage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: input + output,
    }
}

fn reported_usage(rx: &mut mpsc::Receiver<AgentStreamEvent>) -> Vec<TokenUsage> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.extend(ev.usage);
    }
    out
}

fn assert_cumulative(reports: &[TokenUsage], last: TokenUsage) {
    assert!(
        reports
            .windows(2)
            .all(|w| w[0].since(&w[1]) == TokenUsage::default()),
        "stream usage must never decrease: {reports:?}"
    );
    assert_eq!(reports.last().copied(), Some(last), "{reports:?}");
}

#[tokio::test]
async fn codex_thread_totals_become_per_turn_outcomes() {
    let dir = tempfile::tempdir().unwrap();
    // Thread totals: turn 1 ends at 300 + 200, turn 2 at 500 + 300; reports repeat and the
    // second turn's first report arrives before the `turn/start` response.
    let script = dir.path().join("mock_codex.js");
    std::fs::write(
        &script,
        r#"
const readline = require('readline');
const rl = readline.createInterface({ input: process.stdin, terminal: false });
const send = (msg) => process.stdout.write(JSON.stringify(msg) + '\n');
const total = (i, o) => send({ method: 'thread/tokenUsage/updated', params: { tokenUsage: {
  total: { inputTokens: i, outputTokens: o, totalTokens: i + o },
  last: { inputTokens: 1, outputTokens: 1, totalTokens: 2 } } } });
let turn = 0;
rl.on('line', (line) => {
  if (!line.trim()) return;
  const msg = JSON.parse(line);
  if (msg.method === 'initialize') {
    send({ id: msg.id, result: { serverInfo: { name: 'codex-app-server' } } });
  } else if (msg.method === 'thread/start') {
    send({ id: msg.id, result: { thread: { id: 'thread_1' } } });
  } else if (msg.method === 'turn/start') {
    turn += 1;
    if (turn === 1) {
      send({ id: msg.id, result: { turn: { id: 'turn_1' } } });
      total(100, 50); total(300, 200); total(300, 200);
    } else {
      total(400, 250);
      send({ id: msg.id, result: { turn: { id: 'turn_2' } } });
      total(500, 300); total(500, 300);
    }
    send({ method: 'turn/completed', params: { turnId: 'turn_' + turn } });
  }
});
"#,
    )
    .unwrap();
    let (tx, mut rx) = mpsc::channel::<AgentStreamEvent>(256);
    let mut session = start_agent_session(
        config(
            ProviderKind::Codex,
            format!("node \"{}\"", script.display()),
            dir.path(),
        ),
        None,
        None,
        Arc::new(AutoApprovalGate),
        tx.clone(),
    )
    .await
    .unwrap();

    let first = session.run_turn("one", tx.clone()).await.unwrap();
    let second = session.run_turn("two", tx).await.unwrap();
    let _ = session.stop().await;

    assert!(first.succeeded && second.succeeded);
    assert_eq!(first.usage, usage(300, 200));
    assert_eq!(second.usage, usage(200, 100));
    assert_cumulative(&reported_usage(&mut rx), usage(500, 300));
}

#[cfg(unix)]
#[tokio::test]
async fn claude_code_runs_add_up_to_session_usage() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    // Every `claude -p` run reports only its own usage, on its final `result` line.
    let source = dir.path().join("fake-claude.src");
    std::fs::write(
        &source,
        "#!/bin/sh\necho '{\"type\":\"system\",\"session_id\":\"s-1\"}'\necho '{\"type\":\"result\",\"result\":\"done\",\"session_id\":\"s-1\",\"usage\":{\"input_tokens\":100,\"output_tokens\":50}}'\n",
    )
    .unwrap();
    // Copied by a separate process, see `cli_process_lifecycle.rs` (ETXTBSY).
    let agent = dir.path().join("fake-claude.sh");
    assert!(std::process::Command::new("cp")
        .arg(&source)
        .arg(&agent)
        .status()
        .unwrap()
        .success());
    std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).unwrap();

    let (tx, mut rx) = mpsc::channel::<AgentStreamEvent>(64);
    let mut session = start_agent_session(
        config(
            ProviderKind::ClaudeCode,
            agent.display().to_string(),
            dir.path(),
        ),
        None,
        None,
        Arc::new(AutoApprovalGate),
        tx.clone(),
    )
    .await
    .unwrap();

    let first = session.run_turn("one", tx.clone()).await.unwrap();
    let second = session.run_turn("two", tx).await.unwrap();

    assert_eq!(first.usage, usage(100, 50));
    assert_eq!(second.usage, usage(100, 50));
    assert_cumulative(&reported_usage(&mut rx), usage(200, 100));
}

#[tokio::test]
async fn simulated_turns_report_session_usage() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::channel::<AgentStreamEvent>(256);
    let mut session = start_agent_session(
        config(ProviderKind::Simulated, String::new(), dir.path()),
        None,
        None,
        Arc::new(AutoApprovalGate),
        tx.clone(),
    )
    .await
    .unwrap();

    let first = session.run_turn("one", tx.clone()).await.unwrap();
    let second = session.run_turn("two", tx).await.unwrap();

    assert_eq!(first.usage, usage(450, 210));
    assert_eq!(second.usage, usage(450, 210));
    // Intermediate reports of the second turn include the first turn as well.
    assert_cumulative(&reported_usage(&mut rx), first.usage.plus(&second.usage));
}

#[tokio::test]
async fn a_failed_simulated_turn_streams_its_usage() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::channel::<AgentStreamEvent>(64);
    let mut session = start_agent_session(
        config(ProviderKind::Simulated, String::new(), dir.path()),
        None,
        None,
        Arc::new(AutoApprovalGate),
        tx.clone(),
    )
    .await
    .unwrap();

    let outcome = session.run_turn("SIMULATE_FAILURE", tx).await.unwrap();

    assert!(!outcome.succeeded);
    assert_eq!(outcome.usage, usage(120, 40));
    // Runtime totals are credited from stream events, so the failed turn's usage must be in one.
    assert_cumulative(&reported_usage(&mut rx), usage(120, 40));
}
