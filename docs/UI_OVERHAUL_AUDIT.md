# Symphony Desktop — UI Overhaul Audit (baseline `main` @ `e63d920`)

This audit was performed before the 2.0 overhaul by reading the implementation (not only the README)
and running the verification suite. Every finding below was verified against code; where a behaviour
was confirmed by executing it, that is stated.

## 1. Baseline verification (before any change)

| Command | Result | Notes |
| --- | --- | --- |
| `pnpm install --frozen-lockfile` | ✅ | pnpm 11.15.1 (from `packageManager`) |
| `pnpm typecheck` | ✅ | TS types were self-consistent but did not match Rust (see §3) |
| `pnpm test` | ✅ | 7 desktop render tests with mocked `fetch`; `shared-types`, `api-client`, `ui`, `mobile` "tests" were `node --eval console.log(...)` |
| `pnpm build` | ✅ | desktop main chunk > 500 kB warning |
| `cargo fmt --all -- --check` | ✅ | |
| `cargo clippy --workspace -- -D warnings` | ❌ on Linux | `unused import: warn` in `symphony-platform` (only used under `cfg(windows)`); Windows CI unaffected |
| `cargo test --workspace` | ✅ 19 tests | No test exercised the HTTP router |

Open PRs at audit time: #3 (CI repair: pnpm action/version alignment, `--frozen-lockfile`, `--locked`) and #2
(dependabot). The overhaul does not modify `.github/workflows/ci.yml` to avoid conflicting with #3.

## 2. Severity scale

- **P0** — application cannot work, or a security boundary is broken.
- **P1** — a core workflow is wrong, unsafe or misleading.
- **P2** — architecture, maintainability, performance, accessibility or localization debt.
- **P3** — polish / documentation drift.

## 3. Findings

### P0

| # | Area | Finding | Evidence |
| --- | --- | --- | --- |
| P0-1 | Server | **The HTTP API could never start.** `Cargo.lock` pins axum 0.8.9, which panics when a route uses the pre-0.8 `:param` syntax; every parameterised route (`/projects/:id`, `/tasks/:id/…`, `/api/v1/:issue_identifier`) used it. `build_router` therefore panicked in both the desktop host and `symphony-cli serve`. No test built the router. | `crates/symphony-server/src/lib.rs` (old); axum 0.8 changelog |
| P0-2 | Security | **Unauthenticated local API with `CorsLayer::allow_origin(Any)`.** The desktop host passed `None` as token; any web page open in the user's browser could read state, dispatch agents, approve commands, write `WORKFLOW.md` and store credentials on `127.0.0.1`. | `apps/desktop/src-tauri/src/main.rs`, `build_router` |
| P0-3 | Security | **Arbitrary environment injection.** `POST /api/v1/credentials` stored any key and called `std::env::set_var(key, value)` — e.g. `PATH`, `LD_PRELOAD` — inherited by agent subprocesses. | `set_credential_handler` |
| P0-4 | Security | **Arbitrary file write.** `PUT /api/v1/workflow` accepted a client-supplied `path` and wrote the body there. | `update_workflow_handler` |
| P0-5 | Security | **Secret disclosure.** `ServiceConfig::from_workflow` resolves `$GITHUB_TOKEN`/`$LINEAR_API_KEY` into `tracker_provider`; `GET /workflow` and `POST /workflow/preview` returned that config verbatim. | `workflow.rs`, workflow handlers |
| P0-6 | Contract | **TypeScript types did not match Rust for almost every payload**: `Task` (`assigned_provider`/`current_attempt`/`session_id` vs Rust `provider`/`attempt_count`/`current_session_id`), `AgentEvent`, `ApprovalRequest` (`command`/`file_paths` vs `title`/`description`/`command_or_diff`), `ApprovalDecision` (`approve/deny/abort` vs `accept/accept_for_session/decline/cancel` — every approval click would fail to deserialize), `PullRequestReview`, `MergeReadinessReport` (`ci_passing…` vs `label/evidence/blockers`), `HardwareSnapshot`, `OrchestratorStateSnapshot` (`recent_events` did not exist), `ProviderProfile` vs the `ProviderCapabilities` the server returned. | `packages/shared-types/src/index.ts` vs `crates/symphony-core/src/domain.rs` |
| P0-7 | Contract | **Client routes did not exist on the server**: `GET /tasks/{id}` (server: `/tasks/id/{id}`), `POST /tasks/{id}/status` (server: `PATCH`), `POST /tasks/{id}/start` (server: `/dispatch`), `POST /approvals/{id}/resolve` (server: `/decide`), `POST /prs/{id}/approve` (server: `/approve-merge`), `POST /workflow {raw_content}` (server: `PUT {raw_markdown}`), `POST /scheduler/pause {paused:false}` (always paused; resume route unused), SSE at `/events/stream` (server: `/events`). | `packages/api-client/src/index.ts` |
| P0-8 | Contract | **Settings schema mismatch**: TS `language/close_action/start_on_login/local_api_port/remote_api_bind` vs Rust `locale/minimize_to_tray_on_close/start_background_runtime_on_login/remote_port/remote_bind_address/polling_interval_ms/default_model/extra_env_vars`. Saving settings from the UI could not deserialize. | `AppSettings` in both |
| P0-9 | Contract | **Immediate dispatch silently ignored**: the UI sent `dispatch_immediately`, the server reads `auto_start`. | `CreateTaskRequest` |
| P0-10 | Tauri | **IPC/endpoint discovery broken**: the frontend invoked `get_local_api_config`, `set_close_behavior`, `open_workspace_in_explorer`, `stop_background_orchestrator`; the host registered `get_api_base_url`, `get_orchestrator_state`, `list_tasks_ipc`, `set_autostart_ipc`, `exit_application_completely`. Discovery always fell back to `127.0.0.1:47820` while the host bound `4780` (or an ephemeral port). The UI could not connect. | `main.rs`, `api-client` |
| P0-11 | Streaming | The server emits **named** SSE events (`event: task/created`), but the client only set `EventSource.onmessage` (which receives unnamed events only), so no live event ever reached the UI even if the URL had been right; `EventSource` also cannot send `Authorization`. | `subscribeEvents` |

