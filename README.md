# Symphony Desktop 1.0

**A Native Windows-First AI Coding Orchestration Platform Built in Rust, Tauri 2, Fluent UI 2, and OpenAI Apps SDK UI**

Symphony Desktop 1.0 is a local-first autonomous coding orchestration platform inspired by the [OpenAI Symphony Specification (`SPEC.md`)](https://github.com/openai/symphony/blob/main/SPEC.md). It turns GitHub Issues, Linear tasks, and manually created development tasks into isolated, multi-turn AI coding workflows with deterministic Git worktree management, multi-provider agent execution (`Codex App Server`, `Claude Code`, `OpenCode`), automated validation pipelines, evidence-backed pull request review ratings, and a real-time Windows 11 desktop and iOS remote companion interface.

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
- **Unified Fluent UI 2 + OpenAI Apps SDK UI Desktop Experience (`apps/desktop`, `packages/ui`)**:
  - **Microsoft Fluent UI React v9 (`@fluentui/react-components`)** powers the Windows 11 application shell, navigation sidebar, command bar, dialogs, Kanban board cards, and settings forms.
  - **OpenAI Apps SDK UI (`@openai/apps-sdk-ui`)** powers AI-native surfaces: `AppsSDKUIProvider`, `Alert`, `Badge`, `Button`, `CopyButton`, `CodeBlock`, `Markdown`, `Textarea`, `Tooltip`, `Popover`, `EmptyMessage`, `SegmentedControl`, and `ShimmerText`.
  - Synchronized light/dark themes, reduced-motion support, and full English (`en`) and Swedish (`sv`) localization.
- **Hardware-Aware Windows 11 Execution (`symphony-platform`)**:
  - Optimized for hybrid laptops (e.g., ASUS ROG Zephyrus G16 with AMD Ryzen AI 9 HX 370 + Radeon 890M + RTX 4070).
  - Queries Windows power status via `GetSystemPowerStatus` (`windows-sys`) without waking disabled discrete GPUs on battery.
  - Adjusts max concurrent coding agents dynamically for AC power (default 2), battery power (default 1), and memory/CPU pressure.
  - Persistent background runtime with Windows system tray integration (`Close to Tray`, `Minimize`, `Stop Orchestrator`, `Exit Completely`) and single-instance lock protection.
- **Lumen-Ready API & iOS Companion Foundation (`symphony-server`, `apps/mobile`)**:
  - Versioned Axum HTTP/SSE API (`/api/v1/*`) compatible with the Symphony specification (`/api/v1/state`, `/api/v1/{issue_identifier}`, `/api/v1/refresh`) plus extended desktop/mobile endpoints and scoped Bearer token authentication.

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
│   ├── shared-types/       # Shared TypeScript interfaces, state machine, EN/SV i18n
│   ├── api-client/         # Typed HTTP/SSE & Tauri IPC client (`SymphonyApiClient`)
│   └── ui/                 # Unified Fluent UI v9 + @openai/apps-sdk-ui components
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
- **Node.js**: `v22+` or `v24+`
- **pnpm**: `v10+` or `v11+`
- **Git**: `2.40+` (for Git worktree isolation)
- **Codex CLI** *(optional for live Codex runs)*: `@openai/codex` (`codex app-server`)

### 1. Install Frontend Dependencies

```powershell
pnpm install
```

### 2. Build and Verify the Rust Workspace

```powershell
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

### 3. Typecheck, Test, and Build the Frontend & Companion Apps

```powershell
pnpm typecheck
pnpm test
pnpm build
```

### 4. Run the Standalone Orchestration Daemon / CLI

```powershell
# Validate workflow configuration and print parsed ServiceConfig
cargo run -p symphony-cli -- validate .\WORKFLOW.md

# Start the orchestration engine and HTTP/SSE API on 127.0.0.1:47800
cargo run -p symphony-cli -- .\WORKFLOW.md --port 47800
```

### 5. Launch Symphony Desktop (Development Mode)

```powershell
# Start the Vite desktop UI against the running local runtime
pnpm --filter @symphony/desktop dev

# Or launch the full native Tauri 2 Windows application
cd apps\desktop
pnpm tauri dev
```

---

## Documentation Index

- [ARCHITECTURE.md](./ARCHITECTURE.md) — Detailed architecture, Symphony `SPEC.md` compliance, and component design.
- [SECURITY.md](./SECURITY.md) — Threat model, workspace isolation invariants, credential handling, and sandboxing.
- [CONTRIBUTING.md](./CONTRIBUTING.md) — Development standards, testing matrix, and PR workflow.
- [AGENTS.md](./AGENTS.md) — Guidelines for autonomous AI coding agents working on Symphony Desktop.
