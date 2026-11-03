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
    /// Made by the automatic schedule; only these are deleted by [`prune_scheduled_backups`].
    /// Backups written before the schedule existed read as made by hand.
    #[serde(default)]
    pub scheduled: bool,
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
    /// Made by the automatic schedule (see [`BackupManifest::scheduled`]).
    pub scheduled: bool,
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
    create(storage, root, workflow, false).await
}

/// Create a backup like [`create_backup`], marked as made by the automatic schedule so that
/// [`prune_scheduled_backups`] may delete it later.
pub async fn create_scheduled_backup(
    storage: &Storage,
    root: &Path,
    workflow: Option<&Path>,
) -> SymphonyResult<BackupInfo> {
    create(storage, root, workflow, true).await
}

/// Whether an automatic backup is due: automatic backups are on (`interval_hours > 0`) and no
/// backup of any kind under `backups` is younger than the interval.
pub fn scheduled_backup_due(
    backups: &[BackupInfo],
    now: DateTime<Utc>,
    interval_hours: u32,
) -> bool {
    if interval_hours == 0 {
        return false;
    }
    let interval = chrono::Duration::hours(i64::from(interval_hours));
    backups
        .iter()
        .map(|b| b.created_at)
        .max()
        .is_none_or(|newest| now - newest >= interval)
}

/// Delete automatic backups beyond the newest `keep` of them and return the ids removed. Backups
/// made by hand, incomplete (`.partial`) directories and anything that is not a backup are never
/// touched; `keep` is at least 1, so the newest automatic backup always stays. The backup a pending
/// restore request names stays too, however old: it is deleted only after the restore was applied
/// or withdrawn, or the requested restore would fail on the next start.
pub fn prune_scheduled_backups(root: &Path, keep: u32) -> SymphonyResult<Vec<String>> {
    let keep = keep.max(1) as usize;
    let requested = pending_restore(root).map(|p| p.backup_id);
    let mut removed = Vec::new();
    for backup in list_backups(root)?
        .into_iter()
        .filter(|b| b.scheduled)
        .skip(keep)
        .filter(|b| requested.as_deref() != Some(b.id.as_str()))
    {
        std::fs::remove_dir_all(&backup.path)?;
        removed.push(backup.id);
    }
    Ok(removed)
}

