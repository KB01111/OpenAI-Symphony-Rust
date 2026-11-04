//! OpenCode under an approval policy: the driver runs its own `opencode serve`, drives the session
//! over HTTP and answers `permission.asked` events through the approval gate. A scripted `opencode`
//! points the driver at a stand-in server that replays the event shapes of opencode 1.19.4 (checked
//! against the real CLI, see docs/PRODUCTION_GAP_AUDIT.md, AG-04). Also: usage of `opencode run`.
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
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// Written by `cp` so a concurrently forking test cannot hold it open for writing (ETXTBSY).
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let source = dir.join(format!("{name}.src"));
    std::fs::write(&source, format!("#!/bin/sh\n{body}\n")).unwrap();
    let path = dir.join(name);
    assert!(std::process::Command::new("cp")
        .arg(&source)
        .arg(&path)
        .status()
        .unwrap()
        .success());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn config(
    command: &Path,
    workspace: &Path,
    policy: Value,
    model: Option<&str>,
) -> AgentLaunchConfig {
    AgentLaunchConfig {
        provider: ProviderKind::OpenCode,
        command: command.display().to_string(),
        model: model.map(str::to_string),
        workspace_path: workspace.to_path_buf(),
        task_id: "task-1".into(),
        issue_identifier: "SYM-2".into(),
        issue_title: "Supervised".into(),
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

struct Gate {
    decision: ApprovalDecision,
    seen: Mutex<Vec<ApprovalRequest>>,
}

#[async_trait]
impl ApprovalGate for Gate {
    async fn request_approval(&self, request: ApprovalRequest) -> Result<ApprovalDecision> {
        self.seen.lock().unwrap().push(request);
        Ok(self.decision)
    }
}

#[derive(Default)]
struct Seen {
    /// `METHOD path?query` + body of every request.
    requests: Vec<(String, Value)>,
    authorization: Vec<String>,
}

/// The stand-in `opencode serve`: one session `ses_1`; prompting asks for a bash permission; the
/// reply decides whether the tool part completes or fails; then a text part, a step and idle.
async fn stand_in(listener: TcpListener) -> Arc<Mutex<Seen>> {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (events_tx, events_rx) = mpsc::unbounded_channel::<Value>();
    let grand_lookups = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let events_rx = Arc::new(tokio::sync::Mutex::new(Some(events_rx)));
    let state = seen.clone();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (state, events_tx, events_rx, grand_lookups) = (
                state.clone(),
                events_tx.clone(),
                events_rx.clone(),
                grand_lookups.clone(),
            );
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buf = [0u8; 8192];
                let (head, body) = loop {
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&raw).to_string();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length = head
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if body.len() >= length {
                            break (head.to_string(), body.to_string());
                        }
                    }
                };
                let mut first = head.lines().next().unwrap_or("").split_whitespace();
                let (method, target) = (
                    first.next().unwrap_or("").to_string(),
                    first.next().unwrap_or("").to_string(),
                );
                let path = target.split('?').next().unwrap_or("").to_string();
                let auth = head
                    .lines()
                    .find(|l| l.to_lowercase().starts_with("authorization:"))
                    .map(|l| l.split_once(':').unwrap().1.trim().to_string())
                    .unwrap_or_default();
                let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                {
                    let mut s = state.lock().unwrap();
                    s.requests
                        .push((format!("{method} {target}"), body.clone()));
                    s.authorization.push(auth);
                }
                if path == "/event" {
                    let mut rx = events_rx.lock().await.take().expect("one event subscriber");
                    let _ = stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\n")
                        .await;
                    let _ = stream
                        .write_all(b"data: {\"type\":\"server.connected\",\"properties\":{}}\n\n")
                        .await;
                    while let Some(event) = rx.recv().await {
                        if stream
                            .write_all(format!("data: {event}\n\n").as_bytes())
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    return;
                }
                let session = "ses_1";
                let part = |part: Value| json!({ "type": "message.part.updated", "properties": { "sessionID": session, "part": part } });
                let answer = match (method.as_str(), path.as_str()) {
                    ("POST", "/session") => json!({ "id": session }),
                    ("POST", "/session/ses_1/prompt_async") => {
                        // Another session's events are ignored (it is looked up once).
                        for _ in 0..2 {
                            let _ = events_tx.send(json!({ "type": "session.idle", "properties": { "sessionID": "ses_other" } }));
                        }
                        // Sessions whose parents go round in a circle never reach this one.
                        for _ in 0..2 {
                            let _ = events_tx.send(json!({ "type": "session.idle", "properties": { "sessionID": "ses_loop_a" } }));
                        }
                        // A sub-agent (`task` tool) of this session: its own idle does not end
                        // the turn.
                        let _ = events_tx.send(json!({ "type": "session.created", "properties": { "info": { "id": "ses_child", "parentID": session } } }));
                        // The sub-agent's own step: its tokens count towards the turn.
                        let _ = events_tx.send(json!({ "type": "message.part.updated", "properties": { "sessionID": "ses_child", "part": {
                            "id": "prt_child_s1", "sessionID": "ses_child", "type": "step-finish",
                            "tokens": { "input": 2, "output": 3, "reasoning": 0, "cache": { "read": 0, "write": 0 } } } } }));
                        let _ = events_tx.send(json!({ "type": "session.idle", "properties": { "sessionID": "ses_child" } }));
                        // The permission comes from the sub-agent of a sub-agent that existed
                        // before this turn, so no `session.created` announced either. The first
                        // lookup fails; the permission is sent only once, so it is asked again.
                        let _ = events_tx.send(json!({ "type": "permission.asked", "properties": {
                            "id": "per_1", "sessionID": "ses_grand", "permission": "bash", "patterns": ["cargo test"],
                            "metadata": { "command": "cargo test" }, "always": ["cargo *"] } }));
                        Value::Null
                    }
                    ("POST", "/permission/per_1/reply") => {
                        let allowed = body["reply"] != "reject";
                        let state = if allowed {
                            json!({ "status": "completed", "input": { "command": "cargo test" }, "output": "ok" })
                        } else {
                            json!({ "status": "error", "input": { "command": "cargo test" }, "error": "The user rejected permission to use this specific tool call." })
                        };
                        let _ = events_tx.send(part(json!({ "id": "prt_t", "sessionID": session, "type": "text", "text": "Run", "time": { "start": 1 } })));
                        let _ = events_tx.send(part(json!({ "id": "prt_tool", "sessionID": session, "type": "tool", "tool": "bash", "state": state })));
                        let _ = events_tx.send(part(json!({ "id": "prt_t", "sessionID": session, "type": "text", "text": "Running the tests.", "time": { "start": 1, "end": 2 } })));
                        let step = json!({ "id": "prt_s1", "sessionID": session, "type": "step-finish", "tokens": { "input": 10, "output": 5, "reasoning": 1, "cache": { "read": 4, "write": 0 } } });
                        let _ = events_tx.send(part(step.clone()));
                        let _ = events_tx.send(part(step)); // repeated update: counted once
                        let _ = events_tx.send(json!({ "type": "session.idle", "properties": { "sessionID": session } }));
                        json!(true)
                    }
                    ("POST", "/session/ses_1/abort") => json!(true),
                    ("GET", "/session/ses_grand")
                        if grand_lookups.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 =>
                    {
                        let _ = stream
                            .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                            .await;
                        return;
                    }
                    ("GET", "/session/ses_grand") => {
                        json!({ "id": "ses_grand", "parentID": "ses_old" })
                    }
                    ("GET", "/session/ses_old") => json!({ "id": "ses_old", "parentID": session }),
                    ("GET", "/session/ses_other") => json!({ "id": "ses_other" }),
                    ("GET", "/session/ses_loop_a") => {
                        json!({ "id": "ses_loop_a", "parentID": "ses_loop_b" })
                    }
                    ("GET", "/session/ses_loop_b") => {
                        json!({ "id": "ses_loop_b", "parentID": "ses_loop_a" })
                    }
                    _ => json!({ "error": "unexpected" }),
                };
                let text = answer.to_string();
                let status = if answer.is_null() {
                    "204 No Content"
                } else {
                    "200 OK"
                };
                let body = if answer.is_null() {
                    String::new()
                } else {
                    text
                };
                let _ = stream
                    .write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes())
                    .await;
            });
        }
    });
    seen
}

