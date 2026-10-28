use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use symphony_core::domain::{
    ApprovalDecision, ApprovalRequest, ApprovalStatus, ProviderCapabilities, ProviderKind,
    ProviderProfile, TokenUsage,
};
use symphony_core::error::{Result, SymphonyError};
use symphony_core::redaction::redact_secrets;
use symphony_core::usage::{ReportOrigin, ThreadUsageLedger};
use symphony_core::{ToolCallKind, ToolCallStatus, ToolCallUpdate};
use symphony_workspaces::{prepare_tree_command, ProcessTree, SECRET_ENV_VARS_TO_STRIP};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::timeout;

pub mod detection;
mod opencode_server;
pub mod tool_events;
pub use detection::{
    clear_model_catalogues, detect_provider_status, detect_provider_statuses, find_on_path,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLaunchConfig {
    pub provider: ProviderKind,
    pub command: String,
    pub model: Option<String>,
    pub workspace_path: PathBuf,
    pub task_id: String,
    pub issue_identifier: String,
    pub issue_title: String,
    pub approval_policy: Value,
    pub thread_sandbox: Value,
    pub turn_sandbox_policy: Option<Value>,
    pub read_timeout_ms: u64,
    pub turn_timeout_ms: u64,
    pub extra_env: HashMap<String, String>,
    pub dynamic_tools: Vec<Value>,
    pub require_human_approval_for_requests: bool,
    /// Absolute usage of the thread being resumed that earlier attempts already accounted for
    /// (`Task::resume_usage_baseline`); `None` for a new thread or when it was never recorded.
    #[serde(default)]
    pub resume_usage_baseline: Option<TokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStreamEvent {
    pub event_type: String,
    pub timestamp: chrono::DateTime<Utc>,
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub pid: Option<u32>,
    pub message: Option<String>,
    /// Cumulative usage of this agent session so far (all turns since the session started),
    /// never a per-request or per-turn value, so consumers can count the growth between reports
    /// without double-counting repeated reports.
    pub usage: Option<TokenUsage>,
    pub rate_limits: Option<Value>,
    pub raw: Value,
    /// Tool calls this event starts or finishes, normalized for the tool-call history.
    #[serde(default)]
    pub tool_calls: Vec<symphony_core::ToolCallUpdate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnExecutionOutcome {
    pub thread_id: String,
    pub turn_id: String,
    pub session_id: String,
    pub succeeded: bool,
    pub cancelled: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    /// Usage of this turn alone.
    pub usage: TokenUsage,
    /// Absolute usage of the provider thread after this turn, for providers whose usage reports
    /// are thread-wide totals (Codex). Persisted as the baseline for the next resume.
    #[serde(default)]
    pub thread_usage_total: Option<TokenUsage>,
    /// `true` when `usage` may include usage that predates this session (see
    /// `symphony_core::usage::ThreadUsageLedger`).
    #[serde(default)]
    pub usage_estimated: bool,
    pub rate_limits: Option<Value>,
    pub assistant_messages: Vec<String>,
}

/// What a session has consumed so far, readable at any time (also after a failed or interrupted
/// turn that produced no [`TurnExecutionOutcome`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageSnapshot {
    /// Cumulative usage of this session since it started, never including earlier sessions.
    pub session_usage: TokenUsage,
    /// Absolute thread usage, for providers that report thread-wide totals.
    pub thread_total: Option<TokenUsage>,
    pub estimated: bool,
}

#[async_trait]
pub trait DynamicToolHandler: Send + Sync {
    async fn execute_tool(&self, tool_name: &str, arguments: &Value) -> Value;
}

#[async_trait]
pub trait ApprovalGate: Send + Sync {
    async fn request_approval(&self, request: ApprovalRequest) -> Result<ApprovalDecision>;
}

pub struct AutoApprovalGate;

#[async_trait]
impl ApprovalGate for AutoApprovalGate {
    async fn request_approval(&self, _request: ApprovalRequest) -> Result<ApprovalDecision> {
        Ok(ApprovalDecision::AcceptForSession)
    }
}

#[async_trait]
pub trait AgentSession: Send + Sync {
    fn provider_kind(&self) -> ProviderKind;
    fn capabilities(&self) -> ProviderCapabilities;
    fn thread_id(&self) -> Option<String>;
    fn pid(&self) -> Option<u32>;
    fn usage_snapshot(&self) -> UsageSnapshot;

    async fn run_turn(
        &mut self,
        prompt: &str,
        event_tx: mpsc::Sender<AgentStreamEvent>,
    ) -> Result<TurnExecutionOutcome>;

    async fn cancel(&mut self) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
}

pub async fn start_agent_session(
    config: AgentLaunchConfig,
    existing_thread_id: Option<String>,
    tool_handler: Option<Arc<dyn DynamicToolHandler>>,
    approval_gate: Arc<dyn ApprovalGate>,
    event_tx: mpsc::Sender<AgentStreamEvent>,
) -> Result<Box<dyn AgentSession>> {
    // Enforce SPEC.md invariant: workspace_path must exist and be a directory
    if !config.workspace_path.exists() || !config.workspace_path.is_dir() {
        return Err(SymphonyError::InvalidWorkspaceCwd(format!(
            "Agent launch workspace does not exist: {}",
            config.workspace_path.display()
        )));
    }

    match config.provider {
        ProviderKind::Codex => {
            let session = CodexAppServerSession::start(
                config,
                existing_thread_id,
                tool_handler,
                approval_gate,
                event_tx,
            )
            .await?;
            Ok(Box::new(session))
        }
        ProviderKind::ClaudeCode => {
            let session = ClaudeCodeSession::new(config, existing_thread_id, approval_gate);
            Ok(Box::new(session))
        }
        ProviderKind::OpenCode => {
            let session = OpenCodeSession::new(config, existing_thread_id, approval_gate);
            Ok(Box::new(session))
        }
        ProviderKind::Simulated => {
            let session = SimulatedAgentSession::new(config, existing_thread_id)
                .with_approval_gate(approval_gate);
            Ok(Box::new(session))
        }
    }
}

pub fn detect_all_providers() -> Vec<ProviderProfile> {
    vec![
        ProviderProfile {
            id: "codex-default".into(),
            name: "OpenAI Codex App Server".into(),
            provider: ProviderKind::Codex,
            model: None,
            command_override: Some("codex app-server".into()),
            approval_policy: "never".into(),
            sandbox_mode: "workspace-write".into(),
            credential_ref: None,
            is_default: true,
        },
        ProviderProfile {
            id: "claude-code-default".into(),
            name: "Anthropic Claude Code".into(),
            provider: ProviderKind::ClaudeCode,
            model: None,
            command_override: Some("claude".into()),
            approval_policy: "never".into(),
            sandbox_mode: "workspace-write".into(),
            credential_ref: None,
            is_default: false,
        },
        ProviderProfile {
            id: "opencode-default".into(),
            name: "OpenCode CLI".into(),
            provider: ProviderKind::OpenCode,
            model: None,
            command_override: Some("opencode".into()),
            approval_policy: "never".into(),
            sandbox_mode: "workspace-write".into(),
            credential_ref: None,
            is_default: false,
        },
        ProviderProfile {
            id: "simulated-default".into(),
            name: "Simulated Deterministic Agent".into(),
            provider: ProviderKind::Simulated,
            model: Some("symphony-sim-1".into()),
            command_override: None,
            approval_policy: "never".into(),
            sandbox_mode: "workspace-write".into(),
            credential_ref: None,
            is_default: false,
        },
    ]
}

/// Official OpenAI Codex App Server JSON-RPC 2.0 stdio session
pub struct CodexAppServerSession {
    config: AgentLaunchConfig,
    // Declared before `child`: fields drop in order, so the tree is killed while the app-server
    // process is still unreaped (see `ProcessTree::kill`).
    tree: ProcessTree,
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    stdout_rx: mpsc::Receiver<String>,
    thread_id: String,
    current_turn_id: Option<String>,
    /// Separates the thread's historical usage from what this session adds; fed with the absolute
    /// totals of `thread/tokenUsage/updated`.
    usage: ThreadUsageLedger,
    /// Session usage already handed out in earlier turn outcomes. A turn's outcome carries the
    /// growth since then, so usage reported between turns (a replay while resuming, a late report)
    /// lands in the next outcome instead of being lost.
    outcome_usage_reported: TokenUsage,
    pid: Option<u32>,
    next_id: i64,
    tool_handler: Option<Arc<dyn DynamicToolHandler>>,
    approval_gate: Arc<dyn ApprovalGate>,
}

impl CodexAppServerSession {
    pub async fn start(
        config: AgentLaunchConfig,
        existing_thread_id: Option<String>,
        tool_handler: Option<Arc<dyn DynamicToolHandler>>,
        approval_gate: Arc<dyn ApprovalGate>,
        event_tx: mpsc::Sender<AgentStreamEvent>,
    ) -> Result<Self> {
        let mut cmd =
            build_codex_command(&config.command, &config.workspace_path, &config.extra_env);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        prepare_tree_command(&mut cmd);

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                SymphonyError::CodexNotFound(format!(
                    "Failed to launch Codex command '{}': {}",
                    config.command, e
                ))
            } else {
                SymphonyError::AgentProtocolError {
                    code: "port_exit".into(),
                    message: format!("Failed to spawn Codex app-server: {e}"),
                }
            }
        })?;
        let tree = ProcessTree::attach(&child);

        let pid = child.id();
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SymphonyError::AgentProtocolError {
                code: "stdin_unavailable".into(),
                message: "Codex child process stdin was not piped".into(),
            })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SymphonyError::AgentProtocolError {
                code: "stdout_unavailable".into(),
                message: "Codex child process stdout was not piped".into(),
            })?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SymphonyError::AgentProtocolError {
                code: "stderr_unavailable".into(),
                message: "Codex child process stderr was not piped".into(),
            })?;

        let (stdout_tx, stdout_rx) = mpsc::channel::<String>(512);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if stdout_tx.send(line).await.is_err() {
                    break;
                }
            }
        });

        forward_stderr(stderr, pid, event_tx.clone());

        let resume_usage_baseline = config.resume_usage_baseline;
        let mut session = Self {
            config,
            tree,
            child,
            stdin: Arc::new(Mutex::new(stdin)),
            stdout_rx,
            // Replayed usage during `thread/resume` already carries this id.
            thread_id: existing_thread_id.clone().unwrap_or_default(),
            current_turn_id: None,
            usage: if existing_thread_id.is_some() {
                ThreadUsageLedger::resumed(resume_usage_baseline)
            } else {
                ThreadUsageLedger::fresh()
            },
            outcome_usage_reported: TokenUsage::default(),
            pid,
            next_id: 1,
            tool_handler,
            approval_gate,
        };

        session.initialize_protocol(&event_tx).await?;
        let thread_id = session
            .start_or_resume_thread(existing_thread_id.clone(), &event_tx)
            .await?;
        if existing_thread_id.as_deref() != Some(thread_id.as_str()) {
            // The provider answered with a different thread than the one asked for: nothing of
            // the old thread's usage applies to it.
            session.usage = ThreadUsageLedger::fresh();
        }
        session.thread_id = thread_id;

        Ok(session)
    }

    async fn send_json(&self, payload: &Value) -> Result<()> {
        let mut line = serde_json::to_string(payload)?;
        line.push('\n');
        let mut guard = self.stdin.lock().await;
        guard
            .write_all(line.as_bytes())
            .await
            .map_err(|e| SymphonyError::AgentProtocolError {
                code: "stdin_write_failed".into(),
                message: format!("Failed writing to Codex stdin: {e}"),
            })?;
        guard
            .flush()
            .await
            .map_err(|e| SymphonyError::AgentProtocolError {
                code: "stdin_flush_failed".into(),
                message: format!("Failed flushing Codex stdin: {e}"),
            })?;
        Ok(())
    }

    async fn initialize_protocol(
        &mut self,
        event_tx: &mpsc::Sender<AgentStreamEvent>,
    ) -> Result<()> {
        let req_id = self.next_id;
        self.next_id += 1;

        let init_req = json!({
            "method": "initialize",
            "id": req_id,
            "params": {
                "capabilities": {
                    "experimentalApi": true
                },
                "clientInfo": {
                    "name": "symphony-desktop-orchestrator",
                    "title": "Symphony Desktop Orchestrator",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        });

        self.send_json(&init_req).await?;
        let _resp = self.await_response(req_id, event_tx).await?;

        let initialized_notif = json!({
            "method": "initialized",
            "params": {}
        });
        self.send_json(&initialized_notif).await?;
        Ok(())
    }

    async fn start_or_resume_thread(
        &mut self,
        existing_thread_id: Option<String>,
        event_tx: &mpsc::Sender<AgentStreamEvent>,
    ) -> Result<String> {
        let req_id = self.next_id;
        self.next_id += 1;

        let cwd = self.config.workspace_path.to_string_lossy().to_string();
        let req = if let Some(ref tid) = existing_thread_id {
            json!({
                "method": "thread/resume",
                "id": req_id,
                "params": {
                    "threadId": tid
                }
            })
        } else {
            let mut params = json!({
                "approvalPolicy": self.config.approval_policy,
                "sandbox": self.config.thread_sandbox,
                "cwd": cwd,
            });
            if let Some(ref model) = self.config.model {
                params["model"] = json!(model);
            }
            if !self.config.dynamic_tools.is_empty() {
                params["dynamicTools"] = json!(self.config.dynamic_tools);
            }
            json!({
                "method": "thread/start",
                "id": req_id,
                "params": params
            })
        };

        self.send_json(&req).await?;
        let result = self.await_response(req_id, event_tx).await?;

        let thread_id = result
            .get("thread")
            .and_then(|t| t.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or(existing_thread_id)
            .ok_or_else(|| SymphonyError::AgentProtocolError {
                code: "invalid_thread_payload".into(),
                message: format!("Missing thread.id in thread response: {result}"),
            })?;

        Ok(thread_id)
    }

    async fn await_response(
        &mut self,
        expected_id: i64,
        event_tx: &mpsc::Sender<AgentStreamEvent>,
    ) -> Result<Value> {
        let read_timeout = Duration::from_millis(self.config.read_timeout_ms.max(1000));
        loop {
            let line_opt = timeout(read_timeout, self.stdout_rx.recv())
                .await
                .map_err(|_| SymphonyError::AgentProtocolError {
                    code: "response_timeout".into(),
                    message: format!(
                        "Timed out waiting for response ({} ms)",
                        self.config.read_timeout_ms
                    ),
                })?;

            let Some(line) = line_opt else {
                return Err(SymphonyError::AgentProtocolError {
                    code: "port_exit".into(),
                    message: "Codex subprocess closed stdout while awaiting response".into(),
                });
            };

            let parsed: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => {
                    let _ = event_tx
                        .send(AgentStreamEvent {
                            event_type: "malformed".into(),
                            timestamp: Utc::now(),
                            session_id: None,
                            thread_id: Some(self.thread_id.clone()),
                            turn_id: self.current_turn_id.clone(),
                            pid: self.pid,
                            message: Some(line),
                            usage: None,
                            rate_limits: None,
                            raw: Value::Null,
                            tool_calls: Vec::new(),
                        })
                        .await;
                    continue;
                }
            };

            if parsed.get("id").and_then(|v| v.as_i64()) == Some(expected_id)
                && (parsed.get("result").is_some() || parsed.get("error").is_some())
            {
                if let Some(err) = parsed.get("error") {
                    return Err(SymphonyError::AgentProtocolError {
                        code: "response_error".into(),
                        message: err.to_string(),
                    });
                }
                return Ok(parsed.get("result").cloned().unwrap_or(Value::Null));
            }

            // Otherwise handle notification or intermediate request
            self.handle_protocol_message(&parsed, event_tx).await?;
        }
    }

    /// Feed a usage report to the ledger and return this session's cumulative usage.
    ///
    /// `tokenUsage.total` is the absolute usage of the whole thread, and `thread/resume` replays
    /// the persisted total (with the id of the thread's previous turn) before any new turn runs.
    fn observe_usage(&mut self, msg: &Value) -> Option<TokenUsage> {
        let reported = extract_token_usage(msg)?;
        if let Some(reported_thread) = msg.pointer("/params/threadId").and_then(Value::as_str) {
            if !self.thread_id.is_empty() && reported_thread != self.thread_id {
                return None;
            }
        }
        let report_turn = msg.pointer("/params/turnId").and_then(Value::as_str);
        let origin = match (report_turn, self.current_turn_id.as_deref()) {
            (Some(report), Some(live)) if report == live => ReportOrigin::Live,
            (Some(_), Some(_)) | (_, None) => ReportOrigin::Replay,
            (None, Some(_)) => ReportOrigin::Live,
        };
        self.usage.observe(reported, origin);
        Some(self.usage.session_usage())
    }

    async fn handle_protocol_message(
        &mut self,
        msg: &Value,
        event_tx: &mpsc::Sender<AgentStreamEvent>,
    ) -> Result<Option<TurnCompletionSignal>> {
        let method = msg
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("other");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let session_id = self
            .current_turn_id
            .as_ref()
            .map(|turn_id| format!("{}-{}", self.thread_id, turn_id));

        let usage = self.observe_usage(msg);
        let rate_limits = extract_rate_limits(msg);
        let text_message = extract_display_message(method, &params);
        // A host-side tool call keeps one id from its request to the result Symphony answers.
        let dynamic_call_id = (method == "item/tool/call").then(|| {
            tool_events::codex_dynamic_call_id(&params)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
        });
        let tool_calls: Vec<ToolCallUpdate> = match &dynamic_call_id {
            Some(call_id) => vec![ToolCallUpdate {
                call_id: call_id.clone(),
                tool_name: dynamic_tool_name(&params).to_string(),
                kind: ToolCallKind::Dynamic,
                arguments: params.get("arguments").cloned(),
                output: None,
                status: ToolCallStatus::Started,
                duration_ms: None,
            }],
            None => tool_events::codex_item_tool_call(method, &params)
                .into_iter()
                .collect(),
        };

        let _ = event_tx
            .send(AgentStreamEvent {
                event_type: method.to_string(),
                timestamp: Utc::now(),
                session_id: session_id.clone(),
                thread_id: if self.thread_id.is_empty() {
                    None
                } else {
                    Some(self.thread_id.clone())
                },
                turn_id: self.current_turn_id.clone(),
                pid: self.pid,
                message: text_message.clone(),
                usage,
                rate_limits: rate_limits.clone(),
                raw: msg.clone(),
                tool_calls,
            })
            .await;

        // Check if this is a server-to-client request requiring a response
        if let Some(req_id) = msg.get("id") {
            match method {
                "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                    let decision = self
                        .resolve_approval(
                            method,
                            &params,
                            "acceptForSession",
                            "accept",
                            "decline",
                            "cancel",
                        )
                        .await?;
                    self.send_json(&json!({
                        "id": req_id,
                        "result": { "decision": decision }
                    }))
                    .await?;
                    return Ok(None);
                }
                "execCommandApproval" | "applyPatchApproval" => {
                    let decision = self
                        .resolve_approval(
                            method,
                            &params,
                            "approved_for_session",
                            "approved",
                            "denied",
                            "abort",
                        )
                        .await?;
                    self.send_json(&json!({
                        "id": req_id,
                        "result": { "decision": decision }
                    }))
                    .await?;
                    return Ok(None);
                }
                "item/tool/requestUserInput" => {
                    let answers = build_tool_user_input_answers(&params);
                    self.send_json(&json!({
                        "id": req_id,
                        "result": { "answers": answers }
                    }))
                    .await?;
                    return Ok(None);
                }
                "item/tool/call" => {
                    let tool_name = dynamic_tool_name(&params);
                    let arguments = params
                        .get("arguments")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    let started = std::time::Instant::now();

                    let result = if let Some(ref handler) = self.tool_handler {
                        handler.execute_tool(tool_name, &arguments).await
                    } else {
                        let output = json!({
                            "error": {
                                "message": format!("Unsupported dynamic tool call: {}", tool_name)
                            }
                        })
                        .to_string();
                        json!({
                            "success": false,
                            "output": output,
                            "contentItems": [{ "type": "inputText", "text": output }]
                        })
                    };

                    self.send_json(&json!({
                        "id": req_id,
                        "result": result
                    }))
                    .await?;
                    // The answer is not part of the provider's stream: record it for the
                    // tool-call history.
                    let update = tool_events::codex_dynamic_tool_result(
                        dynamic_call_id.unwrap_or_default(),
                        tool_name,
                        &arguments,
                        &result,
                        started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                    );
                    let _ = event_tx
                        .send(AgentStreamEvent {
                            event_type: "tool/result".into(),
                            timestamp: Utc::now(),
                            session_id,
                            thread_id: (!self.thread_id.is_empty()).then(|| self.thread_id.clone()),
                            turn_id: self.current_turn_id.clone(),
                            pid: self.pid,
                            message: Some(format!("Tool {tool_name} answered")),
                            usage: None,
                            rate_limits: None,
                            raw: json!({ "method": "tool/result", "params": {
                                "tool": tool_name, "result": result } }),
                            tool_calls: vec![update],
                        })
                        .await;
                    return Ok(None);
                }
                _ => {}
            }
        }

        match method {
            "turn/completed" => Ok(Some(TurnCompletionSignal::Completed)),
            "turn/failed" => Ok(Some(TurnCompletionSignal::Failed(
                params
                    .get("error")
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "Turn failed".into()),
            ))),
            "turn/cancelled" => Ok(Some(TurnCompletionSignal::Cancelled)),
            _ => Ok(None),
        }
    }

    async fn resolve_approval(
        &self,
        kind: &str,
        params: &Value,
        approve_session_str: &str,
        approve_once_str: &str,
        deny_str: &str,
        cancel_str: &str,
    ) -> Result<String> {
        let auto_approve = self.config.approval_policy == json!("never")
            && !self.config.require_human_approval_for_requests;
        if auto_approve {
            return Ok(approve_session_str.to_string());
        }

        let summary = params
            .get("reason")
            .or_else(|| params.get("command"))
            .and_then(|v| v.as_str())
            .unwrap_or(kind)
            .to_string();

        let req = ApprovalRequest {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: self.config.task_id.clone(),
            issue_identifier: self.config.issue_identifier.clone(),
            session_id: format!(
                "{}-{}",
                self.thread_id,
                self.current_turn_id.as_deref().unwrap_or("init")
            ),
            provider: ProviderKind::Codex,
            kind: kind.to_string(),
            title: format!("Approval requested: {kind}"),
            description: summary,
            command_or_diff: params
                .get("command")
                .map(|c| c.to_string())
                .or_else(|| Some(params.to_string())),
            status: ApprovalStatus::Pending,
            decision: None,
            created_at: Utc::now(),
            resolved_at: None,
        };

        let decision = self.approval_gate.request_approval(req).await?;
        let mapped = match decision {
            ApprovalDecision::AcceptForSession => approve_session_str,
            ApprovalDecision::Accept => approve_once_str,
            ApprovalDecision::Decline => deny_str,
            ApprovalDecision::Cancel => cancel_str,
        };
        Ok(mapped.to_string())
    }
}

