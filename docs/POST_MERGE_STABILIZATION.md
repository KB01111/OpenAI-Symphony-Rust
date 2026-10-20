# Post-Merge Stabilization Report (Symphony Desktop 2.0)

Baseline: `main` at `2dc6c719` (merge of PR #4, *Symphony Desktop 2.0 UI Overhaul*).
Scope: security, reliability, Windows 11 integration and contract enforcement of what PR #4 introduced.
No new frameworks, services or product features were added.

Each item is marked:

- **Verified defect** — a concrete flaw confirmed by reading the code at the baseline (and, where
  possible, reproduced by a test that fails on the old behaviour).
- **Hardening** — a preventative improvement; the baseline was not demonstrably exploitable in its
  default configuration, but the change removes a risk or makes behaviour observable.

---

## 1. Findings and changes

### 1.1 Credential vault (`crates/symphony-platform`) — P0

| Finding | Class | Change |
| --- | --- | --- |
| `SecretStore` "encrypted" credentials by XOR with `SHA-256("symphony-desktop-v1:{USERNAME}@{COMPUTERNAME}")`, a key derivable by anyone who knows the user and host name; no integrity protection. | Verified defect | New versioned vault (format 2): **Windows DPAPI** (`CryptProtectData`, current user, `CRYPTPROTECT_UI_FORBIDDEN`, envelope header as entropy); **other platforms** ChaCha20-Poly1305 (`ring`) with a random 256-bit key in an owner-only `credentials.key` that is verified on load. Key names are encrypted too. A `VaultProtector` trait isolates the platform mechanism. No XOR fallback exists. |
| `std::fs::write` without restrictive permissions (vault inherited the directory ACL / umask). | Verified defect | All writes go through `secure_fs::write_private_atomic` (owner-only temp file → `fsync` → verify → rename). |
| Non-atomic persistence: a crash mid-write could truncate the vault. | Verified defect | Atomic replace; a failed write leaves the previous file untouched. |
| `SecretStore::new` silently started with an **empty** vault when the file could not be decoded; the next save overwrote the old file → silent credential loss. | Verified defect | Unreadable, unauthenticated or newer-format vaults fail with `SymphonyError::CredentialVault` and the file is never touched. Recovery is explicit: desktop dialog (*move aside / quit*) or `symphony-cli --quarantine-credential-vault`, keeping `credentials.vault.corrupt-<time>`. A missing key file next to an existing vault is an error, never a reason to generate a new key. |
| The in-memory cache was updated before the write, so a failed write left memory and disk inconsistent. | Verified defect | Changes are applied to a copy, persisted, and only then swapped in; discarded values are zeroized. |
| Legacy vaults. | — | Format-1 files are decoded with the old algorithm, re-written in format 2, read back and compared before use. Requires the same user and computer name (the legacy key), as before. |

The public interface (`new`, `set_secret`, `get_secret`, `get_vault_secret`, `delete_secret`,
`list_configured_keys`, `all_secret_values`) is unchanged; `quarantine_vault`, `vault_path` and
`protection` were added. Redaction still receives every vault value (`all_secret_values`); `Debug` output
lists key names only.

Tests: `vault::tests` (restart persistence, no plaintext on disk, delete, legacy migration, corrupt file
untouched + quarantine, tampered ciphertext, newer format, failed write consistency, missing / world-readable
key file, `Debug` without values), `windows_native.rs` (DPAPI vault owner-only in a world-writable
directory), `runtime_lifecycle.rs` (CLI refuses a corrupt vault until quarantined).

### 1.2 Runtime discovery on Windows — P0

| Finding | Class | Change |
| --- | --- | --- |
| `RuntimeDiscovery::write` set mode `0600` on Unix only; on Windows `runtime.json` (bearer token) inherited the directory ACL, so a shared `--data-dir` exposed the token (PR #4 threads by entelligence-ai, still open). | Verified defect (non-default configuration) | `secure_fs::create_private_file` creates the temp file with `CreateFileW` and a **protected DACL** (`D:P(A;;FA;;;<user SID>)(A;;FA;;;SY)`) — applied atomically at creation, before any data is written. The DACL is verified (no NULL DACL, no allow-ACE for any other principal, owner = user or Administrators) before the atomic rename; if it cannot be enforced (e.g. FAT) nothing is published. |
| `RuntimeDiscovery::read_live` trusted any `runtime.json`, so a file planted by another account in a shared directory could redirect clients to a server it controls. | Hardening | Readers ignore discovery files that are not private to the current user (Rust on all platforms; the Node reader on Unix). |
| The desktop host ignored `RuntimeDiscovery::write` errors (`let _ = …`). | Verified defect | Failures are logged (without the token), raised as a native notification and returned by `get_local_api_config` (`discovery_error`), shown in Settings → Application. No insecure fallback: nothing is published. The CLI already treated the error as fatal. |
| `RuntimeDiscovery` derived `Debug` including the token. | Hardening | Manual `Debug` with `token: [redacted]`. |

Tests: `secure_fs::tests` (Unix mode/owner/symlink rejection, atomic replace, failed replace keeps the old
file), `crates/symphony-platform/tests/windows_native.rs` (owner-only protected DACL; protection inside a
world-writable directory; an inherited-permission file is rejected and replaced; rewrite after restart),
`runtime_lifecycle.rs`, `scripts/windows-native-smoke.ps1`.

### 1.3 Remote API access — P1

| Finding | Class | Change |
| --- | --- | --- |
| `SYMPHONY_REMOTE_TOKEN` could not be configured from the product: it was not in the credential allowlist and the Remote settings had no token workflow. | Verified defect | Runtime-generated 256-bit token: `POST /remote-access/token` (issue/rotate, returned once, `Cache-Control: no-store`), `DELETE /remote-access/token` (revoke). Settings → Remote access shows the token once with copy, and asks before rotating/revoking. The generic credential API still cannot read or write it. |
| The remote listener was started once at boot; settings changes did nothing until restart, and the UI only showed persisted settings. | Verified defect | `RemoteAccessManager` (desktop and CLI) reconciles the listener on every settings save and token change (start/stop/rebind), and `GET /remote-access` reports the **effective** state (`disabled`, `token_missing`, `running`, `failed` + error such as *address in use*). |
| The remote listener used the full local router: a valid remote token could read/write credentials, settings and `WORKFLOW.md`, export diagnostics, administer projects and approve merges. | Verified defect | Remote scope: only `REMOTE_OPERATIONS` (state, events, projects/tasks read, task create/dispatch/cancel/retry, agent approvals, PR review read, scheduler pause/resume). Everything else answers `403 forbidden_scope`. The OpenAPI document marks each operation `x-symphony-remote`; a test fails on unclassified endpoints. |
| Remote CORS reused the local origins (desktop webview + Vite dev servers). | Verified defect | Remote CORS = companion origin (`tauri://localhost`) + validated `remote_allowed_origins` (bare `https://`, `http://` only for loopback, no `*`). |
| The bearer token could travel in cleartext (`remote_bind_address` accepted `0.0.0.0`; the companion defaulted to `http://`). | Verified defect | Loopback-only binding (validated by `PUT /settings`, refused by the manager), `X-Forwarded-Proto: http` refused (`insecure_transport`), companion defaults to `https://` and refuses `http://` to non-loopback hosts, including URLs saved by older versions. |
| Open SSE streams outlived token changes. | Hardening | Streams end when their token is rotated or revoked; stopping the listener drops all connections. |
| Raw `WORKFLOW.md` exposes literal credentials (Devin thread on PR #4). | Partly accepted design | Locally this remains as designed (the editor needs the source; only same-user processes hold the local token). The remote scope now blocks it entirely. |

Tests: `crates/symphony-server/tests/remote_access.rs` (13 tests over real TCP listeners: disabled by
default, token required, local/remote tokens not interchangeable, 21 sensitive operations forbidden,
companion operations allowed, rotation/revocation, SSE stream ended by rotation + reconnect, CORS
allow/deny, cleartext forwarding refused, rebind on port change, bind failure + recovery, non-loopback and
local-port refusal, local admin endpoints), `remote.rs` unit tests, `packages/api-client/test/integration/remote.test.ts`
(the generated client against `symphony-cli`: enable → issue → companion calls → forbidden admin calls →
rotate → reconnect with `setConnection` → revoke → unsafe settings rejected), desktop `settings.test.tsx`,
mobile `connection.test.ts`.

### 1.4 Dependabot: `@vitest/mocker` — P1

| Finding | Class | Change |
| --- | --- | --- |
| Dependabot run 39487565760 could not raise `@vitest/mocker` 3.2.7 to the fixed 4.1.11 because all four packages pinned `vitest ^3`. | Verified defect | `vitest` upgraded to `^4.1.11` in `apps/desktop`, `apps/mobile`, `packages/api-client`, `packages/ui` (no overrides). The lockfile resolves one `@vitest/mocker@4.1.11`. 4.1.11 (Aug 2026) was chosen over the 5-day-old 4.1.12. No test or config changes were needed with Vite 6.4, React 19 and TypeScript 5.8. |

**Remaining advisories** (`pnpm audit`, documented, not suppressed; out of scope for this upgrade):

| Package | Severity | Path | Notes |
| --- | --- | --- | --- |
| lodash 4.17.21 | high + 2 moderate | `@openai/apps-sdk-ui` → lodash | Fixed in ≥ 4.17.24; needs an in-range refresh or an Apps SDK UI update. |
| js-yaml 4.2.0 | 3 high (CPU) | `@hey-api/openapi-ts` → `@hey-api/json-schema-ref-parser` → js-yaml | Build-time only (client generation from a committed, trusted document). |
| katex 0.16.47 | low | `@openai/apps-sdk-ui` → rehype-katex / remark-math → katex | Fixed in 0.18.2, outside the parents' current range. |

### 1.5 API contract versioning — P1

| Finding | Class | Change |
| --- | --- | --- |
| With the `api-breaking-change` label, `oasdiff breaking` ran without `--fail-on`, so any breaking change passed; the documented `API_VERSION` bump was never checked. | Verified defect | `scripts/api-contract-policy.mjs` evaluates oasdiff's JSON findings: breaking (level ERR) ⇒ label **and** `API_VERSION` above the base branch; `API_VERSION` never decreases; non-breaking ⇒ nothing required. The job uses the base branch's copy of the policy, re-runs on `labeled`/`unlabeled`, and fails if oasdiff cannot load a document. |
| Generation ran `cargo test` without `--locked`; no determinism check; no line-ending policy. | Hardening | `--locked`, a second-regeneration no-diff check in CI, `.gitattributes` (LF for generated contracts). |

This PR's own contract change is additive (new `/remote-access` operations, an optional
`AppSettings.remote_allowed_origins`, `x-symphony-remote` extensions): `oasdiff breaking` against the base
reports no findings, so `API_VERSION` stays 2.

Tests: `scripts/api-contract-policy.test.mjs` (12 cases: every invalid breaking scenario rejected, valid
combinations accepted, decrease rejected, CLI exit codes, malformed input), run by `pnpm test` and the
contract workflow.

### 1.6 Windows 11 native validation — P1

| Finding | Class | Change |
| --- | --- | --- |
| `configure_windows_autostart` shelled out to `reg.exe`, logged a failed `add` but returned `Ok`, and ignored `delete` failures; the UI reported success. | Verified defect | Registry API (`RegCreateKeyExW`/`RegSetValueExW`/`RegDeleteValueW`) with read-back verification; every failure is an error. The settings save reverts the persisted flag when Windows rejects the change, so the switch never shows an unregistered entry. |
| Native CI only proved compilation (`tauri build --no-bundle` + "exe exists"). | Verified gap | `scripts/windows-native-smoke.ps1` runs the release binaries in CI (see §2); Windows-only Rust tests run under `cargo test` on `windows-latest`; an explicitly **UNSIGNED** NSIS installer is built and uploaded for verification (no signing secrets). |
| The desktop host ignored `SYMPHONY_DATA_DIR` (CLI honoured it). | Hardening | Honoured by both, enabling isolated native tests. |
| `default_app_data_dir()` returned `SymphonyDesktop` while both binaries used `symphony-desktop`. | Verified defect (latent; the function was unused) | Fixed and now used by both. |

### 1.7 Development discovery route — PR #4 thread

The Vite dev-only `/__symphony/runtime` route accepted requests with neither `Origin` nor
`Sec-Fetch-Site`. A cross-origin page could never read the response (no CORS), but a navigation could
display it. **Hardening**: the route now also requires `X-Symphony-Discovery: 1`, which navigations cannot
send and cross-origin scripts cannot send without a preflight.

---

## 2. Verification evidence

### Local (Linux container, this branch)

| Command | Result |
| --- | --- |
| `pnpm install --frozen-lockfile` | pass |
| `cargo fmt --all -- --check` | pass |
| `cargo check --workspace --locked` | pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | pass |
| `cargo clippy … --all-targets --locked --target x86_64-pc-windows-gnu -- -D warnings` (platform, server, cli, desktop) | pass — the Windows code paths (DPAPI, DACLs, registry, `MessageBoxW`) compile and lint clean |
| `cargo test --workspace --locked` | 73 tests pass on Linux (the Windows-only `windows_native.rs` tests are compiled out there and run on `windows-latest`) |
| `cargo test -p symphony-platform --target x86_64-pc-windows-gnu` under **Wine 9** | 13 unit + 6 `windows_native` tests pass (DPAPI, DACL creation/verification, autostart registry). Wine does not persist the DACL *protected* flag, so that single assertion is relaxed only when `SYMPHONY_TEST_UNDER_WINE` is set; real Windows enforces it. Wine is a sanity check, not evidence of Windows behaviour. |
| `pnpm generate:api` (twice) | no diff |
| `pnpm typecheck` / `pnpm test` / `pnpm build` | pass — api-client 18 unit + 20 integration (real `symphony-cli`, incl. 6 remote-access), ui 7, desktop 74, mobile 14, policy scripts 12 |
| `pnpm --filter @symphony/desktop e2e` | 30 Playwright tests pass (generated screenshots/perf files were not committed) |
| `oasdiff breaking` (base `2dc6c719` → head) | no findings |

### CI (GitHub Actions on this PR)

See the PR description for run links and results of: *Windows 11 (Rust + Desktop + Mobile Web)* (includes
the Windows-only Rust tests), *Tauri 2 build (windows-latest)* (release build, native smoke test, unsigned
installer), *Generated contracts are up to date*, *API versioning policy*, *Playwright against real
runtime*, *macOS (iOS Companion Validation)*.

### Not verified automatically

Tray menu, folder picker, notifications, window-state restore, autostart at an actual sign-in and the
installer UI need an interactive session: [`WINDOWS_NATIVE_SMOKE_TEST.md`](./WINDOWS_NATIVE_SMOKE_TEST.md)
lists the manual steps. These were **not** executed for this report.

---

## 3. PR #4 review threads

| Thread | Status at baseline | Outcome |
| --- | --- | --- |
| Devin: dev discovery accepts requests without origin metadata (`apps/desktop/vite.config.ts`) | Partly fixed in PR #4 (Sec-Fetch-Site/Origin checks); a header-less request still passed | Hardening added (§1.7). Low risk: dev-server only, never in builds, unreadable cross-origin. |
| Devin: workflow responses expose embedded credentials | Accepted design for local clients | Kept locally (§1.3, last row); remote access to it is now impossible. Users should keep using `$VAR` references. |
| entelligence-ai: restrict the discovery file ACL on Windows (2 threads, same finding) | Genuine defect, documented as a known limitation | Fixed (§1.2) with automated Windows tests. |

---

## 4. Remaining risks and limitations

- **Non-Windows vault** protection is a user-only key file, not an OS keychain: it protects against other
  accounts, not against code running as the same user. macOS Keychain / Secret Service integration would
  be stronger but needs a desktop session (headless CI and servers often lack one).
- **DPAPI** binds the vault to the Windows account: an administrator password reset or a roaming-profile
  problem can make it unreadable. Symphony then stops with an explicit error and offers quarantine;
  credentials must be re-entered.
- **Legacy migration** only works for the same user and computer name, since the old key derived from them.
- **Remote transport**: the runtime has no built-in TLS. Security depends on a correctly configured TLS
  reverse proxy or encrypted tunnel; proxies that do not send `X-Forwarded-Proto` cannot be distinguished
  from direct loopback clients.
- **Remote companion scope** includes scheduler pause/resume, task cancel/retry and agent approval decisions;
  a leaked remote token allows those actions (but no credential, configuration or merge access) until it is
  rotated or revoked.
- **Native UI behaviour** (tray, dialogs, notifications, window state, sign-in autostart) remains manual.
- **Unsigned installer**: verification only; it triggers SmartScreen and must not be distributed.
- **Transitive npm advisories** listed in §1.4 remain.
- **iOS**: the companion is validated as a web build; no native iOS packaging was attempted.
