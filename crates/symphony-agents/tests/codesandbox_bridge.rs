//! The CodeSandbox cloud provider against a scripted fake bridge (a small Node program that speaks
//! the same JSON-lines protocol as `tools/codesandbox-runner`): event mapping, resume by sandbox
//! id, abort / stop, failure reporting and credential handling. No network and no sandbox.
//!
//! The bridge needs Node, which this workspace requires anyway; a machine without it skips these
//! tests with a message.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use symphony_agents::{
    detection::find_on_path, start_agent_session, AgentLaunchConfig, AgentSession,
    AgentStreamEvent, AutoApprovalGate, CloudLaunch, CodeSandboxSession,
};
use symphony_core::domain::{CloudAgentKind, ProviderKind};
use symphony_core::redaction::redact_secrets;
use symphony_core::{ToolCallKind, ToolCallStatus};
use tokio::sync::mpsc;

const GIT_TOKEN: &str = "ghp_exampleexampleexampleexample1234";
const CSB_KEY: &str = "csb_v1_exampleexampleexample12345678";
const MODEL_KEY: &str = "sk-ant-exampleexampleexample1234";

/// Reads the request line, appends a summary to `requests.log` next to itself and plays the
/// scenario the prompt asks for.
const FAKE_BRIDGE: &str = r#"
const readline = require("readline");
const fs = require("fs");
const path = require("path");
const rl = readline.createInterface({ input: process.stdin });
rl.once("line", (line) => {
  const req = JSON.parse(line);
  fs.appendFileSync(
    path.join(__dirname, "requests.log"),
    JSON.stringify({
      type: req.type,
      sandboxId: req.sandboxId || null,
      mode: req.mode || null,
      csbApiKey: req.csbApiKey,
      git: req.git || null,
      agent: req.agent || null,
      model: req.model || null,
      branch: req.branch || null,
      repoUrl: req.repoUrl || null,
      templateId: req.templateId || null,
      env: req.env || null,
      prompt: req.prompt || null,
      argv: process.argv.slice(2),
    }) + "\n"
  );
  const out = (event) => process.stdout.write(JSON.stringify(event) + "\n");
  if (req.type !== "turn") process.exit(0);
  const sandboxId = req.sandboxId || "sb-fake-1";
  out({ type: "started", sandboxId, resumed: !!req.sandboxId });
  if (req.prompt.includes("SCENARIO_CRASH")) process.exit(3);
  if (req.prompt.includes("SCENARIO_HANG_AFTER_USAGE")) {
    out({ type: "usage", inputTokens: 100, outputTokens: 20 });
    setInterval(() => {}, 1000); // never finishes: the host gives up on it
    return;
  }
  if (req.prompt.includes("SCENARIO_UNICODE_SHA")) {
    // Not a hash: a process in the sandbox can print anything, and byte 12 falls inside a character.
    out({ type: "pushed", sha: "ééééééééééééééé", branch: req.branch });
    out({ type: "turn_completed", succeeded: true });
    process.exit(0);
  }
  if (req.prompt.includes("SCENARIO_LEAK")) out({ type: "output", text: "token " + req.git.token });
  out({ type: "output", text: "working" });
  out({ type: "tool", callId: "c1", name: "bash", kind: "command", status: "started", input: { command: "ls" } });
  out({ type: "tool", callId: "c1", name: "bash", kind: "command", status: "succeeded", output: "README.md", durationMs: 12 });
  out({ type: "usage", inputTokens: 100, outputTokens: 20 });
  out({ type: "pushed", sha: "abcdef1234567890abcdef", branch: req.branch });
  out({ type: "usage", inputTokens: 150, outputTokens: 30 });
  if (req.prompt.includes("SCENARIO_FAIL")) {
    out({ type: "turn_completed", succeeded: false, errorCode: "agent_exit_10", message: "The cloud agent exited with code 10" });
    process.exit(1);
  }
  out({ type: "turn_completed", succeeded: true });
  process.exit(0);
});
"#;

fn have_node() -> bool {
    if find_on_path("node").is_some() {
        return true;
    }
    eprintln!("skipping: node is not on PATH");
    false
}