enum TurnCompletionSignal {
    Completed,
    Failed(String),
    Cancelled,
}

#[async_trait]
impl AgentSession for CodexAppServerSession {
    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::Codex
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderKind::Codex.default_capabilities()
    }

    fn thread_id(&self) -> Option<String> {
        Some(self.thread_id.clone())
    }

    fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn usage_snapshot(&self) -> UsageSnapshot {
        UsageSnapshot {
            session_usage: self.usage.session_usage(),
            thread_total: Some(self.usage.thread_total()),
            estimated: self.usage.is_estimated(),
        }
    }

    async fn run_turn(
        &mut self,
        prompt: &str,
        event_tx: mpsc::Sender<AgentStreamEvent>,
    ) -> Result<TurnExecutionOutcome> {
        let req_id = self.next_id;
        self.next_id += 1;

        let cwd = self.config.workspace_path.to_string_lossy().to_string();
        let title = format!(
            "{}: {}",
            self.config.issue_identifier, self.config.issue_title
        );
        let sandbox_policy = self
            .config
            .turn_sandbox_policy
            .clone()
            .unwrap_or_else(|| json!({ "type": "workspaceWrite" }));

        let turn_req = json!({
            "method": "turn/start",
            "id": req_id,
            "params": {
                "threadId": self.thread_id,
                "input": [
                    {
                        "type": "text",
                        "text": prompt
                    }
                ],
                "cwd": cwd,
                "title": title,
                "approvalPolicy": self.config.approval_policy,
                "sandboxPolicy": sandbox_policy
            }
        });

        self.send_json(&turn_req).await?;
        let result = self.await_response(req_id, &event_tx).await?;
        let turn_id = result
            .get("turn")
            .and_then(|t| t.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("turn-{}", req_id));

        self.current_turn_id = Some(turn_id.clone());
        let session_id = format!("{}-{}", self.thread_id, turn_id);

        let _ = event_tx
            .send(AgentStreamEvent {
                event_type: "session_started".into(),
                timestamp: Utc::now(),
                session_id: Some(session_id.clone()),
                thread_id: Some(self.thread_id.clone()),
                turn_id: Some(turn_id.clone()),
                pid: self.pid,
                message: Some(format!("Started session {}", session_id)),
                usage: None,
                rate_limits: None,
                raw: result,
                tool_calls: Vec::new(),
            })
            .await;

        let mut latest_rate_limits = None;
        let mut assistant_messages = Vec::new();
        let turn_timeout = Duration::from_millis(self.config.turn_timeout_ms.max(5000));

        let loop_res = timeout(turn_timeout, async {
            while let Some(line) = self.stdout_rx.recv().await {
                let parsed: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if let Some(rl) = extract_rate_limits(&parsed) {
                    latest_rate_limits = Some(rl);
                }
                if let Some(msg_text) = extract_assistant_text(&parsed) {
                    assistant_messages.push(msg_text);
                }

                if let Some(signal) = self.handle_protocol_message(&parsed, &event_tx).await? {
                    return Ok::<TurnCompletionSignal, SymphonyError>(signal);
                }
            }
            Err(SymphonyError::AgentProtocolError {
                code: "port_exit".into(),
                message: "Codex subprocess exited before turn completion".into(),
            })
        })
        .await;
        // The session usage covers every earlier turn as well; the outcome reports what was added
        // since the previous outcome.
        let session_usage = self.usage.session_usage();
        let latest_usage = session_usage.since(&self.outcome_usage_reported);
        self.outcome_usage_reported = session_usage;
        let thread_usage_total = Some(self.usage.thread_total());
        let usage_estimated = self.usage.is_estimated();

        match loop_res {
            Ok(Ok(TurnCompletionSignal::Completed)) => Ok(TurnExecutionOutcome {
                thread_id: self.thread_id.clone(),
                turn_id,
                session_id,
                succeeded: true,
                cancelled: false,
                error_code: None,
                error_message: None,
                usage: latest_usage,
                thread_usage_total,
                usage_estimated,
                rate_limits: latest_rate_limits,
                assistant_messages,
            }),
            Ok(Ok(TurnCompletionSignal::Failed(err))) => Ok(TurnExecutionOutcome {
                thread_id: self.thread_id.clone(),
                turn_id,
                session_id,
                succeeded: false,
                cancelled: false,
                error_code: Some("turn_failed".into()),
                error_message: Some(err),
                usage: latest_usage,
                thread_usage_total,
                usage_estimated,
                rate_limits: latest_rate_limits,
                assistant_messages,
            }),
            Ok(Ok(TurnCompletionSignal::Cancelled)) => Ok(TurnExecutionOutcome {
                thread_id: self.thread_id.clone(),
                turn_id,
                session_id,
                succeeded: false,
                cancelled: true,
                error_code: Some("turn_cancelled".into()),
                error_message: Some("Turn was cancelled".into()),
                usage: latest_usage,
                thread_usage_total,
                usage_estimated,
                rate_limits: latest_rate_limits,
                assistant_messages,
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                let _ = self.cancel().await;
                Err(SymphonyError::TurnTimeout(self.config.turn_timeout_ms))
            }
        }
    }

    async fn cancel(&mut self) -> Result<()> {
        if let Some(ref turn_id) = self.current_turn_id {
            let req_id = self.next_id;
            self.next_id += 1;
            let _ = self
                .send_json(&json!({
                    "method": "turn/interrupt",
                    "id": req_id,
                    "params": {
                        "threadId": self.thread_id,
                        "turnId": turn_id
                    }
                }))
                .await;
        }
        self.tree.kill();
        let _ = self.child.kill().await;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.tree.kill();
        let _ = self.child.kill().await;
        Ok(())
    }
}

