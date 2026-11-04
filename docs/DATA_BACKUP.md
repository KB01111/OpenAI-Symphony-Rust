# Data, backups and recovery

Where Symphony Desktop keeps its data, what a backup contains, and how to get back to a working state.
Implementation: `crates/symphony-storage/src/backup.rs` (tests in `crates/symphony-storage/tests/`).

## Data directory

`%LOCALAPPDATA%\symphony-desktop` (override: `SYMPHONY_DATA_DIR` / `--data-dir`).

| File | Content | In a backup |
| --- | --- | --- |
| `symphony.sqlite` | projects, tasks, run attempts, agent events, approvals, pull request reviews, validation results, settings, provider profiles | yes: one consistent snapshot that includes the committed write-ahead-log content |
| `symphony.sqlite-wal`, `-shm` | SQLite sidecar files of the running database | no (not copied; the snapshot already contains what they hold) |
| `WORKFLOW.md` | the active workflow document | yes |
| `credentials.vault` | API keys and tokens, **DPAPI-protected for your Windows account** | **never** |
| `runtime.json`, `symphony-runtime.lock` | endpoint and token of the running runtime | no |
| `logs\` | redacted diagnostic logs | no |
| `workspaces\` | Git worktrees of tasks | no (they are Git branches; push what matters) |
| `backups\` | the backups themselves | – |

## What a backup is

Backups are offered for the default database `<data dir>\symphony.sqlite` only. A runtime started with
`--db-path` (a database elsewhere) does not manage backups, because `restore-backup` restores
`<data dir>\symphony.sqlite` and could never restore that database.

`POST /api/v1/backups` (Settings → Backups → *Create backup*) writes
`backups\symphony-backup-<UTC time>\` containing

- `symphony.sqlite`: a transactionally consistent copy made with SQLite's `VACUUM INTO`. It can be taken
  while agents run and includes everything already committed to the write-ahead log. The live file is never
  copied byte for byte.
- `WORKFLOW.md` (when one exists) and `manifest.json` (format, app and schema version, creation time, size
  and SHA-256 of every file).

### Automatic backups

The runtime (desktop host, or `symphony-cli` without `--db-path`) also backs up on a schedule, set under
Settings → Backups:

- **Automatic backup every (hours)** (`backup_interval_hours`, default 24, `0` = off, at most 720): checked
  every 15 minutes (the first check 3 minutes after start); a backup is made when the newest backup **of any
  kind** is older than the interval, so a backup made by hand also resets the clock.
- **Automatic backups to keep** (`backup_keep`, default 7, 1–100): after each automatic backup, older
  automatic backups beyond this number are deleted. Backups made by hand (*Create backup*, the API) and
  backups written before this version (their manifest has no `scheduled` flag) are **never** deleted, and
  neither are incomplete `.partial` directories.

Automatic backups are marked *Automatic* in the list (`scheduled: true` in `manifest.json` and in
`GET /api/v1/backups`). A failed automatic backup is logged and shown as a `backup/failed` event; the next
check tries again.

A backup is built in a `.partial` directory and renamed when complete, so an interrupted backup is never
listed or restorable. Backups live on the same disk as the data: **copy them elsewhere** (another drive,
cloud storage) if they are meant to survive a disk failure.

### Credentials

Credentials are deliberately excluded. The vault can only be decrypted by the Windows account that created it
(on the same machine); a backup restored under another Windows identity could not use it, and shipping it
inside general backups would invite exactly that mistake. To keep your keys, copy `credentials.vault` with
Symphony closed and restore it only for the same Windows user, or enter the keys again under
Settings → Integrations. There is no migration mechanism for the vault between Windows accounts.

## Verify and restore

Restoring replaces the database, which cannot happen while a runtime has it open. There are two ways:

**From the application (Settings → Backups → *Restore…*).** The backup is checked right away (manifest,
checksums, a schema this version can read, SQLite `integrity_check`), so a damaged backup is refused with a
reason instead of failing later. Nothing is replaced yet: the request is recorded
(`backups\pending-restore.json`) and **applied the next time the runtime starts**, after it holds the data
directory's lock and before it opens the database. Quit Symphony (the page offers *Quit Symphony*) and start it
again. The page then shows what happened (`backups\last-restore.json`): a restore that cannot be applied is
reported once, leaves the existing data as it was and is not retried on every start. *Withdraw* cancels a
request that is still waiting. The request is local administration only (`POST /api/v1/backups/{id}/restore`,
`GET`/`DELETE /api/v1/backups/restore`); the remote companion cannot reach it. When the window is attached to a
`symphony-cli` runtime, it is that runtime that must be restarted.

While a request waits, the backup it names is **kept** by the automatic retention, however old it is. The request
file stays in place while the restore is applied and is removed when the restore has finished, or has failed and
been rolled back; if the process dies half-way (the database already set aside), the next start finishes the job
instead of opening an empty database. A request that was started twice without finishing is given up and reported
as *left your data partly replaced*, with the `.pre-restore-<time>` files that hold the previous data. A restore
that fails and cannot put everything back is reported the same way, never as "left as it was". A request that already has its
outcome is never applied a second time, even if its file could not be removed (a locked or read-only folder).
Restoring from the
application replaces the database and the `WORKFLOW.md` **in the data directory**; a workflow file the runtime
reads from elsewhere (`--workflow`, or `WORKFLOW.md` in the working directory) is yours and is not touched, and
still takes precedence.

**From the command line, while nothing runs.** This needs the `symphony-cli` executable from the release
assets (the installer ships the desktop app only).

1. Quit Symphony (tray menu → *Stop runtime and exit*, in Swedish *Stoppa körtiden och avsluta*). The command refuses to run while a runtime owns the
   data directory.
2. Check the backup: `symphony-cli verify-backup "<backup folder>"` (checksums, a schema this version can
   read, SQLite `integrity_check`).
3. Restore: `symphony-cli --data-dir "%LOCALAPPDATA%\symphony-desktop" restore-backup "<backup folder>"`.
4. Start Symphony again.

A restore that was requested from the application and is still waiting is withdrawn by the command (the backup
restored here is the later request); the command says so.

Either way, the replaced `symphony.sqlite` (with its `-wal`/`-shm`) and `WORKFLOW.md` are renamed
`<name>.pre-restore-<time>`; nothing is deleted. If the restored database cannot be opened and migrated, the
previous files are put back and the command fails. A backup made by an older version is migrated forward on
restore; a backup from a **newer** version is refused. Credentials are not part of a backup and are never touched.

## Migrations and failure recovery

- The schema is versioned (`schema_migrations`); each migration runs in its own transaction.
- Before the first migration of a database that holds data, a consistent snapshot is written next to it:
  `symphony.sqlite.pre-migration-v<N>-<time>.bak`. If an upgrade misbehaves, quit Symphony, delete
  `symphony.sqlite-wal` and `-shm`, and rename the snapshot to `symphony.sqlite`.
- A database written by a newer Symphony is refused (`the database uses schema version N, which is newer…`)
  instead of being guessed at. Install the newer version or restore a backup made by this one.
- A corrupt database: restore the newest verified backup as above. The corrupt files are kept as
  `.pre-restore-<time>` and can be sent to the maintainers.
- An unreadable credential vault is never replaced silently; see `SECURITY.md` §2.

## Uninstall and updates

The installer is per-user (`installMode: currentUser`). The data directory above is not part of the install
directory; whether the NSIS uninstaller leaves it untouched has to be confirmed with the manual checklist in
`docs/WINDOWS_NATIVE_SMOKE_TEST.md` (it was not verifiable in the environment that produced this document).
A reinstall or upgrade must find the same data directory.
