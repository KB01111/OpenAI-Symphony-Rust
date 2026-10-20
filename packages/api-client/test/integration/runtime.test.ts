/**
 * Contract integration tests: drives the real Axum router (via `symphony-cli`) with the
 * TypeScript client. Uses the deterministic simulated agent; no paid models are called.
 *
 * Requires a built CLI: `cargo build -p symphony-cli` (done automatically when missing).
 */
import { execFile, execFileSync, spawn, type ChildProcess } from "node:child_process";
import { promisify } from "node:util";
import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import type { AgentEvent, Project, Task } from "@symphony/shared-types";
import { SymphonyApiError, SymphonyClient, type EventStream } from "../../src/index.js";
import { readRuntimeDiscovery } from "../../src/node/index.js";

const repoRoot = resolve(__dirname, "../../../..");
const exe = process.platform === "win32" ? "symphony-cli.exe" : "symphony-cli";
const cliPath = process.env.SYMPHONY_CLI_BIN ?? join(repoRoot, "target", "debug", exe);

let tmp: string;
let dataDir: string;
let repoDir: string;
let runtime: ChildProcess;
let client: SymphonyClient;
let stream: EventStream;
const events: AgentEvent[] = [];
const resyncs: string[] = [];

function git(args: string[], cwd: string) {
  execFileSync("git", args, { cwd, stdio: "ignore" });
}

