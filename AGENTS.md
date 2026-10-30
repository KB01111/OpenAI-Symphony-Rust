# AGENTS.md — Instructions for Autonomous Coding Agents

This repository contains **Symphony Desktop 2.0**, a Rust + Tauri 2 + React 19 + Fluent UI v9 + shadcn/ui + OpenAI Apps SDK UI autonomous coding orchestration platform.

When operating inside this repository (whether dispatched by Symphony Desktop itself, Codex, Claude Code, or OpenCode), follow these rules:

## 1. Repository Map

- `crates/symphony-core`: Core domain types, state transitions (`TaskStatus::can_transition_to`), `WORKFLOW.md` YAML/Liquid parser (`WorkflowDefinition`, `ServiceConfig`), and secret redaction.
- `crates/symphony-storage`: SQLite WAL storage, versioned migrations (`migrations/NNNN_*.sql`, listed in `MIGRATIONS`; never edit an applied one) and backups (`backup.rs`).
- `crates/symphony-workspaces`: Git worktree isolation, workspace key sanitization (`workspace_key_for_identifier`), path containment (`validate_workspace_path`), and validation command execution.
- `crates/symphony-knowledge`: the Repository Knowledge Base (read-only git, snapshots, Graphify/Serena adapters, graph, commit impact, evidence-bound AI prompts, `knowledge.db`); see `docs/KNOWLEDGE_BASE.md`. Its endpoints are local-only.
- `crates/symphony-platform`: Windows 11 power/CPU/memory telemetry (`HardwareMonitor`), single-instance lock, owner-only secret files (`secure_fs`), the DPAPI-backed credential vault (`SecretStore`), runtime discovery and HKCU autostart.
- `crates/symphony-trackers`: GitHub Issues REST adapter, Linear GraphQL adapter, and host-side dynamic tools (`github_api`, `linear_graphql`) whose scope is an allowlist (`tool_policy.rs`).
- `crates/symphony-agents`: `CodexAppServerDriver` (stdio JSON-RPC 2.0), `ClaudeCodeDriver`, `OpenCodeDriver`, and `SimulatedAgentDriver`.
- `crates/symphony-orchestrator`: Single authoritative scheduler, reconciliation loop, approval gate, and PR Merge Readiness evaluator.
- `crates/symphony-server`: Axum HTTP/SSE API (`/api/v1/*`, Bearer-token auth) served through `utoipa-axum`: every handler carries a `#[utoipa::path]` annotation and the OpenAPI document (`openapi/symphony-api.json`) is generated from the same route table. After changing any route or API type run `pnpm generate:api` (OpenAPI document, task state machine and Hey API client) and commit the result; CI fails on a stale contract and on breaking changes (label the PR `api-breaking-change` and bump `API_VERSION` when a break is intended).
- `tools/api-generator`: the Hey API generator behind `pnpm generate:api`; kept on TypeScript 6 because the generator needs the TypeScript 6 compiler API (see `docs/PRODUCTION_READINESS.md` §5).
- `packages/shared-types`: generated API contracts (never hand-edit `src/generated/`): Hey API types/SDK/fetch client from the OpenAPI document (`@symphony/shared-types/api`) and the Rust task state machine; state-machine helpers, EN/SV translations.
- `packages/api-client`: `SymphonyClient` (wraps the generated SDK), the central API configuration for desktop/local/remote runtimes (`src/config/apiConfig.ts`), authenticated SSE streaming, Tauri IPC bridge; integration tests run against `symphony-cli`.
- `packages/ui`: Symphony design system (tokens, `SymphonyDesignProvider`, shadcn/ui + Radix primitives and AI components; Fluent only in `legacy.tsx` for the mobile companion). See `docs/DESIGN_SYSTEM.md` for component ownership.
- `apps/desktop`: Tauri 2 Windows 11 desktop application (`src/app`, `src/layouts`, `src/features/*`; see `docs/UI_ARCHITECTURE.md`).
- `apps/mobile`: iOS/Mobile remote companion application.

## 2. Mandatory Verification Commands

Before completing any task, execute:

```powershell
pnpm install --frozen-lockfile
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
pnpm generate:api
pnpm typecheck
pnpm test
pnpm build
pnpm --filter @symphony/desktop e2e
```

A breaking API change needs the `api-breaking-change` label **and** a higher `API_VERSION`; CI enforces both
(`scripts/api-contract-policy.mjs`). Do not claim native Windows behaviour was verified when only a build
succeeded (see `CONTRIBUTING.md`, "Native Windows vs. Browser Testing").

## 3. Safety Invariants

- Never weaken `validate_workspace_path` or `workspace_key_for_identifier` in `crates/symphony-workspaces`.
- Never remove tracker environment variable scrubbing (`SECRET_ENV_VARS_TO_STRIP`) when spawning agent subprocesses.
- Build desktop UI only from `packages/ui` (shadcn/ui + Radix + Tailwind tokens); do not add Fluent UI or another component library to `apps/desktop`. Lay out forms and tables with container queries, not viewport breakpoints (see `docs/DESIGN_SYSTEM.md` §3).
- Never weaken API authentication, the CORS allowlist, the credential key allowlist, or the Tauri capability allow-list; never add an IPC command that runs arbitrary commands or touches arbitrary paths.
- Never widen `tool_policy.rs` (agent tools run with the host's tokens, outside the approval gate) without tests that show what the new operation can do; there is deliberately no merge operation anywhere.
- Never weaken the credential vault (no plaintext/XOR fallback, no silent replacement of an unreadable vault) or `secure_fs` (owner-only DACL / `0600`); write secret-bearing files only through it.
- Keep the remote companion listener restricted: add an endpoint to `REMOTE_OPERATIONS` only deliberately, keep it loopback-only and never accept the local token there (or the remote token locally).
- Knowledge Base: never run Graphify or Serena inside a user's repository or worktree (only on the snapshot directories under `<data_dir>/knowledge`), never bundle or link Serena (GPL-3.0), never call a Serena tool outside `serena::ALLOWED_TOOLS`, never give the AI analysis a Symphony host tool or repository content, keep every built-in tool of the supported providers denied in `write_tool_restrictions`, and redact everything that reaches the provider, not only the diff; keep its endpoints out of `REMOTE_OPERATIONS`.
- Use only semantic design tokens in feature code (no hard-coded colours) and add EN + SV strings for every user-visible text.
