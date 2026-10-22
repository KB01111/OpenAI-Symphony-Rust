//! Token accounting of Codex threads that are resumed by a later attempt.
//!
//! `tokenUsage.total` of `thread/tokenUsage/updated` is the **absolute** usage of the whole thread.
//! Measured against `codex app-server` 0.163.0 (see `scripts/codex-resume-usage-probe.mjs`),
//! `thread/resume` replays the persisted total as a `thread/tokenUsage/updated` notification that
//! carries the id of the thread's previous turn, before any new turn runs. These tests drive the
//! real `CodexAppServerSession` against a scripted app-server that reproduces that behaviour: they
//! exercise Symphony's accounting, not Codex itself.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::json;
use symphony_agents::{
    start_agent_session, AgentLaunchConfig, AgentStreamEvent, AutoApprovalGate,
    TurnExecutionOutcome, UsageSnapshot,
};
use symphony_core::domain::{ProviderKind, TokenUsage};
use tokio::sync::mpsc;

const MOCK_CODEX: &str = r#"
const readline = require('readline');
const env = process.env;
const rl = readline.createInterface({ input: process.stdin, terminal: false });
const send = (msg) => process.stdout.write(JSON.stringify(msg) + '\n');
// "input:output" pairs, comma separated.
const pairs = (v) => (v || '').split(',').filter(Boolean).map((p) => p.split(':').map(Number));
const usageNote = (threadId, turnId, [i, o]) => send({ method: 'thread/tokenUsage/updated', params: {
  threadId, turnId, tokenUsage: {
    total: { inputTokens: i, outputTokens: o, totalTokens: i + o, cachedInputTokens: 0, reasoningOutputTokens: 0 },
    last: { inputTokens: 1, outputTokens: 1, totalTokens: 2, cachedInputTokens: 0, reasoningOutputTokens: 0 },
    modelContextWindow: 200000 } } });
let thread = 'thread_1';
rl.on('line', (line) => {
  if (!line.trim()) return;
  const msg = JSON.parse(line);
  if (msg.method === 'initialize') {
    send({ id: msg.id, result: { userAgent: 'mock-codex' } });
  } else if (msg.method === 'thread/start' || msg.method === 'thread/resume') {
    const resuming = msg.method === 'thread/resume';
    thread = env.MOCK_THREAD_ID || (resuming ? msg.params.threadId : 'thread_1');
    const replay = pairs(env.MOCK_REPLAY);
    const respond = () => send({ id: msg.id, result: { thread: { id: thread } } });
    const replayAll = () => replay.forEach((p) => usageNote(thread, env.MOCK_REPLAY_TURN || 'turn_prev', p));
    if (env.MOCK_REPLAY_AFTER_RESPONSE) { respond(); replayAll(); } else { replayAll(); respond(); }
  } else if (msg.method === 'turn/start') {
    send({ id: msg.id, result: { turn: { id: 'turn_new' } } });
    pairs(env.MOCK_TURN_REPORTS).forEach((p) => usageNote(thread, 'turn_new', p));
    const end = env.MOCK_END || 'completed';
    if (end === 'die') { process.exit(3); }
    send({ method: 'turn/' + end, params: { turnId: 'turn_new', error: { message: 'boom' } } });
  }
});
"#;

fn usage(input: u64, output: u64) -> TokenUsage {
    TokenUsage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: input + output,
    }
}

struct Run {
    outcome: Result<TurnExecutionOutcome, String>,
    snapshot: UsageSnapshot,
    reported: Vec<TokenUsage>,
}

struct Scenario<'a> {
    dir: &'a Path,
    env: Vec<(&'static str, &'static str)>,
    resume_thread: Option<&'static str>,
    baseline: Option<TokenUsage>,
}

async fn run(scenario: Scenario<'_>) -> Run {
    let script = scenario.dir.join("mock_codex.js");
    std::fs::write(&script, MOCK_CODEX).unwrap();
    let config = AgentLaunchConfig {
        provider: ProviderKind::Codex,
        command: format!("node \"{}\"", script.display()),
        model: None,
        workspace_path: scenario.dir.to_path_buf(),
        task_id: "task-1".into(),
        issue_identifier: "ISSUE-1".into(),
        issue_title: "Resumed usage".into(),
        approval_policy: json!("never"),
        thread_sandbox: json!("workspace-write"),
        turn_sandbox_policy: None,
        // A cold `node` start can take seconds on a loaded runner.
        read_timeout_ms: 30_000,
        turn_timeout_ms: 30_000,
        extra_env: scenario
            .env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<HashMap<_, _>>(),
        dynamic_tools: Vec::new(),
        require_human_approval_for_requests: false,
        resume_usage_baseline: scenario.baseline,
    };
    let (tx, mut rx) = mpsc::channel::<AgentStreamEvent>(256);
    let mut session = start_agent_session(
        config,
        scenario.resume_thread.map(str::to_string),
        None,
        Arc::new(AutoApprovalGate),
        tx.clone(),
    )
    .await
    .expect("session starts");
    let outcome = session
        .run_turn("work", tx)
        .await
        .map_err(|e| e.to_string());
    let snapshot = session.usage_snapshot();
    let _ = session.stop().await;
    let mut reported = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        reported.extend(ev.usage);
    }
    Run {
        outcome,
        snapshot,
        reported,
    }
}

