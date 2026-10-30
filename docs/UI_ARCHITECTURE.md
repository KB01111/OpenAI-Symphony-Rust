# Symphony Desktop 2.0 — UI Architecture

## 1. Layers

```
┌──────────────────────────── Tauri 2 host (apps/desktop/src-tauri) ────────────────────────────┐
│ window · tray · notifications · folder picker · window-state · single instance · autostart    │
│ embedded runtime (SymphonyOrchestrator + Axum on 127.0.0.1, per-launch token)                 │
│   ── or attached mode: another runtime (symphony-cli) owns the data dir; host attaches to it   │
│ IPC (explicit allow-list): get_local_api_config · open_task_workspace · pick_repository_folder │
│                            set_autostart · exit_application                                    │
└───────────────────────────────────────────────────────────────────────────────────────────────┘
          │ IPC (discovery, native actions)                 │ symphony://event (embedded mode)
          ▼                                                  ▼
┌──────────── React 19 webview (apps/desktop/src) ─────────────────────────────────────────────┐
│ RuntimeProvider ── SymphonyClient (@symphony/api-client) ── HTTP /api/v1 + Bearer token        │
│      │                 event stream: Tauri channel (embedded) or authenticated SSE (others)    │
│      ├─ Batcher (100 ms) → event ring buffer (zustand) + targeted TanStack Query invalidation   │
│      └─ fallback polling while the stream is down; throttled while the window is hidden        │
│ TanStack Query cache = durable server state      zustand uiStore = presentation preferences    │
│ DesktopShell (layouts/) + feature views (features/*) + design system (@symphony/ui, shadcn)     │
└───────────────────────────────────────────────────────────────────────────────────────────────┘
```

Rust remains the only authority for execution state: the UI never changes a task's status locally
except as an optimistic move that is rolled back if the runtime rejects it.

## 2. Contracts

Rust is the single source of truth; nothing about the HTTP API is written by hand in TypeScript.

```text
Axum handlers + #[utoipa::path]  ──utoipa-axum──▶  openapi/symphony-api.json  ──Hey API──▶  @symphony/shared-types
(crates/symphony-server)                           (checked in, OpenAPI 3.1)               types · SDK · fetch client
                                                                                                    │
TaskStatus::allowed_transitions  ──────────────────▶  generated/stateMachine.ts                     ▼
                                                                                  SymphonyClient (@symphony/api-client)
```

- API types in `symphony-core`, `symphony-orchestrator` and `symphony-server` derive `utoipa::ToSchema`;
  query structs derive `IntoParams`. Every handler is registered through `OpenApiRouter::routes(routes!(…))`
  with an explicit `operationId`, so the document cannot list a route the server does not serve.
- `ApiContract` (a `utoipa::Modify`) adds the bearer security scheme to every protected operation, the
  shared `ApiErrorBody` error response and exact `required` lists: response fields are always present
  (`null` when empty), only the request bodies in `OPTIONAL_FIELD_SCHEMAS` have optional fields.
- `crates/symphony-server/tests/openapi_contract.rs` fails when `openapi/symphony-api.json` is stale and
  checks the invariants (unique operation ids, tags, error responses, bearer auth on exactly the
  protected routes, path parameters, required fields). `tests/state_machine_contract.rs` does the same
  for `packages/shared-types/src/generated/stateMachine.ts`.
- `@hey-api/openapi-ts` (pinned to an exact version) generates `packages/shared-types/src/generated/openapi/`
  (`tools/api-generator/openapi-ts.config.ts`). `@symphony/shared-types` re-exports the types;
  `@symphony/shared-types/api` exposes the SDK functions.
- `pnpm generate:api` regenerates all three artifacts on any OS. The `API contract` workflow fails when
  they are stale and runs `oasdiff breaking` (pinned, checksum-verified) against the base branch; an
  intended break needs the `api-breaking-change` label and an `API_VERSION` bump.
- `crates/symphony-server/tests/api_router.rs` tests routing, auth, CORS and error mapping in Rust.
- `SymphonyClient` calls only generated SDK functions through its own Hey API client instance and
  adds the deadline, bearer auth, `SymphonyApiError` mapping and the event stream. The authenticated
  SSE stream (`Last-Event-ID` replay, `resync`) and runtime discovery are unchanged; the SDK's own
  SSE helper is not used.
- One configuration describes every connection (`packages/api-client/src/config/apiConfig.ts`):
  `RuntimeTarget` is `desktop` (Tauri IPC), `local` (explicit URL or dev-server discovery) or
  `remote` (mobile companion), and `resolveApiConfig` turns it into the `ApiConfig` that
  `SymphonyClient.fromConfig` / `SymphonyClient.connect` use.