async fn create(
    storage: &Storage,
    root: &Path,
    workflow: Option<&Path>,
    scheduled: bool,
) -> SymphonyResult<BackupInfo> {
    std::fs::create_dir_all(root)?;
    let created_at = Utc::now();
    // Unique even for concurrent backups within the same second: the staging directory is
    // reserved by creating it (`create_dir` fails if it exists), never by checking first.
    let stamp = created_at.format("%Y%m%dT%H%M%SZ").to_string();
    let mut suffix = 1;
    let (id, partial) = loop {
        let id = if suffix == 1 {
            format!("{DIR_PREFIX}{stamp}")
        } else {
            format!("{DIR_PREFIX}{stamp}-{suffix}")
        };
        let partial = root.join(format!(".{id}.partial"));
        if !root.join(&id).exists() {
            match std::fs::create_dir(&partial) {
                Ok(()) => break (id, partial),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        suffix += 1;
    };

    let built = build_backup(storage, &partial, workflow, created_at, scheduled).await;
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
    scheduled: bool,
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
        scheduled,
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
        scheduled: manifest.scheduled,
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

/// One suffix for everything a restore sets aside: the database, its `-wal` and `-shm`, and the
/// workflow. `aside` alone picks a free name per file, which can leave a database under `…-T-2` and
/// its WAL under `…-T`; recovering by one suffix would then restore the database without the
/// transactions that were only committed to the WAL. With a shared suffix they always match.
fn shared_aside_stamp(data_dir: &Path, stamp: &str) -> String {
    let names = [
        DATABASE_FILE.to_string(),
        format!("{DATABASE_FILE}-wal"),
        format!("{DATABASE_FILE}-shm"),
        WORKFLOW_FILE.to_string(),
    ];
    let mut candidate = stamp.to_string();
    let mut attempt = 1;
    while names.iter().any(|name| {
        data_dir
            .join(format!("{name}.pre-restore-{candidate}"))
            .exists()
    }) {
        attempt += 1;
        candidate = format!("{stamp}-{attempt}");
    }
    candidate
}

/// Rename `path` to `<name>.pre-restore-<stamp>`, or `…-2`, `…-3` when that name is taken: a
/// file kept by an earlier restore is never replaced.
fn aside(path: &Path, stamp: &str) -> SymphonyResult<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut target = path.with_file_name(format!("{name}.pre-restore-{stamp}"));
    let mut attempt = 1;
    while target.exists() {
        attempt += 1;
        target = path.with_file_name(format!("{name}.pre-restore-{stamp}-{attempt}"));
    }
    std::fs::rename(path, &target)?;
    Ok(Some(target))
}

/// Why a restore failed, and whether it left the data directory as it found it.
#[derive(Debug)]
struct RestoreFailure {
    error: SymphonyError,
    /// Putting the previous files back did not finish: some of them still lie under their
    /// `.pre-restore-…` names (`displaced`) and the data directory is not as it was.
    rollback_incomplete: bool,
    /// The files that are still displaced.
    displaced: Vec<PathBuf>,
}

impl From<SymphonyError> for RestoreFailure {
    fn from(error: SymphonyError) -> Self {
        Self {
            error,
            rollback_incomplete: false,
            displaced: Vec::new(),
        }
    }
}

impl From<std::io::Error> for RestoreFailure {
    fn from(error: std::io::Error) -> Self {
        SymphonyError::from(error).into()
    }
}

/// Replace the database (and `WORKFLOW.md`, when the backup has one) in `data_dir` with the
/// backup's. **Offline operation**: the caller must hold the data directory's runtime lock, so no
/// runtime has the database open.
///
/// Non-destructive: the replaced files are renamed `<name>.pre-restore-<time>`, never deleted, and
/// are put back if the restored database cannot be opened and migrated. A backup from an older
/// version is migrated forward on the way.
pub async fn restore_backup(backup_dir: &Path, data_dir: &Path) -> SymphonyResult<RestoreReport> {
    restore_detailed(backup_dir, data_dir)
        .await
        .map_err(|failure| failure.error)
}

async fn restore_detailed(
    backup_dir: &Path,
    data_dir: &Path,
) -> Result<RestoreReport, RestoreFailure> {
    let manifest = verify_backup(backup_dir).await?;
    std::fs::create_dir_all(data_dir)?;

    let stamp = shared_aside_stamp(data_dir, &Utc::now().format("%Y%m%dT%H%M%SZ").to_string());
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
        return Err(storage_err("the database copy does not match the backup").into());
    }

    // (original, moved-aside) pairs and every file the restore wrote, to undo on failure.
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut written: Vec<PathBuf> = Vec::new();
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
        written.push(live_db.clone());
        // The workflow is part of the snapshot: one that is not in the backup was created after it,
        // and would otherwise keep being used (the data directory's copy takes precedence).
        move_aside(data_dir.join(WORKFLOW_FILE))?;
        if manifest.files.iter().any(|f| f.name == WORKFLOW_FILE) {
            let workflow = data_dir.join(WORKFLOW_FILE);
            written.push(workflow.clone());
            std::fs::copy(backup_dir.join(WORKFLOW_FILE), &workflow)?;
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
            // Put everything back: remove everything the restore wrote (also where there was no
            // previous file to restore), then rename the originals back. A step that fails is
            // reported with the path that still holds the data: "put back" is only claimed when it
            // is true.
            let mut problems: Vec<String> = Vec::new();
            let mut displaced: Vec<PathBuf> = Vec::new();
            let mut remove = |path: &Path| match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => problems.push(format!("{} could not be removed ({e})", path.display())),
            };
            remove(&staged);
            for file in &written {
                remove(file);
            }
            // SQLite sidecars belong to the database only, never to the other files written.
            for suffix in ["-wal", "-shm"] {
                remove(Path::new(&format!("{}{suffix}", live_db.display())));
            }
            for (original, target) in moved.iter().rev() {
                if let Err(e) = std::fs::rename(target, original) {
                    displaced.push(target.clone());
                    problems.push(format!(
                        "{} could not be moved back to {} ({e})",
                        target.display(),
                        original.display()
                    ));
                }
            }
            if !problems.is_empty() {
                return Err(RestoreFailure {
                    error: storage_err(format!(
                        "the restore failed ({err}) and putting the previous data back was incomplete: {}. Nothing was deleted: inspect the files named above (those named `.pre-restore-…` hold the previous data).",
                        problems.join("; ")
                    )),
                    rollback_incomplete: true,
                    displaced,
                });
            }
            Err(storage_err(format!(
                "the restore failed and the previous data was put back: {err}"
            ))
            .into())
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Restoring from inside the application
// ---------------------------------------------------------------------------------------------
//
// A restore replaces the database file, so it can only happen while no runtime has it open. The
// application therefore does not restore a backup in place: it records the request (a small marker
// file next to the backups) and the host applies it on its next start, after it owns the data
// directory's runtime lock and before it opens the database. The result is kept in a second file
// for the application to show. `symphony-cli restore-backup <folder>` stays the way to restore
// while nothing runs.

/// The request to restore a backup on the next start, written into the backups directory.
pub const PENDING_RESTORE_FILE: &str = "pending-restore.json";
/// What the last applied request did, written into the backups directory.
pub const LAST_RESTORE_FILE: &str = "last-restore.json";

/// A restore that will be applied when the runtime starts next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRestore {
    /// Directory name of the backup (see [`BackupInfo::id`]).
    pub backup_id: String,
    pub requested_at: DateTime<Utc>,
    /// Identifies this one request, and is copied into its [`RestoreOutcome`]: the outcome of a
    /// request is recognised by it, never by comparing clock times (a clock can be set back). A
    /// request file written before ids existed has none; it gets one when its restore starts (the
    /// file is rewritten then to count the attempt), so a file that outlives its restore always
    /// carries the id of its outcome.
    #[serde(default)]
    pub request_id: String,
    /// How many starts began applying this request. The request stays in place while it is being
    /// applied, so a process that dies half-way tries again on the next start, up to
    /// [`MAX_RESTORE_ATTEMPTS`] times; a restore that kills its host every time is then given up
    /// instead of keeping the application from starting.
    #[serde(default)]
    pub attempts: u32,
}

/// The most starts that try to apply one restore request (the first, and one retry after a start
/// that was interrupted while applying it).
pub const MAX_RESTORE_ATTEMPTS: u32 = 2;

/// What applying a [`PendingRestore`] did. A failed restore leaves the previous data in place,
/// unless `rollback_incomplete` says otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreOutcome {
    pub backup_id: String,
    /// The [`PendingRestore::request_id`] this is the outcome of (empty for an older request).
    #[serde(default)]
    pub request_id: String,
    pub applied_at: DateTime<Utc>,
    pub succeeded: bool,
    /// Why it failed, when it did.
    pub error: Option<String>,
    /// The files the restore replaced, kept as `<name>.pre-restore-<time>` (see [`RestoreReport`]).
    /// After a failure with `rollback_incomplete`: the files that are still displaced.
    pub previous_files: Vec<PathBuf>,
    /// The restore failed *and* putting the previous data back did not finish (or the restore was
    /// interrupted too often to tell): the data directory is not as it was. The files in
    /// `previous_files` hold the previous data and have to be moved back by hand.
    #[serde(default)]
    pub rollback_incomplete: bool,
}

