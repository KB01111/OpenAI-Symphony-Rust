//! Local backups of the application data, and their restoration.
//!
//! # What a backup contains
//!
//! * `symphony.sqlite`: a transactionally consistent copy of the database written with SQLite's
//!   `VACUUM INTO` (never a raw copy of the live file, whose committed content may still sit in the
//!   `-wal` file): projects, tasks, run attempts, agent events, approval decisions, pull request
//!   reviews, validation results, settings and provider profiles.
//! * `WORKFLOW.md`: the active workflow document, when one exists.
//! * `manifest.json`: format and schema version, creation time and the SHA-256 of every file.
//!
//! # What it does not contain
//!
//! * **Credentials.** The credential vault is bound to the Windows user account (DPAPI) and must not
//!   travel inside a general-purpose backup; it cannot be restored under another Windows identity.
//!   Copy `credentials.vault` separately (for the same user on the same machine) or re-enter keys.
//! * **Git worktrees and repositories**, which live in their own directories and in Git.
//! * **Logs** and the runtime discovery file.
//!
//! Backups are published atomically (written to a `.partial` directory, then renamed), so an
//! interrupted backup is never listed or restorable.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection};
use symphony_core::{SymphonyError, SymphonyResult};

use crate::{Storage, SCHEMA_VERSION};

pub const MANIFEST_FILE: &str = "manifest.json";
pub const DATABASE_FILE: &str = "symphony.sqlite";
pub const WORKFLOW_FILE: &str = "WORKFLOW.md";
const BACKUP_FORMAT: u32 = 1;
const DIR_PREFIX: &str = "symphony-backup-";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupFile {
    pub name: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupManifest {
    pub format: u32,
    pub app_version: String,
    pub schema_version: i64,
    pub created_at: DateTime<Utc>,
    pub files: Vec<BackupFile>,
    /// Always `false`: credentials are never part of a backup (see the module documentation).
    pub credentials_included: bool,
}

/// A backup directory as listed to users.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupInfo {
    /// Directory name, stable and unique (`symphony-backup-<UTC time>`).
    pub id: String,
    pub path: PathBuf,
    pub created_at: DateTime<Utc>,
    pub app_version: String,
    pub schema_version: i64,
    pub size_bytes: u64,
    pub includes_workflow: bool,
}

/// What a restore changed. The previous files are kept, never deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreReport {
    pub restored_from: PathBuf,
    /// Files that were replaced and moved aside, so the restore can be undone by hand.
    pub previous_files: Vec<PathBuf>,
}

fn storage_err(message: impl Into<String>) -> SymphonyError {
    SymphonyError::StorageError(message.into())
}

fn sha256_of(path: &Path) -> SymphonyResult<(u64, String)> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((size, hex::encode(hasher.finalize())))
}

fn describe(path: &Path, name: &str) -> SymphonyResult<BackupFile> {
    let (size_bytes, sha256) = sha256_of(path)?;
    Ok(BackupFile {
        name: name.to_string(),
        size_bytes,
        sha256,
    })
}

/// Create a backup under `root` and return it. `workflow` is the active `WORKFLOW.md`, if any.
pub async fn create_backup(
    storage: &Storage,
    root: &Path,
    workflow: Option<&Path>,
) -> SymphonyResult<BackupInfo> {
    std::fs::create_dir_all(root)?;
    let created_at = Utc::now();
    // Unique even for two backups within the same second.
    let mut id = format!("{DIR_PREFIX}{}", created_at.format("%Y%m%dT%H%M%SZ"));
    let mut suffix = 1;
    while root.join(&id).exists() || root.join(format!(".{id}.partial")).exists() {
        suffix += 1;
        id = format!(
            "{DIR_PREFIX}{}-{suffix}",
            created_at.format("%Y%m%dT%H%M%SZ")
        );
    }
    let partial = root.join(format!(".{id}.partial"));
    std::fs::create_dir_all(&partial)?;

    let built = build_backup(storage, &partial, workflow, created_at).await;
    if let Err(err) = built {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(err);
    }
    let final_dir = root.join(&id);
    std::fs::rename(&partial, &final_dir)?;
    info_for(&final_dir).ok_or_else(|| storage_err("the new backup could not be read back"))
}

