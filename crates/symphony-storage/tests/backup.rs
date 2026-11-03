//! Backup creation, verification and restoration.

use std::path::Path;

use chrono::Utc;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use symphony_core::{Project, ProviderKind, Task};
use symphony_storage::backup::{
    apply_pending_restore, apply_requested_restore, cancel_pending_restore, create_backup,
    create_scheduled_backup, last_restore, list_backups, pending_restore, prune_scheduled_backups,
    restore_backup, schedule_restore, scheduled_backup_due, verify_backup, BackupManifest,
    PendingRestore, RestoreOutcome, DATABASE_FILE, MANIFEST_FILE, MAX_RESTORE_ATTEMPTS,
    PENDING_RESTORE_FILE, WORKFLOW_FILE,
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
    // The set-aside database comes with its own WAL, under a name of its own: when SQLite did not
    // checkpoint at close (it does not under load), the rows only exist there. Read the pair
    // together, as SQLite would have.
    let probe = dir.path().join("previous.sqlite");
    std::fs::copy(kept_db, &probe).unwrap();
    if let Some(wal) = report.previous_files.iter().find(|p| {
        p.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("symphony.sqlite-wal.pre-restore-")
    }) {
        std::fs::copy(wal, dir.path().join("previous.sqlite-wal")).unwrap();
    }
    let previous = Storage::connect(&probe).await.unwrap();
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

#[tokio::test]
async fn concurrent_backups_in_the_same_second_never_share_a_directory() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (storage, _) = storage_with_task(&data, "t").await;
    let root = data.join("backups");
    let results = futures_join(
        (0..8)
            .map(|_| {
                let storage = storage.clone();
                let root = root.clone();
                async move { create_backup(&storage, &root, None).await }
            })
            .collect(),
    )
    .await;
    let ids: std::collections::BTreeSet<String> =
        results.into_iter().map(|r| r.unwrap().id).collect();
    assert_eq!(ids.len(), 8, "every backup has its own directory");
    let listed = list_backups(&root).unwrap();
    assert_eq!(listed.len(), 8);
    for backup in listed {
        verify_backup(&backup.path).await.unwrap();
    }
}

async fn futures_join<T: Send + 'static>(
    futures: Vec<impl std::future::Future<Output = T> + Send + 'static>,
) -> Vec<T> {
    let handles: Vec<_> = futures.into_iter().map(tokio::spawn).collect();
    let mut out = Vec::new();
    for handle in handles {
        out.push(handle.await.unwrap());
    }
    out
}

#[tokio::test]
async fn restoring_twice_in_a_row_keeps_every_replaced_file() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (storage, task) = storage_with_task(&data, "First").await;
    let first = create_backup(&storage, &data.join("backups"), None)
        .await
        .unwrap();
    let mut renamed = storage.get_task(&task.id).await.unwrap().unwrap();
    renamed.issue.title = "Second".into();
    storage.upsert_task(&renamed).await.unwrap();
    let second = create_backup(&storage, &data.join("backups"), None)
        .await
        .unwrap();
    storage.pool().close().await;

    // Two restores, back to back (the same clock second).
    restore_backup(&first.path, &data).await.unwrap();
    restore_backup(&second.path, &data).await.unwrap();

    let kept: Vec<_> = std::fs::read_dir(&data)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| {
            n.starts_with("symphony.sqlite.pre-restore-")
                && !n.contains("-wal")
                && !n.contains("-shm")
        })
        .collect();
    assert_eq!(
        kept.len(),
        2,
        "the first restore's saved database must survive the second: {kept:?}"
    );
    let live = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    assert_eq!(
        live.get_task(&task.id).await.unwrap().unwrap().issue.title,
        "Second"
    );
}

