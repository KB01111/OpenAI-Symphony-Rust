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
│ DesktopShell (layouts/) + feature views (features/*) + design system (@symphony/ui)            │
└───────────────────────────────────────────────────────────────────────────────────────────────┘
```

Rust remains the only authority for execution state: the UI never changes a task's status locally
except as an optimistic move that is rolled back if the runtime rejects it.

## 2. Contracts

- Rust types in `symphony-core`, `symphony-orchestrator` and `symphony-server` derive `ts_rs::TS`.
- `crates/symphony-server/tests/ts_contract.rs` renders them (plus the `TaskStatus` transition table)
  into `packages/shared-types/src/generated/contracts.ts` and **fails CI when the file is stale**.
  Regenerate with `UPDATE_TS_CONTRACTS=1 cargo test -p symphony-server --test ts_contract`.
- `crates/symphony-server/tests/api_router.rs` tests routing, auth, CORS and error mapping in Rust.
- `packages/api-client/test/integration/runtime.test.ts` drives the real Axum router through
  `symphony-cli` with the TypeScript client (discovery, state, project registration, task creation,
  immediate dispatch, live SSE events, approvals, cancel/retry, PR review, workflow preview/save,
  settings, credentials).

### Settings compatibility

The Rust field names are persisted in SQLite and are authoritative; the TypeScript type is generated
from them, so no renaming/migration was needed. `AppSettings` and `ResourcePolicy` use
`#[serde(default)]`, so new fields (e.g. `provider_default_models`) read older stored JSON safely.

## 3. Runtime discovery & authentication

| Client | Discovery | Auth |
| --- | --- | --- |
| Desktop (embedded) | `get_local_api_config` IPC returns the bound port (4780 or an ephemeral fallback) and the per-launch token | Bearer header |
| Desktop (attached) | Host reads `<data_dir>/runtime.json` of the owning runtime and returns it via IPC | Bearer header |
| Browser dev / E2E | Vite dev-only route `/__symphony/runtime` reads `runtime.json` (loopback Host only) | Bearer header |
| Mobile / remote | Configured URL + token (token in `sessionStorage` unless the user opts to remember it) | Bearer header |

`runtime.json` is written with owner-only permissions by whichever process owns the data directory
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
  layouts/      DesktopShell, Sidebar, CommandBar, ContextInspector, ActivityDrawer, StatusBar
  runtime/      RuntimeProvider, eventStore (ring buffer), invalidation map
  lib/          queries (TanStack hooks), queryKeys, i18n helpers
  store/        uiStore (persisted presentation state), safeStorage
  hooks/        useGlobalShortcuts, useFocusedTaskId
  components/   ErrorBoundary, DiffViewer
  features/     overview · projects · tasks · agents · execution · worktrees · reviews · workflows ·
                providers · settings · onboarding · command-palette
  test/         renderWithProviders (in-memory runtime), fixtures, setup
```

Routes: `/overview`, `/projects[/:projectId]`, `/tasks`, `/runs`, `/runs/:taskId`, `/worktrees`,
`/reviews[/:reviewId]`, `/workflow`, `/providers`, `/settings`. The last route is restored on launch.

## 6. Shell

- **Command bar** (below the native title bar — native decorations are kept for reliable drag, snap,
  maximize and system controls): project breadcrumb, palette launcher, runtime connection status
  (derived from the real stream/state), new task, pause/resume dispatching, refresh, approvals, theme,
  drawer/inspector toggles.
- **Sidebar**: collapsible (persisted), tooltips when collapsed, active indicator, project switcher, badges.
- **Context inspector**: right resizable/collapsible panel for the selected task/project/worktree/PR.
- **Activity drawer**: bottom dock with Events, Agent output, Terminal output (command executions of the
  focused task — no arbitrary shell), Validations, Approvals and Problems.
- **Status bar**: connection, scheduler, counts, tokens/cost and hardware telemetry from the runtime
  snapshot (refreshed by events and every 15 s while visible; it never probes the GPU).

Keyboard: Ctrl+K palette · Ctrl+N new task · Ctrl+P projects · Ctrl+R refresh data · Ctrl+B sidebar ·
Ctrl+J activity drawer · Ctrl+I inspector.

## 7. State rules

- Server data only in TanStack Query; event stream drives invalidation (`runtime/invalidation.ts`)
  with per-type targeting so streaming agent output does not refetch lists.
- Presentation state only in `uiStore` (persisted with guarded `localStorage`); nothing durable.
- Mutations disable their triggers while pending; approval decisions and dispatches cannot be submitted twice.
- Optimistic updates only for status moves, with snapshot rollback on rejection.
