//! Knowledge Base hosting: the orchestrator owns the service, supplies the AI runner (it holds the
//! provider commands, vault credentials and token accounting) and publishes the service's events
//! on the runtime's event stream.

use super::*;
use symphony_knowledge::analysis::{AnalysisOutput, AnalysisRequest, AnalysisRunner, EventSink};
use symphony_knowledge::engine::EngineConfig;
use symphony_knowledge::git::path_match_key;
use symphony_knowledge::model::WorktreeTaskLink;
use symphony_knowledge::service::KnowledgeService;
use symphony_knowledge::timeline::{ProjectRef, TaskLinks};

/// An analysis turn never runs longer than this, whatever the workflow allows an agent.
const ANALYSIS_TURN_TIMEOUT_MS: u64 = 10 * 60 * 1000;

/// Built-in tools of Claude Code, denied for an analysis (a bare tool name denies every use).
const CLAUDE_DENIED_TOOLS: &[&str] = &[
    "Bash",
    "BashOutput",
    "KillShell",
    "Edit",
    "MultiEdit",
    "Write",
    "NotebookEdit",
    "Read",
    "Glob",
    "Grep",
    "LS",
    "WebFetch",
    "WebSearch",
    "Task",
    "Agent",
    "Skill",
    "TodoWrite",
];

/// Ask the provider not to use any built-in tool, through the project configuration it reads from
/// its working directory (an empty scratch directory that exists for this one turn):
///
/// - Claude Code: `.claude/settings.json` with `permissions.deny` for its built-in tools. Deny rules
///   outrank allow rules from the user's own settings, and apply without a trust prompt.
/// - OpenCode: `opencode.json` with `"permission": {"*": "deny"}`.
/// - Codex has no such file; its shell and other native tools are switched off with `-c` flags on
///   its command line ([`codex_analysis_command`]), on top of `thread_sandbox: read-only`.
///
/// These are the providers' own mechanisms, not something Symphony enforces: a tool that comes from
/// the user's own configuration (for example an MCP server) is not covered, which is why the output
/// is also validated and only ever shown as text.
fn write_tool_restrictions(dir: &Path) -> SymphonyResult<()> {
    std::fs::create_dir_all(dir.join(".claude"))?;
    let claude = json!({ "permissions": { "deny": CLAUDE_DENIED_TOOLS } });
    std::fs::write(
        dir.join(".claude").join("settings.json"),
        serde_json::to_vec_pretty(&claude)?,
    )?;
    let opencode = json!({
        "$schema": "https://opencode.ai/config.json",
        "permission": { "*": "deny" }
    });
    std::fs::write(
        dir.join("opencode.json"),
        serde_json::to_vec_pretty(&opencode)?,
    )?;
    Ok(())
}

/// Codex config overrides for an analysis turn: no shell, no web search, no browser, apps, plugins,
/// sub-agents, memories or hooks. Names Codex does not know are ignored by `-c features.<name>`,
/// so the list holds for older and newer versions alike.
const CODEX_ANALYSIS_FLAGS: &[&str] = &[
    "-c features.shell_tool=false",
    "-c features.unified_exec=false",
    "-c features.apps=false",
    "-c features.plugins=false",
    "-c features.browser_use=false",
    "-c features.browser_use_external=false",
    "-c features.computer_use=false",
    "-c features.in_app_browser=false",
    "-c features.image_generation=false",
    "-c features.multi_agent=false",
    "-c features.memories=false",
    "-c features.hooks=false",
    "-c web_search=disabled",
];

/// The program of a command line and the rest of it. The program may be quoted (a path with spaces),
/// optionally after PowerShell's call operator: `& "C:\Program Files\nodejs\codex.cmd" app-server`.
fn split_program(command: &str) -> (&str, &str) {
    let command = command.trim_start();
    let command = command
        .strip_prefix('&')
        .map(str::trim_start)
        .unwrap_or(command);
    match command.chars().next() {
        Some(quote @ ('"' | '\'')) => {
            let inner = &command[1..];
            match inner.split_once(quote) {
                Some((program, rest)) => (program, rest),
                None => (inner, ""),
            }
        }
        _ => command
            .split_once(char::is_whitespace)
            .unwrap_or((command, "")),
    }
}