fn assert_no_history_charged(run: &Run, expected_turn: TokenUsage) {
    let outcome = run.outcome.as_ref().expect("turn finished");
    assert_eq!(outcome.usage, expected_turn, "per-turn usage");
    assert_eq!(run.snapshot.session_usage, expected_turn, "session usage");
    // The stream counts this session only: a consumer that adds it to the usage credited so far
    // never charges the thread's history again.
    assert_eq!(
        run.reported.last().copied(),
        Some(expected_turn),
        "{:?}",
        run.reported
    );
    assert!(
        run.reported
            .iter()
            .all(|r| r.since(&expected_turn) == TokenUsage::default()),
        "no report may exceed this session's usage: {:?}",
        run.reported
    );
}

#[tokio::test]
async fn resumed_thread_replay_is_not_charged_again_with_a_persisted_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let run = run(Scenario {
        dir: dir.path(),
        env: vec![
            ("MOCK_REPLAY", "1000:500"),
            ("MOCK_TURN_REPORTS", "1100:520,1400:650,1400:650"),
        ],
        resume_thread: Some("thread_1"),
        baseline: Some(usage(1000, 500)),
    })
    .await;
    assert_no_history_charged(&run, usage(400, 150));
    let outcome = run.outcome.as_ref().unwrap();
    assert_eq!(outcome.thread_usage_total, Some(usage(1400, 650)));
    assert!(!outcome.usage_estimated);
}

#[tokio::test]
async fn replay_arriving_after_the_resume_response_is_not_charged_either() {
    let dir = tempfile::tempdir().unwrap();
    let run = run(Scenario {
        dir: dir.path(),
        env: vec![
            ("MOCK_REPLAY", "1000:500"),
            ("MOCK_REPLAY_AFTER_RESPONSE", "1"),
            ("MOCK_TURN_REPORTS", "1250:560"),
        ],
        resume_thread: Some("thread_1"),
        baseline: Some(usage(1000, 500)),
    })
    .await;
    assert_no_history_charged(&run, usage(250, 60));
}

#[tokio::test]
async fn unknown_baseline_takes_the_replayed_total_as_history() {
    // A task stored by an older version: no baseline was ever recorded.
    let dir = tempfile::tempdir().unwrap();
    let run = run(Scenario {
        dir: dir.path(),
        env: vec![
            ("MOCK_REPLAY", "1000:500"),
            ("MOCK_TURN_REPORTS", "1250:560"),
        ],
        resume_thread: Some("thread_1"),
        baseline: None,
    })
    .await;
    assert_no_history_charged(&run, usage(250, 60));
    assert!(!run.outcome.as_ref().unwrap().usage_estimated);
}

#[tokio::test]
async fn unknown_baseline_without_replay_is_reported_as_estimated() {
    // An app-server that does not replay the thread total on resume: history and new usage cannot
    // be told apart, so the figure is flagged instead of presented as exact.
    let dir = tempfile::tempdir().unwrap();
    let run = run(Scenario {
        dir: dir.path(),
        env: vec![("MOCK_TURN_REPORTS", "1250:560")],
        resume_thread: Some("thread_1"),
        baseline: None,
    })
    .await;
    let outcome = run.outcome.as_ref().unwrap();
    assert_eq!(outcome.usage, usage(1250, 560));
    assert!(outcome.usage_estimated);
    assert!(run.snapshot.estimated);
}

#[tokio::test]
async fn a_changed_thread_id_discards_the_old_baseline() {
    // Resuming `thread_old` yielded a different thread: its totals start from its own history.
    let dir = tempfile::tempdir().unwrap();
    let run = run(Scenario {
        dir: dir.path(),
        env: vec![
            ("MOCK_THREAD_ID", "thread_new"),
            ("MOCK_TURN_REPORTS", "300:100"),
        ],
        resume_thread: Some("thread_old"),
        baseline: Some(usage(1000, 500)),
    })
    .await;
    assert_no_history_charged(&run, usage(300, 100));
    let outcome = run.outcome.as_ref().unwrap();
    assert_eq!(outcome.thread_id, "thread_new");
    assert_eq!(outcome.thread_usage_total, Some(usage(300, 100)));
    assert!(!outcome.usage_estimated);
}

