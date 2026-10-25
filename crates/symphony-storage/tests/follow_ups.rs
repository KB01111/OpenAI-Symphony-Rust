//! Operator follow-up instructions (`continue_task`) are kept in the database until the attempt
//! that delivers them takes them, so a restart in between does not lose them.

use chrono::Utc;
use symphony_core::{Project, ProviderKind, Task};
use symphony_storage::Storage;

fn project() -> Project {
    let now = Utc::now();
    Project {
        id: "proj-1".into(),
        name: "Follow-ups".into(),
        repo_path: "C:/repos/follow-ups".into(),
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

#[tokio::test]
async fn a_follow_up_is_stored_replaced_and_cleared() {
    let storage = Storage::connect_in_memory().await.unwrap();
    storage.upsert_project(&project()).await.unwrap();
    let t = task(&storage, "SYM-1").await;

    assert_eq!(storage.task_follow_up(&t.id).await.unwrap(), None);
    storage
        .set_task_follow_up(&t.id, "add a test")
        .await
        .unwrap();
    storage
        .set_task_follow_up(&t.id, "add two tests")
        .await
        .unwrap();
    assert_eq!(
        storage.task_follow_up(&t.id).await.unwrap().as_deref(),
        Some("add two tests")
    );
    assert!(storage.clear_task_follow_up(&t.id).await.unwrap());
    assert!(!storage.clear_task_follow_up(&t.id).await.unwrap());
    assert_eq!(storage.task_follow_up(&t.id).await.unwrap(), None);
}

#[tokio::test]
async fn a_follow_up_survives_a_restart_and_goes_with_its_task() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("symphony.sqlite");
    let id = {
        let storage = Storage::connect(&db).await.unwrap();
        storage.upsert_project(&project()).await.unwrap();
        let t = task(&storage, "SYM-2").await;
        storage
            .set_task_follow_up(&t.id, "fix the flag")
            .await
            .unwrap();
        t.id
    };

    let storage = Storage::connect(&db).await.unwrap();
    assert_eq!(
        storage.task_follow_up(&id).await.unwrap().as_deref(),
        Some("fix the flag")
    );
    storage.delete_task(&id).await.unwrap();
    assert_eq!(storage.task_follow_up(&id).await.unwrap(), None);
}