/// A scripted `opencode`: it records how it was started and the port the driver chose, waits for
/// the test to put the stand-in on that port, then announces it like the real server (or the
/// address in `announce`, when the test wants a wrong one).
const FAKE_SERVE: &str = r#"printf '%s\n' "$@" > serve-args.txt
printf '%s' "$OPENCODE_SERVER_PASSWORD" > password.txt
printf '%s' "$OPENCODE_PERMISSION" > permission.txt
port=""; prev=""
for a in "$@"; do [ "$prev" = "--port" ] && port="$a"; prev="$a"; done
printf '%s' "$port" > port.txt.tmp && mv port.txt.tmp port.txt
while [ ! -f ready ]; do sleep 0.05; done
echo "opencode server listening on http://127.0.0.1:$(cat announce 2>/dev/null || echo "$port")"
sleep 60"#;

/// Bind the stand-in to the port the fake reports, then let the fake announce itself.
async fn serve_when_ready(dir: PathBuf) -> Arc<Mutex<Seen>> {
    let port_file = dir.join("port.txt");
    let port: u16 = loop {
        if let Ok(text) = std::fs::read_to_string(&port_file) {
            if let Ok(port) = text.trim().parse() {
                break port;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let seen = stand_in(listener).await;
    std::fs::write(dir.join("ready"), "").unwrap();
    seen
}

struct Run {
    dir: tempfile::TempDir,
    gate: Arc<Gate>,
    seen: Arc<Mutex<Seen>>,
    outcome: symphony_agents::TurnExecutionOutcome,
}

async fn run(decision: ApprovalDecision) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let fake = script(dir.path(), "opencode", FAKE_SERVE);
    let serving = tokio::spawn(serve_when_ready(dir.path().to_path_buf()));
    let gate = Arc::new(Gate {
        decision,
        seen: Mutex::new(Vec::new()),
    });
    let (tx, mut rx) = mpsc::channel::<AgentStreamEvent>(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let mut session = start_agent_session(
        config(
            &fake,
            dir.path(),
            json!("on-request"),
            Some("anthropic/claude-sonnet-5-5"),
        ),
        None,
        None,
        gate.clone(),
        tx.clone(),
    )
    .await
    .unwrap();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        session.run_turn("Run the tests", tx),
    )
    .await
    .expect("the turn ends at session.idle")
    .unwrap();
    let seen = serving.await.unwrap();
    Run {
        dir,
        gate,
        seen,
        outcome,
    }
}

fn requests(r: &Run) -> Vec<(String, Value)> {
    r.seen.lock().unwrap().requests.clone()
}

#[tokio::test]
async fn a_supervised_turn_asks_the_gate_and_replies_on_the_server() {
    let r = run(ApprovalDecision::Accept).await;
    let read = |f: &str| std::fs::read_to_string(r.dir.path().join(f)).unwrap();
    // A loopback server with its own password and asking permissions.
    let args = read("serve-args.txt");
    assert!(
        args.starts_with("serve\n--hostname\n127.0.0.1\n--port\n"),
        "{args}"
    );
    let password = read("password.txt");
    assert!(password.len() >= 32);
    let permission: Value = serde_json::from_str(&read("permission.txt")).unwrap();
    for key in ["bash", "edit", "webfetch", "external_directory"] {
        assert_eq!(permission[key], "ask", "{permission}");
    }
    // Every request carries the password (basic auth) and the workspace directory.
    let expected = format!("Basic {}", base64_encode(&format!("opencode:{password}")));
    assert!(r
        .seen
        .lock()
        .unwrap()
        .authorization
        .iter()
        .all(|a| a == &expected));
    let reqs = requests(&r);
    assert!(
        reqs.iter().all(|(t, _)| t.contains("directory=")),
        "{reqs:?}"
    );

    let prompt = reqs
        .iter()
        .find(|(t, _)| t.starts_with("POST /session/ses_1/prompt_async"))
        .unwrap();
    assert_eq!(
        prompt.1["parts"],
        json!([{ "type": "text", "text": "Run the tests" }])
    );
    assert_eq!(
        prompt.1["model"],
        json!({ "providerID": "anthropic", "modelID": "claude-sonnet-5-5" })
    );

    let asked = r.gate.seen.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].provider, ProviderKind::OpenCode);
    assert_eq!(asked[0].kind, "command_execution");
    assert_eq!(asked[0].command_or_diff.as_deref(), Some("cargo test"));
    // Asked by the sub-agent's session, and recorded as such.
    assert_eq!(asked[0].session_id, "ses_grand");
    // Unannounced sessions are looked up along their parents, each once (a failed lookup is
    // retried); announced ones are not.
    let lookups: Vec<&str> = reqs
        .iter()
        .filter_map(|(t, _)| t.strip_prefix("GET /session/"))
        .map(|t| t.split('?').next().unwrap())
        .collect();
    assert_eq!(
        lookups,
        [
            "ses_other",
            "ses_loop_a",
            "ses_loop_b",
            "ses_grand",
            "ses_grand",
            "ses_old"
        ]
    );
    let reply = reqs
        .iter()
        .find(|(t, _)| t.starts_with("POST /permission/per_1/reply"))
        .unwrap();
    assert_eq!(reply.1, json!({ "reply": "once" }));

    assert!(r.outcome.succeeded);
    assert_eq!(r.outcome.thread_id, "ses_1");
    assert_eq!(
        r.outcome.assistant_messages,
        vec!["Running the tests.".to_string()]
    );
    // input 10 + cache read 4, output 5 + reasoning 1; the repeated step counts once; the
    // sub-agent's step adds input 2, output 3.
    assert_eq!(
        (r.outcome.usage.input_tokens, r.outcome.usage.output_tokens),
        (16, 9)
    );
}

