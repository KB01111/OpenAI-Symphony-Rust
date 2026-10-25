//! OpenCode with Symphony's approval gate.
//!
//! `opencode run` (also `--attach`) rejects every permission request it would have to ask about,
//! and OpenCode's default permissions allow `bash` and `edit` without asking, so a headless run either
//! does everything unasked or nothing. When the approval policy asks for approvals, the driver
//! therefore runs its own `opencode serve` for the turn (loopback only, random password, `ask` for
//! commands, edits, web fetches and paths outside the workspace), drives the session over its HTTP
//! API and answers each `permission.asked` event through the approval gate. Shapes verified against
//! opencode 1.19.4 (docs/PRODUCTION_GAP_AUDIT.md, AG-04).

use std::collections::{HashMap, HashSet};

use futures::StreamExt;

use super::*;

const SERVER_START_TIMEOUT: Duration = Duration::from_secs(30);
/// Permission rules for a supervised turn: everything that changes the machine asks.
const ASK_PERMISSIONS: &str =
    r#"{"edit":"ask","bash":"ask","webfetch":"ask","external_directory":"ask"}"#;

/// Usage of one finished OpenCode step (`step-finish` part `tokens`): cache traffic counts as input,
/// reasoning as output.
pub(crate) fn opencode_step_usage(part: &Value) -> Option<TokenUsage> {
    let tokens = part.get("tokens")?;
    let n = |v: Option<&Value>| v.and_then(Value::as_u64).unwrap_or(0);
    let input = n(tokens.get("input"))
        .saturating_add(n(tokens.pointer("/cache/read")))
        .saturating_add(n(tokens.pointer("/cache/write")));
    let output = n(tokens.get("output")).saturating_add(n(tokens.get("reasoning")));
    Some(TokenUsage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: input.saturating_add(output),
    })
}

/// OpenCode's reply to a permission request for a decision of the approval gate.
pub(crate) fn opencode_reply(decision: ApprovalDecision) -> &'static str {
    match decision {
        ApprovalDecision::Accept => "once",
        ApprovalDecision::AcceptForSession => "always",
        ApprovalDecision::Decline | ApprovalDecision::Cancel => "reject",
    }
}

/// The approval Symphony shows for a `permission.asked` event.
pub(crate) fn opencode_approval_request(
    config: &AgentLaunchConfig,
    session_id: &str,
    asked: &Value,
) -> ApprovalRequest {
    let permission = asked
        .get("permission")
        .and_then(Value::as_str)
        .unwrap_or("tool");
    let kind = match permission {
        "bash" => "command_execution",
        "edit" => "file_change",
        _ => "tool_use",
    };
    let detail = asked
        .pointer("/metadata/command")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            asked.get("patterns").and_then(Value::as_array).map(|p| {
                p.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        })
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| {
            serde_json::to_string_pretty(asked.get("metadata").unwrap_or(&Value::Null))
                .unwrap_or_default()
        });
    ApprovalRequest {
        id: uuid::Uuid::new_v4().to_string(),
        task_id: config.task_id.clone(),
        issue_identifier: config.issue_identifier.clone(),
        session_id: session_id.to_string(),
        provider: ProviderKind::OpenCode,
        kind: kind.to_string(),
        title: format!("Approval requested: {permission}"),
        description: format!("OpenCode wants to use {permission}"),
        command_or_diff: Some(detail),
        status: ApprovalStatus::Pending,
        decision: None,
        created_at: Utc::now(),
        resolved_at: None,
    }
}

struct Server {
    // Declared first: dropping the turn kills the server and everything it started.
    _tree: ProcessTree,
    _child: Child,
    base: String,
    password: String,
    directory: String,
    client: reqwest::Client,
    /// The turn's deadline: no request to the server may outlive it.
    deadline: tokio::time::Instant,
    timeout_ms: u64,
}

