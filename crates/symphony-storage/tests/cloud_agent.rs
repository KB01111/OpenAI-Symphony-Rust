//! Which agent runs a CodeSandbox cloud task is stored with the task (migration 0005); tasks that
//! never chose one (and databases from before the column existed) read back as "use the default".

use chrono::Utc;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;
use symphony_core::{AppSettings, CloudAgentKind, Project, ProviderKind, Task};
use symphony_storage::Storage;

fn project() -> Project {
    let now = Utc::now();
    Project {
        id: "proj-1".into(),
        name: "Cloud".into(),
        repo_path: "C:/repos/cloud".into(),
        default_branch: "main".into(),
        workspace_root: "C:/repos/workspaces".into(),
        workflow_path: None,
        preferred_provider: ProviderKind::CodeSandbox,
        preferred_model: None,
        tracker_kind: "manual".into(),
        github_repo: Some("acme/widgets".into()),
        linear_project_slug: None,
        required_labels: vec![],
        auto_dispatch: false,
        max_concurrent_agents: 1,
        validation_commands: vec![],
        created_at: now,
        updated_at: now,
    }
}

fn task(identifier: &str, agent: Option<CloudAgentKind>) -> Task {
    let mut task = Task::new_manual(
        "proj-1".into(),
        identifier.into(),
        identifier.into(),
        None,
        None,
        ProviderKind::CodeSandbox,
        Some("anthropic/claude-sonnet-5-5".into()),
        vec![],
    );
    task.cloud_agent = agent;
    task
}

#[tokio::test]
async fn the_chosen_cloud_agent_round_trips_and_can_be_changed_or_cleared() {
    let storage = Storage::connect_in_memory().await.unwrap();
    storage.upsert_project(&project()).await.unwrap();

    for agent in [
        None,
        Some(CloudAgentKind::MiniSweAgent),
        Some(CloudAgentKind::Pi),
        Some(CloudAgentKind::PiDurable),
    ] {
        let mut t = task("SYM-1", agent);
        t.id = format!("task-{}", agent.map(|a| a.as_str()).unwrap_or("none"));
        t.issue.identifier = format!("SYM-{}", t.id);
        storage.upsert_task(&t).await.unwrap();
        let stored = storage.get_task(&t.id).await.unwrap().unwrap();
        assert_eq!(stored.cloud_agent, agent);
        assert_eq!(stored.provider, ProviderKind::CodeSandbox);
    }

    let mut t = storage.get_task("task-pi_durable").await.unwrap().unwrap();
    t.cloud_agent = Some(CloudAgentKind::Pi);
    storage.upsert_task(&t).await.unwrap();
    assert_eq!(
        storage
            .get_task("task-pi_durable")
            .await
            .unwrap()
            .unwrap()
            .cloud_agent,
        Some(CloudAgentKind::Pi)
    );
    t.cloud_agent = None;
    storage.upsert_task(&t).await.unwrap();
    assert_eq!(
        storage
            .get_task("task-pi_durable")
            .await
            .unwrap()
            .unwrap()
            .cloud_agent,
        None
    );
}

#[tokio::test]
async fn the_default_cloud_agent_is_part_of_the_settings() {
    let storage = Storage::connect_in_memory().await.unwrap();
    assert_eq!(
        storage.get_settings().await.unwrap().default_cloud_agent,
        Some(CloudAgentKind::MiniSweAgent)
    );
    let settings = AppSettings {
        default_cloud_agent: Some(CloudAgentKind::PiDurable),
        ..AppSettings::default()
    };
    storage.save_settings(&settings).await.unwrap();
    assert_eq!(
        storage.get_settings().await.unwrap().default_cloud_agent,
        Some(CloudAgentKind::PiDurable)
    );

    // Settings saved before the field existed have no value; clients read that as "keep".
    let legacy = serde_json::to_value(AppSettings::default()).unwrap();
    let mut legacy = legacy.as_object().unwrap().clone();
    legacy.remove("default_cloud_agent");
    let parsed: AppSettings = serde_json::from_value(legacy.into()).unwrap();
    assert_eq!(parsed.default_cloud_agent, None);
}

#[tokio::test]
async fn a_database_from_before_the_column_gets_it_and_reads_as_default() {
    // A file database as the previous release left it: schema version 4, with a task in it. It is
    // made by creating one with the current code and undoing migration 5.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let storage = Storage::connect(&path).await.unwrap();
    storage.upsert_project(&project()).await.unwrap();
    storage
        .upsert_task(&task("SYM-9", Some(CloudAgentKind::Pi)))
        .await
        .unwrap();
    drop(storage);

    let pool = SqlitePool::connect_with(SqliteConnectOptions::new().filename(&path))
        .await
        .unwrap();
    sqlx::query("ALTER TABLE tasks DROP COLUMN cloud_agent")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM schema_migrations WHERE version = 5")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    // Opening it again applies migration 5; the existing task has no cloud agent.
    let reopened = Storage::connect(&path).await.unwrap();
    let tasks = reopened.list_tasks(None).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].cloud_agent, None);
    assert_eq!(tasks[0].provider, ProviderKind::CodeSandbox);
}
