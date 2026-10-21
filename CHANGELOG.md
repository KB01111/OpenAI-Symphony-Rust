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

### Security

- DPAPI-backed credential vault (ChaCha20-Poly1305 with an owner-only key elsewhere), owner-only
  runtime discovery file, explicit vault quarantine instead of silent credential loss.
- See `docs/POST_MERGE_STABILIZATION.md` for the full hardening report.

[2.0.0]: https://github.com/KB01111/OpenAI-Symphony-Rust/releases/tag/v2.0.0