/// The directory of backup `backup_id` under `root`. The id is untrusted (it comes from a request
/// and from a file on disk): only the plain name of a backup directory is accepted.
fn backup_dir_for(root: &Path, backup_id: &str) -> SymphonyResult<PathBuf> {
    let plain = backup_id.starts_with(DIR_PREFIX)
        && !backup_id.contains(['/', '\\', '\0'])
        && Path::new(backup_id).components().count() == 1;
    if !plain {
        return Err(SymphonyError::InvalidConfig(format!(
            "`{backup_id}` is not the name of a backup"
        )));
    }
    let dir = root.join(backup_id);
    if !dir.is_dir() {
        return Err(SymphonyError::NotFound(format!(
            "backup `{backup_id}` does not exist"
        )));
    }
    Ok(dir)
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> SymphonyResult<()> {
    let json = serde_json::to_vec_pretty(value).map_err(|e| storage_err(e.to_string()))?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, json)?;
    std::fs::rename(&temp, path)?;
    Ok(())
}

/// Ask for backup `backup_id` (under `root`) to be restored on the next start. The backup is
/// verified first (manifest, checksums, a schema this build reads, SQLite's integrity check), so a
/// request for a damaged backup is refused now instead of failing after a restart. A request that
/// is already pending is replaced.
pub async fn schedule_restore(root: &Path, backup_id: &str) -> SymphonyResult<PendingRestore> {
    let dir = backup_dir_for(root, backup_id)?;
    // A backup that cannot be read is a problem with the backup; only recording the request is a
    // problem with this machine (`IoError`), which callers tell apart.
    verify_backup(&dir).await.map_err(|e| match e {
        SymphonyError::IoError(message) => storage_err(message),
        other => other,
    })?;
    let pending = PendingRestore {
        backup_id: backup_id.to_string(),
        requested_at: Utc::now(),
        request_id: uuid::Uuid::new_v4().to_string(),
        attempts: 0,
    };
    write_json_atomic(&root.join(PENDING_RESTORE_FILE), &pending)?;
    Ok(pending)
}