struct Fixture {
    _dir: tempfile::TempDir,
    workspace: PathBuf,
    bridge: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let bridge = dir.path().join("bridge.cjs");
        std::fs::write(&bridge, FAKE_BRIDGE).unwrap();
        Self {
            _dir: dir,
            workspace,
            bridge,
        }
    }

    fn config(&self) -> AgentLaunchConfig {
        self.config_with_runner(Some(self.bridge.display().to_string()))
    }

    fn config_with_runner(&self, runner_path: Option<String>) -> AgentLaunchConfig {
        AgentLaunchConfig {
            provider: ProviderKind::CodeSandbox,
            command: String::new(),
            model: Some("anthropic/claude-sonnet-5-5".into()),
            workspace_path: self.workspace.clone(),
            task_id: "task-1".into(),
            issue_identifier: "SYM-1".into(),
            issue_title: "Cloud".into(),
            approval_policy: json!("never"),
            thread_sandbox: json!("workspace-write"),
            turn_sandbox_policy: None,
            read_timeout_ms: 5_000,
            turn_timeout_ms: 60_000,
            extra_env: HashMap::from([("ANTHROPIC_API_KEY".to_string(), MODEL_KEY.to_string())]),
            dynamic_tools: Vec::new(),
            require_human_approval_for_requests: false,
            resume_usage_baseline: None,
            cloud: Some(CloudLaunch {
                agent: CloudAgentKind::PiDurable,
                template_id: "symphony-agents".into(),
                repo_url: "https://github.com/acme/widgets.git".into(),
                branch: "symphony/sym-1".into(),
                base_branch: "main".into(),
                vm_tier: "small".into(),
                hibernation_timeout_secs: 1800,
                cost_limit_usd: 3.0,
                git_user_name: "Symphony Cloud Agent".into(),
                git_user_email: "bot@example.com".into(),
                runner_path,
                csb_api_key: CSB_KEY.into(),
                git_token: GIT_TOKEN.into(),
            }),
        }
    }

    /// One JSON object per request the fake bridge received, in order.
    fn requests(&self) -> Vec<Value> {
        let log = self.bridge.parent().unwrap().join("requests.log");
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

fn drain(rx: &mut mpsc::Receiver<AgentStreamEvent>) -> Vec<AgentStreamEvent> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

fn types(events: &[AgentStreamEvent]) -> Vec<&str> {
    events
        .iter()
        .filter(|e| e.event_type != "agent/stderr")
        .map(|e| e.event_type.as_str())
        .collect()
}

#[tokio::test]
async fn a_turn_streams_the_bridge_events_and_reports_the_outcome() {
    if !have_node() {
        return;
    }
    let fx = Fixture::new();
    let (tx, mut rx) = mpsc::channel(64);
    let mut session = CodeSandboxSession::new(fx.config(), None);

    let outcome = session.run_turn("Fix the bug", tx).await.unwrap();

    assert!(outcome.succeeded, "{outcome:?}");
    assert_eq!(outcome.thread_id, "sb-fake-1");
    assert_eq!(outcome.turn_id, "turn-1");
    assert_eq!(outcome.session_id, "sb-fake-1-turn-1");
    assert_eq!(outcome.assistant_messages, vec!["working".to_string()]);
    // The bridge reports cumulative usage of the turn: the last report counts, not the sum.
    assert_eq!(outcome.usage.input_tokens, 150);
    assert_eq!(outcome.usage.output_tokens, 30);
    assert_eq!(outcome.usage.total_tokens, 180);
    assert_eq!(session.thread_id().as_deref(), Some("sb-fake-1"));
    assert_eq!(session.usage_snapshot().session_usage.total_tokens, 180);

    let events = drain(&mut rx);
    assert_eq!(
        types(&events),
        [
            "codesandbox/started",
            "codesandbox/output",
            "codesandbox/tool",
            "codesandbox/tool",
            "codesandbox/usage",
            "codesandbox/pushed",
            "codesandbox/usage",
            "codesandbox/turn_completed",
        ]
    );
    assert_eq!(
        events[0].message.as_deref(),
        Some("Started CodeSandbox sandbox sb-fake-1")
    );
    // The orchestrator turns pushes into worktree updates: the event carries the commit.
    let pushed = &events[5];
    assert_eq!(pushed.message.as_deref(), Some("Pushed abcdef123456"));
    assert_eq!(pushed.raw["sha"], "abcdef1234567890abcdef");
    assert_eq!(pushed.thread_id.as_deref(), Some("sb-fake-1"));

    // Start and result of a tool call share an id, so they land on one record.
    let started = &events[2].tool_calls[0];
    let finished = &events[3].tool_calls[0];
    assert_eq!(started.call_id, finished.call_id);
    assert_eq!(started.kind, ToolCallKind::Command);
    assert_eq!(started.status, ToolCallStatus::Started);
    assert_eq!(finished.status, ToolCallStatus::Succeeded);
    assert_eq!(finished.output.as_deref(), Some("README.md"));
    assert_eq!(finished.duration_ms, Some(12));

    // Session usage is reported cumulatively, on top of earlier turns.
    assert_eq!(events[6].usage.unwrap().total_tokens, 180);
}

#[tokio::test]
async fn the_request_carries_the_launch_settings_and_credentials_only_on_stdin() {
    if !have_node() {
        return;
    }
    let fx = Fixture::new();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = CodeSandboxSession::new(fx.config(), None);

    session.run_turn("Fix the bug", tx).await.unwrap();

    let request = &fx.requests()[0];
    assert_eq!(request["type"], "turn");
    assert_eq!(request["agent"], "pi_durable");
    assert_eq!(request["model"], "anthropic/claude-sonnet-5-5");
    assert_eq!(request["templateId"], "symphony-agents");
    assert_eq!(request["repoUrl"], "https://github.com/acme/widgets.git");
    assert_eq!(request["branch"], "symphony/sym-1");
    assert_eq!(request["csbApiKey"], CSB_KEY);
    assert_eq!(request["git"]["token"], GIT_TOKEN);
    assert_eq!(request["env"]["ANTHROPIC_API_KEY"], MODEL_KEY);
    assert_eq!(request["prompt"], "Fix the bug");
    // Nothing secret on the command line: only the script path was passed.
    assert_eq!(request["argv"], json!([]));
}

#[tokio::test]
async fn credentials_are_masked_even_if_a_bridge_prints_them() {
    if !have_node() {
        return;
    }
    let fx = Fixture::new();
    let (tx, mut rx) = mpsc::channel(64);
    let mut session = CodeSandboxSession::new(fx.config(), None);

    session.run_turn("SCENARIO_LEAK", tx).await.unwrap();

    let leaked = drain(&mut rx)
        .into_iter()
        .filter_map(|e| e.message)
        .find(|m| m.contains("token "))
        .expect("the leak scenario prints the token");
    // The session registered its secrets, so the orchestrator's redaction removes them.
    let shown = redact_secrets(&leaked, &[]);
    assert!(!shown.contains(GIT_TOKEN), "{shown}");
    assert!(shown.contains("[REDACTED]"));
}

#[tokio::test]
async fn the_next_turn_resumes_the_same_sandbox() {
    if !have_node() {
        return;
    }
    let fx = Fixture::new();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = CodeSandboxSession::new(fx.config(), None);
    session.run_turn("first", tx.clone()).await.unwrap();
    let second = session.run_turn("second", tx).await.unwrap();

    let requests = fx.requests();
    assert_eq!(requests[0]["sandboxId"], Value::Null);
    assert_eq!(requests[1]["sandboxId"], "sb-fake-1");
    assert_eq!(second.turn_id, "turn-2");
    // Usage of finished turns accumulates in the session.
    assert_eq!(session.usage_snapshot().session_usage.total_tokens, 360);

    // A session restored from a task's saved thread id resumes that sandbox from the start.
    let (tx, mut rx) = mpsc::channel(64);
    let mut restored = CodeSandboxSession::new(fx.config(), Some("sb-earlier".into()));
    restored.run_turn("again", tx).await.unwrap();
    assert_eq!(fx.requests()[2]["sandboxId"], "sb-earlier");
    assert_eq!(
        drain(&mut rx)[0].message.as_deref(),
        Some("Resumed CodeSandbox sandbox sb-earlier")
    );
}

#[tokio::test]
async fn a_failed_turn_reports_the_agents_error() {
    if !have_node() {
        return;
    }
    let fx = Fixture::new();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = CodeSandboxSession::new(fx.config(), None);

    let outcome = session.run_turn("SCENARIO_FAIL", tx).await.unwrap();

    assert!(!outcome.succeeded);
    assert!(!outcome.cancelled);
    assert_eq!(outcome.error_code.as_deref(), Some("agent_exit_10"));
    assert_eq!(
        outcome.error_message.as_deref(),
        Some("The cloud agent exited with code 10")
    );
    // What the agent consumed before it failed still counts.
    assert_eq!(outcome.usage.total_tokens, 180);
}

#[tokio::test]
async fn a_bridge_that_dies_before_finishing_fails_the_turn() {
    if !have_node() {
        return;
    }
    let fx = Fixture::new();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = CodeSandboxSession::new(fx.config(), None);

    let outcome = session.run_turn("SCENARIO_CRASH", tx).await.unwrap();

    assert!(!outcome.succeeded);
    assert_eq!(
        outcome.error_code.as_deref(),
        Some("codesandbox_bridge_exit")
    );
    // The sandbox it had started is kept, so a retry resumes it.
    assert_eq!(session.thread_id().as_deref(), Some("sb-fake-1"));
}

#[tokio::test]
async fn cancel_and_stop_act_on_the_sandbox_that_was_started() {
    if !have_node() {
        return;
    }
    let fx = Fixture::new();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = CodeSandboxSession::new(fx.config(), None);

    // Nothing started yet: there is no sandbox to act on.
    session.cancel().await.unwrap();
    session.stop().await.unwrap();
    assert!(fx.requests().is_empty());

    session.run_turn("work", tx).await.unwrap();
    session.cancel().await.unwrap();
    session.stop().await.unwrap();

    let requests = fx.requests();
    assert_eq!(requests[1]["type"], "abort");
    assert_eq!(requests[1]["sandboxId"], "sb-fake-1");
    assert_eq!(requests[1]["csbApiKey"], CSB_KEY);
    assert_eq!(requests[2]["type"], "stop");
    assert_eq!(requests[2]["mode"], "hibernate");
}

#[tokio::test]
async fn a_cancelled_session_reports_its_turn_as_cancelled() {
    if !have_node() {
        return;
    }
    let fx = Fixture::new();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = CodeSandboxSession::new(fx.config(), Some("sb-earlier".into()));
    session.cancel().await.unwrap();

    let outcome = session.run_turn("work", tx).await.unwrap();

    assert!(outcome.cancelled);
    assert!(!outcome.succeeded);
}

#[tokio::test]
async fn a_missing_bridge_or_launch_config_is_a_clear_error() {
    if !have_node() {
        return;
    }
    let fx = Fixture::new();
    let (tx, _rx) = mpsc::channel(64);

    // An explicitly configured bridge that does not exist is an error, even where a built bridge
    // could be found elsewhere.
    let missing = fx.config_with_runner(Some(
        fx.workspace
            .join("no-such-bridge.mjs")
            .display()
            .to_string(),
    ));
    let err = CodeSandboxSession::new(missing, None)
        .run_turn("work", tx.clone())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "agent_protocol_error");
    assert!(
        err.to_string().contains("codesandbox_runner_not_found"),
        "{err}"
    );

    let mut without_cloud = fx.config();
    without_cloud.cloud = None;
    let err = CodeSandboxSession::new(without_cloud, None)
        .run_turn("work", tx)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("codesandbox_not_configured"),
        "{err}"
    );
}