### P1

| # | Area | Finding |
| --- | --- | --- |
| P1-1 | Contract | `ProviderKind::OpenCode` serialized as `"open_code"` over JSON (serde `snake_case`) but `"opencode"` in SQLite and TS. |
| P1-2 | Providers | Provider "detection" returned hard-coded defaults: all real providers `available: false`, no version, no auth state. |
| P1-3 | State machine | `dispatch_task_now` ignored task status (a completed/cancelled task could be dispatched); `retry_task` and `cancel_task` set statuses directly without `validate_transition`. |
| P1-4 | Approvals | Approvals could be resolved repeatedly; approvals belonging to cancelled/failed runs stayed `pending` forever. |
| P1-5 | Scheduler | Desktop and CLI used **different** lock files (and passed file paths where a directory was expected), so two authoritative schedulers could run on one database. |
| P1-6 | CLI | `run-task` waited for `task/completed`/`task/human_review_ready`/`task/failed`, which are never emitted → hang. SIGTERM did not release the runtime lock. |
| P1-7 | Workspaces | `inspect_changed_files` reported `additions: 1, deletions: 0` for every file; `git diff HEAD` excluded untracked files, so new files (e.g. the simulated agent's change) were invisible in diffs. |
| P1-8 | Tauri | `"csp": null`; IPC commands were not permission-scoped; window close behaviour read settings with blocking calls but no IPC existed to update autostart; `--background` autostart flag ignored; remote access setting was never wired to a listener. |
| P1-9 | Persistence | `AppSettings` lacked `#[serde(default)]`; adding a field would make previously stored settings unreadable. |
| P1-10 | Workflow | `GET /workflow` returned only the prompt body when no file existed; saving that in an editor would silently drop the YAML front matter. |
| P1-11 | UI truthfulness | The dashboard showed "Orchestrator Active" from a local default; `GET /` served live counts without auth; merge-readiness UI rendered fields that did not exist. |

### P2

| # | Area | Finding |
| --- | --- | --- |
| P2-1 | Architecture | `apps/desktop/src/App.tsx` was a 2,474-line component holding all views, forms and mutations; no routing (section switch in zustand), no error boundaries. |
| P2-2 | State | UI store not persisted (sidebar, view, theme reset on every launch); task records duplicated between local state and query cache. |
| P2-3 | Performance | Six queries polled every 2–4 s regardless of streaming; event lists re-rendered Markdown for every event on every update; no virtualization; single >500 kB JS chunk. |
| P2-4 | Design system | Hard-coded hex/Tailwind palette colours (`#0d1117`, `slate-*`) in feature code; Fluent theme and Apps SDK theme only partially synchronised; no density support; competing tooltip/popover implementations. |
| P2-5 | i18n | Many hard-coded English strings in the desktop UI; Swedish coverage incomplete. |
| P2-6 | Accessibility | Icon-only buttons without labels; drag-and-drop without keyboard alternative; no live-region announcements. |
| P2-7 | Tests | No contract or integration tests; frontend tests mocked `fetch` with shapes that did not match the server. |
| P2-8 | Mobile | Mobile companion depended on the broken client and stored the bearer token in `localStorage` unconditionally. |

### P3

| # | Finding |
| --- | --- |
| P3-1 | README/ARCHITECTURE ports disagree (47800, 4780, 47820) and describe features (constant-time token compare, remote binding) that were not implemented. |
| P3-2 | Branding/versioning still "1.0" across manifests. |
| P3-3 | Clippy fails on non-Windows hosts (unused import). |

## 4. Resolution map

All P0 and P1 items are addressed in the 2.0 overhaul; see `docs/UI_IMPLEMENTATION_REPORT.md` for the
file-level changes and the tests that cover each item.
