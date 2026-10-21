# Symphony Desktop 2.0

**A Native Windows-First AI Coding Orchestration Platform Built in Rust, Tauri 2, Fluent UI 2, shadcn/ui, and OpenAI Apps SDK UI**

Symphony Desktop is a local-first autonomous coding orchestration platform inspired by the [OpenAI Symphony Specification (`SPEC.md`)](https://github.com/openai/symphony/blob/main/SPEC.md). It turns GitHub Issues, Linear tasks, and manually created development tasks into isolated, multi-turn AI coding workflows with deterministic Git worktree management, multi-provider agent execution (`Codex App Server`, `Claude Code`, `OpenCode`), automated validation pipelines, evidence-backed pull request review ratings, and a real-time Windows 11 desktop and iOS remote companion interface.

---

## Key Capabilities

- **Native Rust Orchestration Engine (`symphony-orchestrator`)**:
  - Implements the complete OpenAI Symphony lifecycle: tracker polling, label-based opt-in eligibility, priority-aware candidate sorting, workspace allocation, multi-turn agent sessions, Part A stall detection, Part B tracker reconciliation, exponential retry backoff, and crash recovery.
  - Authoritative single-scheduler state machine backed by SQLite in WAL mode (`symphony-storage`).
  - Dynamic `WORKFLOW.md` watcher (`notify`) with YAML front-matter validation and strict Liquid prompt rendering (`liquid`).
- **Multi-Provider AI Agent Runtime (`symphony-agents`)**:
  - **OpenAI Codex App Server** primary integration over stdio JSON-RPC 2.0 (`initialize`, `initialized`, `thread/start`, `thread/resume`, `turn/start`, `turn/interrupt`, `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `item/tool/call`, `thread/tokenUsage/updated`).
  - **Claude Code** CLI/stream-json adapter and **OpenCode** HTTP/CLI adapter with explicit capability reporting (`ProviderCapabilities`).
  - Strict stdout protocol / stderr diagnostic separation and automatic stripping of tracker credentials from child agent environments.
- **First-Class Git Worktree Isolation (`symphony-workspaces`)**:
  - Collision-resistant workspace keys (`[A-Za-z0-9._-]` sanitization + 64-bit SHA-256 suffix when modified).
  - Canonical path containment and symlink escape prevention.
  - Per-task Git worktree creation (`git worktree add -B`), dirty-worktree guards, lifecycle hooks (`after_create`, `before_run`, `after_run`, `before_remove`), and automated validation command execution.
- **Symphony Desktop 2.0 UI (`apps/desktop`, `packages/ui`)** — see [docs/UI_ARCHITECTURE.md](./docs/UI_ARCHITECTURE.md) and [docs/DESIGN_SYSTEM.md](./docs/DESIGN_SYSTEM.md):
  - Modular desktop shell: command bar, collapsible sidebar with project switcher, Ctrl+K command palette, resizable context inspector, bottom activity drawer (events, agent/terminal output, validations, approvals, problems) and a status bar with live hardware telemetry.
  - Views: Overview, Projects (guided Add Repository with native folder picker), Tasks (9-state Kanban with state-machine-validated drag and drop + data table), Agent Runs (control center), Live Execution workspace, Worktrees, Pull Request Review Center (1–5 merge readiness), Workflow Studio (CodeMirror + runtime validation/preview), Providers, Settings.
  - One token source shared by **Fluent UI React v9** (Windows-native controls, dialogs, menus, forms), **shadcn/ui** (resizable panels, command palette, data tables, context menus) and **OpenAI Apps SDK UI** (AI-native approval, event, markdown, code and readiness surfaces); dark/light/system themes, compact/comfortable density, reduced motion, English and Swedish.
  - OpenAPI 3.1 document generated from the Axum routes (`utoipa-axum`), TypeScript types and SDK generated from it with Hey API; CI checks freshness and breaking changes. Authenticated SSE streaming with resume, dedupe and reconnection.
- **Hardware-Aware Windows 11 Execution (`symphony-platform`)**:
  - Optimized for hybrid laptops (e.g., ASUS ROG Zephyrus G16 with AMD Ryzen AI 9 HX 370 + Radeon 890M + RTX 4070).
  - Queries Windows power status via `GetSystemPowerStatus` (`windows-sys`) without waking disabled discrete GPUs on battery.
  - Adjusts max concurrent coding agents dynamically for AC power (default 2), battery power (default 1), and memory/CPU pressure.
  - Persistent background runtime with Windows system tray integration (`Close to Tray`, `Minimize`, `Stop Orchestrator`, `Exit Completely`) and single-instance lock protection.
- **Lumen-Ready API & iOS Companion Foundation (`symphony-server`, `apps/mobile`)**:
  - Versioned Axum HTTP/SSE API (`/api/v1/*`) compatible with the Symphony specification (`/api/v1/state`, `/api/v1/{issue_identifier}`, `/api/v1/refresh`) plus desktop/mobile endpoints. Every route except `health`/`runtime` requires a Bearer token (random per launch locally), CORS is restricted to explicit origins, and credentials are stored in an OS-protected vault (DPAPI on Windows).
  - Optional remote companion listener (off by default): its own runtime-generated token (rotate/revoke in Settings → Remote access), a restricted operation set (no credentials, settings, workflow or administration), loopback-only binding behind a TLS reverse proxy or tunnel, and live reconfiguration.

---

## Repository Structure

```text
OpenAI-Symphony-Rust/
├── apps/
│   ├── desktop/            # Tauri 2 + React 19 Windows desktop application
│   │   ├── src/            # Frontend views, Zustand store, Vitest suites
│   │   └── src-tauri/      # Tauri 2 Rust host, system tray, IPC commands
│   └── mobile/             # Tauri 2 iOS / Mobile remote companion foundation
├── crates/
│   ├── symphony-core/      # Domain models, SPEC.md workflow parser, redaction
│   ├── symphony-storage/   # SQLite WAL persistence, migrations, crash recovery
│   ├── symphony-workspaces/# Git worktrees, path safety, lifecycle hooks
│   ├── symphony-platform/  # Windows 11 power/CPU/RAM telemetry, vault, lockfile
│   ├── symphony-trackers/  # GitHub Issues, Linear GraphQL, & dynamic tools
│   ├── symphony-agents/    # Codex App Server JSON-RPC, Claude Code, OpenCode
│   ├── symphony-orchestrator/ # Authoritative scheduler, reconciliation, PR review
│   ├── symphony-server/    # Axum HTTP + SSE local & authenticated remote server
│   └── symphony-cli/       # Headless daemon & CLI (`symphony-cli`)
├── packages/
│   ├── shared-types/       # Rust-generated API contracts, state machine table, EN/SV i18n
│   ├── api-client/         # Typed HTTP client, authenticated SSE, Tauri IPC bridge (`SymphonyClient`)
│   └── ui/                 # Symphony design system: tokens, Fluent v9 + shadcn/ui + Apps SDK UI
├── WORKFLOW.md             # Repository Symphony workflow configuration
├── ARCHITECTURE.md         # System architecture & SPEC.md compliance document
├── SECURITY.md             # Security boundaries, sandboxing, and secret isolation
├── CONTRIBUTING.md         # Developer setup and verification guide
└── AGENTS.md               # Instructions for coding agents operating in this repo
```

---

## Windows 11 Setup & Development (PowerShell)

### Prerequisites

- **Rust**: Stable toolchain (`1.90+`, MSVC target `x86_64-pc-windows-msvc`)
- **Node.js**: `>= 22.13.0`
- **pnpm**: `11.15.1` (pinned by `packageManager`; use Corepack)
- **Git**: `2.40+` (for Git worktree isolation)
- **Codex CLI** *(optional for live Codex runs)*: `@openai/codex` (`codex app-server`)

### 1. Install Frontend Dependencies

```powershell
pnpm install --frozen-lockfile
```

### 2. Build and Verify the Rust Workspace

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

### 3. Regenerate Contracts, Typecheck, Test, and Build the Frontend & Companion Apps

```powershell
pnpm generate:api
pnpm typecheck
pnpm test
pnpm build
```

### 4. Run the Standalone Orchestration Daemon / CLI

```powershell
# Validate workflow configuration and print parsed ServiceConfig
cargo run -p symphony-cli -- validate .\WORKFLOW.md

# Start the orchestration engine and HTTP/SSE API (port from WORKFLOW.md `server.port`, else 4780;
# falls back to an ephemeral port when busy). A random access token is generated and published,
# with the endpoint, in <data_dir>\runtime.json (owner-only); it is never printed.
cargo run -p symphony-cli -- .\WORKFLOW.md

# Isolated data directory (separate database, vault and runtime lock). The desktop host honours the
# same SYMPHONY_DATA_DIR environment variable.
cargo run -p symphony-cli -- --data-dir .\.symphony-dev

# Move an unreadable credential vault aside explicitly (it is never replaced silently)
cargo run -p symphony-cli -- --quarantine-credential-vault
```

### 5. Launch Symphony Desktop (Development Mode)

```powershell
# Browser UI against a running runtime: the dev server discovers it through runtime.json
# (set SYMPHONY_DATA_DIR if the runtime uses --data-dir)
pnpm --filter @symphony/desktop dev

# Full native Tauri 2 application (embeds the runtime, or attaches to a running symphony-cli)
pnpm --filter @symphony/desktop tauri dev
```

### 6. End-to-End Tests

```powershell
# Builds symphony-cli if needed, starts it with a throwaway data dir + fixture repo, runs Playwright
pnpm --filter @symphony/desktop e2e
```

---

## Logs & Troubleshooting

- Logs: `%LOCALAPPDATA%\symphony-desktop\logs\desktop-YYYY-MM-DD.log` (desktop) and `cli-YYYY-MM-DD.log`
  (CLI), or `<data_dir>\logs` with `--data-dir` / `SYMPHONY_DATA_DIR`. They are owner-only, secrets are
  redacted, 14 days are kept. Set `RUST_LOG` (e.g. `RUST_LOG=debug`) for more detail.
- Settings → Diagnostics → *Export diagnostics* produces a redacted bundle for bug reports.
- An unreadable credential vault is never replaced silently: use the desktop recovery dialog or
  `symphony-cli --quarantine-credential-vault`.

## Releases

Releases are cut by pushing a `vX.Y.Z` tag after the version in every manifest and a dated `CHANGELOG.md`
section match it (`node scripts/release-version.mjs --tag vX.Y.Z`). The release workflow builds, tests,
smoke-tests and drafts the GitHub release; see [CONTRIBUTING.md](./CONTRIBUTING.md#releasing) and
[docs/PRODUCTION_READINESS.md](./docs/PRODUCTION_READINESS.md).

## License

[MIT](./LICENSE).

---

## Documentation Index

- [docs/UI_ARCHITECTURE.md](./docs/UI_ARCHITECTURE.md) — Desktop UI architecture, contracts, discovery and streaming.
- [docs/DESIGN_SYSTEM.md](./docs/DESIGN_SYSTEM.md) — Tokens, providers and component ownership.
- [docs/UI_OVERHAUL_AUDIT.md](./docs/UI_OVERHAUL_AUDIT.md) — Pre-2.0 audit (P0–P3 findings).
- [docs/UI_IMPLEMENTATION_REPORT.md](./docs/UI_IMPLEMENTATION_REPORT.md) — 2.0 implementation report and evidence.
- [docs/POST_MERGE_STABILIZATION.md](./docs/POST_MERGE_STABILIZATION.md) — Post-merge security/reliability findings, fixes and verification.
- [docs/WINDOWS_NATIVE_SMOKE_TEST.md](./docs/WINDOWS_NATIVE_SMOKE_TEST.md) — Automated and manual native Windows 11 validation.
- [docs/PRODUCTION_READINESS.md](./docs/PRODUCTION_READINESS.md) — Release gates, operations and the remaining owner actions.
- [CHANGELOG.md](./CHANGELOG.md) — Release notes (the release workflow publishes the tagged section).
- [ARCHITECTURE.md](./ARCHITECTURE.md) — Detailed architecture, Symphony `SPEC.md` compliance, and component design.
- [SECURITY.md](./SECURITY.md) — Threat model, workspace isolation invariants, credential handling, and sandboxing.
- [CONTRIBUTING.md](./CONTRIBUTING.md) — Development standards, testing matrix, and PR workflow.
- [AGENTS.md](./AGENTS.md) — Guidelines for autonomous AI coding agents working on Symphony Desktop.
