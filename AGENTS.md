# AGENTS.md — Instructions for Autonomous Coding Agents

This repository contains **Symphony Desktop 2.0**, a Rust + Tauri 2 + React 19 + Fluent UI v9 + shadcn/ui + OpenAI Apps SDK UI autonomous coding orchestration platform.

When operating inside this repository (whether dispatched by Symphony Desktop itself, Codex, Claude Code, or OpenCode), follow these rules:

## 1. Repository Map

- `crates/symphony-core`: Core domain types, state transitions (`TaskStatus::can_transition_to`), `WORKFLOW.md` YAML/Liquid parser (`WorkflowDefinition`, `ServiceConfig`), and secret redaction.
- `crates/symphony-storage`: SQLite WAL storage and migrations (`migrations/0001_init.sql`).
- `crates/symphony-workspaces`: Git worktree isolation, workspace key sanitization (`workspace_key_for_identifier`), path containment (`validate_workspace_path`), and validation command execution.
- `crates/symphony-platform`: Windows 11 power/CPU/memory telemetry (`HardwareMonitor`), single-instance lock, and credential vault.
- `crates/symphony-trackers`: GitHub Issues REST adapter, Linear GraphQL adapter, and host-side dynamic tools (`github_api`, `linear_graphql`).
- `crates/symphony-agents`: `CodexAppServerDriver` (stdio JSON-RPC 2.0), `ClaudeCodeDriver`, `OpenCodeDriver`, and `SimulatedAgentDriver`.
- `crates/symphony-orchestrator`: Single authoritative scheduler, reconciliation loop, approval gate, and PR Merge Readiness evaluator.
- `crates/symphony-server`: Axum HTTP/SSE API (`/api/v1/*`, Bearer-token auth). `tests/ts_contract.rs` generates `packages/shared-types/src/generated/contracts.ts` — regenerate with `UPDATE_TS_CONTRACTS=1 cargo test -p symphony-server --test ts_contract` after changing any API type.
- `packages/shared-types`: Rust-generated API contracts (never hand-edit `src/generated/`), state-machine helpers, EN/SV translations.
- `packages/api-client`: Typed HTTP client, authenticated SSE streaming, Tauri IPC bridge; integration tests run against `symphony-cli`.
- `packages/ui`: Symphony design system (tokens, `SymphonyDesignProvider`, Fluent v9 + shadcn/ui primitives + Apps SDK UI AI components). See `docs/DESIGN_SYSTEM.md` for component ownership.
- `apps/desktop`: Tauri 2 Windows 11 desktop application (`src/app`, `src/layouts`, `src/features/*`; see `docs/UI_ARCHITECTURE.md`).
- `apps/mobile`: iOS/Mobile remote companion application.

## 2. Mandatory Verification Commands

Before completing any task, execute:

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --locked -- -D warnings
cargo test --workspace --locked
pnpm typecheck
pnpm test
pnpm build
```

## 3. Safety Invariants

- Never weaken `validate_workspace_path` or `workspace_key_for_identifier` in `crates/symphony-workspaces`.
- Never remove tracker environment variable scrubbing (`SECRET_ENV_VARS_TO_STRIP`) when spawning agent subprocesses.
- Preserve full compatibility with `@openai/apps-sdk-ui` and `@fluentui/react-components` in `packages/ui` and `apps/desktop`.
- Never weaken API authentication, the CORS allowlist, the credential key allowlist, or the Tauri capability allow-list; never add an IPC command that runs arbitrary commands or touches arbitrary paths.
- Use only semantic design tokens in feature code (no hard-coded colours) and add EN + SV strings for every user-visible text.
