//! Provider: CodeSandbox cloud agents.
//!
//! The agent (mini-swe-agent, Pi or Pi Durable) runs inside a CodeSandbox microVM built from a
//! template (`tools/codesandbox-runner/template`). The CodeSandbox SDK is TypeScript-only, so each
//! turn starts the Node bridge (`tools/codesandbox-runner/dist/bridge.mjs`), sends it one JSON
//! request on stdin and reads JSON events from stdout. Credentials (CodeSandbox key, git token,
//! model keys) travel in that request only: never on a command line, in the bridge's environment
//! or in an event.
//!
//! Inside the sandbox the agent commits and pushes to the task branch after every edit; each push
//! arrives here as a `codesandbox/pushed` event, which the orchestrator uses to fast-forward the
//! local worktree.

use super::{
    AgentLaunchConfig, AgentSession, AgentStreamEvent, CliTurnProcess, TurnExecutionOutcome,
    UsageSnapshot,
};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use symphony_core::domain::{CloudAgentKind, ProviderCapabilities, ProviderKind, TokenUsage};
use symphony_core::error::{Result, SymphonyError};
use symphony_core::redaction::register_secret;
use symphony_core::{ToolCallKind, ToolCallStatus, ToolCallUpdate};
use symphony_workspaces::{prepare_tree_command, ProcessTree, SECRET_ENV_VARS_TO_STRIP};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::mpsc;

/// Environment variable that points at the compiled bridge (`bridge.mjs`).
pub const RUNNER_ENV: &str = "SYMPHONY_CODESANDBOX_RUNNER";
/// Credential key for the CodeSandbox API token (create one at https://codesandbox.io/t/api).
pub const CSB_API_KEY: &str = "CSB_API_KEY";
const CONTROL_TIMEOUT: Duration = Duration::from_secs(45);
/// Longest assistant text kept per event.
const MAX_OUTPUT_CHARS: usize = 8 * 1024;

/// Everything a cloud run needs besides the common launch settings.
#[derive(Clone, Serialize, Deserialize)]
pub struct CloudLaunch {
    pub agent: CloudAgentKind,
    pub template_id: String,
    /// `https://` URL of the repository's `origin`, which the sandbox clones and pushes to.
    pub repo_url: String,
    /// Task branch the sandbox pushes to (the branch the local worktree has checked out).
    pub branch: String,
    pub base_branch: String,
    pub vm_tier: String,
    pub hibernation_timeout_secs: u64,
    /// mini-swe-agent spend limit in USD; `0` = none.
    pub cost_limit_usd: f64,
    pub git_user_name: String,
    pub git_user_email: String,
    /// Compiled bridge to run; `None` looks for the bundled one.
    #[serde(default)]
    pub runner_path: Option<String>,
    /// CodeSandbox API token. Never serialized, never logged.
    #[serde(skip)]
    pub csb_api_key: String,
    /// Token the sandbox pushes with. Never serialized, never logged.
    #[serde(skip)]
    pub git_token: String,
}