- `packages/api-client/test/integration/runtime.test.ts` drives the real Axum router through
  `symphony-cli` with the TypeScript client (discovery, state, project registration, task creation,
  immediate dispatch, live SSE events, approvals, cancel/retry, PR review, workflow preview/save,
  settings, credentials).

### Settings compatibility

The Rust field names are persisted in SQLite and are authoritative; the TypeScript type is generated
from them through the OpenAPI document, so no renaming/migration was needed. `AppSettings` and `ResourcePolicy` use
`#[serde(default)]`, so new fields (e.g. `provider_default_models`) read older stored JSON safely.

## 3. Runtime discovery & authentication

| Client | Discovery | Auth |
| --- | --- | --- |
| Desktop (embedded) | `get_local_api_config` IPC returns the bound port (4780 or an ephemeral fallback) and the per-launch token | Bearer header |
| Desktop (attached) | Host reads `<data_dir>/runtime.json` of the owning runtime and returns it via IPC | Bearer header |
| Browser dev / E2E | Vite dev-only route `/__symphony/runtime` reads `runtime.json` (loopback Host, `X-Symphony-Discovery` header, same origin only) | Bearer header |
| Mobile / remote | Configured `https://` URL (plain `http://` only for loopback) + the remote access token issued in Settings → Remote access (kept in `sessionStorage` unless the user opts to remember it) | Bearer header (remote listener, restricted scope) |

`runtime.json` is written with owner-only permissions (Unix `0600`, Windows protected DACL for the user and
`SYSTEM`) by whichever process owns the data directory
(guarded by the shared `symphony-runtime.lock`, which enforces a single scheduler per data directory).

## 4. Streaming

`GET /api/v1/events` is SSE with **named events** (`event:` = `AgentEvent.event_type`) and `id:` = event id.
On connect the server sends `ready`; with `Last-Event-ID` it replays persisted events after that id, or
sends `resync` when it cannot (unknown id, broadcast lag). The client (`AuthenticatedSseStream`):

- uses `fetch` streaming so the token travels in `Authorization` (never in the URL);
- parses the SSE wire format incrementally and dispatches named events;
- dedupes by id (bounded set), keeps emission order, resumes with `Last-Event-ID`;
- reconnects with exponential backoff + jitter, stops on 401, and aborts dead connections with a
  heartbeat watchdog (server keep-alive every 15 s) — this also recovers after sleep/resume;
- `reconnectNow()` is called on `online` and after the window was hidden for > 60 s.

In embedded mode the Tauri host forwards the same events on `symphony://event` (and `symphony://resync`
on broadcast lag), avoiding an HTTP connection.

## 5. Desktop source layout

```
apps/desktop/src/
  app/          App, AppProviders, AppRouter (hash routes, lazy views), RuntimeGate, PreferencesBridge, messages
  layouts/      DesktopShell, Sidebar, ThreadList, TopBar, StatusBar
  runtime/      RuntimeProvider, eventStore (ring buffer), invalidation map
  lib/          queries (TanStack hooks), queryKeys, i18n helpers
  store/        uiStore (persisted presentation state), safeStorage
  hooks/        useGlobalShortcuts, useFocusedTaskId
  components/   ErrorBoundary, DiffViewer
  features/     composer · activity · overview · projects · tasks · agents · execution · worktrees · reviews ·
                workflows · providers · settings · onboarding · command-palette
  test/         renderWithProviders (in-memory runtime), fixtures, setup
```

Routes: `/` (prompt composer), `/overview`, `/projects[/:projectId]`, `/tasks` (Board, Table and Runs views), `/runs/:taskId`
(a task as a thread; the bare `/runs` redirects to the Runs view), `/activity`, `/worktrees`, `/reviews[/:reviewId]`, `/knowledge`, `/workflow`, `/providers`, `/settings`.
The app always opens on the composer.

The app uses a data router (`createHashRouter`; tests use `createMemoryRouter`), which lets a view block
navigation while it holds unsaved edits (`useBlocker` in Workflow Studio). Every feature view is a lazily
loaded chunk; the CodeMirror editor is a separate lazy chunk loaded only by Workflow Studio.

### Feature views