#[tokio::test]
async fn the_provider_is_started_through_the_common_entry_point() {
    if !have_node() {
        return;
    }
    let fx = Fixture::new();
    let (tx, _rx) = mpsc::channel(8);
    let session = start_agent_session(
        fx.config(),
        Some("sb-9".into()),
        None,
        Arc::new(AutoApprovalGate),
        tx,
    )
    .await
    .unwrap();

    assert_eq!(session.provider_kind(), ProviderKind::CodeSandbox);
    assert_eq!(session.thread_id().as_deref(), Some("sb-9"));
    assert!(session.capabilities().supports_session_resume);
    assert!(!session.capabilities().supports_approvals);
}

#[test]
fn debug_output_and_serialization_never_contain_the_credentials() {
    let fx = Fixture::new();
    let cloud = fx.config().cloud.unwrap();

    let debug = format!("{cloud:?}");
    assert!(
        !debug.contains(GIT_TOKEN) && !debug.contains(CSB_KEY),
        "{debug}"
    );

    let serialized = serde_json::to_string(&cloud).unwrap();
    assert!(
        !serialized.contains(GIT_TOKEN) && !serialized.contains(CSB_KEY),
        "{serialized}"
    );
    // And a round trip loses them: they are never persisted with a launch config.
    let restored: CloudLaunch = serde_json::from_str(&serialized).unwrap();
    assert!(restored.git_token.is_empty() && restored.csb_api_key.is_empty());
}

