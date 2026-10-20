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
use symphony_workspaces::SECRET_ENV_VARS_TO_STRIP;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::timeout;

pub mod detection;
pub use detection::{detect_provider_status, detect_provider_statuses, find_on_path};

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
    pub usage: Option<TokenUsage>,
    pub rate_limits: Option<Value>,
    pub raw: Value,
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
    pub usage: TokenUsage,
    pub rate_limits: Option<Value>,
    pub assistant_messages: Vec<String>,
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
            let session = ClaudeCodeSession::new(config, existing_thread_id);
            Ok(Box::new(session))
        }
        ProviderKind::OpenCode => {
            let session = OpenCodeSession::new(config, existing_thread_id);
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
            model: Some("gpt-5-codex".into()),
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
            model: Some("claude-sonnet-4-5".into()),
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
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    stdout_rx: mpsc::Receiver<String>,
    thread_id: String,
    current_turn_id: Option<String>,
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

        let stderr_event_tx = event_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let redacted = redact_secrets(&line, &[]);
                tracing::debug!(target: "codex_stderr", pid = ?pid, "{}", redacted);
                let _ = stderr_event_tx
                    .send(AgentStreamEvent {
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
                    })
                    .await;
            }
        });

        let mut session = Self {
            config,
            child,
            stdin: Arc::new(Mutex::new(stdin)),
            stdout_rx,
            thread_id: String::new(),
            current_turn_id: None,
            pid,
            next_id: 1,
            tool_handler,
            approval_gate,
        };

        session.initialize_protocol(&event_tx).await?;
        let thread_id = session
            .start_or_resume_thread(existing_thread_id, &event_tx)
            .await?;
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

        let usage = extract_token_usage(msg);
        let rate_limits = extract_rate_limits(msg);
        let text_message = extract_display_message(method, &params);

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
                    let tool_name = params
                        .get("name")
                        .or_else(|| params.get("tool"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let arguments = params
                        .get("arguments")
                        .cloned()
                        .unwrap_or_else(|| json!({}));

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
            })
            .await;

        let mut latest_usage = TokenUsage::default();
        let mut latest_rate_limits = None;
        let mut assistant_messages = Vec::new();
        let turn_timeout = Duration::from_millis(self.config.turn_timeout_ms.max(5000));

        let loop_res = timeout(turn_timeout, async {
            while let Some(line) = self.stdout_rx.recv().await {
                let parsed: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if let Some(u) = extract_token_usage(&parsed) {
                    latest_usage = u;
                }
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
        let _ = self.child.kill().await;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        let _ = self.child.kill().await;
        Ok(())
    }
}

/// Provider 2: Claude Code CLI (`claude -p --output-format stream-json --verbose`)
pub struct ClaudeCodeSession {
    config: AgentLaunchConfig,
    session_id: Option<String>,
    pid: Option<u32>,
    cancelled: Arc<Mutex<bool>>,
}

impl ClaudeCodeSession {
    pub fn new(config: AgentLaunchConfig, existing_session_id: Option<String>) -> Self {
        Self {
            config,
            session_id: existing_session_id,
            pid: None,
            cancelled: Arc::new(Mutex::new(false)),
        }
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
            .arg(prompt)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .current_dir(&self.config.workspace_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

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

        let mut child = cmd.spawn().map_err(|e| SymphonyError::AgentProtocolError {
            code: "claude_code_not_found".into(),
            message: format!("Failed to spawn Claude Code CLI ('{exe}'): {e}"),
        })?;
        self.pid = child.id();

        let stdout = child.stdout.take().unwrap();
        let mut lines = BufReader::new(stdout).lines();
        let mut usage = TokenUsage::default();
        let mut messages = Vec::new();
        let mut discovered_session = self
            .session_id
            .clone()
            .unwrap_or_else(|| format!("claude-{}", uuid::Uuid::new_v4()));

        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(val) = serde_json::from_str::<Value>(&line) {
                if let Some(sid) = val.get("session_id").and_then(|v| v.as_str()) {
                    discovered_session = sid.to_string();
                }
                if let Some(u) = extract_token_usage(&val) {
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
                let _ = event_tx
                    .send(AgentStreamEvent {
                        event_type: format!("claude/{}", msg_type),
                        timestamp: Utc::now(),
                        session_id: Some(discovered_session.clone()),
                        thread_id: Some(discovered_session.clone()),
                        turn_id: Some("turn-1".into()),
                        pid: self.pid,
                        message: text,
                        usage: Some(usage),
                        rate_limits: None,
                        raw: val,
                    })
                    .await;
            }
        }

        let status = child.wait().await?;
        self.session_id = Some(discovered_session.clone());
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
}

impl OpenCodeSession {
    pub fn new(config: AgentLaunchConfig, existing_session_id: Option<String>) -> Self {
        Self {
            config,
            session_id: existing_session_id,
            pid: None,
            cancelled: Arc::new(Mutex::new(false)),
        }
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

        let mut cmd = Command::new(exe);
        cmd.arg("run")
            .arg("--format")
            .arg("json")
            .arg(prompt)
            .current_dir(&self.config.workspace_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

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

        let mut child = cmd.spawn().map_err(|e| SymphonyError::AgentProtocolError {
            code: "opencode_not_found".into(),
            message: format!("Failed to spawn OpenCode CLI ('{exe}'): {e}"),
        })?;
        self.pid = child.id();

        let stdout = child.stdout.take().unwrap();
        let mut lines = BufReader::new(stdout).lines();
        let mut usage = TokenUsage::default();
        let mut messages = Vec::new();
        let mut discovered_session = self
            .session_id
            .clone()
            .unwrap_or_else(|| format!("opencode-{}", uuid::Uuid::new_v4()));

        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(val) = serde_json::from_str::<Value>(&line) {
                if let Some(sid) = val
                    .get("sessionID")
                    .or_else(|| val.get("session_id"))
                    .and_then(|v| v.as_str())
                {
                    discovered_session = sid.to_string();
                }
                if let Some(u) = extract_token_usage(&val) {
                    usage = u;
                }
                let text = val
                    .get("text")
                    .or_else(|| val.pointer("/part/text"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if let Some(ref t) = text {
                    messages.push(t.clone());
                }
                let _ = event_tx
                    .send(AgentStreamEvent {
                        event_type: "opencode/event".into(),
                        timestamp: Utc::now(),
                        session_id: Some(discovered_session.clone()),
                        thread_id: Some(discovered_session.clone()),
                        turn_id: Some("turn-1".into()),
                        pid: self.pid,
                        message: text,
                        usage: Some(usage),
                        rate_limits: None,
                        raw: val,
                    })
                    .await;
            }
        }

        let status = child.wait().await?;
        self.session_id = Some(discovered_session.clone());
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
            return Ok(self.outcome(
                turn_id,
                session_id,
                false,
                false,
                Some((
                    "simulated_error",
                    "Simulated turn failure triggered by prompt".into(),
                )),
                TokenUsage {
                    input_tokens: 120,
                    output_tokens: 40,
                    total_tokens: 160,
                },
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
                Some(TokenUsage {
                    input_tokens: 180,
                    output_tokens: 60,
                    total_tokens: 240,
                }),
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
            Some(usage),
            json!({ "status": "completed", "usage": usage }),
        );
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

pub fn extract_token_usage(msg: &Value) -> Option<TokenUsage> {
    let usage_node = msg
        .pointer("/params/tokenUsage/total")
        .or_else(|| msg.pointer("/params/usage"))
        .or_else(|| msg.pointer("/params/msg/payload/info/total_token_usage"))
        .or_else(|| msg.get("usage"))?;

    let input = usage_node
        .get("inputTokens")
        .or_else(|| usage_node.get("input_tokens"))
        .or_else(|| usage_node.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
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
}
