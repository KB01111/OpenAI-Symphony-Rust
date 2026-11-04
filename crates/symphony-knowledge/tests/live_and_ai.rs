//! The service's live updates and its AI flows, with a scripted model in place of a provider.

use async_trait::async_trait;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use symphony_core::{ProviderKind, SymphonyResult};
use symphony_knowledge::analysis::{AnalysisOutput, AnalysisRequest, AnalysisRunner, EventSink};
use symphony_knowledge::engine::EngineConfig;
use symphony_knowledge::model::{ChatRole, JobStatus};
use symphony_knowledge::service::{KnowledgeService, EVENT_ANALYSIS_COMPLETED, EVENT_REPO_CHANGED};
use symphony_knowledge::timeline::ProjectRef;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(dir: &Path, file: &str, content: &str, message: &str) -> String {
    std::fs::write(dir.join(file), content).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<(String, String)>>,
}

impl EventSink for Recorder {
    fn publish(&self, event_type: &str, project_id: &str, _payload: serde_json::Value) {
        self.events
            .lock()
            .unwrap()
            .push((event_type.to_string(), project_id.to_string()));
    }
}

impl Recorder {
    fn count(&self, event_type: &str) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| t == event_type)
            .count()
    }
}

struct ScriptedRunner {
    reply: Mutex<String>,
    calls: AtomicUsize,
    prompts: Mutex<Vec<String>>,
}

#[async_trait]
impl AnalysisRunner for ScriptedRunner {
    async fn selection(&self, _project_id: &str) -> SymphonyResult<(ProviderKind, Option<String>)> {
        Ok((ProviderKind::Codex, Some("scripted".into())))
    }

    async fn run(&self, request: AnalysisRequest) -> SymphonyResult<AnalysisOutput> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // The analysis runs in an empty directory of its own, never in the repository.
        assert!(request.scratch_dir.is_dir());
        assert_eq!(std::fs::read_dir(&request.scratch_dir).unwrap().count(), 0);
        self.prompts.lock().unwrap().push(request.prompt.clone());
        Ok(AnalysisOutput {
            text: self.reply.lock().unwrap().clone(),
            input_tokens: 111,
            output_tokens: 22,
        })
    }
}

async fn wait_until<F, Fut>(what: &str, timeout: Duration, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + timeout;
    while !check().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    repo: std::path::PathBuf,
    project: ProjectRef,
    service: Arc<KnowledgeService>,
    recorder: Arc<Recorder>,
    runner: Arc<ScriptedRunner>,
}

async fn fixture() -> Fixture {
    let temp = tempfile::TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    commit(&repo, "a.txt", "one\n", "initial");
    let service = KnowledgeService::open(&temp.path().join("data"), EngineConfig::default())
        .await
        .unwrap();
    let recorder = Arc::new(Recorder::default());
    service.set_event_sink(recorder.clone());
    let runner = Arc::new(ScriptedRunner {
        reply: Mutex::new(String::new()),
        calls: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
    });
    service.set_runner(runner.clone());
    let project = ProjectRef {
        id: "p1".into(),
        name: "demo".into(),
        repo_path: repo.clone(),
        default_branch: "main".into(),
    };
    Fixture {
        _temp: temp,
        repo,
        project,
        service,
        recorder,
        runner,
    }
}

#[tokio::test]
async fn a_commit_in_any_worktree_produces_a_repo_changed_event() {
    let f = fixture().await;
    f.service.watch_project(&f.project);
    // Let the watcher register and take its baseline before the commit.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    commit(&f.repo, "a.txt", "two\n", "second");
    let recorder = f.recorder.clone();
    wait_until("a repo_changed event", Duration::from_secs(15), || {
        let recorder = recorder.clone();
        async move { recorder.count(EVENT_REPO_CHANGED) > 0 }
    })
    .await;
    // Asking again while watching does not start a second watcher.
    f.service.watch_project(&f.project);
    assert_eq!(f.service.watchers_len(), 1);
}

