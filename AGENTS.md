# AGENTS.md — Instructions for Autonomous Coding Agents

This repository contains **Symphony Desktop 1.0**, a Rust + Tauri 2 + React 19 + Fluent UI v9 + OpenAI Apps SDK UI autonomous coding orchestration platform.

When operating inside this repository (whether dispatched by Symphony Desktop itself, Codex, Claude Code, or OpenCode), follow these rules:

## 1. Repository Map

- `crates/symphony-core`: Core domain types, state transitions (`TaskStatus::can_transition_to`), `WORKFLOW.md` YAML/Liquid parser (`WorkflowDefinition`, `ServiceConfig`), and secret redaction.
- `crates/symphony-storage`: SQLite WAL storage and migrations (`migrations/0001_init.sql`).
- `crates/symphony-workspaces`: Git worktree isolation, workspace key sanitization (`workspace_key_for_identifier`), path containment (`validate_workspace_path`), and validation command execution.
- `crates/symphony-platform`: Windows 11 power/CPU/memory telemetry (`HardwareMonitor`), single-instance lock, and credential vault.
- `crates/symphony-trackers`: GitHub Issues REST adapter, Linear GraphQL adapter, and host-side dynamic tools (`github_api`, `linear_graphql`).
- `crates/symphony-agents`: `CodexAppServerDriver` (stdio JSON-RPC 2.0), `ClaudeCodeDriver`, `OpenCodeDriver`, and `SimulatedAgentDriver`.
- `crates/symphony-orchestrator`: Single authoritative scheduler, reconciliation loop, approval gate, and PR Merge Readiness evaluator.
- `crates/symphony-server`: Axum HTTP/SSE API (`/api/v1/*`).
- `packages/shared-types`: Shared TypeScript types and EN/SV translations.
- `packages/api-client`: Typed HTTP/SSE + Tauri IPC client.
- `packages/ui`: Unified Fluent UI v9 + `@openai/apps-sdk-ui` components.
- `apps/desktop`: Tauri 2 Windows 11 desktop application.
- `apps/mobile`: iOS/Mobile remote companion application.

## 2. Mandatory Verification Commands

Before completing any task, execute:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
pnpm typecheck
pnpm test
pnpm build
```

## 3. Safety Invariants

- Never weaken `validate_workspace_path` or `workspace_key_for_identifier` in `crates/symphony-workspaces`.
- Never remove tracker environment variable scrubbing (`SECRET_ENV_VARS_TO_STRIP`) when spawning agent subprocesses.
- Preserve full compatibility with `@openai/apps-sdk-ui` and `@fluentui/react-components` in `packages/ui` and `apps/desktop`.