impl fmt::Debug for CloudLaunch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudLaunch")
            .field("agent", &self.agent)
            .field("template_id", &self.template_id)
            .field("repo_url", &self.repo_url)
            .field("branch", &self.branch)
            .field("base_branch", &self.base_branch)
            .field("vm_tier", &self.vm_tier)
            .field("csb_api_key", &"<redacted>")
            .field("git_token", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// Where the compiled bridge is: the configured path, `SYMPHONY_CODESANDBOX_RUNNER`, a bundled
/// `resources/codesandbox-runner/bridge.mjs` next to the executable, or the source checkout's
/// `tools/codesandbox-runner/dist/bridge.mjs` (searched upwards from the executable and the
/// working directory). A path that was set explicitly is never second-guessed: if it is not a file
/// the answer is `None`, not some other bridge.
pub fn find_runner_bundle(configured: Option<&str>) -> Option<PathBuf> {
    let from_env = std::env::var(RUNNER_ENV).ok();
    for explicit in [configured, from_env.as_deref()].into_iter().flatten() {
        let explicit = explicit.trim();
        if explicit.is_empty() {
            continue;
        }
        let path = PathBuf::from(explicit);
        return path.is_file().then(|| absolute(path));
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    for root in roots {
        for dir in root.ancestors().take(8) {
            for relative in [
                "resources/codesandbox-runner/bridge.mjs",
                "codesandbox-runner/bridge.mjs",
                "tools/codesandbox-runner/dist/bridge.mjs",
            ] {
                let candidate = dir.join(relative);
                if candidate.is_file() {
                    return Some(absolute(candidate));
                }
            }
        }
    }
    None
}

/// The bridge is started with the task's workspace as its working directory, so a path that was
/// relative to wherever Symphony runs must be made absolute first.
fn absolute(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}

fn tool_kind(kind: &str) -> ToolCallKind {
    match kind {
        "command" => ToolCallKind::Command,
        "file_change" => ToolCallKind::FileChange,
        "mcp" => ToolCallKind::Mcp,
        _ => ToolCallKind::ProviderTool,
    }
}

fn tool_status(status: &str) -> ToolCallStatus {
    match status {
        "started" => ToolCallStatus::Started,
        "failed" => ToolCallStatus::Failed,
        _ => ToolCallStatus::Succeeded,
    }
}

fn tool_update(event: &Value) -> Option<ToolCallUpdate> {
    let call_id = event.get("callId").and_then(Value::as_str)?.to_string();
    Some(ToolCallUpdate {
        call_id,
        tool_name: event
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        kind: tool_kind(event.get("kind").and_then(Value::as_str).unwrap_or("")),
        arguments: event.get("input").cloned().filter(|v| !v.is_null()),
        output: event
            .get("output")
            .and_then(Value::as_str)
            .map(str::to_string),
        status: tool_status(event.get("status").and_then(Value::as_str).unwrap_or("")),
        duration_ms: event.get("durationMs").and_then(Value::as_u64),
    })
}

fn cap(text: &str) -> String {
    if text.chars().count() <= MAX_OUTPUT_CHARS {
        text.to_string()
    } else {
        let mut cut: String = text.chars().take(MAX_OUTPUT_CHARS).collect();
        cut.push('…');
        cut
    }
}

/// A cloud agent session: `thread_id` is the CodeSandbox sandbox id, so a retry or follow-up
/// resumes the same sandbox (its files and the agent's checkpoint).
pub struct CodeSandboxSession {
    config: AgentLaunchConfig,
    sandbox_id: Option<String>,
    pid: Option<u32>,
    turn_counter: u32,
    cancelled: bool,
    /// Usage of the finished turns; each turn reports only its own.
    completed_usage: TokenUsage,
    /// What the turn in progress has used so far. Kept on the session (not a local of `run_turn`) so
    /// a turn that ends by timeout, error or cancellation still counts what it consumed.
    turn_usage: TokenUsage,
}

impl CodeSandboxSession {
    pub fn new(config: AgentLaunchConfig, existing_sandbox_id: Option<String>) -> Self {
        if let Some(cloud) = config.cloud.as_ref() {
            register_secret("codesandbox:CSB_API_KEY", &cloud.csb_api_key);
            register_secret("codesandbox:git_token", &cloud.git_token);
        }
        for (key, value) in &config.extra_env {
            register_secret(&format!("codesandbox:env:{key}"), value);
        }
        Self {
            config,
            sandbox_id: existing_sandbox_id,
            pid: None,
            turn_counter: 0,
            cancelled: false,
            completed_usage: TokenUsage::default(),
            turn_usage: TokenUsage::default(),
        }
    }

    fn cloud(&self) -> Result<&CloudLaunch> {
        self.config
            .cloud
            .as_ref()
            .ok_or_else(|| SymphonyError::AgentProtocolError {
                code: "codesandbox_not_configured".into(),
                message: "A CodeSandbox run needs its cloud launch settings".into(),
            })
    }

    fn bridge_command(&self) -> Result<(Command, String)> {
        let cloud = self.cloud()?;
        let runner = find_runner_bundle(cloud.runner_path.as_deref()).ok_or_else(|| {
            SymphonyError::AgentProtocolError {
                code: "codesandbox_runner_not_found".into(),
                message: format!(
                    "The CodeSandbox bridge (bridge.mjs) was not found. Build it with \
                     `pnpm --filter @symphony/codesandbox-runner build` or set {RUNNER_ENV}."
                ),
            }
        })?;
        let node = crate::detection::find_on_path("node").ok_or_else(|| {
            SymphonyError::AgentProtocolError {
                code: "node_not_found".into(),
                message: "Node.js (>= 22) is required to run CodeSandbox cloud agents".into(),
            }
        })?;
        let exe = node.to_string_lossy().to_string();
        let mut cmd = Command::new(&node);
        cmd.arg(&runner).current_dir(&self.config.workspace_path);
        for key in SECRET_ENV_VARS_TO_STRIP {
            cmd.env_remove(key);
        }
        Ok((cmd, exe))
    }

    fn turn_request(&self, prompt: &str, turn_id: &str) -> Result<Value> {
        let cloud = self.cloud()?;
        let mut env: HashMap<&str, &str> = HashMap::new();
        for (key, value) in &self.config.extra_env {
            env.insert(key.as_str(), value.as_str());
        }
        Ok(json!({
            "type": "turn",
            "csbApiKey": cloud.csb_api_key,
            "templateId": cloud.template_id,
            "sandboxId": self.sandbox_id,
            "taskId": self.config.task_id,
            "issueIdentifier": self.config.issue_identifier,
            "turnId": turn_id,
            "prompt": prompt,
            "agent": cloud.agent.as_str(),
            "model": self.config.model,
            "repoUrl": cloud.repo_url,
            "branch": cloud.branch,
            "baseBranch": cloud.base_branch,
            "vmTier": cloud.vm_tier,
            "hibernationTimeoutSecs": cloud.hibernation_timeout_secs,
            "costLimitUsd": cloud.cost_limit_usd,
            "turnTimeoutMs": self.config.turn_timeout_ms,
            "git": {
                "token": cloud.git_token,
                "name": cloud.git_user_name,
                "email": cloud.git_user_email,
            },
            "env": env,
        }))
    }

    /// Short-lived bridge call that acts on an existing sandbox (`abort`, `stop`). Best effort:
    /// failures are logged, never raised, so cleanup cannot mask the turn's own result.
    async fn control(&self, request: Value) {
        let Ok((mut cmd, _)) = self.bridge_command() else {
            return;
        };
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        prepare_tree_command(&mut cmd);
        let Ok(mut child) = cmd.spawn() else {
            return;
        };
        // On Windows the child starts suspended and only runs once it is attached to its tree.
        let mut tree = ProcessTree::attach(&child);
        if let Some(mut stdin) = child.stdin.take() {
            let mut line = request.to_string();
            line.push('\n');
            let _ = stdin.write_all(line.as_bytes()).await;
            let _ = stdin.flush().await;
        }
        match tokio::time::timeout(CONTROL_TIMEOUT, tree.wait(&mut child)).await {
            Err(_) => {
                tracing::warn!("CodeSandbox control call timed out; killing the bridge");
                tree.kill();
                let _ = child.kill().await;
            }
            // The bridge exits non-zero when it could not reach the sandbox (an abort that did not
            // land leaves the agent running), which is worth knowing even though cleanup is best effort.
            Ok(Ok(status)) if !status.success() => {
                tracing::warn!(%status, "CodeSandbox control call did not complete");
            }
            Ok(_) => {}
        }
    }

    fn control_request(&self, kind: &str, mode: Option<&str>) -> Option<Value> {
        let cloud = self.config.cloud.as_ref()?;
        let sandbox_id = self.sandbox_id.as_ref()?;
        Some(json!({
            "type": kind,
            "csbApiKey": cloud.csb_api_key,
            "sandboxId": sandbox_id,
            "mode": mode,
        }))
    }
}

#[async_trait]
impl AgentSession for CodeSandboxSession {
    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::CodeSandbox
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderKind::CodeSandbox.default_capabilities()
    }

    fn thread_id(&self) -> Option<String> {
        self.sandbox_id.clone()
    }

    fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn usage_snapshot(&self) -> UsageSnapshot {
        UsageSnapshot {
            session_usage: self.completed_usage.plus(&self.turn_usage),
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
        // A turn that was cut off earlier (its future dropped) left its usage here: keep it.
        self.completed_usage.add(&self.turn_usage);
        self.turn_usage = TokenUsage::default();
        let turn_id = format!("turn-{}", self.turn_counter);
        let request = self.turn_request(prompt, &turn_id)?;
        let (mut cmd, exe) = self.bridge_command()?;
        let mut process = CliTurnProcess::spawn_with_stdin(
            &mut cmd,
            &self.config,
            &event_tx,
            "codesandbox_bridge_failed",
            "the CodeSandbox bridge",
            &exe,
            true,
        )?;
        self.pid = process.pid();
        process.write_json(&request).await?;

        let mut usage = TokenUsage::default();
        let mut messages: Vec<String> = Vec::new();
        let mut completed: Option<(bool, Option<String>, Option<String>)> = None;
        let mut failure: Option<(String, String)> = None;

        while let Some(line) = process.next_line().await? {
            let Ok(event) = serde_json::from_str::<Value>(&line) else {
                tracing::debug!("CodeSandbox bridge printed a non-JSON line");
                continue;
            };
            let kind = event
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let mut message: Option<String> = None;
            let mut tool_calls = Vec::new();
            let mut event_usage = None;
            match kind.as_str() {
                "started" => {
                    if let Some(id) = event.get("sandboxId").and_then(Value::as_str) {
                        self.sandbox_id = Some(id.to_string());
                    }
                    let resumed = event.get("resumed").and_then(Value::as_bool) == Some(true);
                    message = Some(format!(
                        "{} CodeSandbox sandbox {}",
                        if resumed { "Resumed" } else { "Started" },
                        self.sandbox_id.as_deref().unwrap_or("?")
                    ));
                }
                "output" => {
                    if let Some(text) = event.get("text").and_then(Value::as_str) {
                        let text = cap(text);
                        messages.push(text.clone());
                        message = Some(text);
                    }
                }
                "tool" => {
                    tool_calls.extend(tool_update(&event));
                }
                "usage" => {
                    let input = event
                        .get("inputTokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let output = event
                        .get("outputTokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    // The bridge reports cumulative usage of this turn.
                    usage = usage.max(&TokenUsage {
                        input_tokens: input,
                        output_tokens: output,
                        total_tokens: input.saturating_add(output),
                    });
                    self.turn_usage = usage;
                    event_usage = Some(self.completed_usage.plus(&usage));
                }
                "pushed" => {
                    let sha = event.get("sha").and_then(Value::as_str).unwrap_or("?");
                    message = Some(format!("Pushed {}", &sha[..sha.len().min(12)]));
                }
                "turn_completed" => {
                    completed = Some((
                        event
                            .get("succeeded")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        event
                            .get("errorCode")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        event
                            .get("message")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    ));
                }
                "error" => {
                    failure = Some((
                        event
                            .get("code")
                            .and_then(Value::as_str)
                            .unwrap_or("codesandbox_error")
                            .to_string(),
                        event
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("CodeSandbox bridge error")
                            .to_string(),
                    ));
                    message = failure.as_ref().map(|(_, m)| m.clone());
                }
                _ => {}
            }
            process
                .send(
                    &event_tx,
                    AgentStreamEvent {
                        event_type: format!("codesandbox/{kind}"),
                        timestamp: Utc::now(),
                        session_id: self.sandbox_id.clone(),
                        thread_id: self.sandbox_id.clone(),
                        turn_id: Some(turn_id.clone()),
                        pid: self.pid,
                        message,
                        usage: event_usage,
                        rate_limits: None,
                        raw: event,
                        tool_calls,
                    },
                )
                .await?;
        }

        let status = process.wait().await?;
        self.completed_usage.add(&usage);
        self.turn_usage = TokenUsage::default();
        let cancelled = self.cancelled;
        let sandbox_id = self.sandbox_id.clone().unwrap_or_default();
        let (turn_ok, error_code, error_message) = match (&completed, &failure) {
            (Some((ok, code, msg)), _) => (*ok, code.clone(), msg.clone()),
            (None, Some((code, msg))) => (false, Some(code.clone()), Some(msg.clone())),
            (None, None) => (
                false,
                Some("codesandbox_bridge_exit".into()),
                Some(format!(
                    "The CodeSandbox bridge exited with {status} before the turn finished"
                )),
            ),
        };
        let error_message = error_message.or_else(|| failure.map(|(_, m)| m));
        let succeeded = turn_ok && status.success() && !cancelled;

        Ok(TurnExecutionOutcome {
            thread_id: sandbox_id.clone(),
            turn_id: turn_id.clone(),
            session_id: format!("{sandbox_id}-{turn_id}"),
            succeeded,
            cancelled,
            error_code: if succeeded { None } else { error_code },
            error_message: if succeeded { None } else { error_message },
            usage,
            thread_usage_total: None,
            usage_estimated: false,
            rate_limits: None,
            assistant_messages: messages,
        })
    }

    async fn cancel(&mut self) -> Result<()> {
        self.cancelled = true;
        // Dropping the bridge does not stop the agent inside the sandbox.
        if let Some(request) = self.control_request("abort", None) {
            self.control(request).await;
        }
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(request) = self.control_request("stop", Some("hibernate")) {
            self.control(request).await;
        }
        Ok(())
    }
}
