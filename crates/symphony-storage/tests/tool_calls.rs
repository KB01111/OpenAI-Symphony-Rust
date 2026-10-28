//! Tool-call history: one row per call (start and result merged), cursor pages without gaps or
//! repeats, and the activity-log retention rule (finished tasks only).

use chrono::{Duration, TimeZone, Utc};
use serde_json::json;
use symphony_core::{
    Project, ProviderKind, Task, TaskStatus, ToolCallKind, ToolCallRecord, ToolCallStatus,
    ToolCallUpdate,
};
use symphony_storage::Storage;

fn project() -> Project {
    let now = Utc::now();
    Project {
        id: "proj-1".into(),
        name: "Tools".into(),
        repo_path: "C:/repos/tools".into(),
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

fn call(task_id: &str, n: i64, status: ToolCallStatus) -> ToolCallRecord {
    ToolCallRecord::apply(
        None,
        task_id,
        Some("thread-turn"),
        ProviderKind::Codex,
        ToolCallUpdate {
            call_id: format!("call-{n}"),
            tool_name: "shell".into(),
            kind: ToolCallKind::Command,
            arguments: Some(json!({ "command": format!("echo {n}") })),
            output: None,
            status,
            duration_ms: None,
        },
        // Two calls share each second, so the pages must also order by insertion.
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap() + Duration::seconds(n / 2),
    )
}

#[tokio::test]
async fn a_call_is_one_row_and_pages_continue_without_gaps() {
    let temp = tempfile::TempDir::new().unwrap();
    let storage = Storage::connect(temp.path().join("tools.db"))
        .await
        .unwrap();
    storage.upsert_project(&project()).await.unwrap();
    let task = task(&storage, "SYM-1").await;

    for n in 0..7 {
        storage
            .upsert_tool_call(&call(&task.id, n, ToolCallStatus::Started))
            .await
            .unwrap();
    }
    // The result of call 3 replaces its row instead of adding one.
    let started = storage
        .get_tool_call(&ToolCallRecord::record_id(&task.id, "call-3"))
        .await
        .unwrap()
        .unwrap();
    let finished = ToolCallRecord::apply(
        Some(started),
        &task.id,
        None,
        ProviderKind::Codex,
        ToolCallUpdate {
            call_id: "call-3".into(),
            tool_name: String::new(),
            kind: ToolCallKind::Command,
            arguments: None,
            output: Some("3\n".into()),
            status: ToolCallStatus::Succeeded,
            duration_ms: Some(12),
        },
        Utc::now(),
    );
    storage.upsert_tool_call(&finished).await.unwrap();

    let mut seen = Vec::new();
    let mut before: Option<String> = None;
    loop {
        let page = storage
            .list_tool_calls(&task.id, 3, before.as_deref())
            .await
            .unwrap()
            .unwrap();
        if page.is_empty() {
            break;
        }
        before = page.last().map(|c| c.id.clone());
        seen.extend(page);
    }
    let ids: Vec<&str> = seen.iter().map(|c| c.call_id.as_str()).collect();
    assert_eq!(
        ids,
        ["call-6", "call-5", "call-4", "call-3", "call-2", "call-1", "call-0"]
    );
    let three = seen.iter().find(|c| c.call_id == "call-3").unwrap();
    assert_eq!(three.status, ToolCallStatus::Succeeded);
    assert_eq!(three.output.as_deref(), Some("3\n"));
    assert_eq!(three.duration_ms, Some(12));
    assert_eq!(three.tool_name, "shell");
    assert_eq!(three.arguments, Some(json!({ "command": "echo 3" })));

    // A cursor that is not a call of this task is reported, not read as "no more calls".
    assert!(storage
        .list_tool_calls(&task.id, 3, Some("other:call-1"))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn old_calls_of_finished_tasks_are_pruned_and_active_tasks_keep_theirs() {
    let temp = tempfile::TempDir::new().unwrap();
    let storage = Storage::connect(temp.path().join("prune.db"))
        .await
        .unwrap();
    storage.upsert_project(&project()).await.unwrap();
    let mut done = task(&storage, "SYM-DONE").await;
    done.status = TaskStatus::Completed;
    storage.upsert_task(&done).await.unwrap();
    let active = task(&storage, "SYM-ACTIVE").await;

    for task_id in [&done.id, &active.id] {
        storage
            .upsert_tool_call(&call(task_id, 1, ToolCallStatus::Succeeded))
            .await
            .unwrap();
    }
    let removed = storage
        .prune_tool_calls(Utc.with_ymd_and_hms(2026, 10, 2, 0, 0, 0).unwrap())
        .await
        .unwrap();
    assert_eq!(removed, 1);
    let left = |id: String| {
        let storage = storage.clone();
        async move {
            storage
                .list_tool_calls(&id, 10, None)
                .await
                .unwrap()
                .unwrap()
                .len()
        }
    };
    assert_eq!(left(done.id.clone()).await, 0);
    assert_eq!(left(active.id.clone()).await, 1);
}