#[tokio::test]
async fn a_failed_restore_into_an_empty_data_directory_leaves_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let (storage, _) = storage_with_task(&source, "t").await;
    std::fs::write(source.join(WORKFLOW_FILE), "workflow from the backup").unwrap();
    let backup = create_backup(
        &storage,
        &source.join("backups"),
        Some(&source.join(WORKFLOW_FILE)),
    )
    .await
    .unwrap();
    storage.pool().close().await;

    // A copy that verifies but records a schema this build refuses to open.
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

    // A data directory with neither a database nor a workflow.
    let empty = dir.path().join("fresh");
    // An unrelated file that merely looks like a SQLite sidecar of the workflow must survive.
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::write(empty.join("WORKFLOW.md-wal"), "not ours").unwrap();
    let err = restore_backup(&backup.path, &empty)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("put back"), "{err}");
    assert!(
        !empty.join(DATABASE_FILE).exists(),
        "the rejected database must not stay"
    );
    assert!(
        !empty.join(WORKFLOW_FILE).exists(),
        "the backup's workflow must not stay"
    );
    let leftovers: Vec<String> = std::fs::read_dir(&empty)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, vec!["WORKFLOW.md-wal".to_string()]);
}

#[tokio::test]
async fn restoring_a_backup_without_a_workflow_sets_a_newer_workflow_aside() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (storage, _) = storage_with_task(&data, "t").await;
    // Backed up while the data directory had no workflow of its own.
    let backup = create_backup(&storage, &data.join("backups"), None)
        .await
        .unwrap();
    storage.pool().close().await;
    // A workflow created afterwards would otherwise keep being used after the restore.
    std::fs::write(data.join(WORKFLOW_FILE), "created after the backup").unwrap();

    let report = restore_backup(&backup.path, &data).await.unwrap();

    assert!(
        !data.join(WORKFLOW_FILE).exists(),
        "the snapshot had no workflow, so none may remain live"
    );
    let kept = report
        .previous_files
        .iter()
        .find(|p| p.to_string_lossy().contains("WORKFLOW.md.pre-restore-"))
        .expect("the newer workflow is kept, not deleted");
    assert_eq!(
        std::fs::read_to_string(kept).unwrap(),
        "created after the backup"
    );
}

#[tokio::test]
async fn stopping_a_task_from_a_stale_copy_keeps_the_usage_the_worker_settled() {
    use symphony_core::{TaskStatus, TokenUsage};
    let dir = tempfile::tempdir().unwrap();
    let (storage, task) = storage_with_task(&dir.path().join("data"), "t").await;
    // The stopping side reads the task, then the worker settles usage, then the stopping side writes.
    let mut stale = storage.get_task(&task.id).await.unwrap().unwrap();
    storage
        .add_task_usage(
            &task.id,
            &TokenUsage {
                input_tokens: 70,
                output_tokens: 30,
                total_tokens: 100,
            },
            0.5,
            true,
            None,
        )
        .await
        .unwrap();
    stale.status = TaskStatus::Cancelled;
    stale.last_error = Some("operator".into());
    storage.update_task_lifecycle(&stale).await.unwrap();

    let stored = storage.get_task(&task.id).await.unwrap().unwrap();
    assert_eq!(stored.status, TaskStatus::Cancelled);
    assert_eq!(stored.last_error.as_deref(), Some("operator"));
    assert_eq!(stored.total_tokens, 100, "the settled usage survives");
    assert!(stored.usage_estimated);
}

#[tokio::test]
async fn retention_deletes_only_old_automatic_backups() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    let (storage, _) = storage_with_task(&data, "Retention").await;

    let manual = create_backup(&storage, &root, None).await.unwrap();
    assert!(!manual.scheduled);
    let mut automatic = Vec::new();
    for _ in 0..4 {
        automatic.push(
            create_scheduled_backup(&storage, &root, None)
                .await
                .unwrap(),
        );
    }
    assert!(automatic.iter().all(|b| b.scheduled));
    // An interrupted backup is never touched.
    std::fs::create_dir_all(root.join(".symphony-backup-x.partial")).unwrap();

    let removed = prune_scheduled_backups(&root, 2).unwrap();
    assert_eq!(removed.len(), 2);
    let left: Vec<String> = list_backups(&root)
        .unwrap()
        .into_iter()
        .map(|b| b.id)
        .collect();
    assert!(left.contains(&manual.id), "a backup made by hand is kept");
    // The two newest automatic backups stay (list order is newest first).
    let newest: Vec<&String> = automatic.iter().rev().take(2).map(|b| &b.id).collect();
    for id in newest {
        assert!(left.contains(id), "{id} kept");
    }
    assert_eq!(left.len(), 3);
    assert!(root.join(".symphony-backup-x.partial").is_dir());
    // `keep` is at least 1: the newest automatic backup always stays.
    prune_scheduled_backups(&root, 0).unwrap();
    assert_eq!(
        list_backups(&root)
            .unwrap()
            .iter()
            .filter(|b| b.scheduled)
            .count(),
        1
    );
}