/// Forward a child's stderr as redacted `agent/stderr` events (and debug logs). Reading it also
/// keeps the pipe drained: an unread stderr pipe fills up and blocks the agent forever. Events are
/// therefore offered without waiting: while the event pump is full, stderr lines reach the log
/// but not the timeline, and the pipe keeps draining.
fn forward_stderr(stderr: ChildStderr, pid: Option<u32>, event_tx: mpsc::Sender<AgentStreamEvent>) {
    use tokio::io::AsyncReadExt;
    // A line longer than this (an agent writing without newlines) is forwarded in pieces, cut only
    // where redacting each piece is as good as redacting the whole line
    // (`redaction_safe_split`: at ASCII whitespace, so no UTF-8 character is cut either).
    const MAX_STDERR_LINE: usize = 16 * 1024;
    // A run this long without a safe split point is replaced by a notice instead of being cut,
    // which keeps memory bounded. The rest of that line is dropped too: emitting it could expose
    // the unredactable tail of a secret that straddled the omitted part.
    const MAX_UNBROKEN: usize = 64 * 1024;

    let emit = move |bytes: &[u8]| {
        let line = String::from_utf8_lossy(bytes);
        let redacted = redact_secrets(line.trim_end_matches(['\r', '\n']), &[]);
        // Under `symphony::` so the default filter (`symphony=debug`) keeps it in the log files.
        tracing::debug!(target: "symphony::agent_stderr", pid = ?pid, "{}", redacted);
        let _ = event_tx.try_send(AgentStreamEvent {
            event_type: "agent/stderr".into(),
            timestamp: Utc::now(),
            session_id: None,
            thread_id: None,
            turn_id: None,
            pid,
            message: Some(redacted.clone()),
            usage: None,
            rate_limits: None,
            raw: json!({ "stderr": redacted }),
            tool_calls: Vec::new(),
        });
    };

    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut pending = Vec::new();
        let mut omitting_rest_of_line = false;
        // Raw bytes, decoded lossily: invalid UTF-8 must not stop the draining.
        while matches!(
            (&mut reader)
                .take(MAX_STDERR_LINE as u64)
                .read_until(b'\n', &mut pending)
                .await,
            Ok(n) if n > 0
        ) {
            if omitting_rest_of_line {
                omitting_rest_of_line = !pending.ends_with(b"\n");
                pending.clear();
            } else if pending.ends_with(b"\n") {
                emit(&pending);
                pending.clear();
            } else if pending.len() >= MAX_STDERR_LINE {
                if let Some(split) = symphony_core::redaction::redaction_safe_split(&pending) {
                    let rest = pending.split_off(split + 1);
                    emit(&pending);
                    pending = rest;
                } else if pending.len() >= MAX_UNBROKEN {
                    emit(
                        format!(
                            "[agent stderr line omitted: {} bytes without whitespace]",
                            pending.len()
                        )
                        .as_bytes(),
                    );
                    pending.clear();
                    omitting_rest_of_line = true;
                }
            }
        }
        if !pending.is_empty() {
            emit(&pending);
        }
    });
}