/// The restore that will be applied on the next start, if one was requested.
pub fn pending_restore(root: &Path) -> Option<PendingRestore> {
    let raw = std::fs::read(root.join(PENDING_RESTORE_FILE)).ok()?;
    serde_json::from_slice(&raw).ok()
}

/// Withdraw a requested restore. Returns whether there was one.
pub fn cancel_pending_restore(root: &Path) -> SymphonyResult<bool> {
    match std::fs::remove_file(root.join(PENDING_RESTORE_FILE)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// What the last applied restore request did, if any was applied.
pub fn last_restore(root: &Path) -> Option<RestoreOutcome> {
    let raw = std::fs::read(root.join(LAST_RESTORE_FILE)).ok()?;
    serde_json::from_slice(&raw).ok()
}

/// What [`recover_displaced_database`] found and did.
enum Recovery {
    /// The database is in place, or no earlier attempt displaced one.
    NotNeeded,
    /// The database an interrupted attempt set aside was put back.
    Recovered,
    /// Putting it back failed; these files still hold the data.
    Failed(Vec<PathBuf>),
}

/// A set-aside suffix as something that sorts by when it was made: the `%Y%m%dT%H%M%SZ` stamp, and
/// the collision number `shared_aside_stamp` appends (`-2`, `-3`, … `-10`); no number is the first.
fn suffix_order(suffix: &str) -> (String, u32) {
    match suffix.rsplit_once('-') {
        Some((stamp, number))
            if !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) =>
        {
            (stamp.to_string(), number.parse().unwrap_or(u32::MAX))
        }
        _ => (suffix.to_string(), 1),
    }
}

/// Put back the database an *interrupted* restore attempt set aside. An attempt that dies after it
/// renamed `symphony.sqlite` to `symphony.sqlite.pre-restore-<time>` leaves no database behind, and
/// a start that then fails for another reason (the backup is gone, say) would open a new, empty one
/// with the real data stranded under that name. The newest displaced database, with its `-wal` and
/// `-shm`, is renamed back; nothing is deleted. Only called when an earlier attempt began, so a
/// leftover from some older, finished restore is never mistaken for it.
fn recover_displaced_database(data_dir: &Path) -> Recovery {
    if data_dir.join(DATABASE_FILE).exists() {
        return Recovery::NotNeeded;
    }
    let prefix = format!("{DATABASE_FILE}.pre-restore-");
    let newest = std::fs::read_dir(data_dir).ok().and_then(|entries| {
        entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .strip_prefix(&prefix)
                    .map(str::to_string)
            })
            // Newest by time, then by the collision number: `-10` is later than `-9`.
            .max_by_key(|suffix| suffix_order(suffix))
    });
    let Some(suffix) = newest else {
        return Recovery::NotNeeded;
    };
    let mut still_displaced = Vec::new();
    for base in [
        DATABASE_FILE.to_string(),
        format!("{DATABASE_FILE}-wal"),
        format!("{DATABASE_FILE}-shm"),
    ] {
        let from = data_dir.join(format!("{base}.pre-restore-{suffix}"));
        if from.exists() && std::fs::rename(&from, data_dir.join(&base)).is_err() {
            still_displaced.push(from);
        }
    }
    if still_displaced.is_empty() {
        Recovery::Recovered
    } else {
        Recovery::Failed(still_displaced)
    }
}