#[tokio::test]
async fn analysis_is_validated_cached_and_rerun_on_demand() {
    let f = fixture().await;
    let sha = commit(&f.repo, "b.txt", "hello\n", "add hello");
    *f.runner.reply.lock().unwrap() = r#"{
      "summary": "Adds b.txt.",
      "what_changed": [{"title": "New file", "detail": "b.txt added", "severity": "info", "evidence": ["f1"]}],
      "risks": [{"title": "Invented", "detail": "x", "severity": "high", "evidence": ["nope"]}],
      "security": [], "regressions": [],
      "tests": [{"title": "Check b.txt", "rationale": "it is new", "evidence": ["commit"]}],
      "recommendations": []
    }"#
    .to_string();

    let started = f
        .service
        .start_commit_analysis(&f.project, &sha, false)
        .await
        .unwrap();
    assert_eq!(started.status, JobStatus::Running);
    assert_eq!(started.provider, ProviderKind::Codex);
    let service = f.service.clone();
    let project = f.project.clone();
    let full = sha.clone();
    wait_until("the analysis", Duration::from_secs(30), || {
        let (service, project, full) = (service.clone(), project.clone(), full.clone());
        async move {
            service
                .commit_analysis(&project, &full)
                .await
                .unwrap()
                .map(|a| a.status != JobStatus::Running)
                .unwrap_or(false)
        }
    })
    .await;
    let done = f
        .service
        .commit_analysis(&f.project, &sha)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, JobStatus::Completed, "{:?}", done.error);
    let result = done.result.unwrap();
    assert_eq!(result.what_changed.len(), 1);
    assert!(
        result.risks.is_empty(),
        "a finding citing nothing real is dropped"
    );
    assert_eq!(done.dropped_findings, 1);
    assert_eq!(done.input_tokens, 111);
    let cited: Vec<_> = done.evidence.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(cited, vec!["commit", "f1"], "only cited evidence is kept");
    assert!(f.recorder.count(EVENT_ANALYSIS_COMPLETED) >= 1);
    // The model saw the diff and the numbered evidence, nothing from outside the commit.
    let prompt = f.runner.prompts.lock().unwrap()[0].clone();
    assert!(prompt.contains("[f1] b.txt (added") && prompt.contains("+hello"));

    // Same commit, same configuration: served from the cache without calling the model.
    let again = f
        .service
        .start_commit_analysis(&f.project, &sha, false)
        .await
        .unwrap();
    assert_eq!(again.id, done.id);
    assert_eq!(f.runner.calls.load(Ordering::SeqCst), 1);
    // `force` runs it again.
    let forced = f
        .service
        .start_commit_analysis(&f.project, &sha, true)
        .await
        .unwrap();
    assert_ne!(forced.id, done.id);
}

#[tokio::test]
async fn a_failed_model_reply_is_recorded_not_hidden() {
    let f = fixture().await;
    let sha = commit(&f.repo, "c.txt", "x\n", "change");
    *f.runner.reply.lock().unwrap() = "Sorry, I cannot do that.".to_string();
    f.service
        .start_commit_analysis(&f.project, &sha, false)
        .await
        .unwrap();
    let (service, project, full) = (f.service.clone(), f.project.clone(), sha.clone());
    wait_until("the failed analysis", Duration::from_secs(30), || {
        let (service, project, full) = (service.clone(), project.clone(), full.clone());
        async move {
            service
                .commit_analysis(&project, &full)
                .await
                .unwrap()
                .map(|a| a.status != JobStatus::Running)
                .unwrap_or(false)
        }
    })
    .await;
    let failed = f
        .service
        .commit_analysis(&f.project, &sha)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, JobStatus::Failed);
    assert!(failed.error.unwrap().contains("JSON"));
    assert!(failed.result.is_none());
}

#[tokio::test]
async fn chat_answers_cite_retrieved_repository_material_only() {
    let f = fixture().await;
    commit(&f.repo, "greeting.txt", "hello\n", "Add the hello greeting");
    *f.runner.reply.lock().unwrap() =
        r#"{"answer": "The greeting was added in a commit [e1]; also [e9].", "citations": ["e1", "e9"]}"#.to_string();

    let started = f
        .service
        .ask(&f.project, "Where was the hello greeting added?")
        .await
        .unwrap();
    assert_eq!(started.len(), 2);
    assert_eq!(started[0].role, ChatRole::User);
    assert_eq!(started[1].status, JobStatus::Running);
    // A second question while one is being answered is refused.
    assert!(f.service.ask(&f.project, "And another?").await.is_err());

    let (service, project) = (f.service.clone(), f.project.clone());
    wait_until("the answer", Duration::from_secs(30), || {
        let (service, project) = (service.clone(), project.clone());
        async move {
            service
                .chat_messages(&project)
                .await
                .unwrap()
                .last()
                .map(|m| m.status != JobStatus::Running)
                .unwrap_or(false)
        }
    })
    .await;
    let messages = f.service.chat_messages(&f.project).await.unwrap();
    let answer = messages.last().unwrap();
    assert_eq!(answer.status, JobStatus::Completed, "{:?}", answer.error);
    assert!(answer.content.contains("[e1]") && !answer.content.contains("e9"));
    assert_eq!(answer.evidence.len(), 1);
    assert!(answer.evidence[0].label.contains("Add the hello greeting"));
    assert!(answer.evidence[0].sha.is_some());
    // The prompt carried the question and the retrieved commit, marked as data.
    let prompt = f.runner.prompts.lock().unwrap()[0].clone();
    assert!(
        prompt.contains("[e1] commit") && prompt.contains("Where was the hello greeting added?")
    );

    // Over-long questions are refused before anything runs.
    assert!(f.service.ask(&f.project, &"x".repeat(2001)).await.is_err());
    assert!(f.service.ask(&f.project, "   ").await.is_err());
    f.service.clear_chat(&f.project).await.unwrap();
    assert!(f
        .service
        .chat_messages(&f.project)
        .await
        .unwrap()
        .is_empty());
}
