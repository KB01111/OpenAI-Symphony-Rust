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
2. **Dedicated Secret Vault**:
   - Sensitive tokens are stored in `SecretStore` (`credentials.vault` outside SQLite) rather than plaintext SQLite columns.
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
  launch; it is handed to the webview over Tauri IPC (or published in the owner-only
  `<data_dir>/runtime.json` for local tools) and is never logged or printed. Tokens are compared in
  constant time (`subtle`).
- **No tokens in URLs**: the event stream uses `fetch` streaming with the `Authorization` header instead of
  `EventSource` query parameters.
- **CORS allowlist**: only the Tauri webview origins and the local dev servers are allowed (no `*`).
- **Remote access** (mobile companion / Lumen) is disabled by default. When `remote_api_enabled` is set, the
  desktop host starts a second listener on `remote_bind_address:remote_port` only if `SYMPHONY_REMOTE_TOKEN`
  (≥ 16 characters) exists in the credential vault. Terminate TLS in front of it.
- **Credential API allowlist**: only `GITHUB_TOKEN`, `GH_TOKEN`, `GITHUB_ENTERPRISE_TOKEN`, `LINEAR_API_KEY`,
  `OPENAI_API_KEY` and `ANTHROPIC_API_KEY` can be stored; values are never returned (`GET /credentials`
  lists key names only). Only provider API keys are exported to the runtime environment.
- **Workflow writes** go only to the active `WORKFLOW.md` (or the runtime's data directory); clients cannot
  choose a path. Invalid documents are rejected before writing.
- **Redaction**: workflow configuration returned by the API masks tracker `token`/`api_key`/`secret` values;
  task diffs and diagnostics are passed through `redact_secrets` with the vault's values.

## 5. Desktop Host (Tauri 2)

- **Production CSP**: `default-src 'self'`, `script-src 'self'`, `connect-src` limited to IPC and loopback,
  `object-src 'none'`, `frame-ancestors 'none'`, `form-action 'none'`; prototype freezing enabled.
- **Least-privilege IPC**: app commands are declared in `build.rs` (`AppManifest::commands`), so each needs an
  explicit `allow-*` permission in `capabilities/default.json`. The exposed commands are
  `get_local_api_config`, `open_task_workspace` (opens only a task worktree that validates inside its
  project's workspace root), `pick_repository_folder`, `set_autostart` and `exit_application`. There is no
  shell, filesystem or arbitrary command bridge.
- **Single authoritative scheduler**: desktop and CLI share the `symphony-runtime.lock` in the data
  directory; a second process attaches to the running runtime instead of starting another scheduler.