#[tokio::test]
async fn a_backup_is_due_when_none_is_younger_than_the_interval() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    let (storage, _) = storage_with_task(&data, "Due").await;
    let now = Utc::now();
    assert!(scheduled_backup_due(&[], now, 24), "never backed up");
    assert!(!scheduled_backup_due(&[], now, 0), "automatic backups off");

    // A backup made by hand also resets the clock.
    let backup = create_backup(&storage, &root, None).await.unwrap();
    let backups = list_backups(&root).unwrap();
    assert!(!scheduled_backup_due(&backups, now, 24));
    assert!(scheduled_backup_due(
        &backups,
        backup.created_at + chrono::Duration::hours(24),
        24
    ));
}

#[test]
fn a_manifest_written_before_the_schedule_reads_as_made_by_hand() {
    let manifest: BackupManifest = serde_json::from_value(serde_json::json!({
        "format": 1, "app_version": "2.0.0", "schema_version": 3,
        "created_at": "2026-10-01T00:00:00Z", "files": [], "credentials_included": false
    }))
    .unwrap();
    assert!(!manifest.scheduled);
}

// ---------------------------------------------------------------------------------------------
// Restoring from inside the application: scheduled now, applied by the next start
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_scheduled_restore_is_applied_by_the_next_start_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    let (storage, task) = storage_with_task(&data, "Before the mistake").await;
    let backup = create_backup(&storage, &root, None).await.unwrap();
    let mut changed = storage.get_task(&task.id).await.unwrap().unwrap();
    changed.issue.title = "After the mistake".into();
    storage.upsert_task(&changed).await.unwrap();

    // The running application asks for the restore; nothing is replaced yet.
    let pending = schedule_restore(&root, &backup.id).await.unwrap();
    assert_eq!(pending.backup_id, backup.id);
    assert_eq!(pending_restore(&root), Some(pending));
    assert_eq!(
        storage
            .get_task(&task.id)
            .await
            .unwrap()
            .unwrap()
            .issue
            .title,
        "After the mistake",
        "scheduling changes nothing"
    );
    assert!(last_restore(&root).is_none());

    // The runtime stops; the next start applies the request before it opens the database.
    storage.pool().close().await;
    let outcome = apply_pending_restore(&root, &data)
        .await
        .unwrap()
        .expect("a restore was pending");

    assert!(outcome.succeeded, "{outcome:?}");
    assert_eq!(outcome.backup_id, backup.id);
    assert!(
        !outcome.previous_files.is_empty(),
        "the replaced data is kept"
    );
    assert!(pending_restore(&root).is_none(), "the request is consumed");
    assert!(!root.join(PENDING_RESTORE_FILE).exists());
    assert_eq!(last_restore(&root), Some(outcome));
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
    // Nothing more to do on the start after that.
    assert!(apply_pending_restore(&root, &data).await.unwrap().is_none());
}

#[tokio::test]
async fn a_restore_request_is_refused_for_anything_but_a_valid_backup() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    let (storage, _task) = storage_with_task(&data, "Live").await;
    let backup = create_backup(&storage, &root, None).await.unwrap();
    // A directory next to the backups must not be reachable through a crafted name.
    std::fs::create_dir_all(data.join("elsewhere")).unwrap();

    for bad in [
        "../elsewhere",
        "..",
        ".",
        "",
        "elsewhere",
        "symphony-backup-../elsewhere",
        "symphony-backup-20200101T000000Z/../x",
        r"symphony-backup-a\b",
        "symphony-backup-does-not-exist",
    ] {
        assert!(
            schedule_restore(&root, bad).await.is_err(),
            "{bad:?} must be refused"
        );
        assert!(pending_restore(&root).is_none(), "{bad:?} left a request");
    }

    // A damaged backup is refused now, not after a restart.
    let db = backup.path.join(DATABASE_FILE);
    let mut bytes = std::fs::read(&db).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    std::fs::write(&db, bytes).unwrap();
    let err = schedule_restore(&root, &backup.id)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("checksum"), "{err}");
    assert!(pending_restore(&root).is_none());
}