/// The password and the prompt go only to the address Symphony chose for the server.
#[tokio::test]
async fn a_server_announcing_another_address_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("announce"), "1").unwrap();
    std::fs::write(dir.path().join("ready"), "").unwrap();
    let fake = script(dir.path(), "opencode", FAKE_SERVE);
    let gate = Arc::new(Gate {
        decision: ApprovalDecision::Accept,
        seen: Mutex::new(Vec::new()),
    });
    let (tx, mut rx) = mpsc::channel::<AgentStreamEvent>(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let mut session = start_agent_session(
        config(&fake, dir.path(), json!("on-request"), None),
        None,
        None,
        gate,
        tx.clone(),
    )
    .await
    .unwrap();
    let err = session.run_turn("work", tx).await.unwrap_err();
    assert!(
        err.to_string().contains("not on http://127.0.0.1:"),
        "{err}"
    );
}

#[tokio::test]
async fn decisions_map_to_opencode_replies() {
    let session = run(ApprovalDecision::AcceptForSession).await;
    let reply = |r: &Run| {
        requests(r)
            .into_iter()
            .find(|(t, _)| t.starts_with("POST /permission/per_1/reply"))
            .unwrap()
            .1
    };
    assert_eq!(reply(&session), json!({ "reply": "always" }));

    let declined = run(ApprovalDecision::Decline).await;
    assert_eq!(reply(&declined), json!({ "reply": "reject" }));
    assert!(!declined.outcome.cancelled);

    let aborted = run(ApprovalDecision::Cancel).await;
    assert_eq!(reply(&aborted), json!({ "reply": "reject" }));
    assert!(requests(&aborted)
        .iter()
        .any(|(t, _)| t.starts_with("POST /session/ses_1/abort")));
    assert!(aborted.outcome.cancelled && !aborted.outcome.succeeded);
}

