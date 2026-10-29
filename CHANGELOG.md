# Changelog

All notable changes to Symphony Desktop are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). The release workflow publishes the section that matches
the release tag as the release notes (`scripts/release-version.mjs --notes`), so every release needs
one. The HTTP API is versioned separately (`API_VERSION`, see `CONTRIBUTING.md`).

## [Unreleased]

### Added

- **CodeSandbox cloud agents**: a new *CodeSandbox Cloud Agent* provider runs a task's agent in a
  [CodeSandbox](https://codesandbox.io) microVM built from an SDK template, using a CodeSandbox API token
  stored under *Providers* (`CSB_API_KEY`). The agent is chosen per task in the composer and defaults to
  **mini-swe-agent** (configurable in *Settings → Default provider* and `codesandbox.default_agent` in
  `WORKFLOW.md`); the **Pi coding agent** and **Pi Durable** are the alternatives. mini-swe-agent runs as a
  checkpointing agent class that reloads its progress after an interruption. Whatever the agent, it commits
  and pushes to the task branch after every edit, and Symphony fast-forwards the local worktree after each
  push (it never touches a dirty or diverged worktree). A retry or follow-up resumes the same sandbox.
  Needs Node.js ≥ 22, a GitHub token and an `https://github.com` origin; see
  `tools/codesandbox-runner/README.md`. New: `ProviderKind::CodeSandbox`, `CloudAgentKind`,
  `Task.cloud_agent` (migration 0005), `AppSettings.default_cloud_agent`, `CreateTaskRequest.cloud_agent`,
  the `codesandbox:` section of `WORKFLOW.md`, and `WorkspaceManager::fast_forward_from_origin`.

- **Thread-first desktop UI (Codex-style)**: the app opens on a prompt composer. One prompt box plus chips
  for repository (with its base branch), provider and model (with install state), priority, labels and
  *Run now* / *Add to queue* creates a task; the first line becomes the title and the whole prompt the
  description. Tasks appear in the sidebar as threads grouped by project (status dots, approval marks,
  search). A thread shows the prompt, the live timeline, pending approvals, an inline follow-up box
  (continue a finished run in the same worktree) and a run-details panel (changes, validation, tool
  calls, usage, pull request, context). The previous Overview, Projects, Tasks, Agent runs, Worktrees,
  Pull requests, Workflow Studio, Providers and Settings views are kept under *Manage*, plus a new
  *Activity* view with the former bottom dock (events, agent/terminal output, validations, approvals,
  problems).

- **Model discovery**: provider detection asks the installed CLI which models it offers (Codex
  `model/list`, `opencode models`) instead of relying on a built-in list, cached for 30 minutes per
  binary and version (a forced refresh re-reads it). A configured provider default that the CLI no
  longer accepts is no longer offered as the model of new tasks; the provider status says which model
  is used instead (a model already saved on a project or task is still passed as it is). Claude
  Code offers its aliases (`opus`, `sonnet`, `haiku`, `fable`) and the current model ids; a
  configured default that is another provider's model (such as an older settings file's
  `gpt-5-codex`) is not offered for it, while custom deployment names and inference profiles are.
- **Continue a run**: *Continue…* on a task in review, failed or completed sends follow-up instructions;
  the agent resumes its conversation in the same worktree and the result is reviewed again
  (`POST /api/v1/tasks/{id}/continue`, local only). The instructions are stored (schema migration 3)
  until the agent's first turn with them has finished, so a restart does not lose them; a plain
  *Retry* drops them. If the task already has a pull request, the new review links it, says the work
  is not on it yet and offers *Push changes to #N*, which pushes the branch and so updates that pull
  request; if it has been merged or closed in the meantime, a new pull request is opened instead.
- **Older activity**: the run timeline loads older events on request (`GET /api/v1/tasks/{id}/events?before=<event id>`,
  a cursor that neither skips nor repeats events with equal timestamps).
- **Claude Code approvals**: Claude Code runs as a host-controlled session, so tool uses its permission
  rules would ask about become Symphony approvals (approve once, for the session, decline, abort) instead
  of being silently denied in a headless run. Under `approval_policy: never` they stay denied.