#[tokio::test]
async fn a_pending_restore_can_be_withdrawn() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    let (storage, _task) = storage_with_task(&data, "Live").await;
    let backup = create_backup(&storage, &root, None).await.unwrap();

    assert!(
        !cancel_pending_restore(&root).unwrap(),
        "nothing to withdraw"
    );
    schedule_restore(&root, &backup.id).await.unwrap();
    assert!(cancel_pending_restore(&root).unwrap());
    assert!(pending_restore(&root).is_none());
    storage.pool().close().await;
    assert!(apply_pending_restore(&root, &data).await.unwrap().is_none());
}

#[tokio::test]
async fn a_restore_that_fails_at_start_is_reported_once_and_leaves_the_data_alone() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    let (storage, task) = storage_with_task(&data, "Live").await;
    let backup = create_backup(&storage, &root, None).await.unwrap();
    schedule_restore(&root, &backup.id).await.unwrap();
    storage.pool().close().await;

    // The backup is damaged between the request and the restart.
    let db = backup.path.join(DATABASE_FILE);
    let mut bytes = std::fs::read(&db).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    std::fs::write(&db, bytes).unwrap();

    let outcome = apply_pending_restore(&root, &data)
        .await
        .unwrap()
        .expect("a restore was pending");
    assert!(!outcome.succeeded);
    assert!(
        outcome.error.as_deref().unwrap().contains("checksum"),
        "{outcome:?}"
    );
    assert_eq!(last_restore(&root), Some(outcome));
    // Not retried on every start, and the live data is as it was.
    assert!(pending_restore(&root).is_none());
    assert!(apply_pending_restore(&root, &data).await.unwrap().is_none());
    let live = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    assert_eq!(
        live.get_task(&task.id).await.unwrap().unwrap().issue.title,
        "Live"
    );
}

#[tokio::test]
async fn the_backup_a_pending_restore_names_survives_retention() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    let (storage, _) = storage_with_task(&data, "Retention").await;
    let mut automatic = Vec::new();
    for _ in 0..3 {
        automatic.push(
            create_scheduled_backup(&storage, &root, None)
                .await
                .unwrap(),
        );
    }
    // The oldest automatic backup is the one the user chose to restore.
    let chosen = automatic[0].id.clone();
    schedule_restore(&root, &chosen).await.unwrap();

    let removed = prune_scheduled_backups(&root, 1).unwrap();
    assert!(!removed.contains(&chosen), "the requested backup is kept");
    assert_eq!(removed.len(), 1, "the other old one is still pruned");
    assert!(root.join(&chosen).is_dir());

    // Once the request is gone, it is an ordinary old backup again.
    cancel_pending_restore(&root).unwrap();
    let removed = prune_scheduled_backups(&root, 1).unwrap();
    assert_eq!(removed, vec![chosen]);
}

#[tokio::test]
async fn a_restore_interrupted_with_the_database_set_aside_is_finished_by_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    let (storage, task) = storage_with_task(&data, "Before").await;
    let backup = create_backup(&storage, &root, None).await.unwrap();
    let mut changed = storage.get_task(&task.id).await.unwrap().unwrap();
    changed.issue.title = "After".into();
    storage.upsert_task(&changed).await.unwrap();
    schedule_restore(&root, &backup.id).await.unwrap();
    storage.pool().close().await;

    // A start began the restore, set the database aside and died: the request is still there with
    // its attempt counted, and there is no database.
    let started = PendingRestore {
        attempts: 1,
        ..pending_restore(&root).unwrap()
    };
    std::fs::write(
        root.join(PENDING_RESTORE_FILE),
        serde_json::to_vec(&started).unwrap(),
    )
    .unwrap();
    let aside = data.join(format!("{DATABASE_FILE}.pre-restore-crashed"));
    std::fs::rename(data.join(DATABASE_FILE), &aside).unwrap();
    for sidecar in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(data.join(format!("{DATABASE_FILE}{sidecar}")));
    }

    let outcome = apply_pending_restore(&root, &data)
        .await
        .unwrap()
        .expect("the request was still pending");

    assert!(outcome.succeeded, "{outcome:?}");
    assert!(pending_restore(&root).is_none());
    assert!(
        aside.is_file(),
        "the data set aside by the first try is kept"
    );
    let restored = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    assert_eq!(
        restored
            .get_task(&task.id)
            .await
            .unwrap()
            .unwrap()
            .issue
            .title,
        "Before"
    );
}