#[tokio::test]
async fn usage_of_a_turn_that_was_cut_off_still_counts() {
    if !have_node() {
        return;
    }
    let fx = Fixture::new();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = CodeSandboxSession::new(fx.config(), None);

    // The bridge reports usage and then never finishes; the orchestrator drops the turn (timeout,
    // cancel). What it consumed is read from the session afterwards, to settle the attempt.
    let cut_off = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        session.run_turn("SCENARIO_HANG_AFTER_USAGE", tx.clone()),
    )
    .await;
    assert!(cut_off.is_err(), "the scripted bridge never finishes");
    assert_eq!(session.usage_snapshot().session_usage.total_tokens, 120);

    // The next turn starts from that and does not count it twice.
    session.run_turn("again", tx).await.unwrap();
    assert_eq!(
        session.usage_snapshot().session_usage.total_tokens,
        120 + 180
    );
}

#[test]
fn a_relative_runner_path_is_made_absolute() {
    // The bridge is started in the task's workspace, so a path relative to where Symphony runs would
    // point nowhere. `Cargo.toml` is relative to this test's working directory (the crate).
    let found = symphony_agents::find_runner_bundle(Some("Cargo.toml")).unwrap();
    assert!(found.is_absolute(), "{}", found.display());
    assert!(found.ends_with("Cargo.toml"));
    // An explicit path that does not exist is not replaced by a search.
    assert!(symphony_agents::find_runner_bundle(Some("no-such-bridge.mjs")).is_none());
}