async fn build_backup(
    storage: &Storage,
    dir: &Path,
    workflow: Option<&Path>,
    created_at: DateTime<Utc>,
) -> SymphonyResult<()> {
    let db_copy = dir.join(DATABASE_FILE);
    storage.backup_to(&db_copy).await?;
    let mut files = vec![describe(&db_copy, DATABASE_FILE)?];
    if let Some(source) = workflow.filter(|p| p.is_file()) {
        let target = dir.join(WORKFLOW_FILE);
        std::fs::copy(source, &target)?;
        files.push(describe(&target, WORKFLOW_FILE)?);
    }
    let manifest = BackupManifest {
        format: BACKUP_FORMAT,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version: SCHEMA_VERSION,
        created_at,
        files,
        credentials_included: false,
    };
    let json = serde_json::to_vec_pretty(&manifest).map_err(|e| storage_err(e.to_string()))?;
    std::fs::write(dir.join(MANIFEST_FILE), json)?;
    Ok(())
}

fn read_manifest(dir: &Path) -> SymphonyResult<BackupManifest> {
    let raw = std::fs::read(dir.join(MANIFEST_FILE))
        .map_err(|e| storage_err(format!("{} has no readable manifest: {e}", dir.display())))?;
    serde_json::from_slice(&raw)
        .map_err(|e| storage_err(format!("{} has an invalid manifest: {e}", dir.display())))
}

fn info_for(dir: &Path) -> Option<BackupInfo> {
    let manifest = read_manifest(dir).ok()?;
    let id = dir.file_name()?.to_string_lossy().into_owned();
    Some(BackupInfo {
        id,
        path: dir.to_path_buf(),
        created_at: manifest.created_at,
        app_version: manifest.app_version.clone(),
        schema_version: manifest.schema_version,
        size_bytes: manifest.files.iter().map(|f| f.size_bytes).sum(),
        includes_workflow: manifest.files.iter().any(|f| f.name == WORKFLOW_FILE),
    })
}

/// Backups under `root`, newest first. Directories that are not complete backups (partial,
/// unrelated) are skipped; a missing `root` is an empty list.
pub fn list_backups(root: &Path) -> SymphonyResult<Vec<BackupInfo>> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut backups: Vec<BackupInfo> = std::fs::read_dir(root)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.path().is_dir() && entry.file_name().to_string_lossy().starts_with(DIR_PREFIX)
        })
        .filter_map(|entry| info_for(&entry.path()))
        .collect();
    backups.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    Ok(backups)
}

/// Check a backup without changing anything: manifest, file checksums, a schema this build can
/// read and SQLite's own integrity check of the database copy.
pub async fn verify_backup(dir: &Path) -> SymphonyResult<BackupManifest> {
    let manifest = read_manifest(dir)?;
    if manifest.format != BACKUP_FORMAT {
        return Err(storage_err(format!(
            "backup format {} is not supported (expected {BACKUP_FORMAT})",
            manifest.format
        )));
    }
    if manifest.schema_version > SCHEMA_VERSION {
        return Err(storage_err(format!(
            "the backup uses database schema version {}, newer than the version {SCHEMA_VERSION} \
             this build supports; restore it with the newer Symphony Desktop",
            manifest.schema_version
        )));
    }
    if !manifest.files.iter().any(|f| f.name == DATABASE_FILE) {
        return Err(storage_err("the backup contains no database"));
    }
    for file in &manifest.files {
        // Manifest names are untrusted: only plain file names inside the backup directory.
        if Path::new(&file.name).components().count() != 1 || file.name.contains(['/', '\\']) {
            return Err(storage_err(format!(
                "invalid file name `{}` in manifest",
                file.name
            )));
        }
        let path = dir.join(&file.name);
        let (size, digest) = sha256_of(&path)
            .map_err(|e| storage_err(format!("backup file {} is unreadable: {e}", file.name)))?;
        if size != file.size_bytes || digest != file.sha256 {
            return Err(storage_err(format!(
                "backup file {} does not match its checksum; the backup is damaged",
                file.name
            )));
        }
    }
    check_integrity(&dir.join(DATABASE_FILE)).await?;
    Ok(manifest)
}