#[tokio::test]
async fn opencode_run_reports_the_tokens_of_every_step() {
    let dir = tempfile::tempdir().unwrap();
    let fake = script(
        dir.path(),
        "opencode",
        r#"echo '{"type":"step_finish","sessionID":"ses_9","part":{"type":"step-finish","tokens":{"input":10,"output":5,"reasoning":0,"cache":{"read":0,"write":2}}}}'
echo '{"type":"text","sessionID":"ses_9","part":{"type":"text","text":"Done."}}'
echo '{"type":"step_finish","sessionID":"ses_9","part":{"type":"step-finish","tokens":{"input":20,"output":3,"reasoning":2,"cache":{"read":1,"write":0}}}}'"#,
    );
    let (tx, mut rx) = mpsc::channel::<AgentStreamEvent>(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let gate = Arc::new(Gate {
        decision: ApprovalDecision::Decline,
        seen: Mutex::new(Vec::new()),
    });
    let mut session = start_agent_session(
        config(&fake, dir.path(), json!("never"), None),
        None,
        None,
        gate.clone(),
        tx.clone(),
    )
    .await
    .unwrap();
    let outcome = session.run_turn("work", tx).await.unwrap();
    assert!(outcome.succeeded);
    assert_eq!(
        (outcome.usage.input_tokens, outcome.usage.output_tokens),
        (33, 10)
    );
    // `never`: `opencode run`, the user's own OpenCode permissions decide; Symphony asks nobody.
    assert!(gate.seen.lock().unwrap().is_empty());
}

fn base64_encode(input: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