#[tokio::test]
async fn a_retry_that_fails_puts_the_database_an_interrupted_attempt_set_aside_back() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    let (storage, task) = storage_with_task(&data, "Live").await;
    let backup = create_backup(&storage, &root, None).await.unwrap();
    schedule_restore(&root, &backup.id).await.unwrap();
    storage.pool().close().await;

    // The first start set the database aside and died; before the second start the backup was
    // removed, so the retry fails before it replaces anything.
    let started = PendingRestore {
        attempts: 1,
        ..pending_restore(&root).unwrap()
    };
    std::fs::write(
        root.join(PENDING_RESTORE_FILE),
        serde_json::to_vec(&started).unwrap(),
    )
    .unwrap();
    let aside = data.join(format!("{DATABASE_FILE}.pre-restore-20261103T020000Z"));
    std::fs::rename(data.join(DATABASE_FILE), &aside).unwrap();
    std::fs::remove_dir_all(&backup.path).unwrap();

    let outcome = apply_pending_restore(&root, &data)
        .await
        .unwrap()
        .expect("the request was still pending");

    assert!(!outcome.succeeded);
    assert!(!root.join(PENDING_RESTORE_FILE).exists());
    assert!(
        outcome.error.as_deref().unwrap().contains("put back"),
        "{outcome:?}"
    );
    // The runtime that starts next finds its own data, not a new empty database.
    assert!(!aside.exists(), "the displaced database was moved back");
    let live = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    assert_eq!(
        live.get_task(&task.id).await.unwrap().unwrap().issue.title,
        "Live"
    );
}

#[tokio::test]
async fn a_failed_retry_after_an_interrupted_start_never_claims_the_data_is_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    let (storage, _task) = storage_with_task(&data, "Live").await;
    let backup = create_backup(&storage, &root, None).await.unwrap();
    schedule_restore(&root, &backup.id).await.unwrap();
    storage.pool().close().await;
    // An earlier start began applying the request and died; a database is in place (it may be
    // the restored one), and the backup is gone before the retry.
    let started = PendingRestore {
        attempts: 1,
        ..pending_restore(&root).unwrap()
    };
    std::fs::write(
        root.join(PENDING_RESTORE_FILE),
        serde_json::to_vec(&started).unwrap(),
    )
    .unwrap();
    std::fs::remove_dir_all(&backup.path).unwrap();

    let outcome = apply_pending_restore(&root, &data)
        .await
        .unwrap()
        .expect("the request was still pending");

    assert!(!outcome.succeeded);
    assert!(outcome.rollback_incomplete, "{outcome:?}");
    assert!(
        outcome.error.as_deref().unwrap().contains("interrupted"),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_settled_request_that_could_not_be_removed_is_not_applied_a_second_time() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    let (storage, task) = storage_with_task(&data, "Before").await;
    let backup = create_backup(&storage, &root, None).await.unwrap();
    let mut changed = storage.get_task(&task.id).await.unwrap().unwrap();
    changed.issue.title = "After".into();
    storage.upsert_task(&changed).await.unwrap();
    schedule_restore(&root, &backup.id).await.unwrap();
    let request = std::fs::read(root.join(PENDING_RESTORE_FILE)).unwrap();
    storage.pool().close().await;

    let outcome = apply_pending_restore(&root, &data).await.unwrap().unwrap();
    assert!(outcome.succeeded, "{outcome:?}");

    // Work done after the restore...
    let live = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    let mut newer = live.get_task(&task.id).await.unwrap().unwrap();
    newer.issue.title = "Newer work".into();
    live.upsert_task(&newer).await.unwrap();
    live.pool().close().await;
    // ...and the request file is back, as if its removal had failed.
    std::fs::write(root.join(PENDING_RESTORE_FILE), request).unwrap();

    assert!(
        apply_pending_restore(&root, &data).await.unwrap().is_none(),
        "a settled request is not applied again"
    );

    assert!(!root.join(PENDING_RESTORE_FILE).exists());
    let live = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    assert_eq!(
        live.get_task(&task.id).await.unwrap().unwrap().issue.title,
        "Newer work",
        "the restore did not run over the newer work"
    );
    // A new request for the same backup is a new request.
    schedule_restore(&root, &backup.id).await.unwrap();
    live.pool().close().await;
    assert!(apply_pending_restore(&root, &data).await.unwrap().is_some());
}