/// Apply the restore requested with [`schedule_restore`], if any. **Call it at startup**, once the
/// data directory's runtime lock is held and before the database is opened ([`restore_backup`] is an
/// offline operation). The request stays in place while it is applied and is removed once the
/// restore has finished, or has failed and been rolled back: a process that dies half-way, with
/// the database already set aside, finds the request again on the next start and finishes the job
/// instead of starting on an empty database. A request that has been started
/// [`MAX_RESTORE_ATTEMPTS`] times is given up (reported, with `rollback_incomplete`, because what
/// the interrupted attempts left behind is unknown). The previous data stays in place when a
/// restore fails ([`restore_backup`] puts everything back). `Ok(None)` when nothing was requested.
pub async fn apply_pending_restore(
    root: &Path,
    data_dir: &Path,
) -> SymphonyResult<Option<RestoreOutcome>> {
    let marker = root.join(PENDING_RESTORE_FILE);
    if !marker.is_file() {
        return Ok(None);
    }
    let pending: Option<PendingRestore> = std::fs::read(&marker)
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok());

    // A request that already has its outcome is settled, whatever is left of its file: the start
    // that applied it could not remove the file (a locked or read-only folder), and applying it
    // again would put the backup over everything done since.
    if let (Some(request), Some(last)) = (&pending, last_restore(root)) {
        let settled = !request.request_id.is_empty() && last.request_id == request.request_id;
        if settled {
            tracing::warn!(
                backup = %request.backup_id,
                "A restore request that was already settled was still on disk; it is not applied again"
            );
            if let Err(err) = cancel_pending_restore(root) {
                tracing::error!(error = %err, "could not remove the settled restore request");
            }
            return Ok(None);
        }
    }

    let backup_id = pending
        .as_ref()
        .map(|p| p.backup_id.clone())
        .unwrap_or_default();
    // The id the outcome will carry. A request from before ids existed is given one now, and the
    // rewrite below (which counts the attempt) writes it into the file before anything is touched.
    let request_id = pending
        .as_ref()
        .map(|p| {
            if p.request_id.is_empty() {
                uuid::Uuid::new_v4().to_string()
            } else {
                p.request_id.clone()
            }
        })
        .unwrap_or_default();
    let failed = |failure: RestoreFailure| RestoreOutcome {
        backup_id: backup_id.clone(),
        request_id: request_id.clone(),
        applied_at: Utc::now(),
        succeeded: false,
        error: Some(failure.error.to_string()),
        previous_files: failure.displaced,
        rollback_incomplete: failure.rollback_incomplete,
    };
    // Whether an earlier start already began applying this request.
    let interrupted_before = pending.as_ref().is_some_and(|p| p.attempts >= 1);
    // Whether this attempt itself put back the database an interrupted one had set aside.
    let mut recovered_early = false;
    let result: Result<RestoreReport, RestoreFailure> = match pending {
        None => Err(storage_err("the restore request could not be read").into()),
        Some(pending) if pending.attempts >= MAX_RESTORE_ATTEMPTS => Err(RestoreFailure {
            error: storage_err(format!(
                "the restore was interrupted {} times and was given up. Look for files named `.pre-restore-…` in the data folder: they hold the data that was replaced.",
                pending.attempts
            )),
            rollback_incomplete: true,
            displaced: Vec::new(),
        }),
        Some(pending) => {
            // Counted before anything is touched, and the request kept: see above.
            let started = PendingRestore {
                attempts: pending.attempts + 1,
                request_id: request_id.clone(),
                ..pending.clone()
            };
            match write_json_atomic(&marker, &started) {
                Err(err) => Err(err.into()),
                Ok(()) => match backup_dir_for(root, &pending.backup_id) {
                    // An earlier start may have set the database aside and died. It is put back
                    // first, so that this attempt sets it aside again like any replaced file and
                    // lists it with the others, instead of leaving it behind under a name that no
                    // report mentions.
                    Ok(dir) => match interrupted_before.then(|| recover_displaced_database(data_dir)) {
                        Some(Recovery::Failed(_)) => Err(RestoreFailure {
                            error: storage_err(
                                "the restore was not applied: the database that an interrupted attempt set aside could not be put back first",
                            ),
                            rollback_incomplete: false,
                            // The recovery below names the files that are still displaced.
                            displaced: Vec::new(),
                        }),
                        early => {
                            recovered_early = matches!(early, Some(Recovery::Recovered));
                            restore_detailed(&dir, data_dir).await
                        }
                    },
                    Err(err) => Err(err.into()),
                },
            }
        }
    };
    // A failure after an interrupted attempt must not leave the runtime to open a new, empty
    // database: the one that attempt set aside is put back first.
    let result = match result {
        Err(mut failure) if interrupted_before => {
            let recovery = match recover_displaced_database(data_dir) {
                // Put back by this very attempt before it failed (a damaged backup, say): the
                // database is in place because of that, not because nothing had happened.
                Recovery::NotNeeded if recovered_early => Recovery::Recovered,
                other => other,
            };
            match recovery {
                // The database is in place, but an earlier start was interrupted while applying
                // this request: it may have installed the restored database (or moved the
                // workflow) before it died, so "left as it was" cannot be claimed.
                Recovery::NotNeeded => {
                    failure.error = storage_err(format!(
                        "{}. An earlier start was interrupted while applying this restore, so the data folder may be partly replaced. Look for files named `.pre-restore-…` in it: they hold the data that was replaced.",
                        failure.error
                    ));
                    failure.rollback_incomplete = true;
                }
                Recovery::Recovered => {
                    failure.error = storage_err(format!(
                        "{}. The database that an interrupted attempt had set aside was put back.",
                        failure.error
                    ));
                }
                Recovery::Failed(files) => {
                    failure.error = storage_err(format!(
                        "{}. The database that an interrupted attempt had set aside could not be put back.",
                        failure.error
                    ));
                    failure.rollback_incomplete = true;
                    failure.displaced.extend(files);
                }
            }
            Err(failure)
        }
        other => other,
    };
    let outcome = match result {
        Ok(report) => RestoreOutcome {
            backup_id,
            request_id,
            applied_at: Utc::now(),
            succeeded: true,
            error: None,
            previous_files: report.previous_files,
            rollback_incomplete: false,
        },
        Err(failure) => failed(failure),
    };
    // Finished, or failed and rolled back: the request is spent. The outcome is recorded first, so
    // that a request file which then cannot be removed is still recognised as settled (see above)
    // and is not applied over newer data by the next start.
    let recorded = write_json_atomic(&root.join(LAST_RESTORE_FILE), &outcome);
    if let Err(err) = &recorded {
        tracing::warn!(error = %err, "could not record the outcome of the restore");
    }
    if let Err(err) = cancel_pending_restore(root) {
        if recorded.is_err() {
            // Neither the note nor the removal worked: nothing prevents a replay. Say so loudly;
            // the host still starts, and the folder needs attention.
            tracing::error!(
                error = %err,
                path = %marker.display(),
                "the restore request could not be removed and its outcome could not be recorded; delete this file or the restore may be applied again at the next start"
            );
        } else {
            tracing::warn!(error = %err, "could not remove the settled restore request");
        }
    }
    Ok(Some(outcome))
}

