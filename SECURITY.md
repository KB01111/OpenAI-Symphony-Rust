# Security Policy & Architecture (`SECURITY.md`)

Symphony Desktop orchestrates autonomous AI coding agents that execute commands and modify files. Security boundaries are enforced at the workspace, process, credential, and network layers.

---

## 1. Workspace Isolation & Path Traversal Prevention

Implemented in `crates/symphony-workspaces/src/lib.rs`:

1. **Identifier Sanitization & Collision Resistance**:
   - Every task/issue identifier is sanitized via `workspace_key_for_identifier`: characters outside `[A-Za-z0-9._-]` are replaced with `_`.
   - If sanitization modifies the input string, a 64-bit (16 hex char) SHA-256 digest suffix (`--<sha256_prefix>`) of the raw identifier is appended.
   - Path traversal tokens such as `..` or `/` are neutralized before path construction.
2. **Canonical Root Containment**:
   - `validate_workspace_path` canonicalizes both `workspace_root` and the target `workspace_path`.
   - Execution is rejected with `SymphonyError::WorkspaceOutsideRoot`, `SymphonyError::WorkspaceEqualsRoot`, or `SymphonyError::SymlinkEscape` if the target path equals the root directory, resolves outside the root directory, or traverses a symlink pointing outside `workspace_root`.
3. **Dirty Worktree Protection**:
   - Workspace cleanup refuses to delete a Git worktree containing uncommitted changes unless `force` is explicitly authorized (such as terminal issue cleanup configured by operator policy).
4. **Operating-System Boundary Disclaimer**:
   - Git worktrees provide filesystem directory separation and branch isolation, **not** kernel-level OS sandboxing. OS-level containment relies on provider sandboxes (`workspace-write` / `read-only` in Codex App Server) and least-privilege user execution.

---

## 2. Credential Isolation & Secret Redaction

1. **Subprocess Environment Scrubbing**:
   - When spawning coding agent subprocesses (`codex app-server`, `claude`, `opencode`) or `WORKFLOW.md` lifecycle hooks, Symphony explicitly strips host tracker credentials from the child process environment (`SECRET_ENV_VARS_TO_STRIP`):
     - `GITHUB_TOKEN`, `GH_TOKEN`, `GITHUB_ENTERPRISE_TOKEN`, `GH_ENTERPRISE_TOKEN`
     - `LINEAR_API_KEY`
     - `SYMPHONY_REMOTE_TOKEN`
   - Coding agents cannot read raw tracker tokens from their environment. Tracker mutations happen exclusively through host-side, permission-controlled dynamic tools (`github_api`, `linear_graphql`).
2. **Dedicated Secret Vault** (`crates/symphony-platform/src/vault.rs`):
   - Credentials live in `<data_dir>/credentials.vault`, never in SQLite. The file is a versioned JSON
     envelope (format 2) around one encrypted, authenticated blob (key names are encrypted too).
   - **Windows**: DPAPI (`CryptProtectData`, current-user scope, no UI) with the envelope header as
     additional entropy. Only the same Windows account can decrypt it; tampering fails authentication.
   - **macOS / Linux**: ChaCha20-Poly1305 (`ring`) with a random 256-bit key in `<data_dir>/credentials.key`
     (mode `0600`, verified on every load: a group/world-readable key is refused). This protects against
     other local accounts and accidental disclosure of the vault file, not against code running as the
     same user. There is no XOR/obfuscation fallback.
   - Writes are atomic (owner-only temp file, `fsync`, rename) and the in-memory cache changes only after
     the write succeeded, so a failed write never leaves memory and disk disagreeing. Removed values and
     plaintext buffers are zeroized.
   - A vault that cannot be read, decrypted or authenticated is **never replaced**: the runtime stops with
     `credential_vault_error`. Recovery is explicit — the desktop host asks before moving it aside, and
     `symphony-cli --quarantine-credential-vault` does the same; the old file is kept as
     `credentials.vault.corrupt-<timestamp>`. A missing key file next to an existing vault is an error, never
     a reason to generate a new key.
   - Format-1 vaults (XOR + base64, Symphony Desktop ≤ 2.0.0) are migrated on first load and read back for
     verification before use; a legacy file that cannot be decoded is reported, not discarded.
