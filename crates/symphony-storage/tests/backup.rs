//! Backup creation, verification and restoration.

use std::path::Path;

use chrono::Utc;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use symphony_core::{Project, ProviderKind, Task};
use symphony_storage::backup::{
    create_backup, list_backups, restore_backup, verify_backup, BackupManifest, DATABASE_FILE,
    MANIFEST_FILE, WORKFLOW_FILE,
};
use symphony_storage::{Storage, SCHEMA_VERSION};

fn project() -> Project {
    let now = Utc::now();
    Project {
        id: "p1".into(),
        name: "Backup".into(),
        repo_path: "C:/repo".into(),
        default_branch: "main".into(),
        workspace_root: "C:/ws".into(),
        workflow_path: None,
        preferred_provider: ProviderKind::Codex,
        preferred_model: None,
        tracker_kind: "manual".into(),
        github_repo: None,
        linear_project_slug: None,
        required_labels: Vec::new(),
        auto_dispatch: false,
        max_concurrent_agents: 1,
        validation_commands: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

async fn storage_with_task(data_dir: &Path, title: &str) -> (Storage, Task) {
    let storage = Storage::connect(data_dir.join(DATABASE_FILE))
        .await
        .unwrap();
    storage.upsert_project(&project()).await.unwrap();
    let task = Task::new_manual(
        "p1".into(),
        "SYM-1".into(),
        title.into(),
        None,
        Some(1),
        ProviderKind::Codex,
        None,
        Vec::new(),
    );
    storage.upsert_task(&task).await.unwrap();
    (storage, task)
}

#[tokio::test]
async fn a_backup_holds_a_consistent_copy_and_a_verifiable_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (storage, task) = storage_with_task(&data, "Original title").await;
    std::fs::write(
        data.join(WORKFLOW_FILE),
        "---\ntracker:\n  kind: github\n---\nprompt",
    )
    .unwrap();

    let info = create_backup(
        &storage,
        &data.join("backups"),
        Some(&data.join(WORKFLOW_FILE)),
    )
    .await
    .unwrap();

    assert!(info.path.join(DATABASE_FILE).is_file());
    assert!(info.includes_workflow);
    assert_eq!(info.schema_version, SCHEMA_VERSION);
    let manifest: BackupManifest =
        serde_json::from_slice(&std::fs::read(info.path.join(MANIFEST_FILE)).unwrap()).unwrap();
    assert!(
        !manifest.credentials_included,
        "credentials never travel in a backup"
    );
    assert!(manifest.files.iter().all(|f| f.name != "credentials.vault"));
    verify_backup(&info.path).await.unwrap();
    assert_eq!(
        list_backups(&data.join("backups")).unwrap(),
        vec![info.clone()]
    );

    // The copy is a self-contained database holding the (WAL-resident) task.
    let copy = Storage::connect(info.path.join(DATABASE_FILE))
        .await
        .unwrap();
    assert_eq!(
        copy.get_task(&task.id).await.unwrap().unwrap().issue.title,
        "Original title"
    );
}

#[tokio::test]
async fn listing_skips_partial_and_unrelated_directories_and_orders_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (storage, _) = storage_with_task(&data, "t").await;
    let root = data.join("backups");
    let first = create_backup(&storage, &root, None).await.unwrap();
    let second = create_backup(&storage, &root, None).await.unwrap();
    assert_ne!(
        first.id, second.id,
        "two backups in the same second get distinct ids"
    );
    std::fs::create_dir_all(root.join(".symphony-backup-x.partial")).unwrap();
    std::fs::create_dir_all(root.join("symphony-backup-no-manifest")).unwrap();
    std::fs::create_dir_all(root.join("unrelated")).unwrap();

    let listed: Vec<String> = list_backups(&root)
        .unwrap()
        .into_iter()
        .map(|b| b.id)
        .collect();
    assert_eq!(listed, vec![second.id, first.id]);
    assert!(list_backups(&dir.path().join("missing"))
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn restore_replaces_the_data_and_keeps_the_previous_files() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (storage, task) = storage_with_task(&data, "Before the mistake").await;
    std::fs::write(data.join(WORKFLOW_FILE), "original workflow").unwrap();
    let backup = create_backup(
        &storage,
        &data.join("backups"),
        Some(&data.join(WORKFLOW_FILE)),
    )
    .await
    .unwrap();

    // The data changes after the backup: a task is renamed and another one added.
    let mut changed = storage.get_task(&task.id).await.unwrap().unwrap();
    changed.issue.title = "After the mistake".into();
    storage.upsert_task(&changed).await.unwrap();
    std::fs::write(data.join(WORKFLOW_FILE), "edited workflow").unwrap();
    storage.pool().close().await; // offline operation: the runtime has stopped

    let report = restore_backup(&backup.path, &data).await.unwrap();

    let restored = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    assert_eq!(
        restored
            .get_task(&task.id)
            .await
            .unwrap()
            .unwrap()
            .issue
            .title,
        "Before the mistake"
    );
    assert_eq!(
        std::fs::read_to_string(data.join(WORKFLOW_FILE)).unwrap(),
        "original workflow"
    );
    // Nothing was deleted: the replaced files are still there, with their later content.
    assert!(!report.previous_files.is_empty());
    let kept_workflow = report
        .previous_files
        .iter()
        .find(|p| p.to_string_lossy().contains("WORKFLOW.md.pre-restore-"))
        .expect("workflow kept");
    assert_eq!(
        std::fs::read_to_string(kept_workflow).unwrap(),
        "edited workflow"
    );
    let kept_db = report
        .previous_files
        .iter()
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("symphony.sqlite.pre-restore-")
        })
        .expect("database kept");
    let previous = Storage::connect(kept_db).await.unwrap();
    assert_eq!(
        previous
            .get_task(&task.id)
            .await
            .unwrap()
            .unwrap()
            .issue
            .title,
        "After the mistake"
    );
}