- **OpenCode approvals**: under an approval policy OpenCode runs through Symphony's own `opencode serve`,
  so its bash, edit and web requests become Symphony approvals instead of running unasked (OpenCode's
  default) or being rejected automatically (`opencode run` with `ask` rules).
  Requests from sub-agents (OpenCode's `task` tool, in child sessions) are asked too, and their
  tokens are counted; a session the turn did not see start (a resumed session's earlier sub-agents)
  is looked up along its parents.

- The agent `linear_graphql` tool changes only issues (and their comments) of the configured Linear
  project: each mutation's targets are looked up first, and moving issues to another project or team is
  refused. The mutation is found the same way the document check finds it, so a fragment definition
  before it or a default object in its variable definitions cannot make it pass as an unscoped read,
  and a variable it uses must be passed in `variables` when the operation declares a default for it
  (Linear would apply the unchecked default).
- Pull request CI also reads GitHub **commit statuses** (Jenkins, Buildkite, older integrations): a red
  status fails the review even when check runs are green.
- The inspector shows what is running and what needs attention (approvals, failed runs, reviews) while
  nothing is selected.
- **Tool-call history**: every tool call of a run (Codex commands, file changes and MCP tools, the
  `github_api`/`linear_graphql` tools with the answer Symphony gave, Claude Code and OpenCode tools) is
  stored as one record with its status, duration and redacted arguments and output (capped at 64 KiB),
  shown in the run view's new *Tool calls* tab and served by `GET /api/v1/tasks/{id}/tool-calls` (local
  only, cursor pages). Schema migration 4; it follows the activity-history retention.
- **Automatic backups**: Settings → Backups backs up every 24 hours by default (`backup_interval_hours`,
  `0` = off) and keeps the newest 7 automatic backups (`backup_keep`). Backups made by hand are never
  deleted; automatic ones are marked *Automatic*.
- **Automatic pull request status**: open pull requests' CI, merge state and review comments are re-read
  every 15 minutes by default (`pr_refresh_interval_minutes`, `0` = only on *Refresh status*), one after
  another and only with a GitHub token; a change updates the review and is announced as an event.

### Fixed

- Terminal colour codes (`ESC[2m…`) in an agent's log lines showed up as garbled characters in the run
  timeline and Activity; they are now removed.
- The *Pending approvals* tile on Overview did nothing since the bottom dock was replaced by the
  Activity view; it now opens Activity on its approvals tab.
- Project settings: the Provider dropdown no longer overlaps the Model field, the form no longer shows a
  stray horizontal scrollbar, and the project table is no longer clipped next to the detail pane
  (`min-width: 250px` controls in a viewport-breakpoint grid, nested scrollers, fixed-width detail pane).
- The Concise / Technical log toggle renders as a proper segmented control instead of two mashed labels.
- A task's header badges wrap under a long title instead of running under the action buttons.

- The build was broken on `main` after two dependency updates: the access token generator now uses the
  `rand` 0.10 API, and the API client generator is back on TypeScript 6 (`@hey-api/openapi-ts` needs
  its compiler API); `pnpm generate:api` now says so instead of crashing when another major is installed.
- Token usage follows SPEC 13.5: a generic `usage` map is never read as a cumulative total (Codex usage
  comes only from absolute thread totals, Claude Code's only from its final `result` line).
- A desktop test that timed out under the parallel `pnpm -r test` load: asynchronous queries in tests
  wait up to 5 s.
- OpenCode token usage and cost were always 0: the per-step token reports are now counted.
- Accessibility (axe-core WCAG 2.1 A/AA audit of every view in light and dark against the real
  runtime): caption, status and link colours now reach 4.5:1 on every surface (including the Apps SDK
  badges, alerts and code highlighting, through its own theme variables); the approvals bell and
  every copy button (event cards, code blocks, Markdown code) have an accessible name; scrollable
  diffs and logs can be scrolled from the keyboard.

### Security

- Raw provider event payloads (command output, tool results, diffs) were stored, streamed and exported
  without redaction (only the event message was redacted), so a token an agent printed reached the
  database, the event stream and diagnostics exports. Every string of a payload, and the live
  session's last message in `/state`, is now redacted first.
- `ctutils` 0.4.3 → 0.4.4 (RUSTSEC-2026-0385, non-constant-time comparisons; pulled in by `digest`).

### Changed

- **Agent runs is now a view of Tasks**: *Board*, *Table* and *Runs* sit side by side on the Tasks page, and
  the separate *Agent runs* sidebar entry is gone (its running/approval badge moved to *Tasks*). `/runs`
  still works and opens the Runs view; task threads stay at `/runs/<id>`.
- **Quieter shell**: the sidebar lists the day-to-day views (Overview, Projects, Tasks, Pull
  requests, Activity) above the setup views (Worktrees, Workflow Studio, Providers, Settings); the top
  bar shows the runtime connection only when it is not healthy (the footer always shows it); the refresh
  button is called *Refresh*; a task that has not run yet offers *Start* without a redundant *Retry*.
  The Runs view lists only the status tabs that have runs (and hides *Group by* when there are none), a
  whole row is a single click to open, a finished thread no longer shows its phase next to an
  identical status badge, the Projects tracker filter appears only when repositories use more than one
  tracker, and the sidebar's view list is tall enough to show every entry without its own scrollbar. The
  concise run timeline leaves out the contentless "Event: item/started"-style protocol rows (the
  technical view keeps every event). Empty Kanban columns are slim, so more of the populated ones fit on
  screen, and Settings shows *Reset* only once something has been edited. The Overview live-sessions
  table no longer clips its last column: Project and Tokens appear only on wide windows. The Kanban
  board shows Backlog, Blocked and Cancelled only when they hold a task or while a card is dragged.
  Settings shows one section at a time (a dot in the section list marks one with an invalid field), and
  each provider card keeps capabilities and models behind a single "Capabilities and models" disclosure.
- **One component system on the desktop**: Fluent UI and the OpenAI Apps SDK UI are removed from the
  desktop app and the shared UI package in favour of shadcn/ui (Radix) with Tailwind tokens, so controls,
  dialogs and tabs share one sizing, one set of breakpoints and one CSS layer order. Markdown uses
  `react-markdown` directly. The mobile companion keeps Fluent through `@symphony/ui/legacy`.
- **Layouts are container-driven**: forms, tables and cards size from their own container (`@container`)
  instead of the viewport. The project table hides low-priority columns by its width and project settings
  open in a side sheet.
- The right-hand inspector and the permanent bottom dock are gone; their content lives in the thread's
  run-details panel and in the Activity view. Keyboard shortcut `Ctrl+J` toggles the run-details panel
  (`Ctrl+I` is no longer used).

- The default model is empty by default, meaning each provider's own default; making a provider the
  global default no longer copies another provider's model. Cost estimates use current per-family prices
  (Claude Opus/Sonnet/Haiku/Fable 5.x and older generations).
- Narrow panels stack labels above values (no more per-character wrapping of paths); the activity
  drawer, readiness tiles, overview metrics and tab lists no longer clip their content.
- Counts read naturally in English and Swedish ("1 blocker", "2 körningar") instead of "blocker(s)";
  messages can use the ICU plural subset (`{count, plural, one {…} other {…}}`).
- On a first launch in a window narrower than 1200 px the inspector starts closed; selecting
  something opens it. The companion's connect screen shows the Symphony mark.
- Development dependencies: vitest 5 and `@testing-library/jest-dom` 7 (6.10.0 was deprecated upstream).

## [2.0.0] - 2026-10-22

First release of Symphony Desktop 2.0: a Windows 11 app that runs coding agents against your issues in isolated Git worktrees. Download `Symphony-Desktop_2.0.0_x64-setup.exe` below; later versions can be installed from inside the app (Settings → Application).

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
- **In-app updates** (Settings → Application and a banner in the window): the app looks for a newer GitHub
  release at startup and every six hours (switch off with *Check for updates automatically*), shows the
  release notes and **Update now** downloads the installer, verifies its signature against the update key
  built into the app, runs it and restarts; it asks first when agents are running. Releases built without
  the update key (no `latest.json`) are announced and open the releases page. The release workflow signs
  the installer for the updater and publishes `latest.json` when the `TAURI_SIGNING_PRIVATE_KEY` secret is
  set (`docs/RELEASING.md`). The installer signature records the release version and the app requires it
  to match the manifest (`requireSignedVersion`), so a forged manifest cannot downgrade the app.
- **Activity-history retention** (Settings → Scheduling & resources, `event_retention_days`, default 30, `0` keeps
  everything): the runtime deletes old activity-log events of finished tasks a couple of minutes after startup
  and every six hours, in small batches. Events of tasks that are still active are never deleted. Before this the
  `agent_events` table grew without bound.
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