/// One turn of a one-shot CLI agent (Claude Code, OpenCode): the process tree, its stdout lines
/// and a drained stderr. Dropping it (e.g. when the orchestrator abandons a cancelled or stalled
/// turn) kills every process the agent started.
struct CliTurnProcess {
    // Declared before `child` so it drops first, while the leader is still unreaped.
    tree: ProcessTree,
    child: Child,
    lines: Lines<BufReader<ChildStdout>>,
    /// Open only for agents that talk to the host while they run (Claude Code's control protocol).
    stdin: Option<ChildStdin>,
    deadline: tokio::time::Instant,
    timeout_ms: u64,
}

impl CliTurnProcess {
    fn spawn(
        cmd: &mut Command,
        config: &AgentLaunchConfig,
        event_tx: &mpsc::Sender<AgentStreamEvent>,
        not_found_code: &str,
        label: &str,
        exe: &str,
    ) -> Result<Self> {
        Self::spawn_with_stdin(cmd, config, event_tx, not_found_code, label, exe, false)
    }

    fn spawn_with_stdin(
        cmd: &mut Command,
        config: &AgentLaunchConfig,
        event_tx: &mpsc::Sender<AgentStreamEvent>,
        not_found_code: &str,
        label: &str,
        exe: &str,
        interactive: bool,
    ) -> Result<Self> {
        cmd.stdin(if interactive {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        prepare_tree_command(cmd);
        let mut child = cmd.spawn().map_err(|e| SymphonyError::AgentProtocolError {
            code: not_found_code.into(),
            message: format!("Failed to spawn {label} ('{exe}'): {e}"),
        })?;
        let tree = ProcessTree::attach(&child);
        let stdin = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SymphonyError::AgentProtocolError {
                code: "stdout_unavailable".into(),
                message: format!("{label} stdout was not piped"),
            })?;
        if let Some(stderr) = child.stderr.take() {
            forward_stderr(stderr, child.id(), event_tx.clone());
        }
        // Same floor as the Codex driver.
        let turn_timeout = Duration::from_millis(config.turn_timeout_ms.max(5000));
        Ok(Self {
            tree,
            child,
            lines: BufReader::new(stdout).lines(),
            stdin,
            deadline: tokio::time::Instant::now() + turn_timeout,
            timeout_ms: config.turn_timeout_ms,
        })
    }

    fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// Next stdout line; `Ok(None)` at end of output. Fails (and kills the tree) once
    /// `turn_timeout_ms` has elapsed since the turn started, or when stdout cannot be read (a
    /// broken stream must not pass for a complete, successful turn).
    async fn next_line(&mut self) -> Result<Option<String>> {
        match tokio::time::timeout_at(self.deadline, self.lines.next_line()).await {
            Ok(Ok(line)) => Ok(line),
            Ok(Err(err)) => {
                self.tree.kill();
                Err(SymphonyError::AgentProtocolError {
                    code: "stdout_read_failed".into(),
                    message: format!("Could not read the agent's output: {err}"),
                })
            }
            Err(_) => {
                self.tree.kill();
                Err(SymphonyError::TurnTimeout(self.timeout_ms))
            }
        }
    }

    /// Write one JSON line to the agent's stdin, bounded by the turn deadline. A closed input is not
    /// an error of its own (see below).
    async fn write_json(&mut self, message: &Value) -> Result<()> {
        let Some(stdin) = self.stdin.as_mut() else {
            // Input already closed: nothing can be delivered any more.
            return Ok(());
        };
        let mut line = message.to_string();
        line.push('\n');
        let write = async {
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await
        };
        match tokio::time::timeout_at(self.deadline, write).await {
            Ok(Ok(())) => Ok(()),
            // The agent closed its input (it exited, or stopped reading): what it printed and
            // its exit status decide the turn, not this write.
            Ok(Err(err)) => {
                tracing::debug!("agent input closed: {err}");
                self.stdin = None;
                Ok(())
            }
            Err(_) => {
                self.tree.kill();
                Err(SymphonyError::TurnTimeout(self.timeout_ms))
            }
        }
    }

    /// End of input: the agent finishes and exits once its current work is done.
    fn close_stdin(&mut self) {
        self.stdin = None;
    }

    /// Forward an event, bounded by the same deadline: a consumer that stops reading must not keep
    /// the turn (and its agent) alive past `turn_timeout_ms`. A closed receiver is ignored.
    async fn send(
        &mut self,
        event_tx: &mpsc::Sender<AgentStreamEvent>,
        event: AgentStreamEvent,
    ) -> Result<()> {
        match tokio::time::timeout_at(self.deadline, event_tx.send(event)).await {
            Ok(_) => Ok(()),
            Err(_) => {
                self.tree.kill();
                Err(SymphonyError::TurnTimeout(self.timeout_ms))
            }
        }
    }

    /// Wait for the agent to exit (still bounded by the turn timeout); tools it left running in
    /// the background are ended with it.
    async fn wait(mut self) -> Result<std::process::ExitStatus> {
        match tokio::time::timeout_at(self.deadline, self.tree.wait(&mut self.child)).await {
            Ok(status) => Ok(status?),
            Err(_) => {
                self.tree.kill();
                Err(SymphonyError::TurnTimeout(self.timeout_ms))
            }
        }
    }
}

/// Provider 2: Claude Code CLI in its host mode (`claude -p --input-format stream-json
/// --output-format stream-json --permission-prompt-tool stdio`): every tool use that Claude Code's
/// own permission rules would ask about arrives as a `can_use_tool` control request and is decided
/// by Symphony's approval gate. Without a host, a headless run silently denies such tool uses.
pub struct ClaudeCodeSession {
    config: AgentLaunchConfig,
    session_id: Option<String>,
    pid: Option<u32>,
    cancelled: Arc<Mutex<bool>>,
    /// Usage of the finished turns; each CLI run reports only its own.
    completed_usage: TokenUsage,
    approval_gate: Arc<dyn ApprovalGate>,
    /// Tool uses approved "for this session": the same tool with the same input is not asked
    /// again in later turns.
    session_approvals: std::collections::HashSet<String>,
}

impl ClaudeCodeSession {
    pub fn new(
        config: AgentLaunchConfig,
        existing_session_id: Option<String>,
        approval_gate: Arc<dyn ApprovalGate>,
    ) -> Self {
        Self {
            config,
            session_id: existing_session_id,
            pid: None,
            cancelled: Arc::new(Mutex::new(false)),
            completed_usage: TokenUsage::default(),
            approval_gate,
            session_approvals: std::collections::HashSet::new(),
        }
    }