#[tokio::test]
async fn a_request_file_from_before_ids_existed_gets_an_id_when_its_restore_starts() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    let (storage, task) = storage_with_task(&data, "Before").await;
    let backup = create_backup(&storage, &root, None).await.unwrap();
    storage.pool().close().await;
    // A request written without an id: the clock never decides whether it was settled.
    let requested = Utc::now() - chrono::Duration::minutes(1);
    let legacy = format!(
        r#"{{"backup_id":"{}","requested_at":"{}"}}"#,
        backup.id,
        requested.to_rfc3339()
    );
    std::fs::write(root.join(PENDING_RESTORE_FILE), &legacy).unwrap();
    let outcome = apply_pending_restore(&root, &data).await.unwrap().unwrap();
    assert!(outcome.succeeded, "{outcome:?}");
    assert!(
        !outcome.request_id.is_empty(),
        "it was given an id before it was applied"
    );

    let live = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    let mut newer = live.get_task(&task.id).await.unwrap().unwrap();
    newer.issue.title = "Newer work".into();
    live.upsert_task(&newer).await.unwrap();
    live.pool().close().await;

    // The file the start rewrote (it carries that id) survives a failed removal: skipped.
    let rewritten = PendingRestore {
        backup_id: backup.id.clone(),
        requested_at: requested,
        request_id: outcome.request_id.clone(),
        attempts: 1,
    };
    std::fs::write(
        root.join(PENDING_RESTORE_FILE),
        serde_json::to_vec(&rewritten).unwrap(),
    )
    .unwrap();
    assert!(apply_pending_restore(&root, &data).await.unwrap().is_none());
    let live = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    assert_eq!(
        live.get_task(&task.id).await.unwrap().unwrap().issue.title,
        "Newer work"
    );

    // Another id-less request for the same backup, after a clock that went backwards, is a new
    // request: it is applied.
    let again = format!(
        r#"{{"backup_id":"{}","requested_at":"{}"}}"#,
        backup.id,
        (outcome.applied_at - chrono::Duration::hours(2)).to_rfc3339()
    );
    std::fs::write(root.join(PENDING_RESTORE_FILE), again).unwrap();
    live.pool().close().await;
    let second = apply_pending_restore(&root, &data).await.unwrap().unwrap();
    assert!(second.succeeded, "{second:?}");
    assert_ne!(second.request_id, outcome.request_id);
}

#[tokio::test]
async fn a_new_request_is_not_taken_for_settled_when_the_clock_was_set_back() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    let (storage, task) = storage_with_task(&data, "Before").await;
    let backup = create_backup(&storage, &root, None).await.unwrap();
    schedule_restore(&root, &backup.id).await.unwrap();
    storage.pool().close().await;
    let first = apply_pending_restore(&root, &data).await.unwrap().unwrap();
    assert!(first.succeeded && !first.request_id.is_empty());

    // The clock is corrected backwards, and the same backup is requested again: the new request
    // is stamped *earlier* than the outcome of the old one.
    let live = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    let mut newer = live.get_task(&task.id).await.unwrap().unwrap();
    newer.issue.title = "Work after the first restore".into();
    live.upsert_task(&newer).await.unwrap();
    live.pool().close().await;
    let again = schedule_restore(&root, &backup.id).await.unwrap();
    assert_ne!(again.request_id, first.request_id);
    let earlier = PendingRestore {
        requested_at: first.applied_at - chrono::Duration::hours(1),
        ..again
    };
    std::fs::write(
        root.join(PENDING_RESTORE_FILE),
        serde_json::to_vec(&earlier).unwrap(),
    )
    .unwrap();

    let outcome = apply_pending_restore(&root, &data)
        .await
        .unwrap()
        .expect("a request with another id is not the settled one");
    assert!(outcome.succeeded, "{outcome:?}");
    assert_ne!(outcome.request_id, first.request_id);
    let restored = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    assert_eq!(
        restored
            .get_task(&task.id)
            .await
            .unwrap()
            .unwrap()
            .issue
            .title,
        "Before"
    );
}