3. **Automatic Log & Diagnostic Redaction (`crates/symphony-core/src/redaction.rs`)**:
   - All agent events, terminal outputs, error messages, and exported diagnostic bundles pass through `redact_secrets`, which masks:
     - GitHub PATs (`ghp_...`, `github_pat_...`, `gho_...`, `ghs_...`)
     - OpenAI API keys (`sk-...`, `sk-proj-...`)
     - Anthropic keys (`sk-ant-...`)
     - Linear API keys (`lin_api_...`)
     - HTTP `Bearer <token>` headers and any custom registered secrets.

---

## 3. Agent Execution & Approval Policies

- **Default Approval Policy**: `on-request`. High-impact command executions (`item/commandExecution/requestApproval`) and file patches (`item/fileChange/requestApproval`) pause the turn and surface an interactive `AiApprovalBanner` in the Desktop and Mobile UIs.
- **Timeout & Stall Protection**:
  - Every turn is bounded by `codex.turn_timeout_ms` (default 1 hour) and `codex.stall_timeout_ms` (default 5 minutes).
  - Lifecycle hooks are bounded by `hooks.timeout_ms` (default 60 seconds) and use structured process arguments rather than concatenating untrusted inputs into shells.
- **Human-in-the-Loop Pull Request Merging**:
  - Symphony Desktop **never** merges pull requests automatically by default (`require_human_approval_for_merge: true`).
  - Even when a pull request achieves a `5/5` Merge Readiness Score, a human operator must explicitly click **Approve & Enable Merge**.

---

## 4. Network & Remote API Security

- **Local-only default**: the embedded runtime binds `127.0.0.1` (port 4780, or an ephemeral fallback).
- **Authentication on every route**: all `/api/v1/*` routes except `health` and `runtime` require
  `Authorization: Bearer <token>`. The desktop host and `symphony-cli` generate a random 256-bit token per
  launch (`symphony-cli --token` / `SYMPHONY_API_TOKEN` may supply one instead, but it is rejected if it is
  shorter than 16 characters); it is handed to the webview over Tauri IPC (or published in the owner-only
  `<data_dir>/runtime.json` for local tools) and is never logged or printed. Tokens are compared in
  constant time (`subtle`).
- **No tokens in URLs**: the event stream uses `fetch` streaming with the `Authorization` header instead of
  `EventSource` query parameters.
- **CORS allowlist**: only the Tauri webview origins and the local dev servers are allowed (no `*`).
- **Remote access** (mobile companion / Lumen) is disabled by default (`crates/symphony-server/src/remote.rs`):
  - **Own credential**: a 256-bit random token generated by the runtime (`POST /remote-access/token`),
    stored in the vault as `SYMPHONY_REMOTE_TOKEN`, returned exactly once (`Cache-Control: no-store`) and
    never listed. Rotation (`POST`) and revocation (`DELETE /remote-access/token`) take effect immediately,
    including for event streams that are already open. The remote token is not accepted by the local API and
    the local per-launch token is not accepted by the remote listener. The generic credential API cannot
    read or write it.
  - **Restricted scope**: the remote listener routes only the companion operations in
    `REMOTE_OPERATIONS` (state, events, projects/tasks read, task create/dispatch/cancel/retry, agent
    approval decisions, PR review read, scheduler pause/resume). Credentials, settings, `WORKFLOW.md`,
    diagnostics, providers, worktrees, task diffs, project administration, merge approval and the
    remote-access endpoints answer `403 forbidden_scope`. Each operation in the OpenAPI document carries
    `x-symphony-remote: true|false`, and a test fails if an endpoint is not classified.
  - **No cleartext**: the listener binds only loopback addresses (`127.0.0.1`, `::1`, `localhost`; other
    addresses are rejected by `PUT /settings` and reported as `failed`). Reach it through a TLS reverse proxy
    or an encrypted tunnel; a request that a proxy marks `X-Forwarded-Proto: http` is refused with
    `insecure_transport`. The companion refuses `http://` URLs except for loopback hosts.
  - **Explicit CORS**: the companion app origin (`tauri://localhost`) plus `remote_allowed_origins`
    (bare `https://` origins; `http://` only for loopback; `*` rejected). The local listener's origins are
    not reused.
  - **Live lifecycle**: saving the settings reconfigures the running listener (start, stop, rebind); the
    effective state, including bind failures such as a port in use, is reported by `GET /remote-access` and
    shown in Settings → Remote access. It never shares the local API's port.
  - The companion keeps its token in `sessionStorage` unless the user explicitly opts in to remembering it
    on the device.
