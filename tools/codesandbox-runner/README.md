# CodeSandbox cloud agents (`@symphony/codesandbox-runner`)

Runs a Symphony task's agent in a [CodeSandbox](https://codesandbox.io) microVM instead of on your
machine. The sandbox is created from a **template** ([CodeSandbox SDK templates](https://codesandbox.io/docs/sdk/templates))
that carries three agents:

| Agent (`cloud_agent`) | What it is | Durability |
| --- | --- | --- |
| `mini_swe_agent` (**default**) | [mini-swe-agent](https://mini-swe-agent.com) driven by `mini_durable_agent.DurableAgent` | A checkpoint of its messages after every step; on resume the model is told what the interrupted run already did. |
| `pi` | The [Pi coding agent](https://pi.dev) (`pi --mode json`) | One Pi session per task (`--session-id`), so a restored sandbox continues the same conversation. |
| `pi_durable` | [Pi Durable](https://www.npmjs.com/package/@earendil-works/pi-durable) (experimental) with Pi's `read` / `write` / `edit` / `bash` tools | Every model turn and tool call is committed to SQLite before it is acted on; `harness.resume()` recovers what an interruption left unfinished. |

Whatever the agent, **every edit is committed and pushed to the task branch** (`autocommit.sh`:
`git add -A`, commit, `git push` of `HEAD:<branch>`, never forced). Symphony fast-forwards the local
worktree after each push, so the Changes panel, validation and the pull request flow work as for a
local agent.

## How a run works

```text
Symphony (Rust)                          bridge.mjs (Node)                    CodeSandbox microVM
CodeSandboxSession ──stdin: 1 request──▶ @codesandbox/sdk ───────────────────▶ template "symphony-agents"
        ▲                                      │                                 run.sh: clone / checkout the task branch
        └──── stdout: JSON event lines ◀───────┘ ◀── "@@SYMPHONY {...}" lines ── symphony-run-agent.ts → mini | pi | pi_durable
                                                                                  └─ autocommit.sh after every edit  ──▶ git push origin
```

1. Before the run, Symphony publishes the task branch to `origin` (the sandbox clones it) and starts
   `node dist/bridge.mjs`.
2. The bridge creates a sandbox from the template — or **resumes the sandbox of the previous attempt**
   (`thread_id` is the sandbox id) — and connects with a session that carries the GitHub token as a git
   credential and the model keys as environment.
3. It runs `run.sh`, which fetches the branch, then the driver of the chosen agent.
4. Output, tool calls, token usage and every push stream back as events; each `pushed` event makes
   Symphony fast-forward the worktree (`WorkspaceManager::fast_forward_from_origin`, which never touches a
   dirty or diverged worktree).
5. When the run ends the sandbox is hibernated (files and the agent's checkpoint are kept); cancelling
   first stops the driver and everything it started (`abort.sh`: the agent and the commands it ran, children
   before parents), so nothing keeps working or pushing.

## Setup

1. **CodeSandbox token**: create one at <https://codesandbox.io/t/api> and store it in Symphony under
   *Providers → CodeSandbox Cloud Agent* (`CSB_API_KEY`).
2. **GitHub**: a token (`GITHUB_TOKEN`, *Providers* / *Integrations*) that can push to the repository, and a
   repository whose `origin` is an `https://github.com/...` URL. The sandbox cannot use your local SSH keys.
3. **Build the bridge** (Node ≥ 22 on `PATH`):

   ```bash
   pnpm --filter @symphony/codesandbox-runner build
   ```

   A source checkout finds `tools/codesandbox-runner/dist/bridge.mjs` on its own. Anywhere else set
   `SYMPHONY_CODESANDBOX_RUNNER` or `codesandbox.runner_path` in `WORKFLOW.md`.
4. **Build the template** once (and again after changing `template/`):

   ```bash
   CSB_API_KEY=... pnpm --filter @symphony/codesandbox-runner build:template
   ```

   Put the template id it prints in `WORKFLOW.md`:

   ```yaml
   codesandbox:
     template_id: <template id>
     default_agent: mini_swe_agent   # mini_swe_agent | pi | pi_durable
     vm_tier: small                  # pico | nano | micro | small | medium | large | xlarge
     hibernation_timeout_secs: 1800
     cost_limit_usd: 3               # mini-swe-agent only; 0 = no limit
   ```
5. Choose *CodeSandbox Cloud Agent* as the provider of a task (the composer then offers the agent), or set
   it as a project's preferred provider. Models are `provider/model-id` (for example
   `anthropic/claude-sonnet-5-5`); the key of that provider (`ANTHROPIC_API_KEY` / `OPENAI_API_KEY`) is
   passed into the sandbox.

## Security

- The CodeSandbox key, GitHub token and model keys reach the bridge **only in the request on its stdin**;
  never on a command line or in an environment variable of the bridge process. Inside the sandbox the
  GitHub token is a git credential of the session (`$HOME/.private`), not part of the repository or of the
  environment variables the agent is started with. The agent runs as the same user in that sandbox, so it
  can read the credential file: treat the token as available to the agent.
- Everything the bridge prints is redacted (the request's own secrets plus GitHub / CodeSandbox / `sk-`
  token shapes), and Symphony registers the same values for its own log redaction.
- The template runs repository content (the agent runs commands from the repository's context) inside the
  sandbox, not on your machine, but the sandbox holds the GitHub token you gave it and the model provider
  key (an environment variable of the agent). Anything the agent runs, including scripts from the
  checked-out repository, runs as the same user and can read both; hiding the variable from one process
  would not change that. Use a GitHub token scoped to the repository and a dedicated, spend-limited model key.
- The GitHub token is only sent to the `origin` of the task's repository (an `https://` GitHub URL). If the
  project has a GitHub repository configured, the origin must match it; if not, the destination is written to
  the task's activity log (`codesandbox/launch`) before the branch is published.
- Default agent order: the task's choice, then *Settings → Default provider* (once you have chosen one there),
  then `codesandbox.default_agent` in `WORKFLOW.md`, then mini-swe-agent. Repository hooks never run (`core.hooksPath=/dev/null`, `--no-verify`), and Pi ignores
  repository-local Pi configuration (`--no-approve`).
- A push is never forced: a rejected push is rebased once onto the remote branch and otherwise fails the turn.

## Protocol

One JSON request line on stdin, JSON event lines on stdout (nothing else is written there). See
`src/protocol.ts`. Requests: `turn`, `abort` (kill the agent process in a sandbox), `stop` (hibernate or
shut down). Events: `started`, `output`, `tool`, `usage`, `pushed`, `turn_completed`, `error`.

## Tests

```bash
pnpm --filter @symphony/codesandbox-runner test
```

- `test/bridge.test.ts`: the bridge against a fake sandbox gateway (protocol, redaction, timeout, host
  disconnect, resume, abort / stop).
- `test/drivers.test.ts`: `autocommit.sh` and `run.sh` against a real local bare repository (one commit and
  push per edit, no forcing, rebase on a moved remote, hooks never run, resumed sandboxes keep unpushed work).
- `test/mini-agent.test.ts`: `DurableAgent` driven by mini-swe-agent's scripted test model with its real
  local environment. Needs a Python with `mini-swe-agent` (`SYMPHONY_TEST_PYTHON` or `python3` / `python`);
  skipped otherwise.
- The Pi and Pi Durable drivers are type-checked against the pinned packages (`pnpm typecheck`).

### What has not been exercised

The bridge has not been run against a live CodeSandbox account, and none of the three agents has been run
against a real language model from this repository's tests. In particular the Pi (`pi --mode json`) and Pi
Durable drivers follow the packages' documentation and type declarations (Pi Durable is marked experimental
upstream; pin its version and re-run the template build when you change it). Treat the first real run as an
integration test: start with a throwaway repository.