#[tokio::test]
async fn a_damaged_backup_is_refused_and_the_live_data_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (storage, task) = storage_with_task(&data, "Live").await;
    let backup = create_backup(&storage, &data.join("backups"), None)
        .await
        .unwrap();
    storage.pool().close().await;

    // One flipped byte in the database copy.
    let db = backup.path.join(DATABASE_FILE);
    let mut bytes = std::fs::read(&db).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    std::fs::write(&db, bytes).unwrap();

    let err = verify_backup(&backup.path).await.unwrap_err().to_string();
    assert!(err.contains("checksum"), "{err}");
    let err = restore_backup(&backup.path, &data)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("checksum"), "{err}");

    let live = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    assert_eq!(
        live.get_task(&task.id).await.unwrap().unwrap().issue.title,
        "Live"
    );
    let leftovers: Vec<_> = std::fs::read_dir(&data)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.contains("pre-restore") || n.ends_with(".restoring")
        })
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[tokio::test]
async fn a_backup_from_a_newer_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (storage, _) = storage_with_task(&data, "t").await;
    let backup = create_backup(&storage, &data.join("backups"), None)
        .await
        .unwrap();

    let manifest_path = backup.path.join(MANIFEST_FILE);
    let mut manifest: BackupManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest.schema_version = SCHEMA_VERSION + 1;
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let err = verify_backup(&backup.path).await.unwrap_err().to_string();
    assert!(err.contains("newer"), "{err}");
}

#[tokio::test]
async fn a_restore_that_cannot_open_the_database_puts_everything_back() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (storage, task) = storage_with_task(&data, "Live").await;
    std::fs::write(data.join(WORKFLOW_FILE), "live workflow").unwrap();
    let backup = create_backup(
        &storage,
        &data.join("backups"),
        Some(&data.join(WORKFLOW_FILE)),
    )
    .await
    .unwrap();
    storage.pool().close().await;

    // The database copy passes the manifest and integrity checks but records a schema version this
    // build refuses to open; the manifest is regenerated so the checksums match.
    let db = backup.path.join(DATABASE_FILE);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(&db))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, 'future', 'x')",
    )
    .bind(SCHEMA_VERSION + 5)
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    let manifest_path = backup.path.join(MANIFEST_FILE);
    let mut manifest: BackupManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    for file in &mut manifest.files {
        let bytes = std::fs::read(backup.path.join(&file.name)).unwrap();
        file.size_bytes = bytes.len() as u64;
        file.sha256 = {
            use sha2::{Digest, Sha256};
            hex::encode(Sha256::digest(&bytes))
        };
    }
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    verify_backup(&backup.path)
        .await
        .expect("passes the pre-checks");
    std::fs::write(data.join(WORKFLOW_FILE), "newer live workflow").unwrap();

    let err = restore_backup(&backup.path, &data)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("previous data was put back"), "{err}");

    let live = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    assert_eq!(
        live.get_task(&task.id).await.unwrap().unwrap().issue.title,
        "Live"
    );
    assert_eq!(
        std::fs::read_to_string(data.join(WORKFLOW_FILE)).unwrap(),
        "newer live workflow"
    );
}