async function waitFor<T>(fn: () => T | undefined | null | false | Promise<T | undefined | null | false>, label: string, timeoutMs = 30_000): Promise<T> {
  const start = Date.now();
  for (;;) {
    const value = await fn();
    if (value) return value;
    if (Date.now() - start > timeoutMs) throw new Error(`Timed out waiting for ${label}`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

const eventFor = (taskId: string, type: string) =>
  events.find((e) => e.task_id === taskId && e.event_type === type);

beforeAll(async () => {
  if (!existsSync(cliPath)) {
    // Async so the (possibly minutes-long) build does not block the Vitest worker's RPC.
    await promisify(execFile)("cargo", ["build", "-p", "symphony-cli"], { cwd: repoRoot, maxBuffer: 64 * 1024 * 1024 });
  }
  tmp = mkdtempSync(join(tmpdir(), "symphony-it-"));
  dataDir = join(tmp, "data");
  repoDir = join(tmp, "repo");
  execFileSync("git", ["init", "-q", "-b", "main", repoDir]);
  git(["config", "user.email", "it@symphony.local"], repoDir);
  git(["config", "user.name", "Symphony IT"], repoDir);
  writeFileSync(join(repoDir, "README.md"), "# fixture\n");
  git(["add", "."], repoDir);
  git(["commit", "-q", "-m", "init"], repoDir);

  runtime = spawn(cliPath, ["--data-dir", dataDir, "--port", "0"], {
    cwd: tmp,
    env: { ...process.env, SYMPHONY_SIMULATED_STEP_MS: "40", RUST_LOG: "warn" },
    stdio: ["ignore", "ignore", "inherit"]
  });
}, 240_000);

afterAll(async () => {
  stream?.stop();
  if (runtime && runtime.exitCode === null && runtime.signalCode === null) {
    // Wait for the process to exit: Windows keeps its files (SQLite, lock) busy until then.
    const exited = new Promise<void>((resolve) => runtime.once("exit", () => resolve()));
    runtime.kill();
    await Promise.race([exited, new Promise((r) => setTimeout(r, 10_000))]);
  }
  if (tmp) {
    try {
      rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
    } catch (error) {
      // Leftover temp files must not fail the suite; the OS cleans the temp dir.
      console.warn(`could not remove ${tmp}:`, error);
    }
  }
});

describe("Symphony runtime contract (real Axum router)", () => {
  let project: Project;

  it("1. discovers the local runtime endpoint and token", async () => {
    const record = await waitFor(() => readRuntimeDiscovery(dataDir), "runtime.json");
    expect(record.base_url).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/);
    expect(record.token).toHaveLength(64);
    client = new SymphonyClient({ baseUrl: record.base_url, token: record.token });
    const info = await client.runtime();
    expect(info.auth_required).toBe(true);
    expect(info.api_version).toBe(2);

    const anonymous = new SymphonyClient({ baseUrl: record.base_url });
    const err = (await anonymous.getState().catch((e: unknown) => e)) as SymphonyApiError;
    expect(err.kind).toBe("unauthorized");

    stream = client.createEventStream({
      onEvent: (e) => events.push(e),
      onResync: (r) => resyncs.push(r)
    });
    stream.start();
    await waitFor(() => stream.status === "open", "event stream open");
  });

  it("2. fetches orchestrator state", async () => {
    const state = await client.getState();
    expect(state.scheduler_paused).toBe(false);
    expect(state.counts.running).toBe(0);
    expect(state.hardware.recommended_max_concurrency).toBeGreaterThanOrEqual(0);
    const providers = await client.listProviders();
    const simulated = providers.find((p) => p.provider === "simulated");
    expect(simulated?.installed).toBe(true);
    expect(simulated?.auth_state).toBe("not_required");
  });

  it("3. validates and registers a repository", async () => {
    const bad = await client.validateRepository(join(tmp, "missing"));
    expect(bad.exists).toBe(false);
    expect(bad.errors.length).toBeGreaterThan(0);

    const good = await client.validateRepository(repoDir);
    expect(good.is_git_repository).toBe(true);
    expect(good.current_branch).toBe("main");

    const rejected = (await client
      .createProject({ name: "bad", repo_path: join(tmp, "missing") })
      .catch((e: unknown) => e)) as SymphonyApiError;
    expect(rejected.code).toBe("invalid_repository");

    project = await client.createProject({
      name: "Fixture",
      repo_path: repoDir,
      preferred_provider: "simulated",
      workspace_root: join(tmp, "workspaces")
    });
    expect(project.default_branch).toBe("main");
    expect(project.preferred_provider).toBe("simulated");
    const updated = await client.updateProject(project.id, { max_concurrent_agents: 3 });
    expect(updated.max_concurrent_agents).toBe(3);
    expect((await client.listProjects()).map((p) => p.id)).toContain(project.id);
  });

  let reviewTask: Task;

  it("4. creates a task without dispatching it", async () => {
    const task = await client.createTask({ project_id: project.id, title: "Manual task", priority: 2 });
    expect(task.status).toBe("ready");
    expect(task.provider).toBe("simulated");
    const err = (await client.moveTaskStatus(task.id, "running").catch((e: unknown) => e)) as SymphonyApiError;
    expect(err.kind).toBe("conflict");
    const moved = await client.moveTaskStatus(task.id, "backlog");
    expect(moved.status).toBe("backlog");
  });

  it("5-6. dispatches immediately and receives live agent events", async () => {
    reviewTask = await client.createTask({
      project_id: project.id,
      title: "Add note",
      description: "Please run tests. SIMULATE_APPROVAL",
      auto_start: true
    });
    expect(reviewTask.status).toBe("running");
    await waitFor(() => eventFor(reviewTask.id, "task/created"), "task/created");
    await waitFor(() => eventFor(reviewTask.id, "workspace/prepared"), "workspace/prepared");
    await waitFor(() => eventFor(reviewTask.id, "item/agentMessage"), "agent message");
    await waitFor(() => eventFor(reviewTask.id, "approval/requested"), "approval request");
    const state = await client.getState();
    expect(state.running.map((r) => r.task_id)).toContain(reviewTask.id);
  });

  it("8. resolves the approval exactly once", async () => {
    const pending = await client.listApprovals(true);
    const approval = pending.find((a) => a.task_id === reviewTask.id);
    expect(approval?.command_or_diff).toBe("cargo test --workspace");
    const resolved = await client.decideApproval(approval!.id, "accept");
    expect(resolved.status).toBe("approved");
    const dup = (await client.decideApproval(approval!.id, "accept").catch((e: unknown) => e)) as SymphonyApiError;
    expect(dup.kind).toBe("conflict");
    await waitFor(() => eventFor(reviewTask.id, "attempt/succeeded"), "attempt/succeeded");
    expect((await client.getTask(reviewTask.id)).status).toBe("human_review");
  });

  it("exposes worktree, diff, and issue detail for the run", async () => {
    const diff = await client.getTaskDiff(reviewTask.id);
    expect(diff.exists).toBe(true);
    const note = diff.changed_files.find((f) => f.path === "SYMPHONY_TASK_NOTE.md");
    expect(note?.status).toBe("added");
    expect(note!.additions).toBeGreaterThan(0);
    expect(diff.diff).toContain("SYMPHONY_TASK_NOTE.md");

    const worktrees = await client.listWorktrees();
    expect(worktrees.find((w) => w.task_id === reviewTask.id)?.exists).toBe(true);

    const detail = await client.getIssueDetail(reviewTask.issue.identifier);
    expect(detail.task.id).toBe(reviewTask.id);
    expect(detail.attempts.restart_count).toBe(0);
    expect((await client.listTaskAttempts(reviewTask.id))[0]?.phase).toBe("succeeded");
    expect((await client.listTaskEvents(reviewTask.id)).length).toBeGreaterThan(3);
    const recent = await client.listRecentEvents(20);
    expect(recent.length).toBeGreaterThan(0);
    expect(recent[0]!.timestamp >= recent[recent.length - 1]!.timestamp).toBe(true);
  });

  it("9. fetches PR review information and records human approval", async () => {
    const prs = await client.listPullRequests(project.id);
    const pr = prs.find((p) => p.task_id === reviewTask.id)!;
    expect(pr.merge_readiness.score).toBeGreaterThanOrEqual(1);
    expect(pr.merge_readiness.score).toBeLessThanOrEqual(5);
    expect(pr.human_approved).toBe(false);
    const approved = await client.approvePullRequest(pr.id);
    expect(approved.human_approved).toBe(true);
    expect((await client.getTask(reviewTask.id)).status).toBe("completed");
    const invalid = (await client.moveTaskStatus(reviewTask.id, "backlog").catch((e: unknown) => e)) as SymphonyApiError;
    expect(invalid.code).toBe("invalid_state_transition");
  });

  it("7. cancels a running execution and retries it", async () => {
    const task = await client.createTask({
      project_id: project.id,
      title: "Cancel me",
      description: "SIMULATE_APPROVAL",
      auto_start: true
    });
    await waitFor(() => eventFor(task.id, "approval/requested"), "approval before cancel");
    const cancelled = await client.cancelTask(task.id, "integration test");
    expect(cancelled.status).toBe("cancelled");
    await waitFor(() => eventFor(task.id, "approval/expired"), "approval expired");
    expect((await client.listApprovals(true)).some((a) => a.task_id === task.id)).toBe(false);

    const retried = await client.retryTask(task.id);
    expect(retried.status).toBe("running");
    expect(retried.attempt_count).toBe(2);
    const approval = await waitFor(
      async () => (await client.listApprovals(true)).find((a) => a.task_id === task.id),
      "approval after retry"
    );
    await client.decideApproval(approval.id, "decline");
    await waitFor(
      () => events.filter((e) => e.task_id === task.id && e.event_type === "attempt/succeeded").length > 0,
      "retried attempt completes"
    );
  });

  it("10-11. previews and saves workflow configuration with the real parser", async () => {
    const invalid = await client.previewWorkflow({ raw_markdown: "---\n- not\n- a map\n---\nHi" });
    expect(invalid.valid).toBe(false);
    expect(invalid.error_code).toBe("workflow_front_matter_not_a_map");

    const raw = "---\ntracker:\n  kind: manual\nagent:\n  max_concurrent_agents: 3\n---\nWork on {{ issue.identifier }}: {{ issue.title }}\n";
    const preview = await client.previewWorkflow({ raw_markdown: raw, sample_identifier: "SYM-42" });
    expect(preview.valid).toBe(true);
    expect(preview.rendered_prompt).toContain("SYM-42");
    expect(preview.config?.agent.max_concurrent_agents).toBe(3);

    const rejected = (await client.saveWorkflow("---\n- bad\n---\n").catch((e: unknown) => e)) as SymphonyApiError;
    expect(rejected.kind).toBe("validation");

    const saved = await client.saveWorkflow(raw);
    expect(saved.valid).toBe(true);
    expect(saved.path).toBe(join(dataDir, "WORKFLOW.md"));
    const doc = await client.getWorkflow();
    expect(doc.raw_markdown).toBe(raw);
    expect(doc.valid).toBe(true);
  });

  it("12. persists settings and rejects invalid values", async () => {
    const settings = await client.getSettings();
    const saved = await client.saveSettings({
      ...settings,
      theme: "light",
      locale: "sv",
      provider_default_models: { simulated: "simulated-deterministic-v1" }
    });
    expect(saved.locale).toBe("sv");
    const reloaded = await client.getSettings();
    expect(reloaded.theme).toBe("light");
    expect(reloaded.provider_default_models.simulated).toBe("simulated-deterministic-v1");
    const err = (await client
      .saveSettings({ ...reloaded, polling_interval_ms: 10 })
      .catch((e: unknown) => e)) as SymphonyApiError;
    expect(err.code).toBe("invalid_settings");
  });

  it("stores credentials without ever returning their values", async () => {
    await client.saveCredential("LINEAR_API_KEY", "lin_api_testvalue1234567890");
    const listed = await client.listCredentials();
    expect(listed.configured).toContain("LINEAR_API_KEY");
    expect(JSON.stringify(listed)).not.toContain("lin_api_testvalue");
    const diag = JSON.stringify(await client.exportDiagnostics());
    expect(diag).not.toContain("lin_api_testvalue1234567890");
    await client.deleteCredential("LINEAR_API_KEY");
    expect((await client.listCredentials()).configured).not.toContain("LINEAR_API_KEY");
  });

  it("delivers each event once over SSE, in emission order", () => {
    const ids = events.map((e) => e.id);
    expect(new Set(ids).size).toBe(ids.length);
    const created = events.findIndex((e) => e.task_id === reviewTask.id && e.event_type === "task/created");
    const succeeded = events.findIndex((e) => e.task_id === reviewTask.id && e.event_type === "attempt/succeeded");
    expect(created).toBeLessThan(succeeded);
    expect(resyncs).toEqual([]);
  });

  it("pauses and resumes the scheduler", async () => {
    expect((await client.pauseScheduler()).scheduler_paused).toBe(true);
    expect((await client.getState()).scheduler_paused).toBe(true);
    expect((await client.resumeScheduler()).scheduler_paused).toBe(false);
  });
});