impl Server {
    async fn start(
        exe: &str,
        config: &AgentLaunchConfig,
        event_tx: &mpsc::Sender<AgentStreamEvent>,
        deadline: tokio::time::Instant,
    ) -> Result<Self> {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .map_err(|e| SymphonyError::AgentProtocolError {
                code: "opencode_port_unavailable".into(),
                message: format!("No local port for the OpenCode server: {e}"),
            })?;
        let password = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let mut cmd = Command::new(exe);
        cmd.arg("serve")
            .arg("--hostname")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .current_dir(&config.workspace_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for key in SECRET_ENV_VARS_TO_STRIP {
            cmd.env_remove(key);
        }
        for (k, v) in &config.extra_env {
            cmd.env(k, v);
        }
        cmd.env("OPENCODE_SERVER_PASSWORD", &password)
            .env("OPENCODE_PERMISSION", ASK_PERMISSIONS);
        prepare_tree_command(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| SymphonyError::AgentProtocolError {
            code: "opencode_not_found".into(),
            message: format!("Failed to spawn OpenCode server ('{exe}'): {e}"),
        })?;
        let tree = ProcessTree::attach(&child);
        if let Some(stderr) = child.stderr.take() {
            forward_stderr(stderr, child.id(), event_tx.clone());
        }
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SymphonyError::AgentProtocolError {
                code: "stdout_unavailable".into(),
                message: "OpenCode server stdout was not piped".into(),
            })?;
        let mut lines = BufReader::new(stdout).lines();
        let start_deadline = deadline.min(tokio::time::Instant::now() + SERVER_START_TIMEOUT);
        let base = loop {
            match tokio::time::timeout_at(start_deadline, lines.next_line()).await {
                Ok(Ok(Some(line))) => {
                    if let Some(url) = line.split_whitespace().find(|w| w.starts_with("http://")) {
                        // The password and the prompt only ever go to the address Symphony chose.
                        let expected = format!("http://127.0.0.1:{port}");
                        if url.trim_end_matches('/') != expected {
                            return Err(SymphonyError::AgentProtocolError {
                                code: "opencode_server_address".into(),
                                message: format!(
                                    "The OpenCode server listens on {url}, not on {expected}"
                                ),
                            });
                        }
                        break expected;
                    }
                }
                Ok(_) => {
                    return Err(SymphonyError::AgentProtocolError {
                        code: "opencode_server_exited".into(),
                        message: "The OpenCode server exited before it was listening".into(),
                    })
                }
                Err(_) => {
                    return Err(SymphonyError::AgentProtocolError {
                        code: "opencode_server_timeout".into(),
                        message: "The OpenCode server did not start listening in time".into(),
                    })
                }
            }
        };
        // Keep draining stdout so the server never blocks on a full pipe.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| SymphonyError::AgentProtocolError {
                code: "opencode_client".into(),
                message: e.to_string(),
            })?;
        Ok(Self {
            _tree: tree,
            _child: child,
            base,
            password,
            directory: config.workspace_path.to_string_lossy().to_string(),
            client,
            deadline,
            timeout_ms: config.turn_timeout_ms,
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, format!("{}{path}", self.base))
            .basic_auth("opencode", Some(&self.password))
            .query(&[("directory", self.directory.as_str())])
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.call(reqwest::Method::POST, path, Some(body)).await
    }

    async fn get(&self, path: &str) -> Result<Value> {
        self.call(reqwest::Method::GET, path, None).await
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let exchange = async {
            let mut request = self.request(method, path);
            if let Some(body) = &body {
                request = request.json(body);
            }
            let resp = request.send().await?;
            let status = resp.status();
            Ok::<_, reqwest::Error>((status, resp.text().await?))
        };
        let (status, text) = tokio::time::timeout_at(self.deadline, exchange)
            .await
            .map_err(|_| SymphonyError::TurnTimeout(self.timeout_ms))?
            .map_err(|e| protocol_error("opencode_request_failed", e))?;
        if !status.is_success() {
            return Err(SymphonyError::AgentProtocolError {
                code: "opencode_request_failed".into(),
                message: format!("OpenCode answered {status} for {path}"),
            });
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// Server-sent events, one JSON object per event, until the server goes away.
    async fn events(&self) -> Result<mpsc::Receiver<Value>> {
        let resp = tokio::time::timeout_at(
            self.deadline,
            self.request(reqwest::Method::GET, "/event").send(),
        )
        .await
        .map_err(|_| SymphonyError::TurnTimeout(self.timeout_ms))?
        .map_err(|e| protocol_error("opencode_events_failed", e))?;
        if !resp.status().is_success() {
            return Err(SymphonyError::AgentProtocolError {
                code: "opencode_events_failed".into(),
                message: format!("OpenCode answered {} for /event", resp.status()),
            });
        }
        let (tx, rx) = mpsc::channel(256);
        tokio::spawn(async move {
            let mut stream = resp.bytes_stream();
            // Bytes, not text: a UTF-8 character may be split across chunks. Only complete
            // events (ending in a blank line) are decoded.
            let mut buffer: Vec<u8> = Vec::new();
            while let Some(Ok(chunk)) = stream.next().await {
                buffer.extend_from_slice(&chunk);
                while let Some(end) = buffer.windows(2).position(|w| w == b"\n\n") {
                    let block_bytes: Vec<u8> = buffer.drain(..end + 2).collect();
                    let block = String::from_utf8_lossy(&block_bytes);
                    let data: String = block
                        .lines()
                        .filter_map(|l| l.strip_prefix("data:"))
                        .map(str::trim_start)
                        .collect();
                    if let Ok(event) = serde_json::from_str::<Value>(&data) {
                        if tx.send(event).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Ok(rx)
    }
}

fn protocol_error(code: &str, e: impl std::fmt::Display) -> SymphonyError {
    SymphonyError::AgentProtocolError {
        code: code.into(),
        message: format!("OpenCode server: {e}"),
    }
}

/// How many `parentID` links a session lookup follows before it gives up.
const MAX_SESSION_DEPTH: usize = 8;

/// Whether `id` is a sub-agent session under one of `sessions`, asked from the server. Sessions
/// that existed before this turn (a resumed session's earlier sub-agents) announce no
/// `session.created`, and their permission requests must still reach the approval gate. The chain
/// found is added to `sessions`; a session that leads elsewhere is remembered in `foreign`.
async fn session_belongs(
    server: &Server,
    id: &str,
    sessions: &mut HashSet<String>,
    foreign: &mut HashSet<String>,
) -> bool {
    if sessions.contains(id) {
        return true;
    }
    let mut chain = Vec::new();
    let mut current = id.to_string();
    let belongs = loop {
        if foreign.contains(&current) || chain.len() >= MAX_SESSION_DEPTH {
            break false;
        }
        let parent = match server.get(&format!("/session/{current}")).await {
            Ok(info) => info
                .get("parentID")
                .and_then(Value::as_str)
                .map(str::to_string),
            Err(_) => None,
        };
        chain.push(current);
        match parent {
            Some(parent) if sessions.contains(&parent) => break true,
            Some(parent) => current = parent,
            None => break false,
        }
    };
    if belongs {
        sessions.extend(chain);
    } else {
        foreign.extend(chain);
    }
    belongs
}

fn event_session(props: &Value) -> Option<&str> {
    props
        .get("sessionID")
        .or_else(|| props.pointer("/part/sessionID"))
        .or_else(|| props.pointer("/info/id"))
        .and_then(Value::as_str)
}

impl OpenCodeSession {
    /// One supervised turn through `opencode serve` (see the module documentation).
    pub(crate) async fn run_server_turn(
        &mut self,
        exe: &str,
        prompt: &str,
        event_tx: mpsc::Sender<AgentStreamEvent>,
    ) -> Result<TurnExecutionOutcome> {
        let timeout_ms = self.config.turn_timeout_ms;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms.max(5000));
        let server = Server::start(exe, &self.config, &event_tx, deadline).await?;
        self.pid = server._child.id();
        let mut events = server.events().await?;

        let session_id = match self.session_id.clone().filter(|s| s.starts_with("ses")) {
            Some(existing) => existing,
            None => server
                .post("/session", json!({ "title": self.config.issue_title }))
                .await?
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| SymphonyError::AgentProtocolError {
                    code: "opencode_session_failed".into(),
                    message: "OpenCode did not return a session id".into(),
                })?,
        };
        self.session_id = Some(session_id.clone());

        let mut body = json!({ "parts": [{ "type": "text", "text": prompt }] });
        if let Some((provider, model)) =
            self.config.model.as_deref().and_then(|m| m.split_once('/'))
        {
            body["model"] = json!({ "providerID": provider, "modelID": model });
        }
        server
            .post(&format!("/session/{session_id}/prompt_async"), body)
            .await?;

        let mut usage = TokenUsage::default();
        let mut counted_steps = HashSet::new();
        let mut text_order: Vec<String> = Vec::new();
        let mut texts: HashMap<String, String> = HashMap::new();
        let mut error: Option<String> = None;
        // This turn's session and the sub-agent sessions it starts (OpenCode's `task` tool): their
        // permission requests and token use belong to this turn too.
        let mut sessions: HashSet<String> = HashSet::from([session_id.clone()]);
        let mut foreign: HashSet<String> = HashSet::new();
        loop {
            let event = match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Some(event)) => event,
                Ok(None) => {
                    error.get_or_insert_with(|| {
                        "The OpenCode server closed its event stream".into()
                    });
                    break;
                }
                Err(_) => return Err(SymphonyError::TurnTimeout(timeout_ms)),
            };
            let props = event.get("properties").cloned().unwrap_or(Value::Null);
            let kind = event
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if matches!(kind, "session.created" | "session.updated") {
                if let (Some(child), Some(parent)) = (
                    props.pointer("/info/id").and_then(Value::as_str),
                    props.pointer("/info/parentID").and_then(Value::as_str),
                ) {
                    if sessions.contains(parent) {
                        sessions.insert(child.to_string());
                    }
                }
            }
            let Some(event_session_id) = event_session(&props).map(str::to_string) else {
                continue;
            };
            if !session_belongs(&server, &event_session_id, &mut sessions, &mut foreign).await {
                continue;
            }
            let root = event_session_id == session_id;
            match event
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "permission.asked" => {
                    let Some(request_id) = props.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let decision = if self.config.approval_policy == json!("never")
                        && !self.config.require_human_approval_for_requests
                    {
                        ApprovalDecision::Decline
                    } else {
                        let approval =
                            opencode_approval_request(&self.config, &event_session_id, &props);
                        tokio::time::timeout_at(
                            deadline,
                            self.approval_gate.request_approval(approval),
                        )
                        .await
                        .map_err(|_| SymphonyError::TurnTimeout(timeout_ms))??
                    };
                    server
                        .post(
                            &format!("/permission/{request_id}/reply"),
                            json!({ "reply": opencode_reply(decision) }),
                        )
                        .await?;
                    if decision == ApprovalDecision::Cancel {
                        *self.cancelled.lock().await = true;
                        let _ = server
                            .post(&format!("/session/{session_id}/abort"), json!({}))
                            .await;
                    }
                }
                "message.part.updated" => {
                    let part = props.get("part").cloned().unwrap_or(Value::Null);
                    let part_type = part.get("type").and_then(Value::as_str).unwrap_or_default();
                    let part_id = part
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let finished = match part_type {
                        "text" => part.pointer("/time/end").is_some(),
                        "tool" => matches!(
                            part.pointer("/state/status").and_then(Value::as_str),
                            Some("completed" | "error")
                        ),
                        "step-start" | "step-finish" => true,
                        _ => false,
                    };
                    // A sub-agent's text is its report to this session, not the turn's answer.
                    if part_type == "text" && root {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            if !texts.contains_key(&part_id) {
                                text_order.push(part_id.clone());
                            }
                            texts.insert(part_id.clone(), text.to_string());
                        }
                    }
                    if part_type == "step-finish" && counted_steps.insert(part_id.clone()) {
                        if let Some(step) = opencode_step_usage(&part) {
                            usage.add(&step);
                        }
                    }
                    if finished {
                        let message = (part_type == "text")
                            .then(|| part.get("text").and_then(Value::as_str).map(str::to_string))
                            .flatten();
                        let _ = tokio::time::timeout_at(
                            deadline,
                            event_tx.send(AgentStreamEvent {
                                event_type: "opencode/event".into(),
                                timestamp: Utc::now(),
                                session_id: Some(session_id.clone()),
                                thread_id: Some(session_id.clone()),
                                turn_id: Some("turn-1".into()),
                                pid: self.pid,
                                message,
                                usage: Some(self.completed_usage.plus(&usage)),
                                rate_limits: None,
                                raw: json!({ "type": part_type, "part": part }),
                            }),
                        )
                        .await;
                    }
                }
                // A sub-agent's error is reported to (and handled by) this session; its idle is
                // not the end of the turn.
                "session.error" if root => {
                    error = Some(
                        props
                            .pointer("/error/data/message")
                            .or_else(|| props.pointer("/error/name"))
                            .and_then(Value::as_str)
                            .unwrap_or("OpenCode reported an error")
                            .to_string(),
                    );
                }
                "session.idle" if root => break,
                _ => {}
            }
        }
        drop(server);
        self.completed_usage.add(&usage);
        let cancelled = *self.cancelled.lock().await;
        let succeeded = error.is_none() && !cancelled;
        Ok(TurnExecutionOutcome {
            thread_id: session_id.clone(),
            turn_id: "turn-1".into(),
            session_id: format!("{session_id}-turn-1"),
            succeeded,
            cancelled,
            error_code: error.as_ref().map(|_| "opencode_session_error".to_string()),
            error_message: error,
            usage,
            thread_usage_total: None,
            usage_estimated: false,
            rate_limits: None,
            assistant_messages: text_order
                .into_iter()
                .filter_map(|id| texts.remove(&id))
                .collect(),
        })
    }
}
