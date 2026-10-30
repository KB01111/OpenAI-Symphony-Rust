# Repository Knowledge Base

The Knowledge Base (`/knowledge` in the desktop app, crate `symphony-knowledge`) is a per-repository
intelligence view that joins four things:

| Area | What it shows | Where it comes from |
| --- | --- | --- |
| **Worktrees** | Every git worktree of the repository: branch, HEAD, uncommitted files, ahead-of-default count, and the Symphony task/agent that owns it | `git worktree list --porcelain`, `git status`, joined to Symphony tasks by path |
| **Commit timeline** | One row per commit across *all* worktrees, with the worktrees that contain it, unpushed state, refs and uncommitted work | one `git log` over every worktree HEAD |
| **Code graph** | Modules, files, types, functions and methods, and who calls/imports/references whom | Graphify (`graph.json`), imported into `knowledge.db` |
| **Commit impact + AI** | Select a commit: changed symbols, what depends on them, an AI review that cites its evidence, suggested tests, and repository-aware chat | git hunks + Graphify + the project's coding-agent provider |

Nothing on the page is invented. Every node and edge is read from a repository by an engine; every
commit is read from git; every AI finding cites evidence that was in the prompt. Where something is
inferred rather than read, the data carries it (`confidence`, `precision`) and the UI says so.

## Architecture

```text
 git (read-only) ───────────────┐
                                ▼
 snapshot of a commit ──► Graphify ──► graph.json ──► import ──► knowledge.db (SQLite, FTS5)
 (Symphony-owned dir)        (offline, separate process)              │
                                                                       ▼
 commit hunks (git) ─► changed files at C and C^ ─► Graphify ─► symbols ─► impact (reverse walk)
                                                                       │
 evidence (diff, symbols, dependents) ─► provider, no tools ─► validated JSON ─► analysis cache
```

* `crates/symphony-knowledge`: `git` (read-only git), `timeline` (worktrees and the deduplicated
  timeline), `snapshot`, `engine` (Graphify adapter, `graph.json` importer), `graph` (in-memory views and
  the reverse dependency walk), `impact`, `store` (`knowledge.db`), `analysis` (prompts and validation),
  `serena`, `watch`, and `service` (`KnowledgeService`, what the API calls).
* Dependency direction avoids a cycle: `knowledge <- orchestrator <- server`. The orchestrator
  (`crates/symphony-orchestrator/src/knowledge.rs`) hosts the service and implements `AnalysisRunner`
  because it already owns provider commands, vault credentials and token accounting; the host (desktop or
  CLI) turns it on with `enable_knowledge(data_dir)`.
* API: `/api/v1/projects/{id}/knowledge/...` plus `/api/v1/knowledge/engines`. All local-only (none is in
  `REMOTE_OPERATIONS`). UI: `apps/desktop/src/features/knowledge`.

## Engines

Symphony never installs anything. `GET /knowledge/engines` reports what is present and the command to
install the rest; the Overview tab shows it with a copy button.

### Graphify (required for the graph)

Verified against **graphifyy 0.9.97** (`uv tool install graphifyy==0.9.97`, Python ≥ 3.10, Apache-2.0/MIT).
It is run as

```text
graphify extract <snapshot> --code-only --no-cluster --out <knowledge>/engine/<project>
```

with `GRAPHIFY_BACKEND=none`, every LLM/API key removed from its environment, and the secret variables
Symphony strips from all subprocesses. It parses with tree-sitter, offline. Output is
`<out>/graphify-out/graph.json`. What the importer relies on (and ignores everything else):

* top level: `nodes`, `edges` (or `links`);
* node: `id`, `label`, `file_type` (`code` | `concept` | `rationale`), `source_file`, `source_location`
  (`"L601"`, **a start line only, no end line**), optional `type` (`package` | `module` | `external`);
* edge: `source`, `target`, `relation`, `confidence` (`EXTRACTED` | `INFERRED`, `AMBIGUOUS` is documented),
  `source_location` (the line of the call/import site);
* relations seen on this repository: `contains`, `method`, `calls`, `indirect_call`, `imports`,
  `imports_from`, `references`, `re_exports`, `extends`, `implements`, `inherits`, `depends_on`,
  `declares`, `defines`, `case_of`, `dynamic_import`.

There is no `schema_version` in this output, so the index records the Graphify version, and a changed
shape fails the run with a readable error instead of being guessed at. Classification of a node into
`file` / `type` / `function` / `method` / `dependency` / `package` is derived from those fields (see
`engine::import_graph`); `rationale` comments are dropped; configuration keys (YAML, JSON, TOML) are kept
as `other` and hidden from symbol views.

### Serena (optional, precise references)

Verified against **Serena 1.28** (`uv tool install -p 3.13 serena-agent && serena init`). Serena is
**GPL-3.0-or-later** (only its SolidLSP library is MIT) and Symphony is MIT, so Serena is only ever run
as a **separate process the user installed**: it is not bundled, linked or redistributed, and the app
never installs it.