- **Development discovery** (`/__symphony/runtime`, Vite dev server only, never in builds) answers only
  requests with a loopback `Host`, the `X-Symphony-Discovery: 1` header (which navigations cannot send and
  cross-origin scripts cannot send without a preflight), `Sec-Fetch-Site: same-origin` when present and a
  matching `Origin` when present, so other web pages cannot read the token.
- **Credential API allowlist**: only `GITHUB_TOKEN`, `GH_TOKEN`, `GITHUB_ENTERPRISE_TOKEN`, `LINEAR_API_KEY`,
  `OPENAI_API_KEY` and `ANTHROPIC_API_KEY` can be stored; values are never returned (`GET /credentials`
  lists key names only). Provider API keys are passed to agent subprocesses explicitly from the vault
  (`AgentLaunchConfig.extra_env`); tracker tokens never reach agents and are used only by host-side tools.
  The host-side GitHub tools read `GITHUB_TOKEN`/`GH_TOKEN` from the vault first and then the environment;
  GitHub Enterprise tokens are never sent to the public `api.github.com` endpoint.
- **Workflow writes** go only to the active `WORKFLOW.md` (or the runtime's data directory); clients cannot
  choose a path. Invalid documents are rejected before writing.
- **Redaction**: workflow configuration returned by the API masks credential-like tracker values
  (`token`, `api_key`, `secret`, `password`, `credential`, `authorization`) at any nesting depth; task diffs
  and diagnostics are passed through `redact_secrets` with the vault's values. The raw `WORKFLOW.md` source
  is returned unmodified to authenticated clients so it can be edited; keep credentials out of it and use
  `$VAR` references (resolved only inside the runtime). The raw source is a local-administration endpoint:
  remote companion clients cannot read it (`403 forbidden_scope`).
- **Bounded diff reads**: worktree diffs are read with a byte budget per `git` invocation (the child is
  stopped when the budget is used), so a large untracked file cannot exhaust runtime memory.

## 5. Desktop Host (Tauri 2)

- **Production CSP**: `default-src 'self'`, `script-src 'self'`, `connect-src` limited to IPC and loopback,
  `object-src 'none'`, `frame-ancestors 'none'`, `form-action 'none'`; prototype freezing enabled.
- **Least-privilege IPC**: app commands are declared in `build.rs` (`AppManifest::commands`), so each needs an
  explicit `allow-*` permission in `capabilities/default.json`. The exposed commands are
  `get_local_api_config`, `open_task_workspace` (opens only a task worktree that validates inside its
  project's workspace root), `pick_repository_folder`, `set_autostart` and `exit_application`. There is no
  shell, filesystem or arbitrary command bridge.
- **Single authoritative scheduler**: desktop and CLI share `symphony-runtime.lock` in the data directory.
  Ownership is an exclusive OS file lock held for the process lifetime (atomic across processes, released
  by the OS on crash); a second process attaches to the running runtime instead of starting a scheduler.
- **Discovery file**: `<data_dir>/runtime.json` (endpoint + token) is published atomically from a fresh
  temp file that is created owner-only *before* any data is written: mode `0600` on Unix, and on Windows an
  explicit **protected DACL** granting only the current user and `SYSTEM` (inheritance from the directory is
  blocked, so a world-writable custom `--data-dir` cannot expose the token). The permissions are verified
  before the rename; if they cannot be enforced (e.g. FAT volumes without ACLs) nothing is published and the
  error is reported (desktop: log, notification and Settings → Application; CLI: startup error). Readers
  (the desktop host when attaching, on every platform; the Node tooling on Unix) ignore a discovery file that is not private to the current user, so a file
  planted by another account cannot redirect clients. The credential vault uses the same primitive
  (`crates/symphony-platform/src/secure_fs.rs`).
- **Autostart**: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` is written through the registry API
  and read back; a rejected or unpersisted change is an error, and the settings revert to the actual state.