| View | Highlights |
| --- | --- |
| Overview | Metrics from the snapshot/tasks, live sessions, pending approvals (decide inline), persisted + live activity. |
| Projects | Full-width repository table (columns drop by container width) with the selected project's settings in a side sheet, guided Add repository (native folder picker in Tauri, runtime validation, checkout-root resolution), delete with confirmation (409 while running). |
| Tasks | Kanban by status (dnd-kit, keyboard + menu moves validated by the shared state machine before any request; the occasional Backlog, Blocked and Cancelled columns appear only when they hold a task or while a card is dragged, empty columns are slim) and a data table; New task dialog (`auto_start`). A third view, *Runs*, lists one row per task by precedence (live > approval > retrying > queued > recent finished) with status tabs, grouping by status/provider/project and capability-gated actions (it has its own tabs, so the task filters are hidden there). `/runs` and `paths.runs()` open `/tasks?view=runs`. |
| Home / composer | Large prompt box with chips: repository (shows the base branch), provider + model, priority, labels, *Run now* / *Add to queue* (`auto_start`). The first line of the prompt is the title, the whole prompt the description; the identifier is generated. Recent threads below. The same composer opens in a dialog (Ctrl+N) from other views. |
| Thread (`/runs/:taskId`) | Prompt, merged persisted + live timeline (concise/technical, kind filters, tail-following), pinned approvals, inline follow-up box (`POST /tasks/{id}/continue`, only after a run ended, 20 000 character limit) and a run-details panel (Changes, Validation, Tool calls, Usage, Pull request, Context). Wide windows dock the panel, narrow ones open it as a sheet. |
| Activity | Cross-task events, scoped agent/terminal output, validations, approvals and problems (the former bottom dock). |
| Worktrees | Worktree table with dirty state, diff viewer, "Open in Explorer" only in the embedded desktop host; no delete action. |
| Knowledge Base (`/knowledge`) | Tabs for Overview (index, engines with install commands, health, modules), Knowledge graph (Sigma.js on a Graphology graph at module/file/symbol level with an accessible node list and an explorer panel), Commits (one row per SHA across all git worktrees; Diff / Impact / AI review / Tests for the selected commit), Worktrees (every git worktree with its agent) and AI insights (cited chat). The selected commit lives in the URL (`?tab=&commit=&node=&level=&module=`) and drives the graph highlight; a strip of the newest commits stays under every tab. The WebGL renderer is its own lazy chunk; graph colours are resolved from the design tokens at runtime. Design and data flow: [`KNOWLEDGE_BASE.md`](./KNOWLEDGE_BASE.md). |
| Pull requests | Evidence-backed readiness (1–5), blockers, diff with review comments; "Record human approval" is a separate confirmed action and never merges. |
| Workflow Studio | CodeMirror 6 (Markdown + YAML front matter), debounced runtime validation with located diagnostics, effective config, prompt preview, Ctrl+S, revert, external-change detection, unsaved-changes guard. |
| Providers / Settings | Real detection (install, version, auth source) and vault-backed credentials; settings validated client-side as a mirror of the runtime. |
| Onboarding | First-run dialog (welcome, detected agents, register repository, finish) persisted as `onboarding_completed`. |

## 6. Shell

- **Sidebar** (collapsible, persisted): *New task*, a search field, tasks grouped by project as **threads**
  (status dot, approval mark, relative time, collapsible groups) and a *Manage* group with the other views:
  the day-to-day ones (Overview, Projects, Tasks, Pull requests, Activity) above a divider, then
  the setup ones (Worktrees, Workflow Studio, Providers, Settings). Collapsed it is an icon rail with tooltips.
- **Top bar** (below the native title bar - native decorations are kept for reliable drag, snap, maximize and
  system controls): current view, palette launcher, runtime connection status (derived from the real
  stream/state; shown only when it is not healthy, the status bar always shows it), pause/resume
  dispatching, refresh, approvals popover, theme.
- **Status bar**: connection, scheduler, counts, tokens/cost and hardware telemetry from the runtime
  snapshot (refreshed by events and every 15 s while visible; it never probes the GPU).
- There are no nested resizable panel groups at shell level; each view owns its layout (resizable panes only
  where a view needs them, e.g. Workflow Studio, Pull requests).

Keyboard: Ctrl+K palette - Ctrl+N new task - Ctrl+P projects - Ctrl+R refresh data - Ctrl+B sidebar -
Ctrl+J run-details panel - F1 shortcut help.

## 7. State rules

- Server data only in TanStack Query; event stream drives invalidation (`runtime/invalidation.ts`)
  with per-type targeting so streaming agent output does not refetch lists.
- Presentation state only in `uiStore` (persisted with guarded `localStorage`); nothing durable. Theme and
  locale are the exception that is mirrored to runtime settings (`PreferencesBridge`): the runtime value is
  adopted on load and later UI changes are written back.
- Mutations disable their triggers while pending; approval decisions and dispatches cannot be submitted twice.
- Optimistic updates only for status moves, with snapshot rollback on rejection.
