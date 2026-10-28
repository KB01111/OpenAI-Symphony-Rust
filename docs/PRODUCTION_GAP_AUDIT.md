# Production Gap Audit

Audit of Symphony Desktop 2.0 at `main` `2840ddd2` (merge of PR #20), carried out on branch
`claude/affectionate-mendel-j3qkyb` ([PR #21](https://github.com/KB01111/OpenAI-Symphony-Rust/pull/21)).

How to read it:

- **Verified defect**: reproduced or shown with concrete evidence (a failing test, a real tool's
  output, a registry response, or a code path that provably cannot do what the product claims).
- **Gap**: required behaviour that does not exist. **Concern**: plausible risk not confirmed.
- **Status** is what this branch actually did: *Fixed* (regression test added), *Open*, *Blocked*
  (needs an owner action or Windows hardware), *Unverified*. Documentation claims were not taken as
  evidence that something works.
- Severity: P0 data loss / security breach in default configuration; P1 wrong results, missing
  release-critical capability or exploitable weakness; P2 reliability / misleading UI; P3 polish.

Where the audit ran: a Linux container (Rust 1.97, Node 22, pnpm 11, Chromium). Everything that needs
Windows 11 (DPAPI, tray, installer, WebView2, autostart, multi-monitor, display scaling, hybrid GPU) is
**Unverified** here and is called out as such.

## Summary

| ID | Sev | Type | Title | Status |
| --- | --- | --- | --- | --- |
| SC-01 | P1 | Verified defect | Yanked `tokio 1.54.0` in `Cargo.lock` | Fixed |
| SC-02 | P1 | Verified defect | `@vitest/mocker` advisories (2 moderate) | Fixed |
| AG-01 | P1 | Verified defect | Resumed Codex thread: earlier usage charged again on every retry | Fixed |
| AG-02 | P2 | Verified defect | Usage of timed-out / crashed / interrupted turns missing from task and attempt | Fixed |
| AG-03 | P2 | Verified defect | Claude Code prompt-cache tokens ignored (input undercounted) | Fixed (shape per Anthropic schema; live run pending) |
| ST-01 | P1 | Verified defect | No schema versioning: `0001_init.sql` re-run on every start | Fixed |
| ST-02 | P3 | Verified defect | A pooled connection that predates a migration misreads altered tables | Fixed |
| RV-01 | P1 | Verified defect | Every review hardcoded `ci_status=passed`, `mergeable=true`, `has_conflicts=false` | Fixed |
| RV-02 | P2 | Verified defect | Attempt count shown as the pull request number | Fixed |
| RV-03 | P1 | Gap | The product never creates a pull request (`pr_number`/`pr_url` never set) | Fixed |
| RV-04 | P2 | Gap | After a pull request exists, its CI status is never refreshed | Fixed (manual refresh; automatic refresh of open pull requests since the follow-up) |
| RV-05 | P1 | Verified defect | `fetch_pull_request_details`: no check run reported as `passed`, `mergeable: null` as mergeable | Fixed |
| DOC-02 | P3 | Verified defect | SECURITY.md documented a `require_human_approval_for_merge` setting and an "Approve & Enable Merge" button that do not exist | Fixed |
| SEC-01 | P1 | Verified defect | `github_api` / `linear_graphql` forward any request with the host's tokens | Fixed |
| SEC-02 | P2 | Verified defect | Credential check on the review diff: 2 patterns, 32 KB preview only | Fixed |
| BK-01 | P1 | Gap | No backup / restore | Fixed |
| SEC-03 | P2 | Gap | `linear_graphql` cannot be limited to the configured Linear project (workspace-wide key) | Fixed for mutations; reads open |
| SEC-04 | P1 | Verified defect (in this branch's first cut) | Create-PR sent the GitHub token to any HTTPS `origin` | Fixed |
| SEC-05 | P2 | Verified defect (in this branch's first cut) | `github_api` followed redirects with the host's credentials | Fixed |
| RV-06 | P1 | Verified defect | Commits the agent made itself were invisible to the review, the credential scan and the changed-file list | Fixed |
| RV-07 | P2 | Verified defect | Reviews stored by 2.0.0 carry an invented number/CI pass and could not be published | Fixed |
| RV-08 | P2 | Verified defect | An unverified merge could still rate 5/5; a moved pull request head kept the old evidence; reopened PRs stayed "closed"; >100 check runs read as passed | Fixed |
| BK-02 | P2 | Verified defect (in this branch's first cut) | Restore rollback left files it wrote; saved files could be overwritten by a second restore in the same second; concurrent backups shared a staging directory | Fixed |
| AG-05 | P2 | Verified defect (in this branch's first cut) | A task cancelled and retried at once could resume from a baseline the old worker was about to move | Fixed |
| SEC-06 | P1 | Verified defect (in this branch's first cut) | A `pushurl`/`pushInsteadOf` or an HTTP redirect could send the push, with the token, somewhere other than the checked origin; a failing `git diff` or an unresolvable base read as "no credentials found"; the agent GitHub tool could open pull requests without the confirmation | Fixed |
| AG-06 | P2 | Verified defect (in this branch's first cut) | The stopping side (cancel, stall) rewrote the whole task row and could overwrite usage the worker had just settled; a refused retry left the task queued; a fallback worktree branch name was not the stored task branch | Fixed |
| BK-03 | P2 | Verified defect (in this branch's first cut) | Restore kept a workflow created after the backup, reported "put back" when rolling back failed, and backups were offered for a custom `--db-path` that restore cannot target | Fixed |
| REL-01 | P2 | Gap | Release has no SBOM and no source-commit trace | Fixed |
| REL-02 | P1 | Gap | No updater / update signing | Implemented; needs the signing secret and a Windows end-to-end check |
| REL-03 | P1 | Gap | Authenticode certificate not configured | Blocked (owner) |
| ID-01 | P2 | Verified defect | App identifier `com.openai.symphony.*` implies OpenAI ownership | Fixed |
| DOC-01 | P3 | Verified defect | `WORKFLOW.md` names "Symphony Desktop 1.0", port 47800 | Fixed |
| SCH-01 | P0 | Verified (negative) | Concurrent ticks / manual dispatch can start a task twice | **No defect** (stress test added) |
| WIN-01 | P1 | Unverified | All native Windows behaviour (see §4) | Blocked (Windows) |
| PROV-01 | P2 | Gap | Provider profiles / compatible endpoints / health model | Fixed as scoped (endpoints via OpenCode, live model catalogues); named profiles and automatic fallback decided against for now |
| HIST-01 | P2 | Gap | Durable session history: pagination and retention | Fixed (retention, cursor pages, continue a run, tool-call history) |
| PERF-01 | P2 | Unverified | Idle CPU / memory on the reference laptop | Blocked (hardware) |
| AG-04 | P2 | Verified defect | Claude Code / OpenCode run headless without an approval path | Fixed |

---

## 1. Verified defects, fixed on this branch

### SC-01 — Yanked `tokio 1.54.0` (P1)

- **Where**: `Cargo.lock` (`tokio 1.54.0`); policy `deny.toml` (`yanked = "deny"`).
- **Evidence**: crates.io API: `1.54.0 … yanked: true` (published 2026-10-19), `1.54.1` published the day of
  the audit. After the change `cargo deny check` → `advisories ok, bans ok, licenses ok, sources ok`.
- **Expected**: no yanked crate in the lockfile.
- **Fix**: `cargo update -p tokio --precise 1.53.3`, the last release that is not yanked and has been out
  for more than a week (the repository's own practice is to prefer aged releases; `1.54.1` was hours old).
  Dependabot can raise it to 1.54.x later. No ignore entry was added.
- **Tests**: `cargo deny check` (CI: `supply-chain.yml`).

### SC-02 — `@vitest/mocker` (P1)

- **Where**: `apps/desktop`, `apps/mobile`, `packages/api-client`, `packages/ui` (`vitest ^4.1.11`).
- **Evidence**: `pnpm audit --audit-level moderate`: GHSA-qxg6-89mg-rqv8 and GHSA-xp26-3454-mjgp,
  vulnerable `>=4.1.11 <4.1.12`, patched `>=4.1.12` (4 paths).
- **Fix**: `vitest ^4.1.12` in all four packages (4.1.12 was published 2026-10-15, six days before);
  `pnpm audit` → "No known vulnerabilities found". No override, no lowered threshold.
- **Tests**: `pnpm audit` (CI), the whole frontend suite passes on the new version.

### AG-01 — Resumed Codex thread usage (P1; PR #20 thread `PRRT_kwDOVP6sDc6u8ygn`)

- **Where**: `CodexAppServerSession` (`crates/symphony-agents/src/lib.rs`), usage pump in
  `SymphonyOrchestrator::run_attempt`.
- **Evidence (real protocol)**: `codex app-server` **0.163.0** (`@openai/codex`), driven by
  `scripts/codex-resume-usage-probe.mjs` (no model call, no authentication): after `thread/resume`
  the server sends `thread/tokenUsage/updated` carrying the thread's **absolute** total
  (`tokenUsage.total`, 1500 in the probe) and the id of the thread's **previous** turn, before any new
  turn runs. The generated schema (`codex app-server generate-json-schema`) defines `total` as the
  thread total and `last` as the last response. The old code started every session at zero, so the replay
  was credited to the runtime totals and to the live session again on **every retry**; the SPEC's
  "track deltas relative to last reported totals" rule does not cover a session that starts mid-thread.
  Before the fix, 8 of the 10 new scenario tests fail.
- **Expected**: current-turn, current-attempt, current-session, previously credited and historical usage
  are distinct; retries and resumes neither duplicate nor lose usage; unknown history is marked estimated.
- **Fix**: `symphony_core::usage::ThreadUsageLedger` (baseline = usage already accounted for, replay vs
  live reports told apart by turn id, monotone totals, inconsistent baselines discarded and flagged);
  migration 0002 stores `thread_usage_*` per task and `usage_estimated` per task and attempt; the session
  ignores other threads, resets on a changed thread id and reports usage *since the previous outcome*,
  so replayed residue of a crashed turn is credited exactly once. API fields `Task.thread_usage`,
  `Task.usage_estimated`, `RunAttempt.usage_estimated`; the desktop shows "≈" for estimated totals.
- **Tests**: `symphony-core` ledger (8), `crates/symphony-agents/tests/codex_usage_resume.rs` (10: persisted
  baseline, replay before/after the resume response, unknown baseline with/without replay, changed thread
  id, other thread, failed, cancelled, crash, chained attempts), orchestrator end-to-end (4), legacy
  migration, `settings`/task UI. These use a **scripted app-server that reproduces the measured
  behaviour**; they prove Symphony's accounting, not Codex. The probe script is the opt-in real check.

### AG-02 — Usage of turns that end without an outcome (P2)

- **Where**: `run_attempt`: the `Err(_)` arm of `run_turn` and the cancel branch of its `select!`.
- **Evidence**: both paths ended the attempt without adding what the session had reported to the task or
  attempt (only the live runtime totals saw it), so task, attempt and runtime totals disagreed after a
  restart.
- **Fix**: `AgentSession::usage_snapshot()`; `settle_session_usage` adds the unreported remainder; for a
  stop from outside only the usage and thread baseline are written, atomically (`Storage::add_task_usage`),
  so a concurrent status change is never overwritten.
- **Tests**: orchestrator crash and interrupted-turn tests (the latter checks the cancellation stays).

### AG-03 — Claude prompt-cache tokens (P2)

- **Where**: `extract_token_usage`. Anthropic reports `cache_creation_input_tokens` and
  `cache_read_input_tokens` next to `input_tokens`; only the latter was read, so input was undercounted by
  the cached share (usually most of it).
- **Fix**: both are added to input; Codex's `cachedInputTokens` (a subset of `inputTokens`) is not.
- **Tests**: `claude_prompt_cache_tokens_count_as_input_but_codex_cached_tokens_do_not`. **Not verified
  against a live Claude Code session** (none available); cost estimates still use one flat rate per model.

### ST-01 / ST-02 — Schema versioning (P1 / P3)

- **Where**: `Storage::run_migrations` executed `0001_init.sql` split on `;` on every start. No version
  table, no way to add a column, a `;` in a comment would have broken it.
- **Fix**: `schema_migrations`, one transaction per migration, refusal of a database written by a newer
  build, a consistent `VACUUM INTO` snapshot (`<db>.pre-migration-v<N>-<time>.bak`) before the first
  migration of a database that holds data. Migrations run on a dedicated connection that is closed before
  the pool opens: a test showed that a pooled connection that cached the old schema panics reading rows
  of an altered table (ST-02).
- **Tests**: `crates/symphony-storage/tests/migrations.rs` (legacy 2.0.0 database upgraded without losing
  rows, snapshot content, idempotence, no snapshot for a new database, newer schema refused untouched,
  backup includes uncheckpointed WAL data) and a rolled-back failing migration.

### RV-01 / RV-02 — Review facts were invented (P1 / P2)

- **Where**: `run_attempt` step 8 (`ci_status: "passed"`, `mergeable: true`, `has_conflicts: false`,
  `pr_number: task.pr_number.unwrap_or(attempt_count)`).
- **Evidence**: no code looked at CI or the merge; `MergeReadinessReport::evaluate` therefore always wrote
  "Remote CI checks passed" and "Clean Git merge base", and the UI showed `#1` for a local worktree.
- **Fix**: `WorkspaceManager::check_mergeability` simulates the merge of the worktree state (uncommitted
  and untracked changes included) into the base with `git merge-tree --write-tree`, touching neither
  worktree nor index nor branches; `ConflictCheck {Clean, Conflicts, NotChecked}`; CI is `none` for a local
  review; `pr_number` is 0 until a pull request exists; the UI says "Local review", "No remote CI",
  "Not checked".
- **Tests**: 3 git-backed mergeability tests, 3 core scoring tests, orchestrator end-to-end assertions,
  UI (`ReviewsPage`, `MergeReadinessCard`).

### RV-03 — No pull request is ever created (P1)

- **Where**: `GitHubTracker::create_pull_request` has no caller; `approve_pr_merge` only marks the review
  and completes the task. Workflow steps 15–16 of the mission (create/inspect the pull request, approve
  the final action explicitly) were impossible.
- **Fix**: `SymphonyOrchestrator::create_pull_request` + local-only `POST /api/v1/prs/{id}/create-pull-request`
  + a confirmed "Create pull request" action: commit to the task branch (hooks and fsmonitor disabled),
  push to `origin` without force (token via git's `GIT_CONFIG_*` environment, never argv or config, scrubbed
  from errors), open the pull request, adopt an existing one on 422. Refused without a GitHub repository or
  token, with conflicts or security warnings, or while the task runs. Idempotent and serialized. **There is
  no merge call anywhere**; merging stays a human act on the Git host.
- **Tests**: real git repository + bare origin + recording GitHub mock (commit content, push, request
  body/auth, idempotence, adoption, refusals); workspace tests (hooks never run — with a control proving the
  test hook does run for plain git —, no force, token never echoed); UI (confirmation, exactly-once,
  blocked states, surfaced errors); router (auth, 404); remote scope forbids it.

### RV-04 / RV-05 — Pull request status (P2 / P1)

- **Where**: `GitHubTracker::fetch_pull_request_details` (never called before this branch).
- **Evidence**: an empty `check_runs` list became `ci_status = "passed"` (a repository without CI would
  read as green); `mergeable: null` (GitHub still computing) and a missing `mergeable_state` became
  "mergeable"/"clean"; only `failure` counted as a failed run (`cancelled`, `timed_out`, `action_required`
  all passed); only the first 30 check runs were read. Same class of defect as RV-01.
- **Fix**: no check run → `none`; unknown merge stays unknown (`mergeable: Option<bool>`); cancelled,
  timed-out, action-required, startup-failure and stale runs fail; 100 runs per page. New
  `SymphonyOrchestrator::refresh_pull_request` + local-only `POST /api/v1/prs/{id}/refresh` + a *Refresh
  status* button re-read CI, merge state, merged/closed state and review comments and re-rate the review.
  Manual only at first: no background polling (nothing runs when nobody asks). Since the follow-up the
  owner chose automatic refresh as well (see §3). A pull request merged by someone shows state `merged`;
  the task is not changed.
- **Tests**: check-run summary unit test; orchestrator test with a GitHub mock whose answers change
  (no CI + uncomputed merge → neither "passed" nor "clean"; green; red with conflicts and a lower score);
  UI refresh test; remote scope forbids it.

### SEC-01 — Agent tools forwarded any request (P1)

- **Where**: `GitHubTracker::execute_github_dynamic_tool`, `LinearTracker::execute_linear_dynamic_tool`,
  reached from `item/tool/call` with no approval gate.
- **Evidence**: the only checks were "path starts with `/`" (GitHub) and "query is non-empty" (Linear).
  An agent steered by an issue body could `PUT /repos/x/y/pulls/1/merge` (auto-merge without policy),
  `DELETE` repositories, read other private repositories the token can see, or run `issueDelete`/
  `projectDelete` mutations. The SPEC allows adapters to narrow their tools; it requires documentation of
  mutation and scope behaviour.
- **Fix**: `symphony-trackers::tool_policy`. GitHub: allowlist confined to `/repos/{owner}/{repo}/` of the
  project, GET (read), POST/PATCH for issues, comments, labels and pull requests only; no PUT/DELETE, no
  merge, no reviews, no secrets/hooks/collaborators, no encoded or relative path tricks. Linear: queries are
  read-only; mutations limited to `commentCreate`, `commentUpdate`, `issueUpdate`, `attachmentCreate`,
  `attachmentLinkGitHubPR`, `attachmentLinkURL`, resolved through a tokenizer (aliases, comments, strings,
  fragments, multiple operations). Tool descriptions advertise the scope.
- **Tests**: 5 policy tests (about 40 allowed/refused requests) and an integration test showing refused calls
  **never reach the network** while allowed ones carry the host token.

### SEC-02 — Credential scan of the change (P2)

- **Where**: `run_attempt` step 8 looked for `PRIVATE KEY` / `ghp_` in the 32 KB preview.
- **Fix**: `detect_secrets_in_added_lines` (GitHub, fine-grained, `sk-`, Linear, AWS, Slack, private key) on
  **added** lines of up to 4 MiB; a larger diff produces a warning instead of silence.
- **Tests**: `secrets_are_detected_only_on_added_lines`.

### BK-01 — Backup and restore (P1)

- Documented scope in `docs/DATA_BACKUP.md`; module `symphony_storage::backup`; `GET/POST /api/v1/backups`
  (local scope only, forbidden remotely); `symphony-cli verify-backup | restore-backup`; Settings → Backups.
- **Tests**: storage (consistent copy, atomic listing, restore keeps replaced files, damaged/newer backups
  refused untouched, rollback when the restored database cannot be opened), CLI binary (restore needs the
  runtime lock), router, generated client against the real runtime (no credential material in a backup),
  settings UI.
- **Not done**: restore from the desktop UI (needs the runtime stopped; the installer does not ship the
  CLI, it is a separate release asset), backing up the credential vault (DPAPI-bound by design).
  Automatic scheduled backups with retention were added in the follow-up (`backup_interval_hours`,
  default daily; `backup_keep`, default 7, deletes only automatic backups), see `docs/DATA_BACKUP.md`.

### DOC-02 — Documentation promised controls that did not exist (P3)

`SECURITY.md` §3 described `require_human_approval_for_merge: true` and an "Approve & Enable Merge" button.
Neither exists in the code or the UI (`grep` finds the setting only in that sentence). There is no merge
operation at all; §3 now says exactly that and documents the real controls.

### Findings from the automated review of this PR (SEC-04, SEC-05, RV-06 – RV-08, BK-02, AG-05)

Bots reviewing PR #21 raised these; each was checked against the code before acting (two claims were
wrong or out of proportion and are answered on their threads, see below). Verified and fixed, with tests:

- **SEC-04** `push_branch` took the credential scope from whatever `origin` was. Now the GitHub token is
  only sent to an `https://` origin whose host is github.com (or the configured Enterprise API host) **and**
  whose path is the project's `owner/name`; any other HTTPS origin is refused; ssh/local origins use the
  user's own git credentials. The per-URL scope keeps an explicit port. The worktree is re-scanned and
  re-checked for conflicts immediately before publishing (the review is a snapshot).
- **SEC-06** `push_branch` refuses an `origin` with a separate or additional push URL and any repository-local
  `http.*` redirect/header/proxy, `url.*.insteadOf` or `credential.*` setting, and sets
  `http.followRedirects=false` for the push; `diff_since_checked` turns a failing `git diff`/`ls-files` into a warning that blocks
  publishing, as does an unresolvable base commit; the `github_api` policy no longer allows `POST /pulls`
  (publishing is the confirmed "Create pull request" action). The base branch used for the merge check and
  the review is `origin/<base>` when it exists (what the pull request merges into), else the local branch.
  Residual: git clean filters still run on staging (`SECURITY.md`).
- **AG-06** `Storage::update_task_lifecycle` writes only lifecycle columns; `cancel_task` and
  `schedule_retry` use it so settled usage is never overwritten. `retry_task` refuses a task that is still
  stopping before it persists anything and restores the previous state if dispatch refuses.
  `prepare_workspace` returns the branch the worktree is really on.
- **BK-03** restore moves aside a live `WORKFLOW.md` that the backup does not contain and reports every
  failed rollback step with the path that still holds the data; backups are not offered with `--db-path`.
- **SEC-05** the agent-facing GitHub client no longer follows redirects (a redirecting answer cannot take
  the host's token to another host).
- **RV-06** the review, the changed-file list and the credential scan looked at `git diff HEAD` only; an
  agent that commits its work showed no changes and no warnings, yet that commit gets published. They now
  use the diff against the commit where the branch left the base (`git merge-base`), committed plus
  uncommitted plus untracked.
- **RV-07** `PullRequestReview::has_pull_request()` requires a web URL, and reviews written by 2.0.0 are
  normalised on read (`ci_status passed → none`, merge unverified, readiness re-rated).
- **RV-08** unknown mergeability costs a point (never "Ready"); `reviewed_commit` records what Symphony
  pushed and a refresh flags a head that moved; a reopened pull request is open again; a check-run list
  larger than what was read is `unknown`; an adopted pull request must target the requested base.
- **BK-02** restore rollback removes every file the restore wrote (also when nothing was there before);
  replaced files get a name that never exists yet (`…-2`); a backup's staging directory is reserved by
  creating it.
- **AG-05** a task whose worker was told to stop is not dispatchable until that worker has settled.
- Smaller: the credential scan skipped added lines beginning with `++`; the real-Codex probe passed
  `OPENAI_API_KEY` & co. through (it now scrubs the environment so it can never make a billable call);
  the npm SBOM is scoped to the shipped desktop app.

Not changed at first: (a) **SEC-03** `linear_graphql` uses a workspace-wide key. Fixed for mutations in the
follow-up: the targets of every allowed mutation (`id`/`issueId` literals and variables, nested inputs
included) are resolved through `issue(id)`/`comment(id)` and must belong to the configured project's
`slugId`; moving an issue (`projectId`/`teamId`/`projectMilestoneId`) and mutations whose targets cannot be
read are refused before anything is sent (tests: `tool_policy` target extraction,
`linear_mutations_change_only_issues_of_the_configured_project`). Reads stay workspace-wide; `SECURITY.md`
says so.
(b) Claude/OpenCode report usage per finished invocation, so a killed invocation reports nothing (already
listed). (c) Pull request status was refreshed on request only (a timer was rejected on purpose at
the time; the owner later chose an automatic refresh, see §3).

### REL-01, ID-01, DOC-01

- Release workflow publishes CycloneDX SBOMs (npm production dependencies; both executables' Rust graph) and
  `BUILD-INFO.txt` (commit, tag, run URL, signed flag), all covered by `SHA256SUMS.txt`. **Not exercised
  here** (needs the Windows release runner).
- Identifier `io.github.kb01111.symphony-desktop` / `-companion` before the first public release (2.0.0 is
  unreleased and untagged). Consequence on existing dev installs: WebView2 profile and window-state file
  move (UI preference and window geometry reset); the data directory (`%LOCALAPPDATA%\symphony-desktop`),
  vault, autostart value and installer product name do not depend on the identifier. Changing it after
  release would orphan WebView data and create a second install entry, so it had to happen now.
- `WORKFLOW.md`: no "Symphony Desktop 1.0", port agrees with the CLI default.

---

## 2. Checked and found correct

- **SCH-01 Single scheduler / idempotent dispatch**: `dispatch_lock` plus the `running` map. A stress test
  starts 16 concurrent manual dispatches and scheduler ticks for one queued task: exactly one attempt and
  one `run_attempts` row, over 5 repeated runs. PR #20's retry and manual-move races have their own tests.
- **Workspace path validation** (`validate_workspace_path`): reviewed, canonicalizes root and target and
  rejects symlink escapes; unchanged (safety invariant).
- **Provider capabilities are typed and honest**: Claude Code and OpenCode report `supports_approvals = true`
  since the follow-up (AG-04); neither supports dynamic tools.
- **Remote scope**: the existing allowlist test fails on unclassified endpoints; the new endpoints
  (`/backups`, `/prs/{id}/create-pull-request`, `approve-merge`) were added to the forbidden-remotely list.
- **Playwright** (`pnpm --filter @symphony/desktop e2e`): 30 passed against the real `symphony-cli` with the
  simulated agent. Screenshots and perf JSON that the run rewrites were not committed (different host).

## 3. Open

- **Pull request status** (see RV-04): *Refresh status* re-reads on request, and since the follow-up the
  owner chose (2026-10-28) automatic refresh too: every `pr_refresh_interval_minutes` (default 15, 5–1440,
  `0` = only on request) the runtime refreshes each open pull request (not merged or closed) one after
  another, only with a GitHub token, and announces a change (`pr/status_changed`) or a failing refresh
  (`pr/refresh_failed`, once until it recovers). Legacy commit statuses are read since the earlier
  follow-up (combined with check runs; any failure fails, an unreadable signal is unknown).
- **REL-02 (P1)**: in-app updates are implemented (`tauri-plugin-updater` driven by host-side commands,
  Settings card and banner, `latest.json` + `.sig` produced by the release workflow). Still needed from the
  owner: the `TAURI_SIGNING_PRIVATE_KEY` / `_PASSWORD` secrets, and a real two-release test on Windows
  (install v-old, publish v-new, update, data intact). Details: `docs/RELEASING.md`.
- **REL-03 (P1, Blocked)**: Authenticode secrets (`WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD`);
  unsigned builds are labelled `UNSIGNED`; the workflow uploads them to a *draft* release and warns maintainers not to publish them, but nothing blocks publication (no pre-publication signing gate exists).
- **PROV-01 (P2)**: provider = per project and per task (`preferred_provider`/`model`, task overrides) with
  CLI detection (`detect_provider_statuses`). Missing: multiple named profiles, runtime arguments, compatible
  endpoints (OpenRouter, OpenCode Zen/Go, local), default/fallback policy, rate-limit/cost metadata. Each
  needs an adapter that provides tool execution, approvals, streaming and sessions; a raw chat endpoint must
  not be advertised as an autonomous coding runtime. Since the follow-up, compatible endpoints are reached
  through OpenCode, which is such an adapter: whatever providers the user configures in OpenCode
  (OpenRouter, OpenCode Zen, local models) are listed by detection (`opencode models`), selectable per
  project or task as `provider/model`, run with Symphony's approvals (AG-04) and report usage.
  **Decided by the owner (2026-10-25):** no multiple named profiles per provider for now (one profile per
  provider; the model is chosen per project or task, other endpoints go through OpenCode), and **no automatic
  fallback**: a task whose provider is missing, signed out or rate limited fails visibly, and the operator
  picks another provider for the retry. This avoids unexpected cost and work by an agent nobody chose.
  Revisit both if users ask for them.
- **HIST-01 (P2)**: events, attempts (thread/turn/session ids) and approvals are persisted, and
  `/events/recent` and `/tasks/{id}/events` take `limit`. Retention is implemented (`event_retention_days`, pruning of
  finished tasks' events). Cursor pagination of a task's events (`before=<event id>`) and a
  *Load older events* control were added in the follow-up, and so was *Continue…* on a run that ended
  (`POST /api/v1/tasks/{id}/continue`, local only): the next attempt resumes the agent's thread in the same
  worktree with the operator's instructions as its prompt (stored until delivered, so they survive a
  restart; a continued task with a pull request gets *Push changes to #N*). Tool-call history was added in
  the follow-up: every tool call (Codex commands, file changes and MCP tools, the host-side `github_api` /
  `linear_graphql` with the answer Symphony gave, Claude Code `tool_use`/`tool_result`, OpenCode tool
  parts) is one row of `tool_calls` (migration 0004) with redacted arguments and output (capped at
  64 KiB), served newest first by the local-only `GET /api/v1/tasks/{id}/tool-calls` and shown in the
  run view's *Tool calls* tab; it follows the activity-log retention. Found on the way: raw event
  payloads were stored and streamed unredacted (only the message was redacted); every string of a payload
  is now redacted before it is stored or broadcast.
- **AG-04 (P2)**: verified against the real `claude` 2.1.310 with a scripted Messages API (no model, no
  key): a headless `claude -p` without a host **silently denies** every tool use its own rules would ask
  about (`permission_denied`) and reports success. Fixed for Claude Code in the follow-up: the driver runs
  it as a host (`--input-format stream-json --permission-prompt-tool stdio`) and answers each
  `can_use_tool` request through Symphony's approval gate (accept, accept for the session — remembered per
  exact tool input, and Claude's own rule suggestions passed on only with the `session` destination, so
  nothing is written to the user's or the repository's settings —, decline, abort = deny + interrupt).
  With `approval_policy: never` nobody is asked and such a request stays denied (Claude Code has no sandbox,
  so auto-approving would widen what a run may do). End to end through `symphony-cli`: with `on-request` the
  Bash command appears as a pending approval and the file is written only after *Approve*; with *Decline*
  or `never` it is not. Opt-in re-check: `node scripts/claude-permission-probe.mjs`.
  **OpenCode** (verified against the real `opencode` 1.19.4 the same way): its default permissions let a
  headless `opencode run` execute `bash` and edit files **without asking**, and with `ask` rules both
  `opencode run` and `run --attach` reject every request automatically. Fixed in the follow-up: under any
  policy other than `never` the driver starts its own `opencode serve` for the turn (loopback, a free port
  chosen by Symphony because `--port 0` means the fixed 4096, a random password, `ask` for bash, edit,
  webfetch and external directories), creates or resumes the session, sends the prompt
  (`prompt_async`), answers `permission.asked` through the approval gate (`once`/`always`/`reject`,
  abort also aborts the session) and ends the turn at `session.idle`; the server's process tree ends with
  the turn. End to end through `symphony-cli`: the command is a pending approval, the file is written
  only after *Approve*. Under `never` the driver keeps `opencode run`, so the user's own OpenCode
  permissions decide, as before (`SECURITY.md`).
  Found on the way: `opencode run --format json` reports tokens per step (`part.tokens`), which the
  driver never read, so OpenCode usage and cost were always 0. Both modes now sum every step (cache
  traffic as input, reasoning as output).
- **Claude/OpenCode usage is exact only per invocation**; a killed invocation reports nothing: a turn that
  times out or is cancelled after the CLI printed usage but before it finished is not in the settlement, and
  the task is not marked estimated for it. Fixing it needs a live run of both CLIs.
- **Codex resume without a persisted baseline** (tasks stored by 2.0.0 only): if a live usage notification
  arrived before the `turn/start` response it would be taken as the replay and undercount that turn once.
  Codex answers `turn/start` first (real-binary probe), and new tasks always persist a baseline.
- **Generic `usage` maps**: fixed in the follow-up. Codex usage is read only from absolute totals
  (`tokenUsage.total`, `total_token_usage`); Claude Code only from the `usage` of its final `result` line
  (an `assistant` line's usage is one API response); OpenCode only from its per-step `part.tokens`. A generic
  `usage` map is never taken as a cumulative total (SPEC 13.5), test
  `generic_usage_maps_are_never_read_as_cumulative_totals`.
- **Provider model lists**: fixed in the follow-up. Detection reads the installed CLI's catalogue
  (Codex `model/list`, verified against codex-cli 0.164.0; `opencode models`, verified against
  opencode 1.19.4); the static lists are only a fallback, and the old default `gpt-5-codex` (no longer
  offered by Codex 0.164.0) is no longer the settings default nor passed to a CLI that does not list it.

## 4. Unverified (needs Windows 11 hardware or a signed build)

Application startup and embedded/attach modes, single instance, native folder picker, Explorer integration,
tray and close-to-tray, notifications, login autostart, window-state and multi-monitor/DPI, WebView2
availability, shutdown and crash recovery of the packaged app, NSIS install/uninstall/upgrade, DPAPI and
ACL behaviour on a real profile, Job Object cleanup. Existing coverage: `ci.yml` (windows-latest unit and
native tests), `desktop-native.yml` + `scripts/windows-native-smoke.ps1`, and the manual checklist in
`docs/WINDOWS_NATIVE_SMOKE_TEST.md`. This audit did **not** run any of them.

Performance (PERF-01): no idle CPU/RAM/startup figures for the reference laptop (Ryzen AI 9 HX 370,
RTX 4070, hybrid graphics). `docs/perf/perf-results.json` measures the web UI in headless Chromium only.
Code review of `HardwareMonitor` found no GPU polling, but "does not wake the discrete GPU" is an
assertion that needs a measurement on that machine.

## 5. Comparison with the OpenAI Symphony specification (`SPEC.md`, Draft v1)

- §13.5 token accounting: absolute totals preferred, `last_token_usage` ignored, deltas against the last
  report — implemented; the delta baseline across a resumed thread is **not specified** and is now covered
  by the ledger (AG-01). Generic `usage` maps: see §3.
- §10 agent tools: tools execute host-side without exposing tracker tokens — implemented; the SPEC leaves
  scope to the adapter and requires it to be documented: now enforced and documented (SEC-01).
- Retry/backoff, stall detection, reconciliation and workspace safety: covered by PR #20 tests; not
  re-derived from the SPEC here.
