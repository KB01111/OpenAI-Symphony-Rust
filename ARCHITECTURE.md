# Symphony Desktop 2.0 — Architecture

## 1. Overview & Alignment with OpenAI Symphony `SPEC.md`

Symphony Desktop 2.0 implements the language-independent [OpenAI Symphony Specification](https://github.com/openai/symphony/blob/main/SPEC.md) natively in Rust while extending it into a full-featured Windows 11 desktop and remote mobile orchestration platform.

### Core Symphony Invariants Preserved

1. **Single Authoritative Scheduler (`symphony-orchestrator`)**:
   - All task claiming, concurrency enforcement, retry timers, and active session reconciliation execute inside a single authoritative Rust orchestrator loop.
   - The React UI (`apps/desktop` and `apps/mobile`) is strictly a projection and control client over Tauri IPC and the `/api/v1/*` HTTP/SSE API; UI state is never trusted as authoritative execution state.
2. **Workspace Safety Invariants (`symphony-workspaces`)**:
   - **Invariant 1 (`cwd == workspace_path`)**: Every coding agent subprocess is spawned with its working directory explicitly set to the validated task workspace path.
   - **Invariant 2 (Path Containment)**: Canonicalized `workspace_path` must reside strictly inside canonicalized `workspace_root` and must never equal `workspace_root` or escape via symlinks.
   - **Invariant 3 (Collision-Resistant Workspace Keys)**: `workspace_key_for_identifier` replaces any character outside `[A-Za-z0-9._-]` with `_`. Whenever sanitization alters the original identifier, a 16-character (64-bit) lowercase hex SHA-256 suffix (`--<hash16>`) is appended so distinct identifiers never collide on disk.
3. **Deterministic Candidate Sorting**:
   - Eligible tasks are sorted by:
     1. Priority `1..=4` ascending (`null` or out-of-range priorities sort after `1..=4`).
     2. `created_at` timestamp oldest-first (`null` sorts last).
     3. `identifier` lexicographical tie-breaker.
4. **Two-Part Active Run Reconciliation**:
   - **Part A (Stall Detection)**: Checks `elapsed_ms > codex.stall_timeout_ms` since the last agent protocol event (`last_codex_timestamp`) or attempt start, interrupting stalled sessions and scheduling exponential backoff retries.
   - **Part B (Tracker State Reconciliation)**: Queries `fetch_issues_by_ids` for running tasks. Tasks in `terminal_states` stop immediately and clean up their workspace; tasks still in `active_states` update their in-memory snapshot; tasks moved to non-active states stop without destroying worktrees.
5. **Retry Backoff & Continuation**:
   - Continuation happens inside an attempt: while the agent ends a turn with `[CONTINUE_TURN]`, the next turn runs on the same thread (up to `agent.max_turns`). A successful attempt then runs hooks and validations and stops in `HumanReview` with its merge-readiness report.
   - A stalled session (Part A) is retried with exponential backoff: `min(10000 * 2^(attempt - 1), agent.max_retry_backoff_ms)`. A task waiting out its backoff is never picked up as a regular candidate, and a due retry is dispatched only while the scheduler runs and a slot is free (otherwise it stays due for the next tick).
   - Any other failed attempt ends in `Failed` and waits for an explicit retry (UI, API or `POST /tasks/{id}/retry`), so a broken setup cannot burn tokens in an endless retry loop.

---

## 2. Rust Workspace Architecture

| Crate | Responsibility |
| --- | --- |
| `symphony-core` | Domain model (`Issue`, `Task`, `TaskStatus`, `RunAttempt`, `ProviderKind`, `AgentEvent`, `ApprovalRequest`, `PullRequestReview`), `WORKFLOW.md` YAML + Liquid parser/renderer, secret redaction (`redact_secrets`), and error taxonomy (`SymphonyError`). |
| `symphony-storage` | SQLite WAL persistence (`sqlx`), versioned transactional schema migrations (`schema_migrations`, pre-migration snapshot, newer-schema refusal), consistent `VACUUM INTO` backups with checksum manifests, automatic scheduled backups with retention and offline restore (`backup.rs`), the tool-call history (`tool_calls`), crash recovery (`recover_stale_running_tasks`), event ring buffer persistence, and redacted diagnostic bundle export. |
| `symphony-workspaces` | Workspace key generation, canonical path validation, Git worktree lifecycle (`git worktree add -B`, `git worktree remove`), dirty-tree inspection, simulated-merge conflict detection (`git merge-tree`), hook-free commit and non-forced push of a task branch, unified diff generation, lifecycle hooks, and validation command runner. |
| `symphony-platform` | Windows 11 `GetSystemPowerStatus` power telemetry, `sysinfo` CPU/RAM pressure monitoring, hardware-aware concurrency calculation, single-instance lockfile guard, owner-only secret files (`secure_fs`: protected DACL on Windows, `0600` on Unix), the versioned credential vault (`credentials.vault`, DPAPI on Windows / ChaCha20-Poly1305 key file elsewhere), runtime discovery (`runtime.json`) and HKCU autostart via the registry API. |
| `symphony-trackers` | `GitHubTracker` (REST v3 issues, label opt-in filter, PR diffs, CI check runs, review comment evaluation, `github_api` dynamic tool), `LinearTracker` (GraphQL polling, `inverseRelations` blocker resolution, `linear_graphql` dynamic tool) and `tool_policy` (the allowlist both agent tools obey; no merge, no deletion, project repository only). |
| `symphony-agents` | Multi-provider coding agent runtime: `CodexAppServerDriver` (JSON-RPC 2.0 over stdio), `ClaudeCodeDriver` (`claude --output-format stream-json`), `OpenCodeDriver`, and deterministic `SimulatedAgentDriver`. |
| `symphony-orchestrator` | Central scheduler, `WORKFLOW.md` hot-reload watcher, approval gate (`OrchestratorApprovalGate`), dynamic tool dispatcher (`HostDynamicToolExecutor`), and evidence-based `MergeReadinessReport` (1–5) calculator (merge conflicts simulated, CI `none` for local reviews — nothing is assumed), explicit-approval pull request publishing (`create_pull_request`: commit, push, open; never merge) and token accounting that separates thread history from session usage (`ThreadUsageLedger` in `symphony-core`). |
| `symphony-server` | Axum HTTP & SSE server implementing SPEC endpoints (`/api/v1/state`, `/api/v1/{issue_identifier}`, `/api/v1/refresh`) and desktop/mobile management endpoints with Bearer token auth, plus the scoped remote companion listener (`RemoteAccessManager`, `REMOTE_OPERATIONS`). |
| `symphony-cli` | Standalone runtime and CLI: serving the API is the default command; `validate` checks a `WORKFLOW.md`; `run-task` runs one task headlessly; `verify-backup` / `restore-backup` check and restore a backup offline. Shares the data directory, lock, vault and remote listener lifecycle with the desktop host. |

---

## 3. Desktop UI & Design System

The 2.0 desktop UI and design system are documented in detail in
[`docs/UI_ARCHITECTURE.md`](./docs/UI_ARCHITECTURE.md) and [`docs/DESIGN_SYSTEM.md`](./docs/DESIGN_SYSTEM.md).
In short:

- **Contracts**: the Axum handlers are annotated with `utoipa`, so the OpenAPI document
  (`openapi/symphony-api.json`) is generated from the served routes. Hey API generates the TypeScript
  types, SDK and fetch client from it; `SymphonyClient` wraps that SDK and is integration-tested against
  the real Axum router. CI checks that the contract is regenerated and has no breaking changes.
- **Transport**: the webview discovers the runtime over Tauri IPC (`get_local_api_config`), calls the
  authenticated HTTP API, and receives events over the in-process `symphony://event` channel (embedded
  runtime) or authenticated SSE with `Last-Event-ID` resume (attached/remote).
- **State**: durable data in TanStack Query (invalidated by events, fallback polling when the stream is
  down); presentation preferences in a persisted zustand store.
- **Design system**: one token source (`packages/ui/src/tokens/tokens.css`) feeding Tailwind v4 and the shadcn/ui (Radix)
  components, coordinated by `SymphonyDesignProvider`.

## 4. Hardware-Aware Scheduling (Windows 11 Hybrid Laptops)

Designed for modern hybrid laptops such as the **ASUS ROG Zephyrus G16** (AMD Ryzen AI 9 HX 370, Radeon 890M iGPU, AMD XDNA 2 NPU, NVIDIA RTX 4070 Laptop GPU, 32 GB RAM):

- **Zero GPU Wake Overhead**: `symphony-platform` uses Win32 `GetSystemPowerStatus` and lightweight CPU/RAM counters without probing DXGI/NVML, ensuring the RTX 4070 remains in D3cold sleep while on battery.
- **Adaptive Concurrency**:
  - **AC Power**: Default up to `2` concurrent coding agents (configurable up to workflow limit).
  - **Battery Power**: Default capped at `1` concurrent coding agent.
  - **High Memory Pressure (`>=88%` RAM used by default, `resource_policy.high_memory_threshold_percent`)**: New task dispatches, retries included, pause automatically until memory pressure subsides.
  - **Thermal / High CPU Load (`>=92%` by default, `resource_policy.high_cpu_threshold_percent`)**: Concurrency is halved (minimum `1`) while `reduce_on_thermal_throttle` is set.

---

## 5. Remote Control, Lumen Integration, and iOS Companion

- **Loopback vs. Remote Binding**: The local API binds strictly to `127.0.0.1` (4780, or an ephemeral fallback published via `runtime.json`/IPC) and always requires the per-launch Bearer token. The remote companion listener is managed by `RemoteAccessManager` (desktop host and `symphony-cli` alike): it runs only when `remote_api_enabled` is set and a runtime-generated remote token exists, binds a loopback `remote_bind_address:remote_port` (default `127.0.0.1:47821`, never the local port), serves only the companion operations, and is reconfigured live when the settings or the token change. Expose it beyond the machine through a TLS reverse proxy or an encrypted tunnel; see `SECURITY.md` §4.
- **iOS Companion (`apps/mobile`)**: Built with React 19, `@symphony/shared-types`, `@symphony/api-client`, and `@symphony/ui`, packaged with a Tauri 2 iOS configuration (`apps/mobile/tauri.ios.conf.json`). Connects remotely to the Windows orchestration daemon to monitor active agents, inspect streaming logs, approve tool calls, create tasks, and cancel runs with confirmation dialogs.
