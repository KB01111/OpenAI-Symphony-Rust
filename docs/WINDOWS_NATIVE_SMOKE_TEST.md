# Windows 11 Native Smoke Test

Symphony Desktop's native behaviour is validated in four layers. Only the first three run in CI; the
last needs a person at a Windows 11 desktop and must be done before a release.

| Layer | Where | Covers |
| --- | --- | --- |
| Rust tests | `cargo test --workspace` on `windows-latest` (`ci.yml`) | `crates/symphony-platform/tests/windows_native.rs`: owner-only DACL on `runtime.json` and the vault (also inside a world-writable directory), replacement of an untrusted discovery file, DPAPI vault round-trip, HKCU autostart write/read-back/remove. `crates/symphony-cli/tests/runtime_lifecycle.rs`: discovery, authenticated API, single scheduler, crash recovery, no token in logs, explicit vault quarantine. |
| Automated native smoke | `scripts/windows-native-smoke.ps1` (`desktop-native.yml`, after `tauri build`) | The release `symphony-desktop.exe` and `symphony-cli.exe`: runtime start in `--background` mode, DACLs in a world-writable `SYMPHONY_DATA_DIR`, token-protected API, remote access off by default, DPAPI vault persistence across restart, CLI refused while the desktop owns the data directory, second desktop instance exits, crash and recovery without orphaned processes, desktop attach mode while the CLI owns the runtime. |
| Installer smoke | `scripts/windows-installer-smoke.ps1` (`desktop-native.yml` and `release.yml`, after the native smoke) | The NSIS installer on a clean per-user profile: silent install, the uninstall entry (name, version, publisher), the **installed** app passing the native smoke test above, upgrade over the installed app, silent uninstall, user data in `%LOCALAPPDATA%\symphony-desktop` surviving. Shortcuts are only warned about. |
| Manual (below) | A Windows 11 machine | Tray, window lifecycle and state restore, folder picker, Explorer integration, notifications, autostart at sign-in, installer. |

Run the automated smoke test locally (PowerShell 7, from the repository root):

```powershell
pnpm install --frozen-lockfile
pnpm --filter @symphony/desktop tauri build --bundles nsis
cargo build -p symphony-cli --release --locked
pwsh -File scripts/windows-native-smoke.ps1 -DesktopExe target/release/symphony-desktop.exe -CliExe target/release/symphony-cli.exe
```

## Manual checklist

Use a fresh data directory so existing state does not interfere:

```powershell
$env:SYMPHONY_DATA_DIR = "$env:TEMP\symphony-manual"
Remove-Item $env:SYMPHONY_DATA_DIR -Recurse -Force -ErrorAction SilentlyContinue
```

`SYMPHONY_DATA_DIR` only reaches processes started from this PowerShell window. From step 2 on, start
Symphony from the same window (`& "$env:LOCALAPPDATA\Symphony Desktop\symphony-desktop.exe"`, or the
path you installed to) so it uses the fresh directory; a copy started from the Start menu or the tray
uses `%LOCALAPPDATA%\symphony-desktop` instead. Steps 1 and 7 deliberately use the Start menu.

Record the Windows build (`winver`), the commit and pass/fail for each step.

1. **Installer (unsigned)** — Download `symphony-desktop-windows-installer-UNSIGNED` from the
   `desktop-native` workflow run. SmartScreen warns about an unknown publisher: this is expected for an
   unsigned build, which must not be distributed as a production release. Install per-user, launch from the
   Start menu (default data directory), exit from the tray, and uninstall at the end. Record whether `%LOCALAPPDATA%\symphony-desktop` (database,
   vault, discovery file — created by the runtime, not by the installer) is still present afterwards.
2. **First start** — The window opens; Settings → Application shows mode *Embedded*, an endpoint on
   `127.0.0.1` and no discovery warning. `icacls "$env:SYMPHONY_DATA_DIR\runtime.json"` lists only your
   account and `NT AUTHORITY\SYSTEM`, with no `(I)` inherited entries.
3. **Credential vault** — Providers → save an API key. Close Symphony completely (tray → *Stop runtime and
   exit*), start it again: the key still shows as configured. `credentials.vault` contains
   `windows-dpapi-user` and no plaintext key. Copy the vault to another Windows account and start Symphony
   there with that data directory: it must stop with a credential vault error, not start empty.
4. **Corrupt vault recovery** — With Symphony stopped, append garbage to `credentials.vault` and start it.
   A dialog explains the problem: *No* quits without changing the file; *Yes* moves it to
   `credentials.vault.corrupt-<time>` and starts with an empty vault.
5. **Tray and window lifecycle** — Close the window with *Minimize to the notification area* on: the window
   hides, the tray icon stays and agents keep running. Left-click the tray icon to restore it. Use *Pause* /
   *Resume task dispatching* and check the Overview reflects it. With the setting off, closing exits.
