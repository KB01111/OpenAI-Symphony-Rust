# Production Readiness

What stands between a commit and a shipped Symphony Desktop build, how to operate it, and what still
needs a decision or an action by the repository owner. Security mechanisms are described in
[`SECURITY.md`](../SECURITY.md); the hardening history is in
[`POST_MERGE_STABILIZATION.md`](./POST_MERGE_STABILIZATION.md).

## 1. Release gates (automated)

| Gate | Where | Fails when |
| --- | --- | --- |
| Rust format, clippy (`-D warnings`), tests incl. Windows-only native tests | `ci.yml` (windows-latest) | any warning or failing test |
| Frontend typecheck, Vitest, production builds; iOS companion build | `ci.yml` (windows-latest, macos-latest) | type error, failing test, build error |
| API contract generated and deterministic; versioning policy | `api-contract.yml` | stale contract, breaking change without label + `API_VERSION` bump |
| Playwright against a real runtime | `ui-e2e.yml` | UI flow regression |
| Tauri release build + native smoke test (DACLs, DPAPI, single scheduler, crash recovery, attach mode, desktop log) | `desktop-native.yml` | the release binaries misbehave on Windows |
| RustSec advisories, licenses, sources; npm advisories (moderate+) | `supply-chain.yml` (dependency changes, `main`, weekly) | a new advisory or an unreviewed license |
| Version = tag = dated changelog section; tests; native smoke test; signature check; checksums; draft release | `release.yml` (`v*` tags) | any of the above |

Every release is a **draft** until a maintainer has run the manual checklist in
[`WINDOWS_NATIVE_SMOKE_TEST.md`](./WINDOWS_NATIVE_SMOKE_TEST.md) with the drafted installer.

## 2. Operations

- **Data directory**: `%LOCALAPPDATA%\symphony-desktop` (override with `SYMPHONY_DATA_DIR` /
  `--data-dir`): `symphony.sqlite` (WAL), `credentials.vault` (DPAPI), `runtime.json`, `WORKFLOW.md`,
  `logs\`. Back it up with Symphony closed (tray → *Stop runtime and exit*) so the SQLite WAL is checkpointed; the vault only
  decrypts for the same Windows account.
- **Logs**: `logs\desktop-YYYY-MM-DD.log` / `logs\cli-YYYY-MM-DD.log`: owner-only, redacted, 14 files and
  50 MiB per day per component, panics included. `RUST_LOG` raises the level.
- **Bug reports**: Settings → Diagnostics → *Export diagnostics* (redacted JSON) plus the day's log file.
- **Crash recovery**: tasks left `Running` by a crash are re-queued on the next start; agent process trees
  die with the host (Job Object), so no orphaned agent keeps editing a worktree.
- **Headless runtime**: `symphony-cli --data-dir <dir>` (Ctrl+C, and SIGTERM on Unix, shut it down gracefully); the
  desktop attaches to it instead of starting a second scheduler.

## 3. Owner actions before the first public release

These cannot be done from the repository and are deliberately not faked:

1. **Code signing**: add the repository secrets `WINDOWS_CERTIFICATE` (base64 `.pfx` of an Authenticode
   certificate, ideally EV/OV) and `WINDOWS_CERTIFICATE_PASSWORD`. Without them the release workflow only
   produces assets named `UNSIGNED`, which SmartScreen blocks and which must not be published.
2. **Branch protection** on `main`: require the checks of `ci.yml`, `desktop-native.yml`,
   `api-contract.yml`, `ui-e2e.yml` and `supply-chain.yml`, and enable Dependabot security updates.
3. **First release**: date the `## [2.0.0]` heading in `CHANGELOG.md`, merge, push the `v2.0.0` tag, run
   the manual Windows 11 checklist with the draft installer, then publish the draft.

## 4. Known limitations and accepted risks

- **No auto-update**: users install new versions manually. Tauri's updater needs an update-signing key
  pair and a hosted manifest; decide on hosting before enabling it.
- **Windows only** for the packaged desktop app. The CLI runs on Linux/macOS; there the vault uses an
  owner-only key file instead of an OS keychain, and agent trees are tracked by process group (a host killed
  with SIGKILL cannot clean up; Windows Job Objects can).
- **Remote access** has no built-in TLS: put it behind a TLS reverse proxy or an encrypted tunnel.
- **Dependencies**: unmaintained
  transitive crates under Tauri/GTK and `serde_yaml` (no maintained drop-in yet). Tracked by the weekly
  supply-chain run and Dependabot.
- **Manual-only native behaviour**: tray, folder picker, notifications, window-state restore and sign-in
  autostart (see the manual checklist).
- **iOS companion** is validated as a web build; no native iOS packaging.

## 5. Dependency upgrades that need an upstream release

Dependabot keeps proposing these; close the PRs until the blocker is gone (`dependabot.yml` deliberately has
no ignore rules, because they would also suppress security updates).

| Upgrade | Blocker | Unblocked when |
| --- | --- | --- |
| TypeScript 7 in `packages/shared-types` | TypeScript 7 (the native compiler) no longer ships the JavaScript compiler API that `@hey-api/openapi-ts` (0.99) uses to generate the API client, so `pnpm generate:api` fails with it. Every other package uses TypeScript 7; `packages/shared-types`, which owns the generator, stays on TypeScript 6 so the generator's `typescript` peer resolves there. | `@hey-api/openapi-ts` supports TypeScript 7; then move `packages/shared-types` to TypeScript 7 too. |
