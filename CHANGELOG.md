# Changelog

All notable changes to Symphony Desktop are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). The release workflow publishes the section that matches
the release tag as the release notes (`scripts/release-version.mjs --notes`), so every release needs
one. The HTTP API is versioned separately (`API_VERSION`, see `CONTRIBUTING.md`).

## [2.0.0] - Unreleased

First release. Replace "Unreleased" with the release date in the commit that is tagged `v2.0.0`.

### Added

- **Backups**: consistent SQLite backups with a checksum manifest (Settings → Backups, `GET/POST
  /api/v1/backups`), `symphony-cli verify-backup` and an offline `symphony-cli restore-backup` that keeps
  the replaced files and rolls back on failure. Credentials are never part of a backup
  (`docs/DATA_BACKUP.md`).
- The review (changed files, diff, credential scan) now covers commits the agent made itself, not only
  uncommitted changes. Reviews stored by earlier builds are normalised when read: their invented
  "CI passed" becomes "No remote CI", and a number that was only an attempt count no longer blocks
  *Create pull request*.
- **Refresh status** on a review with a pull request re-reads CI, merge state and comments from GitHub
  (`POST /api/v1/prs/{id}/refresh`). A repository without any check run now reads as "no CI" instead of
  "passed", and a merge GitHub has not computed yet reads as unknown instead of clean.
- **Create pull request**: a confirmed action on a review that commits the worktree changes to the task
  branch, pushes it to `origin` and opens a GitHub pull request (`POST /api/v1/prs/{id}/create-pull-request`,
  local administration only). It never merges; it is refused without a GitHub repository and token, with
  conflicts or with security warnings.
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
- **Windows installer**: a real application icon set (multi-size `.ico`, Start Menu/Store logos), an NSIS
  installer in English and Swedish with a language selector, license page, branded header/sidebar images and
  a Start Menu folder; only the NSIS bundle is built. The installer is tested in CI
  (`scripts/windows-installer-smoke.ps1`): silent install, the *installed* app passes the native runtime
  smoke test, upgrade over itself, silent uninstall, and user data survives. The release workflow can also
  publish straight from CI (manual run with `publish`; unsigned builds only as an explicit pre-release), and
  every commit on `main` produces an unsigned installer artifact.
- **Check for updates** (Settings → Application): an explicit lookup of the newest GitHub release. It never
  runs on its own, downloads nothing and opens the releases page; signed in-app updates still need the
  update-signing key described in `docs/RELEASING.md`.
- **Open data folder / Open log folder** (Settings → Application) and a **Keyboard shortcuts** help dialog
  (`F1`, `Ctrl+/`, the command palette or Settings).
- The tray menu and native notifications follow the interface language (English/Swedish) instead of being
  English only.

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
  usage, turn outcomes that turn's usage, and only growth is counted.
- A resumed Codex thread no longer charges its earlier usage again on every retry. Verified against
  `codex app-server` 0.163.0: `thread/resume` replays the thread's absolute total. Symphony now keeps
  the accounted thread total per task, measures a session against it, settles the usage of turns that
  time out, crash or are interrupted, and marks a figure it cannot measure exactly as estimated (shown
  as "≈"). Claude Code's prompt-cache tokens now count as input.
- Pull request reviews no longer invent facts. Every review used to claim "Remote CI checks passed",
  "mergeable" and "no conflicts" and showed the attempt count as the pull request number. The merge is
  now simulated against the base branch (`git merge-tree`, Git 2.38+), a local review reports "No remote
  CI", and the number is blank until a pull request exists.
- The credential scan of a change looks at the added lines of up to 4 MiB of diff for GitHub, `sk-`,
  Linear, AWS and Slack credentials and private keys, and warns when it could not scan everything.
- Database schema changes are versioned and transactional (`schema_migrations`); a snapshot of the
  database is taken before the first migration of an existing database, and a database written by a
  newer version is refused instead of guessed at.
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

- The application identifiers are now project-owned (`io.github.kb01111.symphony-desktop`,
  `…-companion`) instead of `com.openai.symphony.*`, which implied ownership by a third party. Decided
  before the first release: the data directory, credential vault and autostart entry do not depend on
  the identifier; the WebView profile (UI preferences) and the saved window position of a development
  build start fresh.
- Releases publish CycloneDX SBOMs and a `BUILD-INFO.txt` naming the source commit; all published files
  are covered by `SHA256SUMS.txt`.
- Dependencies: `tokio` 1.53.3 (1.54.0 was yanked), `vitest` 4.1.12 (`@vitest/mocker` advisories).
- Keyboard focus indicators: a single keyborg copy (Fluent UI 9.74.10); two copies shared one
  registry and could tear down keyboard-navigation tracking.
- Dependencies: TypeScript 7 (the API client generator keeps TypeScript 6 in its own
  `tools/api-generator` package), Vite 8 with `@vitejs/plugin-react` 6, TanStack Table 9, motion 14,
  lucide-react 1, sqlx 0.9 (existing databases open unchanged), tower-http 0.7, sha2 0.11, base64 0.23,
  dirs 7, and current GitHub Actions.

### Security

- The host-side `github_api` and `linear_graphql` agent tools are scoped: GitHub requests are confined to
  the project's own repository and cannot merge, delete or administer; Linear mutations are limited to
  comments, issue updates and PR attachments. Previously any request was forwarded with the host's token.
  Pull requests are opened only by the confirmed "Create pull request" action; a push never goes to a
  separate push URL or through a redirect, and a credential scan that cannot run blocks publishing.

- KaTeX raised to 0.18 (GHSA-238p-pmpm-9mq7) with its own stylesheet, so math rendered in agent
  messages keeps its layout; `pnpm audit` reports no known vulnerabilities.
- DPAPI-backed credential vault (ChaCha20-Poly1305 with an owner-only key elsewhere), owner-only
  runtime discovery file, explicit vault quarantine instead of silent credential loss.
- See `docs/POST_MERGE_STABILIZATION.md` for the full hardening report.

[2.0.0]: https://github.com/KB01111/OpenAI-Symphony-Rust/releases/tag/v2.0.0
