# Changelog

All notable changes to Symphony Desktop are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). The release workflow publishes the section that matches
the release tag as the release notes (`scripts/release-version.mjs --notes`), so every release needs
one. The HTTP API is versioned separately (`API_VERSION`, see `CONTRIBUTING.md`).

## [2.0.0] - Unreleased

First release. Replace "Unreleased" with the release date in the commit that is tagged `v2.0.0`.

### Added

- Rust orchestration runtime with a Tauri 2 desktop host and a React 19 + Fluent UI v9 + shadcn/ui +
  Apps SDK UI interface; Codex App Server, Claude Code, OpenCode and simulated agent drivers; GitHub
  Issues and Linear trackers; isolated Git worktrees, approvals, validations and PR merge readiness.
- Authenticated local HTTP/SSE API, generated into a typed client from the OpenAPI document.
- Remote companion access with a scoped, rotatable token and a loopback-only listener.
- Persistent diagnostic logs in `<data dir>/logs` (owner-only, secrets redacted, daily files, 14
  kept, 50 MiB per day) for the desktop host and the CLI, including panics. The packaged desktop app
  has no console, so without them its logs are lost.
- Tag-driven release workflow: version/tag consistency check, tests, native smoke test, optional
  Authenticode signing, SHA-256 checksums and a draft GitHub release.
- Supply-chain checks: `cargo deny` (advisories, licenses, sources), `pnpm audit` for production
  dependencies, Dependabot for Cargo, npm and GitHub Actions.
- Installer metadata (publisher, description, per-user NSIS install) and an optimized release profile.

### Fixed

- Cancelling or stall-timing-out a Claude Code or OpenCode task terminates the agent and every
  process it started; previously the agent kept running and editing the workspace.
- A Claude Code or OpenCode agent can no longer block forever on a full stderr pipe, and
  `turn_timeout_ms` applies to them as well as to Codex.
- The packaged Windows app no longer flashes console windows for git, agent, hook and validation
  subprocesses.
- Timed-out lifecycle hooks and validation commands terminate their whole process tree (for example
  `node` started by `npm test`), not only the shell.

- A stalled task is retried only after its backoff delay: it was picked up again as a regular
  queued task on the next scheduler tick, and after a restart the backoff resumes from the task's
  saved retry time. Due retries also respect a paused scheduler, host
  resource pressure and the concurrency limit; they no longer start beyond it.
- Token counts and cost estimates are no longer inflated: the runtime totals added every usage
  report of a session (Codex sends many, each with the running total), and a multi-turn Codex task
  counted its earlier turns again on every turn. Stream events now carry the session's cumulative
  usage, turn outcomes that turn's usage, and only growth is counted. The first turn of a resumed
  Codex thread may still include the thread's earlier usage if the app server reports it.
- GitHub and Linear issue listings follow pagination instead of stopping at the first 100 or 50
  issues, so newer issues of a large backlog are dispatched. Candidate listings read up to 100
  pages; the terminal-state listing used for startup workspace cleanup reads up to 10 pages,
  most recently closed GitHub issues first.

- Restarting the desktop app or the CLI right after a crash no longer fails or attaches to the dead
  runtime: Windows releases a terminated process's file lock with a delay, and a process that had
  exited could still be reported as running. A lock left by an exited owner is now waited for (up to
  10 s), process liveness on Windows comes from the process object itself, and a desktop started while
  another runtime is still starting waits for it to publish its endpoint.

### Changed

- Keyboard focus indicators: a single keyborg copy (Fluent UI 9.74.10); two copies shared one
  registry and could tear down keyboard-navigation tracking.
- Dependencies: TypeScript 7 (the API client generator keeps TypeScript 6 in its own
  `tools/api-generator` package), Vite 8 with `@vitejs/plugin-react` 6, TanStack Table 9, motion 14,
  lucide-react 1, sqlx 0.9 (existing databases open unchanged), tower-http 0.7, sha2 0.11, base64 0.23,
  dirs 7, and current GitHub Actions.

### Security

- KaTeX raised to 0.18 (GHSA-238p-pmpm-9mq7) with its own stylesheet, so math rendered in agent
  messages keeps its layout; `pnpm audit` reports no known vulnerabilities.
- DPAPI-backed credential vault (ChaCha20-Poly1305 with an owner-only key elsewhere), owner-only
  runtime discovery file, explicit vault quarantine instead of silent credential loss.
- See `docs/POST_MERGE_STABILIZATION.md` for the full hardening report.

[2.0.0]: https://github.com/KB01111/OpenAI-Symphony-Rust/releases/tag/v2.0.0
