//! Activity-log retention: old events of finished tasks are pruned, nothing else is.

use chrono::{Duration, Utc};
use symphony_core::{AgentEvent, Project, ProviderKind, Task, TaskStatus};
use symphony_storage::Storage;

fn project() -> Project {
    let now = Utc::now();
    Project {
        id: "proj-1".into(),
        name: "Retention".into(),
        repo_path: "C:/repos/retention".into(),
        default_branch: "main".into(),
        workspace_root: "C:/repos/workspaces".into(),
        workflow_path: None,
        preferred_provider: ProviderKind::Codex,
        preferred_model: None,
        tracker_kind: "manual".into(),
        github_repo: None,
        linear_project_slug: None,
        required_labels: vec![],
        auto_dispatch: false,
        max_concurrent_agents: 1,
        validation_commands: vec![],
        created_at: now,
        updated_at: now,
    }
}

async fn task(storage: &Storage, identifier: &str, status: TaskStatus) -> Task {
    let mut task = Task::new_manual(
        "proj-1".into(),
        identifier.into(),
        identifier.into(),
        None,
        None,
        ProviderKind::Codex,
        None,
        vec![],
    );
    task.status = status;
    storage.upsert_task(&task).await.unwrap();
    task
}

fn event(id: &str, task_id: Option<&str>, age_days: i64) -> AgentEvent {
    AgentEvent {
        id: id.into(),
        task_id: task_id.map(str::to_string),
        issue_identifier: None,
        session_id: None,
        provider: None,
        event_type: "message".into(),
        phase: None,
        message: id.into(),
        payload: None,
        timestamp: Utc::now() - Duration::days(age_days),
    }
}

async fn remaining(storage: &Storage) -> Vec<String> {
    let mut ids: Vec<String> = storage
        .list_events(None, 500)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn prunes_old_events_of_finished_tasks_and_keeps_everything_else() {
    let storage = Storage::connect_in_memory().await.unwrap();
    storage.upsert_project(&project()).await.unwrap();
    let done = task(&storage, "SYM-1", TaskStatus::Completed).await;
    let failed = task(&storage, "SYM-2", TaskStatus::Failed).await;
    let cancelled = task(&storage, "SYM-3", TaskStatus::Cancelled).await;
    let running = task(&storage, "SYM-4", TaskStatus::Running).await;
    let review = task(&storage, "SYM-5", TaskStatus::HumanReview).await;

    for e in [
        event("done-old", Some(&done.id), 90),
        event("done-new", Some(&done.id), 1),
        event("failed-old", Some(&failed.id), 45),
        event("cancelled-old", Some(&cancelled.id), 31),
        // Not finished: kept however old.
        event("running-old", Some(&running.id), 400),
        event("review-old", Some(&review.id), 400),
        // No task at all, and a task that was deleted.
        event("orphan-old", None, 60),
        event("gone-old", Some("deleted-task"), 60),
        event("orphan-new", None, 2),
    ] {
        storage.record_event(&e).await.unwrap();
    }

    let removed = storage
        .prune_agent_events(Utc::now() - Duration::days(30))
        .await
        .unwrap();
    assert_eq!(removed, 5);
    assert_eq!(
        remaining(&storage).await,
        vec!["done-new", "orphan-new", "review-old", "running-old"]
    );

    // Idempotent.
    assert_eq!(
        storage
            .prune_agent_events(Utc::now() - Duration::days(30))
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn a_backlog_larger_than_one_batch_is_fully_pruned() {
    let storage = Storage::connect_in_memory().await.unwrap();
    storage.upsert_project(&project()).await.unwrap();
    let done = task(&storage, "SYM-1", TaskStatus::Completed).await;
    for i in 0..4_500 {
        storage
            .record_event(&event(&format!("old-{i}"), Some(&done.id), 100))
            .await
            .unwrap();
    }
    storage
        .record_event(&event("fresh", Some(&done.id), 0))
        .await
        .unwrap();

    let removed = storage
        .prune_agent_events(Utc::now() - Duration::days(7))
        .await
        .unwrap();
    assert_eq!(removed, 4_500);
    assert_eq!(remaining(&storage).await, vec!["fresh"]);
}

#[test]
fn settings_without_the_field_default_to_thirty_days() {
    let old = serde_json::json!({"theme": "dark"});
    let settings: symphony_core::AppSettings = serde_json::from_value(old).unwrap();
    assert_eq!(settings.event_retention_days, 30);
    assert_eq!(settings.theme, "dark");
}