6. **Window state** — Resize and move the window, exit from the tray and start again: size and position are
   restored (tauri-plugin-window-state).
7. **Single instance** — Start Symphony a second time from the Start menu: no second window or runtime
   appears; the existing window is focused.
8. **CLI attach** — Exit Symphony. Run `symphony-cli --data-dir $env:SYMPHONY_DATA_DIR`, then start the
   desktop app: Settings → Application shows *Attached to an external runtime*; closing the window does not
   stop the CLI. Starting a second `symphony-cli` for the same directory fails with *already running*.
9. **Folder picker** — Projects → *Add repository* → *Browse*: the native folder dialog opens; cancelling
   leaves the form unchanged; selecting a Git repository fills the path.
10. **Explorer integration** — Open a task with a worktree → *Open worktree*: Explorer opens that folder only.
11. **Notifications** — With the window unfocused, run a task that requests approval: a Windows notification
    appears. No notification is shown while the window is focused.
12. **Autostart** — Settings → Desktop behaviour → enable *Start the background runtime when I sign in*,
    save. `reg query HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v SymphonyDesktopRuntime` shows the
    quoted executable with `--background`. Sign out and in: the runtime starts hidden in the tray. Disable
    and save: the value is gone. To check the failure path, deny yourself *Set Value* on the `Run` key
    (regedit → Permissions), toggle the setting and save: an error is shown and the switch returns to its
    previous state.
13. **Remote access** — Settings → Remote access: enable, keep `127.0.0.1`, save; the status shows *Waiting
    for an access token*. *Generate token* shows the token once; the status changes to *Running* without a
    restart. Expose the port through a TLS reverse proxy or tunnel and connect the companion with the
    `https://` URL and token. *Rotate token* disconnects the companion; *Revoke token* stops the listener.
    Entering `0.0.0.0` as bind address is rejected.
14. **Shared data directory** — Point `SYMPHONY_DATA_DIR` at a folder where `Everyone` has full control
    (`icacls <dir> /grant *S-1-1-0:(OI)(CI)F`) and start Symphony: `runtime.json` and `credentials.vault` are
    still restricted to your account and SYSTEM (step 2 check).
15. **Backup and restore** — Settings → Backups → *Create backup* while a task runs: a folder
    `backups\symphony-backup-<time>` appears with `symphony.sqlite`, `manifest.json` (and `WORKFLOW.md`), and
    no credential file. Exit from the tray, run `symphony-cli verify-backup "<folder>"`, change something
    (rename a task), then `symphony-cli --data-dir "$env:SYMPHONY_DATA_DIR" restore-backup "<folder>"`: the
    old state is back and `*.pre-restore-*` files hold the replaced ones. Run it once while Symphony is
    still open: it must refuse.
16. **Create pull request** — Against a throw-away GitHub repository with a token saved under Settings →
    Integrations: finish a task, open its review, *Create pull request* → confirm. A branch and a pull
    request appear on GitHub (nothing merged), the review shows its `#number`, repeating the click opens
    nothing new, and a review with a security warning or conflict offers no button.
17. **Upgrade keeps data** — Install the previous build, create data, install this one over it: projects,
    tasks and history are intact and a `symphony.sqlite.pre-migration-v*.bak` was written. Uninstall and
    record whether `%LOCALAPPDATA%\symphony-desktop` survives (`docs/DATA_BACKUP.md` says it should).
18. **Language of native UI** — Settings → Appearance → switch to Svenska: the tray menu (right-click the
    icon) is Swedish at once (*Öppna Symphony Desktop*, *Pausa uppgiftsutskick* …), and a notification
    (step 11) is Swedish. Switch back: both are English again. Restart: the tray starts in the saved language.
19. **Folders, shortcuts, updates** — Settings → Application: *Open data folder* and *Open log folder* open
    Explorer on the data directory the app runs with (the fresh `SYMPHONY_DATA_DIR` folder if you started it
    from the prepared shell, otherwise `%LOCALAPPDATA%\symphony-desktop`) and its `logs` subfolder. *Show keyboard shortcuts* (also `F1`
    and `Ctrl+/`) lists the shortcuts, and each one works. *Check for updates* with network access reports
    *up to date* (or an available version with *Open release page*); with the network off it shows an error,
    never *up to date*.
20. **Installer details** — The installer offers English and Swedish, shows the license and the Symphony
    artwork, installs per user without an administrator prompt, creates a Start Menu entry with the Symphony
    icon, and the uninstall entry in *Settings → Apps* shows the publisher and version. (CI already covers
    silent install/upgrade/uninstall: `scripts/windows-installer-smoke.ps1`.)
21. **Shutdown** — Exit from the tray. Task Manager shows no `symphony-desktop.exe` or `symphony-cli.exe`
    left behind, and `runtime.json` has been removed.