/// The Codex command for an analysis turn: the configured `codex ... app-server` command with the
/// native tools switched off. A command that is not Codex's own (a wrapper script) is refused: there
/// is no telling whether it passes the flags on, and an analysis must not run with Codex's tools.
fn codex_analysis_command(configured: &str) -> SymphonyResult<String> {
    let trimmed = configured.trim();
    let (program, rest) = split_program(trimmed);
    let program = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let program = program.trim_end_matches(".cmd").trim_end_matches(".exe");
    if program.eq_ignore_ascii_case("codex") && rest.split_whitespace().any(|w| w == "app-server") {
        let command = format!("{trimmed} {}", CODEX_ANALYSIS_FLAGS.join(" "));
        // The Windows launcher hands the command to PowerShell, which takes a quoted path for a
        // string unless the call operator is there; bash would not understand the operator.
        let quoted = matches!(trimmed.chars().next(), Some('"' | '\''));
        Ok(if cfg!(windows) && quoted {
            format!("& {command}")
        } else {
            command
        })
    } else {
        Err(SymphonyError::Conflict(format!(
            "AI analysis with Codex needs the command to be `codex app-server` (it switches off Codex's own tools); `{trimmed}` is something else. Use another provider for this project, or set the Codex command back to `codex app-server`."
        )))
    }
}

/// Answers every approval request with a refusal: an analysis has no use for tools, so anything
/// the provider asks to run is denied.
struct DenyAllGate;

#[async_trait]
impl ApprovalGate for DenyAllGate {
    async fn request_approval(
        &self,
        _request: ApprovalRequest,
    ) -> SymphonyResult<ApprovalDecision> {
        Ok(ApprovalDecision::Decline)
    }
}

/// Runs one prompt through the project's coding-agent provider.
struct KnowledgeRunner {
    /// A clone that does not reference the Knowledge Base service (no ownership cycle).
    orch: SymphonyOrchestrator,
}

#[async_trait]
impl AnalysisRunner for KnowledgeRunner {
    async fn selection(&self, project_id: &str) -> SymphonyResult<(ProviderKind, Option<String>)> {
        let project = self
            .orch
            .storage
            .get_project(project_id)
            .await?
            .ok_or_else(|| SymphonyError::ProjectNotFound(project_id.to_string()))?;
        match project.preferred_provider {
            provider @ (ProviderKind::Codex | ProviderKind::ClaudeCode | ProviderKind::OpenCode) => {
                Ok((provider, project.preferred_model.filter(|m| !m.trim().is_empty())))
            }
            ProviderKind::CodeSandbox => Err(SymphonyError::Conflict(
                "AI analysis runs locally: set the project's preferred provider to Codex, Claude Code or OpenCode".into(),
            )),
            ProviderKind::Simulated => Err(SymphonyError::Conflict(
                "the simulated provider cannot analyse code: set the project's preferred provider to Codex, Claude Code or OpenCode".into(),
            )),
        }
    }

    async fn run(&self, request: AnalysisRequest) -> SymphonyResult<AnalysisOutput> {
        let config = self.orch.memory.read().await.active_config.clone();
        write_tool_restrictions(&request.scratch_dir)?;
        let command = if request.provider == ProviderKind::Codex {
            codex_analysis_command(&config.codex.command)?
        } else {
            config.codex.command.clone()
        };
        let launch = AgentLaunchConfig {
            provider: request.provider,
            command,
            model: request.model.clone(),
            workspace_path: request.scratch_dir.clone(),
            task_id: format!("knowledge:{}", request.project_id),
            issue_identifier: "KB".into(),
            issue_title: "Knowledge Base analysis".into(),
            // Nobody can be asked, nothing may be written and the provider's own tools are denied
            // (see `write_tool_restrictions`): the prompt carries all the material.
            approval_policy: json!("never"),
            thread_sandbox: Value::String("read-only".into()),
            turn_sandbox_policy: Some(json!({ "type": "readOnly" })),
            read_timeout_ms: config.codex.read_timeout_ms,
            turn_timeout_ms: config.codex.turn_timeout_ms.min(ANALYSIS_TURN_TIMEOUT_MS),
            extra_env: self.orch.provider_env_from_vault(),
            dynamic_tools: Vec::new(),
            require_human_approval_for_requests: false,
            resume_usage_baseline: None,
            cloud: None,
        };
        let (stream_tx, mut stream_rx) = mpsc::channel::<AgentStreamEvent>(256);
        // Nothing consumes the stream, but a full channel would stall the provider.
        let drain = tokio::spawn(async move { while stream_rx.recv().await.is_some() {} });
        let mut session =
            start_agent_session(launch, None, None, Arc::new(DenyAllGate), stream_tx.clone())
                .await?;
        // The prompt was already pattern-redacted; here the exact values stored in the vault are
        // removed from it as well, whatever shape they have.
        let prompt = redact_secrets(&request.prompt, &self.orch.secret_store.all_secret_values());
        let outcome = session.run_turn(&prompt, stream_tx.clone()).await;
        let _ = session.stop().await;
        drop(stream_tx);
        let _ = drain.await;
        let outcome = outcome?;
        self.orch
            .credit_live_usage(outcome.usage, request.model.as_deref())
            .await;
        if outcome.cancelled || !outcome.succeeded {
            return Err(SymphonyError::TurnFailed(
                outcome
                    .error_message
                    .unwrap_or_else(|| "the provider did not complete the turn".into()),
            ));
        }
        Ok(AnalysisOutput {
            text: outcome.assistant_messages.join("\n"),
            input_tokens: outcome.usage.input_tokens,
            output_tokens: outcome.usage.output_tokens,
        })
    }
}