#[tokio::test]
async fn detection_honours_the_runner_path_from_the_workflow() {
    if !have_node() {
        return;
    }
    let fx = Fixture::new();
    let configured = fx.bridge.display().to_string();

    let with_bridge = symphony_agents::detect_provider_status(
        ProviderKind::CodeSandbox,
        &[],
        None,
        false,
        Some(&configured),
    )
    .await;
    assert!(
        with_bridge.installed,
        "{:?}",
        with_bridge.capabilities.status_message
    );
    assert!(
        !with_bridge
            .capabilities
            .status_message
            .contains("Bridge not built"),
        "{}",
        with_bridge.capabilities.status_message
    );

    // A configured path that is not a file is reported, not silently replaced by another bridge.
    let wrong = fx
        .workspace
        .join("missing-bridge.mjs")
        .display()
        .to_string();
    let without = symphony_agents::detect_provider_status(
        ProviderKind::CodeSandbox,
        &[],
        None,
        false,
        Some(&wrong),
    )
    .await;
    assert!(!without.installed);
    assert!(without
        .capabilities
        .status_message
        .contains("Bridge not built"));
}

#[tokio::test]
async fn a_pushed_event_with_a_non_ascii_commit_id_does_not_panic_the_worker() {
    if !have_node() {
        return;
    }
    let fx = Fixture::new();
    let (tx, mut rx) = mpsc::channel(64);
    let mut session = CodeSandboxSession::new(fx.config(), None);

    let outcome = session.run_turn("SCENARIO_UNICODE_SHA", tx).await.unwrap();

    assert!(outcome.succeeded, "{outcome:?}");
    let pushed = drain(&mut rx)
        .into_iter()
        .find(|e| e.event_type == "codesandbox/pushed")
        .expect("the push is still reported");
    // Cut by characters: twelve of them, not twelve bytes.
    assert_eq!(pushed.message.as_deref(), Some("Pushed éééééééééééé"));
}
