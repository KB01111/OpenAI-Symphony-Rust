# Symphony Desktop 1.0 — Architecture

## 1. Overview & Alignment with OpenAI Symphony `SPEC.md`

Symphony Desktop 1.0 implements the language-independent [OpenAI Symphony Specification](https://github.com/openai/symphony/blob/main/SPEC.md) natively in Rust while extending it into a full-featured Windows 11 desktop and remote mobile orchestration platform.

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
   - Normal worker completion schedules a continuation check (`attempt = 1`, `1000ms` delay).
   - Abnormal worker exit schedules exponential backoff: `min(10000 * 2^(attempt - 1), agent.max_retry_backoff_ms)`.

---

## 2. Rust Workspace Architecture

| Crate | Responsibility |
| --- | --- |
| `symphony-core` | Domain model (`Issue`, `Task`, `TaskStatus`, `RunAttempt`, `ProviderKind`, `AgentEvent`, `ApprovalRequest`, `PullRequestReview`), `WORKFLOW.md` YAML + Liquid parser/renderer, secret redaction (`redact_secrets`), and error taxonomy (`SymphonyError`). |
| `symphony-storage` | SQLite WAL persistence (`sqlx`), schema migrations, crash recovery (`recover_stale_running_tasks`), event ring buffer persistence, and redacted diagnostic bundle export. |
| `symphony-workspaces` | Workspace key generation, canonical path validation, Git worktree lifecycle (`git worktree add -B`, `git worktree remove`), dirty-tree inspection, unified diff generation, lifecycle hooks, and validation command runner. |
| `symphony-platform` | Windows 11 `GetSystemPowerStatus` power telemetry, `sysinfo` CPU/RAM pressure monitoring, hardware-aware concurrency calculation, single-instance lockfile guard, and secret vault storage (`credentials.vault`). |
| `symphony-trackers` | `GitHubTracker` (REST v3 issues, label opt-in filter, PR diffs, CI check runs, review comment evaluation, `github_api` dynamic tool) and `LinearTracker` (GraphQL polling, `inverseRelations` blocker resolution, `linear_graphql` dynamic tool). |
| `symphony-agents` | Multi-provider coding agent runtime: `CodexAppServerDriver` (JSON-RPC 2.0 over stdio), `ClaudeCodeDriver` (`claude --output-format stream-json`), `OpenCodeDriver`, and deterministic `SimulatedAgentDriver`. |
| `symphony-orchestrator` | Central scheduler, `WORKFLOW.md` hot-reload watcher, approval gate (`OrchestratorApprovalGate`), dynamic tool dispatcher (`HostDynamicToolExecutor`), and evidence-based `MergeReadinessReport` (1–5) calculator. |
| `symphony-server` | Axum HTTP & SSE server implementing SPEC endpoints (`/api/v1/state`, `/api/v1/{issue_identifier}`, `/api/v1/refresh`) and desktop/mobile management endpoints with Bearer token auth for remote/Lumen access. |
| `symphony-cli` | Standalone CLI (`serve`, `run-once`, `doctor`, `validate-workflow`) enabling headless background execution independent of the Tauri UI window. |

---

## 3. Unified Design System: Fluent UI 2 + OpenAI Apps SDK UI

Symphony Desktop integrates two complementary design systems inside `packages/ui`:

1. **Microsoft Fluent UI React v9 (`@fluentui/react-components`)**:
   - Provides the Windows 11 native application chrome, navigation rail, command bar, dialogs, form inputs, tabs, switches, and surface tokens (`webDarkTheme` / `webLightTheme`).
2. **OpenAI Apps SDK UI (`@openai/apps-sdk-ui`)**:
   - Loaded via `@import "tailwindcss"; @import "@openai/apps-sdk-ui/css";` and wrapped in `<AppsSDKUIProvider linkComponent="a">`.
   - Powers all AI-native surfaces:
     - `Alert` & `Button` in `AiApprovalBanner` for command and file-change approval requests.
     - `CodeBlock` & `Markdown` in `AiEventStreamCard` for streaming agent reasoning, tool invocations, and unified Git diffs.
     - `Badge` & `Tooltip` in `ProviderBadge`, `TaskStatusBadge`, and `PhaseBadge`.
     - `SegmentedControl` in `LogModeSwitch` (`Concise Progress` vs `Detailed Logs`).
     - `ShimmerText` for live active turn indicators.
     - `EmptyMessage` in `AiEmptyView` for zero-state guidance.
     - `Textarea` & `Popover` in `PromptComposerBox` for interactive operator guidance.
3. **Theme Synchronization**:
   - `SymphonyDesignProvider` synchronizes Fluent UI's `FluentProvider` theme with `@openai/apps-sdk-ui/theme`'s `applyDocumentTheme("light" | "dark")`, setting `data-theme` and `color-scheme` on `document.documentElement` so both libraries share unified surface contrast, typography (`Segoe UI Variable Text` / `Cascadia Code`), and reduced-motion preferences.

---

## 4. Hardware-Aware Scheduling (Windows 11 Hybrid Laptops)

Designed for modern hybrid laptops such as the **ASUS ROG Zephyrus G16** (AMD Ryzen AI 9 HX 370, Radeon 890M iGPU, AMD XDNA 2 NPU, NVIDIA RTX 4070 Laptop GPU, 32 GB RAM):

- **Zero GPU Wake Overhead**: `symphony-platform` uses Win32 `GetSystemPowerStatus` and lightweight CPU/RAM counters without probing DXGI/NVML, ensuring the RTX 4070 remains in D3cold sleep while on battery.
- **Adaptive Concurrency**:
  - **AC Power**: Default up to `2` concurrent coding agents (configurable up to workflow limit).
  - **Battery Power**: Default capped at `1` concurrent coding agent.
  - **High Memory Pressure (`>85%` RAM used)**: New task dispatches pause automatically until memory pressure subsides.
  - **Thermal / High CPU Load (`>90%` sustained)**: Concurrency throttles down to `1`.

---

## 5. Remote Control, Lumen Integration, and iOS Companion

- **Loopback vs. Remote Binding**: By default, `symphony-server` binds strictly to `127.0.0.1:47800`. Remote network binding requires explicit opt-in (`remote_access_enabled: true`), a configured scoped Bearer token, and TLS termination.
- **iOS Companion (`apps/mobile`)**: Built with React 19, `@symphony/shared-types`, `@symphony/api-client`, and `@symphony/ui`, packaged with a Tauri 2 iOS configuration (`apps/mobile/tauri.ios.conf.json`). Connects remotely to the Windows orchestration daemon to monitor active agents, inspect streaming logs, approve tool calls, create tasks, and cancel runs with confirmation dialogs.