/// Publishes Knowledge Base events on the runtime event stream.
struct KnowledgeEvents {
    orch: SymphonyOrchestrator,
}

impl EventSink for KnowledgeEvents {
    fn publish(&self, event_type: &str, project_id: &str, payload: Value) {
        let orch = self.orch.clone();
        let event_type = event_type.to_string();
        let message = format!("{event_type} ({project_id})");
        let mut payload = payload;
        if let Some(object) = payload.as_object_mut() {
            object.insert("project_id".into(), Value::String(project_id.to_string()));
        }
        tokio::spawn(async move {
            orch.emit_event(
                None,
                None,
                None,
                None,
                &event_type,
                None,
                message,
                Some(payload),
            )
            .await;
        });
    }
}

impl SymphonyOrchestrator {
    /// Open the Knowledge Base under `data_dir` and start serving it. Called once by the host
    /// (desktop or CLI) after the orchestrator is built; a runtime that never calls it simply has
    /// no Knowledge Base.
    pub async fn enable_knowledge(&self, data_dir: &Path) -> SymphonyResult<()> {
        let service = KnowledgeService::open(data_dir, EngineConfig::from_env()).await?;
        // The runner and the event sink hold a copy of the orchestrator that does not point back
        // at the service, so dropping the runtime frees everything.
        let mut detached = self.clone();
        detached.knowledge = Arc::new(std::sync::RwLock::new(None));
        service.set_runner(Arc::new(KnowledgeRunner {
            orch: detached.clone(),
        }));
        service.set_event_sink(Arc::new(KnowledgeEvents { orch: detached }));
        *self.knowledge.write().unwrap_or_else(|e| e.into_inner()) = Some(service);
        Ok(())
    }

