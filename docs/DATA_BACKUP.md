# Data, backups and recovery

Where Symphony Desktop keeps its data, what a backup contains, and how to get back to a working state.
Implementation: `crates/symphony-storage/src/backup.rs` (tests in `crates/symphony-storage/tests/`).

## Data directory

`%LOCALAPPDATA%\symphony-desktop` (override: `SYMPHONY_DATA_DIR` / `--data-dir`).

| File | Content | In a backup |
| --- | --- | --- |
| `symphony.sqlite` (+ `-wal`, `-shm`) | projects, tasks, run attempts, agent events, approvals, pull request reviews, validation results, settings, provider profiles | yes |
| `WORKFLOW.md` | the active workflow document | yes |
| `credentials.vault` | API keys and tokens, **DPAPI-protected for your Windows account** | **never** |
| `runtime.json`, `symphony-runtime.lock` | endpoint and token of the running runtime | no |
| `logs\` | redacted diagnostic logs | no |
| `workspaces\` | Git worktrees of tasks | no (they are Git branches; push what matters) |
| `backups\` | the backups themselves | – |

## What a backup is

`POST /api/v1/backups` (Settings → Backups → *Create backup*) writes
`backups\symphony-backup-<UTC time>\` containing

- `symphony.sqlite`: a transactionally consistent copy made with SQLite's `VACUUM INTO`. It can be taken
  while agents run and includes everything already committed to the write-ahead log. The live file is never
  copied byte for byte.
- `WORKFLOW.md` (when one exists) and `manifest.json` (format, app and schema version, creation time, size
  and SHA-256 of every file).

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

Restoring replaces the database, so it is an **offline** operation. It needs the `symphony-cli` executable
from the release assets (the installer ships the desktop app only).

1. Quit Symphony (tray menu → *Stop runtime and exit*). The command refuses to run while a runtime owns the
   data directory.
2. Check the backup: `symphony-cli verify-backup "<backup folder>"` (checksums, a schema this version can
   read, SQLite `integrity_check`).
3. Restore: `symphony-cli --data-dir "%LOCALAPPDATA%\symphony-desktop" restore-backup "<backup folder>"`.
4. Start Symphony again.

The replaced `symphony.sqlite` (with its `-wal`/`-shm`) and `WORKFLOW.md` are renamed
`<name>.pre-restore-<time>`; nothing is deleted. If the restored database cannot be opened and migrated, the
previous files are put back and the command fails. A backup made by an older version is migrated forward on
restore; a backup from a **newer** version is refused.

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