#[tokio::test]
async fn usage_reported_for_another_thread_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let run = run(Scenario {
        dir: dir.path(),
        env: vec![
            ("MOCK_REPLAY", "1000:500"),
            ("MOCK_REPLAY_TURN", "turn_prev"),
            ("MOCK_TURN_REPORTS", "1100:520"),
        ],
        resume_thread: Some("thread_1"),
        baseline: Some(usage(1000, 500)),
    })
    .await;
    assert_no_history_charged(&run, usage(100, 20));
}

#[tokio::test]
async fn a_failed_turn_still_accounts_for_the_usage_it_consumed() {
    let dir = tempfile::tempdir().unwrap();
    let run = run(Scenario {
        dir: dir.path(),
        env: vec![
            ("MOCK_REPLAY", "1000:500"),
            ("MOCK_TURN_REPORTS", "1200:560"),
            ("MOCK_END", "failed"),
        ],
        resume_thread: Some("thread_1"),
        baseline: Some(usage(1000, 500)),
    })
    .await;
    let outcome = run.outcome.as_ref().unwrap();
    assert!(!outcome.succeeded && !outcome.cancelled);
    assert_eq!(outcome.usage, usage(200, 60));
    assert_eq!(outcome.thread_usage_total, Some(usage(1200, 560)));
}

#[tokio::test]
async fn a_cancelled_turn_still_accounts_for_the_usage_it_consumed() {
    let dir = tempfile::tempdir().unwrap();
    let run = run(Scenario {
        dir: dir.path(),
        env: vec![("MOCK_TURN_REPORTS", "700:300"), ("MOCK_END", "cancelled")],
        resume_thread: None,
        baseline: None,
    })
    .await;
    let outcome = run.outcome.as_ref().unwrap();
    assert!(outcome.cancelled);
    assert_eq!(outcome.usage, usage(700, 300));
}

#[tokio::test]
async fn usage_before_a_crash_stays_readable_from_the_session() {
    // The app-server dies mid-turn: `run_turn` returns an error and no outcome, but the usage it
    // reported is still available so the orchestrator can settle it.
    let dir = tempfile::tempdir().unwrap();
    let run = run(Scenario {
        dir: dir.path(),
        env: vec![
            ("MOCK_REPLAY", "1000:500"),
            ("MOCK_TURN_REPORTS", "1150:540"),
            ("MOCK_END", "die"),
        ],
        resume_thread: Some("thread_1"),
        baseline: Some(usage(1000, 500)),
    })
    .await;
    assert!(run.outcome.is_err(), "a dead app-server is an error");
    assert_eq!(run.snapshot.session_usage, usage(150, 40));
    assert_eq!(run.snapshot.thread_total, Some(usage(1150, 540)));
}

#[tokio::test]
async fn consecutive_attempts_chain_through_the_persisted_thread_total() {
    // Attempt 1 starts the thread; attempt 2 resumes it from the total attempt 1 persisted;
    // attempt 3 resumes after attempt 2 crashed mid-turn without persisting anything.
    let dir = tempfile::tempdir().unwrap();
    let first = run(Scenario {
        dir: dir.path(),
        env: vec![("MOCK_TURN_REPORTS", "300:100,500:200")],
        resume_thread: None,
        baseline: None,
    })
    .await;
    let first_outcome = first.outcome.as_ref().unwrap();
    assert_eq!(first_outcome.usage, usage(500, 200));
    let persisted = first_outcome.thread_usage_total.unwrap();
    assert_eq!(persisted, usage(500, 200));

    let second = run(Scenario {
        dir: dir.path(),
        env: vec![("MOCK_REPLAY", "500:200"), ("MOCK_TURN_REPORTS", "800:300")],
        resume_thread: Some("thread_1"),
        baseline: Some(persisted),
    })
    .await;
    assert_no_history_charged(&second, usage(300, 100));

    // Attempt 2 crashed after the thread reached 900:350; only 500:200 was ever persisted. The
    // replay carries the 400:150 nobody accounted for: credited to attempt 3, once.
    let third = run(Scenario {
        dir: dir.path(),
        env: vec![
            ("MOCK_REPLAY", "900:350"),
            ("MOCK_TURN_REPORTS", "1000:380"),
        ],
        resume_thread: Some("thread_1"),
        baseline: Some(persisted),
    })
    .await;
    assert_no_history_charged(&third, usage(500, 180));
    assert_eq!(
        first_outcome
            .usage
            .plus(&third.outcome.as_ref().unwrap().usage),
        usage(1000, 380),
        "attempt 1 + attempt 3 equal the thread's real total"
    );
}