    /// The Knowledge Base service, or a `Conflict` when the host did not enable it.
    pub fn knowledge(&self) -> SymphonyResult<Arc<KnowledgeService>> {
        self.knowledge
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| {
                SymphonyError::Conflict("the Knowledge Base is not enabled in this runtime".into())
            })
    }

    /// The repository of a registered project, as the Knowledge Base addresses it.
    pub async fn knowledge_project(&self, project_id: &str) -> SymphonyResult<ProjectRef> {
        let project = self
            .storage
            .get_project(project_id)
            .await?
            .ok_or_else(|| SymphonyError::ProjectNotFound(project_id.to_string()))?;
        let repo_path = PathBuf::from(&project.repo_path);
        if !repo_path.is_dir() {
            return Err(SymphonyError::NotFound(format!(
                "the repository folder of project `{}` no longer exists: {}",
                project.name, project.repo_path
            )));
        }
        Ok(ProjectRef {
            id: project.id,
            name: project.name,
            repo_path,
            default_branch: if project.default_branch.trim().is_empty() {
                "main".into()
            } else {
                project.default_branch
            },
        })
    }

    /// Which Symphony task (and live agent session) owns each worktree path of a project.
    pub async fn knowledge_task_links(&self, project_id: &str) -> SymphonyResult<TaskLinks> {
        let tasks = self.storage.list_tasks(Some(project_id)).await?;
        let active: HashMap<String, String> = {
            let mem = self.memory.read().await;
            mem.running
                .iter()
                .map(|(id, h)| (id.clone(), h.session.session_id.clone()))
                .collect()
        };
        Ok(tasks
            .into_iter()
            .filter_map(|task| {
                let path = task.workspace_path.clone().filter(|p| !p.is_empty())?;
                Some((
                    path_match_key(&path),
                    WorktreeTaskLink {
                        active_session_id: active.get(&task.id).cloned(),
                        task_id: task.id,
                        issue_identifier: task.issue.identifier,
                        issue_title: task.issue.title,
                        task_status: task.status,
                        provider: task.provider,
                    },
                ))
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_runs_an_analysis_with_its_native_tools_switched_off() {
        let command = codex_analysis_command("codex app-server").unwrap();
        assert!(command.starts_with("codex app-server "), "{command}");
        for flag in [
            "features.shell_tool=false",
            "features.computer_use=false",
            "web_search=disabled",
        ] {
            assert!(command.contains(flag), "{flag} in {command}");
        }
        // The npm shim and a full path are recognised.
        for configured in [
            r"C:\tools\codex.cmd app-server",
            "/usr/bin/codex app-server",
            // A path with spaces is quoted, with or without PowerShell's call operator.
            r#""C:\Program Files\nodejs\codex.cmd" app-server"#,
            r#"& "C:\Program Files\nodejs\codex.cmd" app-server"#,
            r"& 'C:\Program Files\nodejs\codex.exe' app-server",
        ] {
            assert!(codex_analysis_command(configured)
                .unwrap()
                .contains("shell_tool=false"));
        }
    }

    #[test]
    fn a_quoted_codex_path_is_made_invokable_for_powershell_only_where_that_is_the_shell() {
        let quoted = r#""C:\Program Files\nodejs\codex.cmd" app-server"#;
        let command = codex_analysis_command(quoted).unwrap();
        if cfg!(windows) {
            // PowerShell would otherwise read the quoted path as a string and not run it.
            assert!(command.starts_with("& \"C:"), "{command}");
        } else {
            assert!(command.starts_with("\"C:"), "{command}");
        }
        // Already written with the operator: not doubled.
        let with_operator = codex_analysis_command(&format!("& {quoted}")).unwrap();
        assert!(with_operator.starts_with("& \"C:"), "{with_operator}");
        assert!(!with_operator.starts_with("& &"), "{with_operator}");
        // An unquoted command is left as it is.
        assert!(codex_analysis_command("codex app-server")
            .unwrap()
            .starts_with("codex app-server "));
    }

    #[test]
    fn a_codex_command_that_is_not_codex_is_refused_for_analysis() {
        // A wrapper might not pass the flags on, so the analysis would run with Codex's tools.
        for wrapper in [
            "  ./run-codex.sh  ",
            "npx codex app-server",
            "",
            r#""C:\Program Files\run-codex.cmd" app-server"#,
            // An unterminated quote is not a program path.
            r#""C:\Program Files\codex.cmd app-server"#,
        ] {
            let error = codex_analysis_command(wrapper).unwrap_err().to_string();
            assert!(error.contains("codex app-server"), "{wrapper:?}: {error}");
        }
    }

    #[test]
    fn the_scratch_directory_denies_every_built_in_tool_of_each_provider() {
        let dir = tempfile::TempDir::new().unwrap();
        write_tool_restrictions(dir.path()).unwrap();

        let claude: Value = serde_json::from_slice(
            &std::fs::read(dir.path().join(".claude").join("settings.json")).unwrap(),
        )
        .unwrap();
        let denied: Vec<&str> = claude["permissions"]["deny"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        for tool in [
            "Bash",
            "Edit",
            "Write",
            "Read",
            "WebFetch",
            "WebSearch",
            "Task",
        ] {
            assert!(denied.contains(&tool), "{tool} must be denied");
        }
        // Nothing is allowed back.
        assert!(claude["permissions"].get("allow").is_none());

        let opencode: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("opencode.json")).unwrap())
                .unwrap();
        assert_eq!(opencode["permission"]["*"], json!("deny"));
        // The directory stays otherwise empty: no repository content is placed in it.
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
        assert_eq!(entries.len(), 2);
    }
}
