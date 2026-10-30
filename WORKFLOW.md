---
tracker:
  kind: github
  provider:
    api_url: https://api.github.com
    repo: $GITHUB_REPO
    token: $GITHUB_TOKEN
  required_labels:
    - symphony
  active_states:
    - open
    - ready
    - in_progress
  terminal_states:
    - closed
    - completed
    - cancelled

polling:
  interval_ms: 30000

workspace:
  root: ~/.symphony/workspaces

hooks:
  after_create: git status --short
  before_run: git status --short
  after_run: git status --short
  timeout_ms: 60000

agent:
  default_provider: codex
  max_concurrent_agents: 2
  max_turns: 20
  max_retry_backoff_ms: 300000
  max_concurrent_agents_by_state:
    open: 2
    in_progress: 2
  validation_commands:
    - cargo check --workspace
    - cargo test --workspace

codex:
  command: codex app-server
  approval_policy: on-request
  thread_sandbox: workspace-write
  turn_sandbox_policy: workspace-write
  turn_timeout_ms: 3600000
  read_timeout_ms: 5000
  stall_timeout_ms: 300000

# Cloud agents (provider `codesandbox`): the agent runs in a CodeSandbox microVM built from a template
# (`pnpm --filter @symphony/codesandbox-runner build:template`) and pushes to the task branch after every
# edit. These are the defaults; uncomment and edit to change them. See tools/codesandbox-runner/README.md.
# codesandbox:
#   template_id: symphony-agents        # the id printed by build:template
#   default_agent: mini_swe_agent       # mini_swe_agent | pi | pi_durable
#   vm_tier: small                      # pico | nano | micro | small | medium | large | xlarge
#   hibernation_timeout_secs: 1800
#   cost_limit_usd: 3                   # mini-swe-agent only; 0 = no limit
#
# Optional override, not a default: where the built bridge is. Unset, Symphony finds
# tools/codesandbox-runner/dist/bridge.mjs of a source checkout (or SYMPHONY_CODESANDBOX_RUNNER). Set it
# only for an installed app; a path that does not exist makes CodeSandbox runs fail to start.
#   runner_path: ~/symphony/tools/codesandbox-runner/dist/bridge.mjs

server:
  host: 127.0.0.1
  # The CLI's default API port, so this example agrees with the documentation. The desktop app
  # ignores this file's server block: it binds 4780 (or an ephemeral port) and publishes the real
  # endpoint in runtime.json.
  port: 4780
---
You are an autonomous software engineering agent dispatched by **Symphony Desktop** to resolve issue **{{ issue.identifier }}**.

### Issue Details
- **Identifier:** `{{ issue.identifier }}`
- **Title:** {{ issue.title }}
- **State:** `{{ issue.state }}`
- **Attempt:** `{{ attempt }}`
- **Branch:** `{{ issue.branch_name }}`
- **URL:** {{ issue.url }}

### Description
{{ issue.description }}

### Execution Mandate
1. Operate strictly inside the allocated Git worktree workspace.
2. Inspect the affected modules and existing automated tests before making changes.
3. Implement a clean, idiomatic fix or feature that satisfies the issue requirements.
4. Run all relevant project validation commands (`cargo check --workspace`, `cargo test --workspace`, `pnpm typecheck`, `pnpm test`) and verify they pass with zero regressions.
5. Summarize the root cause, changes made, and verification evidence so Symphony Desktop can compute the structured Merge Readiness Report (1–5) for human approval.
