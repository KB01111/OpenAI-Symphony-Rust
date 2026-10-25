//! Paging through a task's activity: `list_task_events_before` continues exactly where the previous
//! page ended, without skipping or repeating an event.

use chrono::{Duration, TimeZone, Utc};
use symphony_core::{AgentEvent, Project, ProviderKind, Task};
use symphony_storage::Storage;

fn project() -> Project {
    let now = Utc::now();
    Project {
        id: "proj-1".into(),
        name: "Pages".into(),
        repo_path: "C:/repos/pages".into(),
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

async fn task(storage: &Storage, identifier: &str) -> Task {
    let task = Task::new_manual(
        "proj-1".into(),
        identifier.into(),
        identifier.into(),
        None,
        None,
        ProviderKind::Codex,
        None,
        vec![],
    );
    storage.upsert_task(&task).await.unwrap();
    task
}

fn event(id: String, task_id: &str, second: i64) -> AgentEvent {
    AgentEvent {
        id: id.clone(),
        task_id: Some(task_id.to_string()),
        issue_identifier: None,
        session_id: None,
        provider: None,
        event_type: "message".into(),
        phase: None,
        message: id,
        payload: None,
        timestamp: Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap() + Duration::seconds(second),
    }
}

#[tokio::test]
async fn pages_cover_every_event_once_in_order_even_with_equal_timestamps() {
    let storage = Storage::connect_in_memory().await.unwrap();
    storage.upsert_project(&project()).await.unwrap();
    let t = task(&storage, "SYM-1").await;
    let other = task(&storage, "SYM-2").await;
    // 25 events, three per second: page boundaries fall inside a run of equal timestamps.
    for i in 0..25 {
        storage
            .record_event(&event(format!("e{i:02}"), &t.id, i / 3))
            .await
            .unwrap();
    }
    storage
        .record_event(&event("foreign".into(), &other.id, 4))
        .await
        .unwrap();

    let newest = storage.list_events(Some(&t.id), 10).await.unwrap();
    let mut seen: Vec<String> = newest.iter().map(|e| e.id.clone()).collect();
    let mut cursor = newest.last().unwrap().id.clone();
    loop {
        let page = storage
            .list_task_events_before(&t.id, &cursor, 10)
            .await
            .unwrap()
            .expect("cursor is an event of the task");
        if page.is_empty() {
            break;
        }
        cursor = page.last().unwrap().id.clone();
        seen.extend(page.into_iter().map(|e| e.id));
    }
    let expected: Vec<String> = (0..25).rev().map(|i| format!("e{i:02}")).collect();
    assert_eq!(seen, expected);
}

#[tokio::test]
async fn a_cursor_from_another_task_or_unknown_is_refused() {
    let storage = Storage::connect_in_memory().await.unwrap();
    storage.upsert_project(&project()).await.unwrap();
    let t = task(&storage, "SYM-1").await;
    let other = task(&storage, "SYM-2").await;
    storage
        .record_event(&event("theirs".into(), &other.id, 0))
        .await
        .unwrap();
    assert_eq!(
        storage
            .list_task_events_before(&t.id, "theirs", 10)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        storage
            .list_task_events_before(&t.id, "missing", 10)
            .await
            .unwrap(),
        None
    );
}