* Spoken to over MCP's stdio transport (one JSON-RPC message per line), in a persistent session per
  project that is closed after 10 idle minutes.
* Read-only twice over: the mode file excludes every editing, shell, memory and file-reading tool
  (`serena::EXCLUDED_TOOLS`, so the server does not offer them), only `find_symbol`,
  `find_referencing_symbols`, `get_symbols_overview`, `find_declaration` and `find_implementations` are ever
  called, and a server that offers an excluded tool anyway is refused.
* Works in its **own copy** of the indexed commit (`<knowledge>/serena/<project>`) because it writes
  `.serena/` next to the code it reads. A repository's own `.serena/` directory is never copied.
* Its lines are 0-based and its ranges include doc comments; `find_symbol` returns the exact range
  (`body_location`), `find_referencing_symbols` returns the containing symbols with `>line:` markers.
* Cost: the first request per indexed commit loads the language server (on this repository rust-analyzer
  needed 25 to 100 s for its first answer and created a ~300 MB `target/` directory in Serena's copy).
  Hence "on request", and a copy of its own that can be deleted freely.

Overrides: `SYMPHONY_GRAPHIFY` and `SYMPHONY_SERENA` name an executable. Otherwise `PATH` is searched, then
the places user-level installers use (`~/.local/bin`, `%APPDATA%\Python\*\Scripts`, `uv`'s bin dirs), because
an app started from the Start menu does not always inherit the shell's `PATH`.

## Why engines never see a user's repository

Both tools write caches into the directory they analyse (`graphify-out/`, `.serena/`). Run inside a
checkout they would dirty it and show up as uncommitted changes. They only ever run on
Symphony-owned directories under `<data dir>/knowledge`:

```text
knowledge.db                       index, impact cache, analyses, chat (own migration table)
snapshots/<project>/current        the indexed commit, written from git objects (no .git, no symlinks,
                                   no submodules; .git/.serena/graphify-out paths skipped; every path
                                   contained; files over 1 MiB and binary assets skipped)
engine/<project>/graphify-out      Graphify output and its per-file cache (so re-indexing is incremental)
serena/<project>                   Serena's copy of the indexed commit
work/, scratch/                    per-commit symbol extraction and AI scratch directories; wiped on start
```

`knowledge.db` is derived data and is not part of the main database, its backups or its migration chain
(`docs/DATA_BACKUP.md` covers `symphony.sqlite`). Deleting the `knowledge` directory costs a re-index and
the cached AI analyses.

## Worktrees and the timeline

* Worktrees are `git worktree list --porcelain -z` of the project's repository, so worktrees that Symphony
  did not create (developers', other tools') appear too; kind is `main`, `symphony` (a task owns the path) or
  `external`. Dirty state is `git status --porcelain` with `GIT_OPTIONAL_LOCKS=0` so observing never takes
  `index.lock` away from an agent.
* The timeline is one `git log --date-order` over the HEAD of every worktree and the default branch, which
  share an object database, so **each SHA appears once**. The listing is a prefix of a topological order, so
  following parent links from each HEAD *inside the listed commits* gives exactly the worktrees that contain
  each commit; a commit in two worktrees is one row with two chips. A worktree HEAD older than the window is
  added so it is never missing. `unpushed` means no remote-tracking ref contains the commit, and is `null`
  (not `true`) when the repository has no remote refs at all. Uncommitted work is a separate row without a SHA.
* Live: a filesystem watcher on the git directory (`refs/`, `packed-refs`, `HEAD`, `index`, each worktree's
  `HEAD` and `index`; objects, logs and lock files ignored), debounced 500 ms, plus a 30 s reconcile of every
  worktree's HEAD and uncommitted-file count, because edits in a working tree never touch the git directory
  until staged. Watching starts when the page asks and stops after 10 idle minutes. Events:
  `knowledge/repo_changed`.
* Hardening of every git call: `core.fsmonitor=false`, `--no-ext-diff --no-textconv`, no secret variables,
  revisions validated as 7-64 lowercase hex digits, paths after `--`.

## From a commit to the graph

The index is Graphify's graph of **one commit** (the default branch tip, or a chosen worktree's HEAD). The
commit being inspected may be older or on another branch, so its line numbers cannot be looked up there.
Instead, for commit `C`:

1. git gives the changed files and, per file, the changed line ranges (`-U0`, new side and old side).
2. Only the changed source files are written to a small directory at `C` and at its parent and run through
   Graphify (a few seconds), giving the symbols and start lines **as they were at `C`**.
3. Hunks are mapped onto those symbols. A symbol is taken to extend from its start line to the line before
   the next symbol in the file (Graphify has no end line; `precision: start_to_next`). Lines between two
   symbols (imports, constants) are attributed to the earlier one; changed blocks before the first symbol
   are reported as `unattributed_hunks`. Symbols present before and absent now are `removed`, new ones `added`.
4. Changed symbols are matched to the index by node id (ids are derived from the file path and name, so they
   are stable across commits; checked by `tests/real_engine.rs`) or by `(file, label, kind)`.
5. The affected set is a reverse walk over dependency edges (`calls`, `imports`, `references`, `implements`,
   ... never `contains`) to a depth of 1-4. Each result keeps the hop count, the relation of the last hop and
   the *weakest* confidence on its path; inferred links can be switched off. External packages are never
   "affected". Results are cached per `(project, sha, index run, depth)`.

Without Graphify the impact still lists the changed files, with `unavailable_reason` saying why symbols are
missing. The UI maps the impact onto whichever level is drawn: modules (the longest module path that
prefixes a changed file), files, or symbols.

## AI analysis and chat

* The prompt carries a bounded (60 KB), secret-redacted diff, the changed symbols and the affected set as
  **numbered evidence** (`commit`, `f1..`, `s1..`, `a1..` for a commit; `e1..` for chat). Replies must be one
  JSON object whose findings cite those ids.
* **The model gets no host tools and no repository content.** It runs through the project's preferred provider
  (Codex, Claude Code or OpenCode) in an empty scratch directory with policy `never`, a read-only sandbox and an
  approval gate that declines everything; Symphony's own host tools are not offered. The provider's *built-in*
  tools are switched off through the providers' own mechanisms, written into the scratch directory for that one
  turn (`write_tool_restrictions` in the orchestrator): Claude Code gets `.claude/settings.json` with
  `permissions.deny` for its built-in tools (deny outranks the user's allow rules), OpenCode gets
  `opencode.json` with `"permission": {"*": "deny"}`, and Codex runs with `thread_sandbox: read-only` (no
  writes, no network). These are the providers' controls, not something Symphony enforces: a tool that comes
  from the user's own configuration (an MCP server, say) is not covered. That is why commit messages and code
  are also treated as untrusted input: the reply is validated (below) and only ever shown as text.
  CodeSandbox and the simulated provider are refused for analysis.
* Validation (`analysis.rs`): ids that were not in the prompt are removed, a finding left without evidence is
  dropped and counted (`dropped_findings`, shown in the UI), text is length-capped and rendered as text. A
  chat answer that cites nothing is rejected unless it states `insufficient_evidence`. Redaction covers the
  whole prompt, not only the diff: author names, subjects, file paths and symbol names go through
  `redact_secrets` too, and so do the stored evidence labels.
* Analyses are cached by `(project, sha, provider, model, prompt version)`; `force` re-runs. Token usage is
  credited to the runtime totals.
* Chat retrieval: FTS5 over symbol and file names (per word, then substring), the graph neighbours of the best
  matches, up to 40 lines of code from the index snapshot for the top four, and commits whose messages mention
  the question's words.

## API

| Operation | Route |
| --- | --- |
| `getKnowledgeEngines` | `GET /knowledge/engines` |
| `getKnowledgeOverview` | `GET /projects/{id}/knowledge` |
| `startKnowledgeIndex` | `POST /projects/{id}/knowledge/index` (202) |
| `getKnowledgeGraph` | `GET /projects/{id}/knowledge/graph?level=modules\|files\|symbols&module=&focus=&depth=` |
| `searchKnowledge`, `getKnowledgeNode`, `getKnowledgeNodeReferences` | `.../search`, `.../node`, `.../node/references` |
| `listKnowledgeWorktrees`, `listKnowledgeCommits` | `.../worktrees`, `.../commits?limit=` |
| `getKnowledgeCommit`, `getKnowledgeCommitDiff`, `getKnowledgeCommitImpact` | `.../commits/{sha}`, `/diff`, `/impact?depth=` |
| `getKnowledgeCommitAnalysis`, `startKnowledgeCommitAnalysis` (202) | `.../commits/{sha}/analysis` |
| `listKnowledgeChat`, `askKnowledge` (202), `clearKnowledgeChat` | `.../chat` |

Events on the normal stream: `knowledge/index_progress`, `index_completed`, `index_failed`,
`analysis_completed`, `chat_completed`, `repo_changed` (all with `payload.project_id`).

## Testing

* `cargo test -p symphony-knowledge`: git parsing and real temporary repositories (including the shared-commit
  and unpushed cases), snapshot containment, the importer, graph views and impact walk, hunk-to-symbol
  mapping, prompt/validation, the store (FTS5), the watcher, and the AI flows with a scripted model.
* `cargo test -p symphony-server --test knowledge_api`: the routes through the real router.
* `tests/real_engine.rs` runs the real engines and is skipped unless asked:
  `SYMPHONY_KB_REAL_ENGINES=1` (Graphify, set `SYMPHONY_GRAPHIFY` if it is not on `PATH`) and
  `SYMPHONY_KB_REAL_SERENA=1` (also needs `serena`; ~2 minutes because of the language server).

## Limits and non-goals

* Symbol ranges are approximate (see above); precise ranges and references need Serena, on request.
* The graph reflects the indexed commit. A commit far from it still gets its changed symbols at its own
  tree, but its dependents are those of the indexed graph.
* `calls` edges are resolved by name by Graphify and some are `INFERRED`; they are drawn lighter and can be
  hidden.
* No telemetry, no network access beyond what the user's own provider does for AI analysis.