async fn check_integrity(db: &Path) -> SymphonyResult<()> {
    let mut conn = SqliteConnectOptions::new()
        .filename(db)
        .read_only(true)
        .connect()
        .await
        .map_err(|e| storage_err(format!("the backup database cannot be opened: {e}")))?;
    let verdict: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut conn)
        .await
        .map_err(|e| storage_err(format!("integrity check failed to run: {e}")))?;
    let _ = conn.close().await;
    if verdict != "ok" {
        return Err(storage_err(format!(
            "the backup database failed SQLite's integrity check: {verdict}"
        )));
    }
    Ok(())
}

fn aside(path: &Path, stamp: &str) -> SymphonyResult<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let target = path.with_file_name(format!("{name}.pre-restore-{stamp}"));
    std::fs::rename(path, &target)?;
    Ok(Some(target))
}

/// Replace the database (and `WORKFLOW.md`, when the backup has one) in `data_dir` with the
/// backup's. **Offline operation**: the caller must hold the data directory's runtime lock, so no
/// runtime has the database open.
///
/// Non-destructive: the replaced files are renamed `<name>.pre-restore-<time>`, never deleted, and
/// are put back if the restored database cannot be opened and migrated. A backup from an older
/// version is migrated forward on the way.
pub async fn restore_backup(backup_dir: &Path, data_dir: &Path) -> SymphonyResult<RestoreReport> {
    let manifest = verify_backup(backup_dir).await?;
    std::fs::create_dir_all(data_dir)?;

    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let live_db = data_dir.join(DATABASE_FILE);
    let staged = data_dir.join(format!("{DATABASE_FILE}.restoring"));
    let _ = std::fs::remove_file(&staged);
    std::fs::copy(backup_dir.join(DATABASE_FILE), &staged)?;
    // The copy must be byte-identical before anything is replaced.
    let expected = manifest
        .files
        .iter()
        .find(|f| f.name == DATABASE_FILE)
        .map(|f| f.sha256.clone())
        .unwrap_or_default();
    if sha256_of(&staged)?.1 != expected {
        let _ = std::fs::remove_file(&staged);
        return Err(storage_err("the database copy does not match the backup"));
    }

    // (original, moved-aside) pairs, to undo on failure.
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut move_aside = |path: PathBuf| -> SymphonyResult<()> {
        if let Some(target) = aside(&path, &stamp)? {
            moved.push((path, target));
        }
        Ok(())
    };
    let result: SymphonyResult<()> = async {
        // A stale WAL next to the new database would be replayed into it: set both aside.
        for suffix in ["", "-wal", "-shm"] {
            move_aside(data_dir.join(format!("{DATABASE_FILE}{suffix}")))?;
        }
        std::fs::rename(&staged, &live_db)?;
        if manifest.files.iter().any(|f| f.name == WORKFLOW_FILE) {
            move_aside(data_dir.join(WORKFLOW_FILE))?;
            std::fs::copy(backup_dir.join(WORKFLOW_FILE), data_dir.join(WORKFLOW_FILE))?;
        }
        // Opening applies pending migrations (an older backup) and proves the file is usable.
        let opened = Storage::connect(&live_db).await?;
        opened.pool().close().await;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => Ok(RestoreReport {
            restored_from: backup_dir.to_path_buf(),
            previous_files: moved.into_iter().map(|(_, target)| target).collect(),
        }),
        Err(err) => {
            // Put everything back: remove what the restore wrote, then rename the originals.
            let _ = std::fs::remove_file(&staged);
            for (original, target) in moved.iter().rev() {
                let _ = std::fs::remove_file(original);
                let _ = std::fs::rename(target, original);
            }
            Err(storage_err(format!(
                "the restore failed and the previous data was put back: {err}"
            )))
        }
    }
}