#[tokio::test]
async fn a_restore_interrupted_too_often_is_given_up_and_reported_as_unfinished() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    let (storage, task) = storage_with_task(&data, "Live").await;
    let backup = create_backup(&storage, &root, None).await.unwrap();
    schedule_restore(&root, &backup.id).await.unwrap();
    storage.pool().close().await;
    let exhausted = PendingRestore {
        attempts: MAX_RESTORE_ATTEMPTS,
        ..pending_restore(&root).unwrap()
    };
    std::fs::write(
        root.join(PENDING_RESTORE_FILE),
        serde_json::to_vec(&exhausted).unwrap(),
    )
    .unwrap();

    let outcome = apply_pending_restore(&root, &data)
        .await
        .unwrap()
        .expect("a request was pending");

    // Not retried again and again, and not reported as "the data is as it was": what the
    // interrupted starts left behind is unknown.
    assert!(!outcome.succeeded);
    assert!(outcome.rollback_incomplete, "{outcome:?}");
    assert!(
        outcome.error.as_deref().unwrap().contains("pre-restore"),
        "{outcome:?}"
    );
    assert!(!root.join(PENDING_RESTORE_FILE).exists());
    assert_eq!(last_restore(&root), Some(outcome));
    assert!(apply_pending_restore(&root, &data).await.unwrap().is_none());
    // Nothing was touched by the given-up attempt.
    let live = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    assert_eq!(
        live.get_task(&task.id).await.unwrap().unwrap().issue.title,
        "Live"
    );
}

#[test]
fn an_outcome_written_by_an_earlier_version_still_reads() {
    // Neither `rollback_incomplete` nor `attempts` existed then.
    let outcome: RestoreOutcome = serde_json::from_str(
        r#"{"backup_id":"symphony-backup-1","applied_at":"2026-11-03T00:00:00Z","succeeded":false,"error":"x","previous_files":[]}"#,
    )
    .unwrap();
    assert!(!outcome.rollback_incomplete);
    let pending: PendingRestore = serde_json::from_str(
        r#"{"backup_id":"symphony-backup-1","requested_at":"2026-11-03T00:00:00Z"}"#,
    )
    .unwrap();
    assert_eq!(pending.attempts, 0);
}

#[tokio::test]
async fn an_unreadable_restore_request_is_discarded_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(PENDING_RESTORE_FILE), "{ not json").unwrap();

    let outcome = apply_pending_restore(&root, &data)
        .await
        .unwrap()
        .expect("there was a request file");
    assert!(!outcome.succeeded);
    assert!(!root.join(PENDING_RESTORE_FILE).exists());
}

#[tokio::test]
async fn the_host_start_hook_applies_a_request_and_ignores_the_absence_of_one() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = data.join("backups");
    let (storage, task) = storage_with_task(&data, "Before").await;
    let backup = create_backup(&storage, &root, None).await.unwrap();
    let mut changed = storage.get_task(&task.id).await.unwrap().unwrap();
    changed.issue.title = "After".into();
    storage.upsert_task(&changed).await.unwrap();
    storage.pool().close().await;

    // Nothing requested: a start is not affected.
    assert!(apply_requested_restore(&data).await.is_none());

    schedule_restore(&root, &backup.id).await.unwrap();
    let outcome = apply_requested_restore(&data).await.expect("applied");
    assert!(outcome.succeeded);
    let restored = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    assert_eq!(
        restored
            .get_task(&task.id)
            .await
            .unwrap()
            .unwrap()
            .issue
            .title,
        "Before"
    );
}
