//! Offline backup verification and restoration through the real `symphony-cli` binary.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use chrono::Utc;
use symphony_core::{Project, ProviderKind, Task};
use symphony_platform::SingleInstanceGuard;
use symphony_storage::backup::{create_backup, DATABASE_FILE};
use symphony_storage::Storage;

const BIN: &str = env!("CARGO_BIN_EXE_symphony-cli");

fn cli(data_dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .env("RUST_LOG", "warn")
        .current_dir(data_dir)
        .stdin(Stdio::null())
        .output()
        .expect("run symphony-cli")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

async fn seeded_data_dir(dir: &Path, title: &str) -> (Storage, Task) {
    let storage = Storage::connect(dir.join(DATABASE_FILE)).await.unwrap();
    let now = Utc::now();
    storage
        .upsert_project(&Project {
            id: "p1".into(),
            name: "CLI".into(),
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
        })
        .await
        .unwrap();
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
async fn restore_backup_replaces_the_data_and_verify_backup_detects_damage() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (storage, task) = seeded_data_dir(&data, "Before").await;
    let backup = create_backup(&storage, &data.join("backups"), None)
        .await
        .unwrap();
    let mut changed = storage.get_task(&task.id).await.unwrap().unwrap();
    changed.issue.title = "After".into();
    storage.upsert_task(&changed).await.unwrap();
    storage.pool().close().await;

    let verified = cli(&data, &["verify-backup", backup.path.to_str().unwrap()]);
    assert!(verified.status.success(), "{}", text(&verified));
    assert!(
        text(&verified).contains("Backup is intact"),
        "{}",
        text(&verified)
    );

    let restored = cli(&data, &["restore-backup", backup.path.to_str().unwrap()]);
    assert!(restored.status.success(), "{}", text(&restored));
    let output = text(&restored);
    assert!(output.contains("Kept the replaced file"), "{output}");
    assert!(output.contains("Credentials were not touched"), "{output}");

    let after = Storage::connect(data.join(DATABASE_FILE)).await.unwrap();
    assert_eq!(
        after.get_task(&task.id).await.unwrap().unwrap().issue.title,
        "Before"
    );
    after.pool().close().await;

    // A damaged backup is rejected with a non-zero exit code and changes nothing.
    let db = backup.path.join(DATABASE_FILE);
    let mut bytes = std::fs::read(&db).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    std::fs::write(&db, bytes).unwrap();
    let damaged = cli(&data, &["verify-backup", backup.path.to_str().unwrap()]);
    assert!(!damaged.status.success());
    assert!(text(&damaged).contains("checksum"), "{}", text(&damaged));
    let refused = cli(&data, &["restore-backup", backup.path.to_str().unwrap()]);
    assert!(!refused.status.success());
}

#[tokio::test]
async fn restore_backup_refuses_while_a_runtime_owns_the_data_directory() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (storage, task) = seeded_data_dir(&data, "Live").await;
    let backup = create_backup(&storage, &data.join("backups"), None)
        .await
        .unwrap();

    // The runtime lock is held, exactly as by a running desktop or CLI runtime.
    let _runtime = SingleInstanceGuard::try_acquire(&data).unwrap();
    let out = cli(&data, &["restore-backup", backup.path.to_str().unwrap()]);
    assert!(!out.status.success(), "{}", text(&out));

    assert_eq!(
        storage
            .get_task(&task.id)
            .await
            .unwrap()
            .unwrap()
            .issue
            .title,
        "Live"
    );
}