/// What a host calls at startup, once it holds the data directory's runtime lock and before it opens
/// the database: apply the restore requested from the application (`<data dir>/backups`), if any,
/// and log what happened. It never fails the start: a restore that cannot be applied is reported
/// (log, and [`last_restore`] for the application) and the runtime starts on the data it has.
pub async fn apply_requested_restore(data_dir: &Path) -> Option<RestoreOutcome> {
    let root = data_dir.join("backups");
    match apply_pending_restore(&root, data_dir).await {
        Ok(Some(outcome)) if outcome.succeeded => {
            tracing::info!(
                backup = %outcome.backup_id,
                kept = outcome.previous_files.len(),
                "Restored the requested backup; the replaced files were kept"
            );
            Some(outcome)
        }
        Ok(Some(outcome)) => {
            tracing::error!(
                backup = %outcome.backup_id,
                error = outcome.error.as_deref().unwrap_or("unknown"),
                "The requested restore could not be applied; starting with the existing data"
            );
            Some(outcome)
        }
        Ok(None) => None,
        Err(err) => {
            tracing::error!(error = %err, "Could not process the requested restore; starting with the existing data");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_set_aside_by_one_restore_shares_one_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let stamp = "20261103T020000Z";
        assert_eq!(shared_aside_stamp(data, stamp), stamp);

        // A file kept by an earlier restore under the plain stamp moves the whole set to `-2`,
        // even if it is only the workflow, so the database and its WAL still match by name.
        std::fs::write(
            data.join(format!("{WORKFLOW_FILE}.pre-restore-{stamp}")),
            "w",
        )
        .unwrap();
        assert_eq!(shared_aside_stamp(data, stamp), format!("{stamp}-2"));
        std::fs::write(
            data.join(format!("{DATABASE_FILE}-wal.pre-restore-{stamp}-2")),
            "wal",
        )
        .unwrap();
        assert_eq!(shared_aside_stamp(data, stamp), format!("{stamp}-3"));
    }

    #[test]
    fn set_aside_suffixes_sort_by_time_and_then_numerically() {
        let mut suffixes = vec![
            "20261103T020000Z-10",
            "20261103T020000Z-2",
            "20261103T020000Z",
            "20261103T020000Z-9",
            "20261103T020001Z",
            "20261103T015959Z-12",
        ];
        suffixes.sort_by_key(|s| suffix_order(s));
        assert_eq!(
            suffixes,
            [
                "20261103T015959Z-12",
                "20261103T020000Z",
                "20261103T020000Z-2",
                "20261103T020000Z-9",
                "20261103T020000Z-10",
                "20261103T020001Z",
            ]
        );
    }

    #[test]
    fn a_database_and_its_wal_set_aside_together_are_recovered_together() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        // An earlier restore left a database under the plain stamp; the interrupted one used the
        // shared suffix for the database and the WAL alike.
        let stamp = "20261103T020000Z";
        std::fs::write(
            data.join(format!("{DATABASE_FILE}.pre-restore-{stamp}")),
            "old",
        )
        .unwrap();
        let shared = shared_aside_stamp(data, stamp);
        assert_eq!(shared, format!("{stamp}-2"));
        std::fs::write(
            data.join(format!("{DATABASE_FILE}.pre-restore-{shared}")),
            "db",
        )
        .unwrap();
        std::fs::write(
            data.join(format!("{DATABASE_FILE}-wal.pre-restore-{shared}")),
            "wal",
        )
        .unwrap();

        assert!(matches!(
            recover_displaced_database(data),
            Recovery::Recovered
        ));
        assert_eq!(
            std::fs::read_to_string(data.join(DATABASE_FILE)).unwrap(),
            "db"
        );
        assert_eq!(
            std::fs::read_to_string(data.join(format!("{DATABASE_FILE}-wal"))).unwrap(),
            "wal",
            "the WAL comes back with its database"
        );
        // The older leftover is untouched.
        assert!(data
            .join(format!("{DATABASE_FILE}.pre-restore-{stamp}"))
            .is_file());
    }
}
