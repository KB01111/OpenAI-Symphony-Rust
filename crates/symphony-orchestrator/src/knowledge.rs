//! Knowledge Base hosting: the orchestrator owns the service, supplies the AI runner (it holds the
//! provider commands, vault credentials and token accounting) and publishes the service's events
//! on the runtime's event stream.

use super::*;
use symphony_knowledge::analysis::{AnalysisOutput, AnalysisRequest, AnalysisRunner, EventSink};
use symphony_knowledge::engine::EngineConfig;
use symphony_knowledge::git::normalize_path_key;
use symphony_knowledge::model::WorktreeTaskLink;
use symphony_knowledge::service::KnowledgeService;
use symphony_knowledge::timeline::{ProjectRef, TaskLinks};

/// An analysis turn never runs longer than this, whatever the workflow allows an agent.
const ANALYSIS_TURN_TIMEOUT_MS: u64 = 10 * 60 * 1000;

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
        let launch = AgentLaunchConfig {
            provider: request.provider,
            command: config.codex.command.clone(),
            model: request.model.clone(),
            workspace_path: request.scratch_dir.clone(),
            task_id: format!("knowledge:{}", request.project_id),
            issue_identifier: "KB".into(),
            issue_title: "Knowledge Base analysis".into(),
            // Nobody can be asked, and nothing may be written: the prompt carries all the material.
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
        let outcome = session.run_turn(&request.prompt, stream_tx.clone()).await;
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
                    normalize_path_key(&path),
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
