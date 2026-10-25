# Production Release Report

Branch `claude/affectionate-mendel-j3qkyb` ([PR #21](https://github.com/KB01111/OpenAI-Symphony-Rust/pull/21)),
based on `main` `2840ddd2` (PR #20). Findings, evidence and per-item status: [`PRODUCTION_GAP_AUDIT.md`](./PRODUCTION_GAP_AUDIT.md).

**Verdict: code-complete for the items below, not release-ready.** The release blockers (§9) need things
this work could not do: Windows 11 hardware, an Authenticode certificate, an update-signing key and hosting,
and the maintainer's manual checklist. A green pipeline is necessary and not sufficient.

## 1. Initial findings

The repository was already in good shape structurally (single Rust scheduler, DPAPI vault, scoped remote
listener, generated contract, Windows CI and native smoke test). The audit nevertheless found, with evidence:

- the lockfile pinned a **yanked** `tokio` and `pnpm audit` reported two `@vitest/mocker` advisories;
- a resumed Codex thread's usage was **charged again on every retry** (shown against a real `codex app-server`);
- the SQLite layer had **no migration system**;
- every review **invented** its CI result, mergeability and pull request number; no pull request was ever
  created; the pull-request status reader treated "no CI" and "not computed yet" as success;
- the host-side agent tools forwarded **any** GitHub/Linear request with the host's tokens (merge, delete,
  other repositories);
- there was no backup, no SBOM, a third-party-looking app identifier, and a documented merge control that
  does not exist.

## 2. Verified defects fixed

SC-01, SC-02, AG-01, AG-02, AG-03, ST-01, ST-02, RV-01, RV-02, RV-04, RV-05, SEC-01, SEC-02, ID-01, DOC-01,
DOC-02 (see the audit for each: location, evidence, expected behaviour, fix, tests).

## 3. Features implemented

- **Exact usage accounting across resumed threads** with an explicit *estimated* marker (`ThreadUsageLedger`).
- **Versioned SQLite migrations** with pre-migration snapshot; **backups** (`VACUUM INTO`, manifest,
  verification) and **offline restore** that keeps and, on failure, restores the replaced files.
- **Truthful merge readiness**: simulated merge, "No remote CI", "Not checked".
- **Create pull request** (commit → push → open; confirmed, idempotent, never merges) and **Refresh status**.
- **Scoped agent tools** (allowlists for `github_api` / `linear_graphql`).
- Credential scan of added diff lines; Claude prompt-cache tokens; CycloneDX SBOMs and `BUILD-INFO.txt` in
  releases; project-owned identifiers; `WORKFLOW.md` and documentation corrections.

## 4. What was not done

| Item | Why |
| --- | --- |
| Tauri updater (`REL-02`) | Needs an update-signing key pair and a hosted HTTPS manifest, and a Windows build to verify; no unverified updater code was merged. Plan: [`RELEASING.md`](./RELEASING.md). |
| Authenticode signing (`REL-03`) | Certificate and password are repository secrets only the owner can add. The workflow signs when present and otherwise labels assets `UNSIGNED`. |
| Provider profiles / compatible endpoints (`PROV-01`) | Compatible endpoints are reached through OpenCode (follow-up). Named profiles and automatic fallback were decided against for now (owner decision 2026-10-25, see the audit). |
| Durable-history pagination and retention (`HIST-01`) | Done in the follow-up: retention, cursor pagination (`before=<event id>`) and *Load older events*. |
| Claude/OpenCode approval path (`AG-04`) | Done in the follow-up, checked against the real `claude` and `opencode` CLIs (no model call). |
| All native Windows behaviour, installer, WebView2, tray, DPAPI on a real profile, autostart, multi-monitor, DPI | No Windows machine in the environment. |
| Performance on the reference laptop | Same. No idle CPU/RAM/startup numbers exist for it. |
| A full UI/accessibility audit pass | Only the screens touched here were reviewed; the 30 Playwright flows pass. |

## 5. Files and modules changed

78 files, +6,960/−228 (generated client and lockfiles included).

| Area | Files |
| --- | --- |
| Core | `symphony-core/src/usage.rs` (new), `domain.rs` (`ThreadUsageMark`, `usage_estimated`, `ConflictCheck`, `evaluate`), `redaction.rs` (`detect_secrets_in_added_lines`) |
| Storage | `symphony-storage/src/lib.rs` (migration runner, `backup_to`, `add_task_usage`), `backup.rs` (new), `migrations/0002_usage_accounting.sql` (new) |
| Agents | `symphony-agents/src/lib.rs` (Codex ledger, `usage_snapshot`, cache tokens) |
| Orchestrator | `symphony-orchestrator/src/lib.rs` (usage settlement, merge check, secret scan, `create_pull_request`, `refresh_pull_request`) |
| Workspaces | `symphony-workspaces/src/lib.rs` (`check_mergeability`, `commit_changes`, `push_branch`) |
| Trackers | `symphony-trackers/src/lib.rs` (PR open/find/status), `tool_policy.rs` (new) |
| Server / CLI | `symphony-server/src/lib.rs` (`/backups`, `/prs/{id}/create-pull-request`, `/prs/{id}/refresh`), `symphony-cli/src/main.rs` (`verify-backup`, `restore-backup`) |
| Contract | `openapi/symphony-api.json`, `packages/shared-types/src/generated/**` (regenerated, additive) |
| Frontend | reviews (local review, create PR, refresh), settings (Backups), task estimated marker, `packages/ui` readiness card, EN/SV strings |
| Release / config | `.github/workflows/release.yml`, `tauri.conf.json`, `tauri.ios.conf.json`, `WORKFLOW.md`, `Cargo.lock`, `pnpm-lock.yaml` |
| Docs | this report, `PRODUCTION_GAP_AUDIT.md`, `DATA_BACKUP.md`, `RELEASING.md`, `SECURITY.md`, `ARCHITECTURE.md`, `CONTRIBUTING.md`, `AGENTS.md`, `README.md`, `CHANGELOG.md`, `PRODUCTION_READINESS.md`, `WINDOWS_NATIVE_SMOKE_TEST.md` |

## 6. Tests executed and their actual results

Environment: Linux container, Rust 1.97.0, Node 22.22, pnpm 11.15, git 2.43, Chromium for Playwright; the
Tauri crate was built and linted with GTK/WebKit dev packages installed. **Nothing below ran on Windows.**

**Final local run (Linux, after the review-round commits):**

- `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -D warnings` (including the Tauri
  crate), `cargo deny check` and `pnpm audit`: clean.
- `cargo test --workspace --locked --no-fail-fast`: every suite passed (core 24, storage 18+, agents 5 + 9
  lifecycle + 10 resume + 4 usage, orchestrator 21, trackers, workspaces 16, server 7 + 9 router + 5 contract
  + 16 remote-listener, CLI 2 backup/restore + 3 lifecycle, platform 37) except one test, see below.
- `pnpm typecheck`, `pnpm test` (desktop 86, `@symphony/ui` 10, API client 18 + 21 against the real
  `symphony-cli`, mobile 15, scripts 26) and `pnpm build`: passing. Playwright (30 flows) passed in an earlier
  run on the same UI code and was not repeated after the last fixture-only change.

What that run caught, all fixed before this report was finalised:

- the committed OpenAPI document and generated client were **stale** (`reviewed_commit` was added to the Rust
  type without regenerating); `openapi_document_is_up_to_date` failed and `pnpm generate:api` produced a diff.
  Regenerated, and a second regeneration is a no-op;
- two UI test fixtures lacked the new required field and failed `pnpm typecheck`;
- `symphony-platform` `test_single_instance_lock` failed once under full-workspace load (re-acquiring the lock
  right after the guard was dropped). It passed in isolation and on rerun. The cause was not proven; the
  hypothesis is a child process forked by a sibling test holding the inherited lock until it execs. The test
  now retries for up to five seconds and fails with a clear message otherwise. Production code was not changed.

A second review round (push URL and redirects, fail-closed credential scan, usage overwrite on cancel, retry
while stopping, restore edge cases, no PR creation through the agent tool; audit SEC-06, AG-06, BK-03) was
followed by `cargo test --workspace --locked --no-fail-fast` (no failures), clippy with `-D warnings`,
`cargo fmt --check`, `pnpm generate:api` (no diff), `pnpm typecheck` and `pnpm test` (desktop 87, ui 10,
API client 18 + 21, mobile 15, scripts 26), all run locally on Linux. `pnpm build`, `cargo deny`,
`pnpm audit` and Playwright were not repeated after that round.

The exact final head has not had one uninterrupted green end-to-end run, because the failures above stopped
the first pass and the fixes were verified per component. The authoritative record is the PR's CI.

Which tests simulate and which use real software:

| Test | Uses |
| --- | --- |
| `codex_usage_resume.rs`, orchestrator usage tests | A **scripted app-server** reproducing the replay behaviour measured on the real binary. They prove Symphony's accounting, not Codex. |
| `scripts/codex-resume-usage-probe.mjs` | The **real** `codex app-server` 0.163.0 (opt-in; no authentication, no model call). Run once for this report: replay observed. |
| `tool_policy`, `agent_tool_scope.rs`, PR creation/refresh tests | Real `git` (repositories, bare origin, `merge-tree`), a **recording mock** of the GitHub API. No real GitHub call was made. |
| Playwright, API-client integration tests | The real `symphony-cli` runtime with the **simulated agent**. No real Codex/Claude/OpenCode run, no paid usage. |
| Real-provider tests | None exist in CI by design; the probe above is the only opt-in real check. |

## 7. Security findings and mitigations

- **SEC-01 (P1)** agent tools unscoped → allowlist, refused calls never reach the network.
- **RV-03 / publishing**: a new outbound capability (push + PR) was added with: explicit confirmation, no
  force, no merge, hooks and fsmonitor disabled, token only via git's environment and scrubbed from errors,
  refusal on security warnings/conflicts, local-scope endpoint only (tested to be forbidden remotely).
- **SEC-02** credential scan strengthened; **BK-01** backups exclude the DPAPI vault by design.
- Dependency checks clean (`cargo deny`, `pnpm audit`).
- Not changed and not re-audited: vault, DACLs, runtime discovery, remote listener (covered by existing
  tests). Git worktrees remain **not** an OS sandbox (`SECURITY.md` §1.4); untrusted agent code is contained
  only by the provider's own sandbox.

## 8. Performance measurements

None new. `docs/perf/perf-results.json` (headless Chromium, Vite dev server, simulated agent) was not
re-committed: a rerun here measured a different host. The new work adds no timers or polling: pull request
status is refreshed only on request, backups only on request, and `git merge-tree` runs once per finished run.
Idle CPU/RAM/startup on the reference laptop and GPU-wake behaviour are unmeasured (§4).

## 9. External actions required before release

1. Add `WINDOWS_CERTIFICATE` and `WINDOWS_CERTIFICATE_PASSWORD` (Authenticode).
2. Decide updater hosting, create the update key pair outside the repository, then implement and test the
   updater per [`RELEASING.md`](./RELEASING.md) (or release without auto-update and say so).
3. Run the Windows build, the native smoke test and the **full manual checklist** (now 18 steps, including
   backup/restore, create pull request and upgrade-keeps-data) on Windows 11, ideally also on the reference laptop.
4. Date the `2.0.0` changelog section, tag, review the draft release, confirm `SHA256SUMS.txt`, SBOMs and
   `BUILD-INFO.txt`, publish.
5. Enable branch protection for the five workflows; consider GitHub Enterprise `api_url` support if needed.
6. Decide whether the repository name should keep "OpenAI". (`AG-04`, `HIST-01` and `PROV-01` are decided;
   see the audit.)

## 10. Final release readiness assessment

| Gate (mission §15) | State |
| --- | --- |
| Rust format, Clippy (no warnings), tests | Passing locally (Linux); Windows CI is the authority — see PR checks |
| TypeScript, frontend tests, Playwright, API contract | Passing locally; contract regenerated deterministically |
| Dependency security | `cargo deny` and `pnpm audit` clean |
| Windows Tauri/NSIS build, native smoke test | **Not run here** (CI `desktop-native.yml` / `release.yml`) |
| Migration compatibility | Tested for a 2.0.0-era database and for failure/rollback |
| Release artifact integrity (checksums, SBOM, commit trace) | Implemented, **unexercised** until a release run |
| Update signatures | **Blocked** (no updater, no key) |
| Windows code signing | **Blocked** (no certificate) |
| Manual Windows checks | **Open** |
| Remaining limitations documented | Yes: audit §3–§4 |

**Code completion**: the scoped items are complete, tested and documented. **Release readiness**: no.
Do not tag `v2.0.0` before items 1, 3 and 4 above; item 2 is a decision, not an option to skip silently.