    /// Decide a `can_use_tool` request; returns the permission result for Claude Code and whether
    /// the operator aborted the turn.
    async fn answer_permission(
        &mut self,
        request: &Value,
        deadline: tokio::time::Instant,
    ) -> Result<(Value, bool)> {
        let key = claude_permission_key(request);
        if self.session_approvals.contains(&key) {
            return Ok((
                claude_permission_result(request, ApprovalDecision::Accept),
                false,
            ));
        }
        // `never` means nobody is asked. Unlike Codex, Claude Code runs without a sandbox, so a
        // request its own permission rules did not already allow is denied rather than approved:
        // the same outcome as a headless run without a host.
        let ask_nobody = self.config.approval_policy == json!("never")
            && !self.config.require_human_approval_for_requests;
        if ask_nobody {
            return Ok((
                json!({
                    "behavior": "deny",
                    "message": "Not allowed by Claude Code's permission rules, and Symphony's approval policy is `never`, so nobody can approve it."
                }),
                false,
            ));
        }
        let approval = claude_approval_request(&self.config, self.session_id.as_deref(), request);
        let decision =
            tokio::time::timeout_at(deadline, self.approval_gate.request_approval(approval))
                .await
                .map_err(|_| SymphonyError::TurnTimeout(self.config.turn_timeout_ms))??;
        if decision == ApprovalDecision::AcceptForSession {
            self.session_approvals.insert(key);
        }
        Ok((
            claude_permission_result(request, decision),
            decision == ApprovalDecision::Cancel,
        ))
    }
}

/// Identity of a tool use for "approve for session": the tool and its exact input.
fn claude_permission_key(request: &Value) -> String {
    format!(
        "{}\u{0}{}",
        request
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        request.get("input").cloned().unwrap_or(Value::Null)
    )
}

const CLAUDE_FILE_TOOLS: &[&str] = &["Edit", "MultiEdit", "Write", "NotebookEdit"];

/// The approval Symphony shows for a Claude Code `can_use_tool` request.
fn claude_approval_request(
    config: &AgentLaunchConfig,
    session_id: Option<&str>,
    request: &Value,
) -> ApprovalRequest {
    let tool = request
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("tool");
    let input = request.get("input").cloned().unwrap_or(Value::Null);
    let kind = if tool == "Bash" {
        "command_execution"
    } else if CLAUDE_FILE_TOOLS.contains(&tool) {
        "file_change"
    } else {
        "tool_use"
    };
    let preview = match input.get("command").and_then(Value::as_str) {
        Some(command) if tool == "Bash" => command.to_string(),
        _ => serde_json::to_string_pretty(&input).unwrap_or_default(),
    };
    const MAX_PREVIEW: usize = 8 * 1024;
    let preview = if preview.len() > MAX_PREVIEW {
        let mut cut = MAX_PREVIEW;
        while !preview.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}\n…", &preview[..cut])
    } else {
        preview
    };
    let description = request
        .get("description")
        .or_else(|| input.get("description"))
        .and_then(Value::as_str)
        .filter(|d| !d.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Claude Code wants to use {tool}"));
    ApprovalRequest {
        id: uuid::Uuid::new_v4().to_string(),
        task_id: config.task_id.clone(),
        issue_identifier: config.issue_identifier.clone(),
        session_id: session_id.unwrap_or("claude-pending").to_string(),
        provider: ProviderKind::ClaudeCode,
        kind: kind.to_string(),
        title: format!("Approval requested: {tool}"),
        description,
        command_or_diff: Some(preview),
        status: ApprovalStatus::Pending,
        decision: None,
        created_at: Utc::now(),
        resolved_at: None,
    }
}

/// Claude Code's permission result for a decision. "For this session" also hands Claude Code its
/// own suggested rules, but only with the `session` destination: Symphony never lets an approval
/// write the user's or the repository's Claude settings.
fn claude_permission_result(request: &Value, decision: ApprovalDecision) -> Value {
    let input = request.get("input").cloned().unwrap_or_else(|| json!({}));
    match decision {
        ApprovalDecision::Accept => json!({ "behavior": "allow", "updatedInput": input }),
        ApprovalDecision::AcceptForSession => {
            let rules: Vec<Value> = request
                .get("permission_suggestions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|s| s.get("type").and_then(Value::as_str) == Some("addRules"))
                .map(|s| {
                    let mut s = s.clone();
                    s["destination"] = json!("session");
                    s
                })
                .collect();
            json!({ "behavior": "allow", "updatedInput": input, "updatedPermissions": rules })
        }
        ApprovalDecision::Decline => {
            json!({ "behavior": "deny", "message": "The operator declined this in Symphony." })
        }
        ApprovalDecision::Cancel => json!({
            "behavior": "deny",
            "message": "The operator aborted the turn in Symphony.",
            "interrupt": true
        }),
    }
}

#[async_trait]
impl AgentSession for ClaudeCodeSession {
    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::ClaudeCode
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderKind::ClaudeCode.default_capabilities()
    }

    fn thread_id(&self) -> Option<String> {
        self.session_id.clone()
    }

    fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn usage_snapshot(&self) -> UsageSnapshot {
        UsageSnapshot {
            session_usage: self.completed_usage,
            thread_total: None,
            estimated: false,
        }
    }

    async fn run_turn(
        &mut self,
        prompt: &str,
        event_tx: mpsc::Sender<AgentStreamEvent>,
    ) -> Result<TurnExecutionOutcome> {
        let exe =
            if self.config.command.trim().is_empty() || self.config.command == "codex app-server" {
                "claude"
            } else {
                self.config.command.trim()
            };

        let mut cmd = Command::new(exe);
        cmd.arg("-p")
            .arg("--input-format")
            .arg("stream-json")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--permission-prompt-tool")
            .arg("stdio")
            .current_dir(&self.config.workspace_path);

        if let Some(ref sid) = self.session_id {
            cmd.arg("--resume").arg(sid);
        }
        if let Some(ref model) = self.config.model {
            cmd.arg("--model").arg(model);
        }
        for key in SECRET_ENV_VARS_TO_STRIP {
            cmd.env_remove(key);
        }
        for (k, v) in &self.config.extra_env {
            cmd.env(k, v);
        }

        let mut process = CliTurnProcess::spawn_with_stdin(
            &mut cmd,
            &self.config,
            &event_tx,
            "claude_code_not_found",
            "Claude Code CLI",
            exe,
            true,
        )?;
        self.pid = process.pid();
        process
            .write_json(&json!({
                "type": "user",
                "message": { "role": "user", "content": prompt }
            }))
            .await?;
        let mut usage = TokenUsage::default();
        let mut messages = Vec::new();
        let mut discovered_session = self
            .session_id
            .clone()
            .unwrap_or_else(|| format!("claude-{}", uuid::Uuid::new_v4()));

        while let Some(line) = process.next_line().await? {
            if let Ok(val) = serde_json::from_str::<Value>(&line) {
                if let Some(sid) = val.get("session_id").and_then(|v| v.as_str()) {
                    discovered_session = sid.to_string();
                }
                match val.get("type").and_then(Value::as_str) {
                    Some("control_request") => {
                        let request_id = val.get("request_id").cloned().unwrap_or(Value::Null);
                        let request = val.get("request").cloned().unwrap_or(Value::Null);
                        let response = if request.get("subtype").and_then(Value::as_str)
                            == Some("can_use_tool")
                        {
                            self.session_id
                                .get_or_insert_with(|| discovered_session.clone());
                            let (result, interrupted) =
                                self.answer_permission(&request, process.deadline).await?;
                            if interrupted {
                                *self.cancelled.lock().await = true;
                            }
                            json!({ "subtype": "success", "request_id": request_id, "response": result })
                        } else {
                            json!({
                                "subtype": "error",
                                "request_id": request_id,
                                "error": "Symphony does not handle this request"
                            })
                        };
                        process
                            .write_json(
                                &json!({ "type": "control_response", "response": response }),
                            )
                            .await?;
                    }
                    // The answer is complete: end the input so the CLI exits.
                    Some("result") => process.close_stdin(),
                    _ => {}
                }
                if let Some(u) = extract_claude_result_usage(&val) {
                    usage = u;
                }
                let msg_type = val
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("claude/event");
                let text = val
                    .pointer("/message/content/0/text")
                    .or_else(|| val.get("result"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if let Some(ref t) = text {
                    messages.push(t.clone());
                }
                process
                    .send(
                        &event_tx,
                        AgentStreamEvent {
                            tool_calls: tool_events::claude_tool_calls(&val),
                            event_type: format!("claude/{}", msg_type),
                            timestamp: Utc::now(),
                            session_id: Some(discovered_session.clone()),
                            thread_id: Some(discovered_session.clone()),
                            turn_id: Some("turn-1".into()),
                            pid: self.pid,
                            message: text,
                            usage: Some(self.completed_usage.plus(&usage)),
                            rate_limits: None,
                            raw: val,
                        },
                    )
                    .await?;
            }
        }

        let status = process.wait().await?;
        self.session_id = Some(discovered_session.clone());
        self.completed_usage.add(&usage);
        let is_cancelled = *self.cancelled.lock().await;

        Ok(TurnExecutionOutcome {
            thread_id: discovered_session.clone(),
            turn_id: "turn-1".into(),
            session_id: format!("{}-turn-1", discovered_session),
            succeeded: status.success() && !is_cancelled,
            cancelled: is_cancelled,
            error_code: if status.success() {
                None
            } else {
                Some("claude_exit_failure".into())
            },
            error_message: if status.success() {
                None
            } else {
                Some(format!("Claude Code exited with status {}", status))
            },
            usage,
            thread_usage_total: None,
            usage_estimated: false,
            rate_limits: None,
            assistant_messages: messages,
        })
    }

    async fn cancel(&mut self) -> Result<()> {
        *self.cancelled.lock().await = true;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Provider 3: OpenCode CLI (`opencode run --format json`)
pub struct OpenCodeSession {
    config: AgentLaunchConfig,
    session_id: Option<String>,
    pid: Option<u32>,
    cancelled: Arc<Mutex<bool>>,
    /// Usage of the finished turns; each CLI run reports only its own.
    completed_usage: TokenUsage,
    approval_gate: Arc<dyn ApprovalGate>,
}

impl OpenCodeSession {
    pub fn new(
        config: AgentLaunchConfig,
        existing_session_id: Option<String>,
        approval_gate: Arc<dyn ApprovalGate>,
    ) -> Self {
        Self {
            config,
            session_id: existing_session_id,
            pid: None,
            cancelled: Arc::new(Mutex::new(false)),
            completed_usage: TokenUsage::default(),
            approval_gate,
        }
    }

    /// Turns under an approval policy run through Symphony's own `opencode serve`, so permission
    /// requests reach the approval gate; with `never`, `opencode run` and the user's own OpenCode
    /// permissions decide, as before.
    fn supervised(&self) -> bool {
        self.config.approval_policy != json!("never")
            || self.config.require_human_approval_for_requests
    }
}

#[async_trait]
impl AgentSession for OpenCodeSession {
    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::OpenCode
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderKind::OpenCode.default_capabilities()
    }

    fn thread_id(&self) -> Option<String> {
        self.session_id.clone()
    }

    fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn usage_snapshot(&self) -> UsageSnapshot {
        UsageSnapshot {
            session_usage: self.completed_usage,
            thread_total: None,
            estimated: false,
        }
    }

    async fn run_turn(
        &mut self,
        prompt: &str,
        event_tx: mpsc::Sender<AgentStreamEvent>,
    ) -> Result<TurnExecutionOutcome> {
        let exe =
            if self.config.command.trim().is_empty() || self.config.command == "codex app-server" {
                "opencode"
            } else {
                self.config.command.trim()
            };
        if self.supervised() {
            let exe = exe.to_string();
            return self.run_server_turn(&exe, prompt, event_tx).await;
        }

        let mut cmd = Command::new(exe);
        cmd.arg("run")
            .arg("--format")
            .arg("json")
            .arg(prompt)
            .current_dir(&self.config.workspace_path);

        if let Some(ref sid) = self.session_id {
            cmd.arg("--session").arg(sid);
        }
        if let Some(ref model) = self.config.model {
            cmd.arg("--model").arg(model);
        }
        for key in SECRET_ENV_VARS_TO_STRIP {
            cmd.env_remove(key);
        }
        for (k, v) in &self.config.extra_env {
            cmd.env(k, v);
        }

        let mut process = CliTurnProcess::spawn(
            &mut cmd,
            &self.config,
            &event_tx,
            "opencode_not_found",
            "OpenCode CLI",
            exe,
        )?;
        self.pid = process.pid();
        let mut usage = TokenUsage::default();
        let mut messages = Vec::new();
        let mut discovered_session = self
            .session_id
            .clone()
            .unwrap_or_else(|| format!("opencode-{}", uuid::Uuid::new_v4()));

        while let Some(line) = process.next_line().await? {
            if let Ok(val) = serde_json::from_str::<Value>(&line) {
                if let Some(sid) = val
                    .get("sessionID")
                    .or_else(|| val.get("session_id"))
                    .and_then(|v| v.as_str())
                {
                    discovered_session = sid.to_string();
                }
                if val.get("type").and_then(Value::as_str) == Some("step_finish") {
                    // `opencode run --format json` reports each step's tokens on its own.
                    if let Some(step) = val
                        .get("part")
                        .and_then(opencode_server::opencode_step_usage)
                    {
                        usage.add(&step);
                    }
                }
                let text = val
                    .get("text")
                    .or_else(|| val.pointer("/part/text"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if let Some(ref t) = text {
                    messages.push(t.clone());
                }
                process
                    .send(
                        &event_tx,
                        AgentStreamEvent {
                            tool_calls: val
                                .get("part")
                                .and_then(tool_events::opencode_tool_call)
                                .into_iter()
                                .collect(),
                            event_type: "opencode/event".into(),
                            timestamp: Utc::now(),
                            session_id: Some(discovered_session.clone()),
                            thread_id: Some(discovered_session.clone()),
                            turn_id: Some("turn-1".into()),
                            pid: self.pid,
                            message: text,
                            usage: Some(self.completed_usage.plus(&usage)),
                            rate_limits: None,
                            raw: val,
                        },
                    )
                    .await?;
            }
        }

        let status = process.wait().await?;
        self.session_id = Some(discovered_session.clone());
        self.completed_usage.add(&usage);
        let is_cancelled = *self.cancelled.lock().await;

        Ok(TurnExecutionOutcome {
            thread_id: discovered_session.clone(),
            turn_id: "turn-1".into(),
            session_id: format!("{}-turn-1", discovered_session),
            succeeded: status.success() && !is_cancelled,
            cancelled: is_cancelled,
            error_code: if status.success() {
                None
            } else {
                Some("opencode_exit_failure".into())
            },
            error_message: if status.success() {
                None
            } else {
                Some(format!("OpenCode exited with status {}", status))
            },
            usage,
            thread_usage_total: None,
            usage_estimated: false,
            rate_limits: None,
            assistant_messages: messages,
        })
    }

    async fn cancel(&mut self) -> Result<()> {
        *self.cancelled.lock().await = true;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Deterministic Simulated Agent Provider for E2E & Automated Acceptance Testing.
///
/// Emits the same structured item events as the Codex App Server (`item/agentMessage`,
/// `item/commandExecution`, `item/fileChange`, `turn/completed`) so the desktop timeline can be
/// exercised end-to-end without paid models. Prompt markers control behaviour:
/// - `SIMULATE_FAILURE` fails the turn.
/// - `SIMULATE_APPROVAL` raises a command-execution approval through the host approval gate and
///   waits for the operator's decision.
///
/// `SYMPHONY_SIMULATED_STEP_MS` (default 150) sets the delay between streamed items.
pub struct SimulatedAgentSession {
    config: AgentLaunchConfig,
    thread_id: String,
    turn_counter: u32,
    /// Unique per session, so the item ids of a retried attempt (whose turn counter starts
    /// again) never collide with an earlier attempt's tool calls.
    item_prefix: String,
    /// Usage of the finished turns, so stream events report session totals.
    completed_usage: TokenUsage,
    cancel_tx: Option<oneshot::Sender<()>>,
    approval_gate: Option<Arc<dyn ApprovalGate>>,
}

impl SimulatedAgentSession {
    pub fn new(config: AgentLaunchConfig, existing_thread_id: Option<String>) -> Self {
        Self {
            config,
            thread_id: existing_thread_id
                .unwrap_or_else(|| format!("thread-sim-{}", uuid::Uuid::new_v4())),
            turn_counter: 0,
            item_prefix: uuid::Uuid::new_v4().simple().to_string(),
            completed_usage: TokenUsage::default(),
            cancel_tx: None,
            approval_gate: None,
        }
    }

    pub fn with_approval_gate(mut self, gate: Arc<dyn ApprovalGate>) -> Self {
        self.approval_gate = Some(gate);
        self
    }

    fn step_delay() -> Duration {
        let ms = std::env::var("SYMPHONY_SIMULATED_STEP_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(150);
        Duration::from_millis(ms)
    }

    fn stream_event(
        &self,
        event_type: &str,
        session_id: &str,
        turn_id: &str,
        message: String,
        usage: Option<TokenUsage>,
        raw: Value,
    ) -> AgentStreamEvent {
        AgentStreamEvent {
            event_type: event_type.into(),
            timestamp: Utc::now(),
            session_id: Some(session_id.to_string()),
            thread_id: Some(self.thread_id.clone()),
            turn_id: Some(turn_id.to_string()),
            pid: Some(std::process::id()),
            message: Some(message),
            usage,
            rate_limits: None,
            // The simulated command and file change are finished Codex-shaped items.
            tool_calls: tool_events::codex_item_tool_call("item/completed", &raw)
                .into_iter()
                .collect(),
            raw,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn outcome(
        &self,
        turn_id: String,
        session_id: String,
        succeeded: bool,
        cancelled: bool,
        error: Option<(&str, String)>,
        usage: TokenUsage,
        messages: Vec<String>,
    ) -> TurnExecutionOutcome {
        TurnExecutionOutcome {
            thread_id: self.thread_id.clone(),
            turn_id,
            session_id,
            succeeded,
            cancelled,
            error_code: error.as_ref().map(|(code, _)| code.to_string()),
            error_message: error.map(|(_, msg)| msg),
            usage,
            thread_usage_total: None,
            usage_estimated: false,
            rate_limits: if succeeded {
                Some(json!({ "requests_remaining": 999 }))
            } else {
                None
            },
            assistant_messages: messages,
        }
    }
}

#[async_trait]
impl AgentSession for SimulatedAgentSession {
    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::Simulated
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderKind::Simulated.default_capabilities()
    }

    fn thread_id(&self) -> Option<String> {
        Some(self.thread_id.clone())
    }

    fn pid(&self) -> Option<u32> {
        Some(std::process::id())
    }

    fn usage_snapshot(&self) -> UsageSnapshot {
        UsageSnapshot {
            session_usage: self.completed_usage,
            thread_total: None,
            estimated: false,
        }
    }

    async fn run_turn(
        &mut self,
        prompt: &str,
        event_tx: mpsc::Sender<AgentStreamEvent>,
    ) -> Result<TurnExecutionOutcome> {
        self.turn_counter += 1;
        let turn_id = format!("turn-{}", self.turn_counter);
        let session_id = format!("{}-{}", self.thread_id, turn_id);
        let (tx, mut rx) = oneshot::channel();
        self.cancel_tx = Some(tx);
        let delay = Self::step_delay();

        let _ = event_tx
            .send(self.stream_event(
                "session_started",
                &session_id,
                &turn_id,
                format!(
                    "Simulated agent started turn for {}",
                    self.config.issue_identifier
                ),
                None,
                json!({ "prompt_len": prompt.len() }),
            ))
            .await;

        if prompt.contains("SIMULATE_FAILURE") {
            let failed_usage = TokenUsage {
                input_tokens: 120,
                output_tokens: 40,
                total_tokens: 160,
            };
            self.completed_usage.add(&failed_usage);
            // Stream the usage like every other turn: runtime totals are credited from events.
            let failed = self.stream_event(
                "turn/failed",
                &session_id,
                &turn_id,
                "Simulated turn failure triggered by prompt".into(),
                Some(self.completed_usage),
                json!({ "status": "failed", "usage": failed_usage }),
            );
            let _ = event_tx.send(failed).await;
            return Ok(self.outcome(
                turn_id,
                session_id,
                false,
                false,
                Some((
                    "simulated_error",
                    "Simulated turn failure triggered by prompt".into(),
                )),
                failed_usage,
                vec!["Encountered simulated failure.".into()],
            ));
        }

        macro_rules! pause_or_cancel {
            () => {
                tokio::time::sleep(delay).await;
                if rx.try_recv().is_ok() {
                    return Ok(self.outcome(
                        turn_id,
                        session_id,
                        false,
                        true,
                        Some(("cancelled", "Cancelled".into())),
                        TokenUsage::default(),
                        Vec::new(),
                    ));
                }
            };
        }

        pause_or_cancel!();
        let plan = format!(
            "Planning changes for **{}**: inspect the workspace, apply the change, then validate.",
            self.config.issue_title
        );
        let _ = event_tx
            .send(self.stream_event(
                "item/agentMessage",
                &session_id,
                &turn_id,
                plan.clone(),
                Some(self.completed_usage.plus(&TokenUsage {
                    input_tokens: 180,
                    output_tokens: 60,
                    total_tokens: 240,
                })),
                json!({ "item": { "type": "agentMessage", "text": plan } }),
            ))
            .await;

        pause_or_cancel!();
        let _ = event_tx
            .send(self.stream_event(
                "item/commandExecution",
                &session_id,
                &turn_id,
                "Ran `git status --short`".into(),
                None,
                json!({ "item": {
                    "type": "commandExecution",
                    "id": format!("{}-{turn_id}-command", self.item_prefix),
                    "durationMs": 40,
                    "command": "git status --short",
                    "cwd": self.config.workspace_path.to_string_lossy(),
                    "status": "completed",
                    "exitCode": 0,
                    "aggregatedOutput": ""
                } }),
            ))
            .await;

        if prompt.contains("SIMULATE_APPROVAL") {
            if let Some(gate) = self.approval_gate.clone() {
                let command = "cargo test --workspace".to_string();
                let request = ApprovalRequest {
                    id: uuid::Uuid::new_v4().to_string(),
                    task_id: self.config.task_id.clone(),
                    issue_identifier: self.config.issue_identifier.clone(),
                    session_id: session_id.clone(),
                    provider: ProviderKind::Simulated,
                    kind: "command_execution".into(),
                    title: "Run validation command".into(),
                    description: "The simulated agent wants to run the workspace test suite."
                        .into(),
                    command_or_diff: Some(command.clone()),
                    status: ApprovalStatus::Pending,
                    created_at: Utc::now(),
                    resolved_at: None,
                    decision: None,
                };
                let decision = gate.request_approval(request).await?;
                let decision_label = match decision {
                    ApprovalDecision::Accept => "accept",
                    ApprovalDecision::AcceptForSession => "accept_for_session",
                    ApprovalDecision::Decline => "decline",
                    ApprovalDecision::Cancel => "cancel",
                };
                let _ = event_tx
                    .send(self.stream_event(
                        "item/commandExecution/approvalResolved",
                        &session_id,
                        &turn_id,
                        format!("Operator decision for `{command}`: {decision_label}"),
                        None,
                        json!({ "command": command, "decision": decision_label }),
                    ))
                    .await;
                if decision == ApprovalDecision::Cancel {
                    return Ok(self.outcome(
                        turn_id,
                        session_id,
                        false,
                        true,
                        Some(("approval_cancelled", "Operator aborted the turn".into())),
                        TokenUsage::default(),
                        Vec::new(),
                    ));
                }
            }
        }

        pause_or_cancel!();
        // Write a deterministic change in the isolated workspace so git diff & validation inspect real changes
        let marker_file = self.config.workspace_path.join("SYMPHONY_TASK_NOTE.md");
        let content = format!(
            "# Task {}\n\nTitle: {}\nExecuted at: {}\n",
            self.config.issue_identifier,
            self.config.issue_title,
            Utc::now().to_rfc3339()
        );
        let _ = tokio::fs::write(&marker_file, &content).await;
        let _ = event_tx
            .send(self.stream_event(
                "item/fileChange",
                &session_id,
                &turn_id,
                "Created `SYMPHONY_TASK_NOTE.md`".into(),
                None,
                json!({ "item": {
                    "type": "fileChange",
                    "id": format!("{}-{turn_id}-file", self.item_prefix),
                    "status": "completed",
                    "changes": [{ "path": "SYMPHONY_TASK_NOTE.md", "kind": "add", "diff": content }]
                } }),
            ))
            .await;

        pause_or_cancel!();
        let usage = TokenUsage {
            input_tokens: 450,
            output_tokens: 210,
            total_tokens: 660,
        };

        let summary_msg = format!(
            "Implemented changes for `{}` ({}) in workspace `{}`.",
            self.config.issue_identifier,
            self.config.issue_title,
            self.config.workspace_path.display()
        );

        let mut completed = self.stream_event(
            "turn/completed",
            &session_id,
            &turn_id,
            summary_msg.clone(),
            Some(self.completed_usage.plus(&usage)),
            json!({ "status": "completed", "usage": usage }),
        );
        self.completed_usage.add(&usage);
        completed.rate_limits = Some(json!({ "requests_remaining": 999 }));
        let _ = event_tx.send(completed).await;

        Ok(self.outcome(
            turn_id,
            session_id,
            true,
            false,
            None,
            usage,
            vec![summary_msg],
        ))
    }

    async fn cancel(&mut self) -> Result<()> {
        if let Some(tx) = self.cancel_tx.take() {
            let _ = tx.send(());
        }
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }
}

fn build_codex_command(
    raw_command: &str,
    workspace_path: &Path,
    extra_env: &HashMap<String, String>,
) -> Command {
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let trimmed = raw_command.trim();
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() == 2 && parts[0].eq_ignore_ascii_case("codex") && parts[1] == "app-server" {
            // On Windows, `codex` may be a global npm `.cmd` shim; invoke via cmd.exe /C safely
            let mut c = Command::new("cmd.exe");
            c.arg("/C").arg("codex").arg("app-server");
            c
        } else {
            let mut c = Command::new("powershell.exe");
            c.arg("-NoLogo")
                .arg("-NoProfile")
                .arg("-NonInteractive")
                .arg("-Command")
                .arg(trimmed);
            c
        }
    };

    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = Command::new("bash");
        c.arg("-lc").arg(raw_command.trim());
        c
    };

    cmd.current_dir(workspace_path);
    for key in SECRET_ENV_VARS_TO_STRIP {
        cmd.env_remove(key);
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd
}

/// Absolute (thread-total) usage of a Codex app-server message: `thread/tokenUsage/updated`'s
/// `tokenUsage.total` or a `total_token_usage` payload. Generic `usage` maps and per-response
/// values (`last`, `last_token_usage`) are deliberately not read: SPEC.md §13.5 says to prefer
/// absolute totals and not to treat a generic `usage` map as cumulative, so a notification that
/// happens to carry one can never move the usage ledger.
pub fn extract_token_usage(msg: &Value) -> Option<TokenUsage> {
    msg.pointer("/params/tokenUsage/total")
        .or_else(|| msg.pointer("/params/msg/payload/info/total_token_usage"))
        .and_then(usage_from_map)
}

/// Usage of a whole Claude Code invocation: the `usage` of its final `result` line. The `usage`
/// that `assistant` lines carry belongs to a single API response and is not a running total.
pub fn extract_claude_result_usage(msg: &Value) -> Option<TokenUsage> {
    if msg.get("type").and_then(Value::as_str) != Some("result") {
        return None;
    }
    msg.get("usage").and_then(usage_from_map)
}

fn usage_from_map(usage_node: &Value) -> Option<TokenUsage> {
    // Anthropic reports prompt-cache traffic next to `input_tokens`, not inside it
    // (`cache_creation_input_tokens`, `cache_read_input_tokens`); all of it is input the model
    // consumed. Codex's `cachedInputTokens` is a subset of `inputTokens` and is not added.
    let input = usage_node
        .get("inputTokens")
        .or_else(|| usage_node.get("input_tokens"))
        .or_else(|| usage_node.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .saturating_add(
            usage_node
                .get("cache_creation_input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        )
        .saturating_add(
            usage_node
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        );
    let output = usage_node
        .get("outputTokens")
        .or_else(|| usage_node.get("output_tokens"))
        .or_else(|| usage_node.get("completion_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let total = usage_node
        .get("totalTokens")
        .or_else(|| usage_node.get("total_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(input + output);

    if input == 0 && output == 0 && total == 0 {
        return None;
    }

    Some(TokenUsage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: total,
    })
}

/// Name of the host-side tool an `item/tool/call` request asks for.
fn dynamic_tool_name(params: &Value) -> &str {
    params
        .get("name")
        .or_else(|| params.get("tool"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

pub fn extract_rate_limits(msg: &Value) -> Option<Value> {
    msg.pointer("/params/rateLimits")
        .or_else(|| msg.pointer("/params/rate_limits"))
        .cloned()
}

fn extract_display_message(method: &str, params: &Value) -> Option<String> {
    if let Some(delta) = params.get("delta").and_then(|v| v.as_str()) {
        return Some(delta.to_string());
    }
    if let Some(msg) = params.get("message").and_then(|v| v.as_str()) {
        return Some(msg.to_string());
    }
    if let Some(text) = params.pointer("/item/text").and_then(|v| v.as_str()) {
        return Some(text.to_string());
    }
    if let Some(cmd) = params.pointer("/item/command").and_then(|v| v.as_str()) {
        return Some(format!("Executing command: {}", cmd));
    }
    Some(format!("Event: {}", method))
}

fn extract_assistant_text(msg: &Value) -> Option<String> {
    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
    if method == "item/completed" {
        if let Some(text) = msg.pointer("/params/item/text").and_then(|v| v.as_str()) {
            return Some(text.to_string());
        }
    }
    None
}

fn build_tool_user_input_answers(params: &Value) -> Value {
    let mut answers = serde_json::Map::new();
    if let Some(questions) = params.get("questions").and_then(|v| v.as_array()) {
        for q in questions {
            if let Some(qid) = q.get("id").and_then(|v| v.as_str()) {
                answers.insert(
                    qid.to_string(),
                    json!({
                        "answers": ["Approve this Session"]
                    }),
                );
            }
        }
    }
    Value::Object(answers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_prompt_cache_tokens_count_as_input_but_codex_cached_tokens_do_not() {
        // Anthropic usage block as found on Claude Code's `result` line.
        let claude = json!({
            "type": "result",
            "usage": {
                "input_tokens": 12,
                "cache_creation_input_tokens": 3000,
                "cache_read_input_tokens": 40000,
                "output_tokens": 800
            }
        });
        let usage = extract_claude_result_usage(&claude).unwrap();
        assert_eq!(usage.input_tokens, 43_012);
        assert_eq!(usage.output_tokens, 800);
        assert_eq!(usage.total_tokens, 43_812);

        // Codex's inputTokens already includes cachedInputTokens.
        let codex = json!({ "params": { "tokenUsage": { "total": {
            "inputTokens": 1000, "cachedInputTokens": 900, "outputTokens": 50, "totalTokens": 1050 } } } });
        let usage = extract_token_usage(&codex).unwrap();
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.total_tokens, 1050);
    }

    #[test]
    fn test_extract_token_usage_codex_format() {
        let msg = json!({
            "method": "thread/tokenUsage/updated",
            "params": {
                "tokenUsage": {
                    "total": {
                        "inputTokens": 1000,
                        "outputTokens": 500,
                        "totalTokens": 1500
                    }
                }
            }
        });
        let usage = extract_token_usage(&msg).expect("should extract token usage");
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.output_tokens, 500);
        assert_eq!(usage.total_tokens, 1500);
        assert!(usage.estimate_cost_usd(None) > 0.0);
    }

    #[test]
    fn generic_usage_maps_are_never_read_as_cumulative_totals() {
        // SPEC.md §13.5: only absolute thread totals count for Codex.
        for msg in [
            json!({ "method": "item/completed", "params": { "usage": { "input_tokens": 10, "output_tokens": 5 } } }),
            json!({ "method": "codex/event", "usage": { "input_tokens": 10, "output_tokens": 5 } }),
            json!({ "method": "thread/tokenUsage/updated", "params": { "tokenUsage": {
                "last": { "inputTokens": 10, "outputTokens": 5, "totalTokens": 15 } } } }),
        ] {
            assert_eq!(extract_token_usage(&msg), None, "{msg}");
        }
        let legacy = json!({ "params": { "msg": { "payload": { "info": { "total_token_usage": {
            "input_tokens": 70, "output_tokens": 30, "total_tokens": 100 } } } } } });
        assert_eq!(extract_token_usage(&legacy).unwrap().total_tokens, 100);

        // Claude: an assistant line's usage is one API response; only the result line totals.
        let assistant = json!({ "type": "assistant", "message": { "usage": { "input_tokens": 5, "output_tokens": 2 } },
            "usage": { "input_tokens": 5, "output_tokens": 2 } });
        assert_eq!(extract_claude_result_usage(&assistant), None);
        let result =
            json!({ "type": "result", "usage": { "input_tokens": 50, "output_tokens": 20 } });
        assert_eq!(
            extract_claude_result_usage(&result).unwrap().total_tokens,
            70
        );
    }
}
