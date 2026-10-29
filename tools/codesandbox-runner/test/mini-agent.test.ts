/**
 * `mini_durable_agent.DurableAgent` driven by mini-swe-agent's own scripted test model and its real
 * local environment, against a local bare repository and the real `autocommit.sh`. Needs a Python
 * with `mini-swe-agent` installed (`SYMPHONY_TEST_PYTHON`, or `python3` / `python` on PATH);
 * skipped otherwise.
 */
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const drivers = resolve(here, "..", "template", "drivers");
const fixture = join(here, "fixtures", "run_durable_agent.py");
const posix = (p: string) => p.replace(/\\/g, "/");

function findPython(): string | undefined {
  const candidates = [process.env.SYMPHONY_TEST_PYTHON, "python3", "python"].filter(Boolean) as string[];
  return candidates.find((py) => spawnSync(py, ["-c", "import minisweagent"], { stdio: "ignore" }).status === 0);
}
const python = findPython();
const haveBash = spawnSync("bash", ["-c", "true"], { stdio: "ignore" }).status === 0;
const runnable = Boolean(python) && haveBash;

/** The bash the tests use, as a path Python can run (on Windows, `bash` for Python may be WSL's). */
function bashExecutable(): string {
  const found = spawnSync("bash", ["-c", 'cygpath -m "$(command -v bash)" 2>/dev/null || command -v bash'], { encoding: "utf8" });
  return found.stdout.trim() || "bash";
}

const BRANCH = "symphony/sym-1";
let root: string;
let origin: string;
let home: string;

function env(extra: Record<string, string> = {}): NodeJS.ProcessEnv {
  return {
    ...process.env,
    HOME: posix(home),
    USERPROFILE: home,
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_AUTHOR_NAME: "Test",
    GIT_AUTHOR_EMAIL: "test@example.com",
    GIT_COMMITTER_NAME: "Test",
    GIT_COMMITTER_EMAIL: "test@example.com",
    ...extra,
  };
}

function git(cwd: string, ...args: string[]): string {
  const result = spawnSync("git", args, { cwd, env: env(), encoding: "utf8" });
  if (result.status !== 0) throw new Error(`git ${args.join(" ")} failed: ${result.stderr}`);
  return result.stdout.trim();
}

type Step = { text: string; commands: string[] };

function runAgent(repo: string, stateDir: string, name: string, steps: Step[]) {
  const script = join(root, `${name}.json`);
  writeFileSync(script, JSON.stringify(steps));
  const result = spawnSync(python!, [fixture, script], {
    cwd: repo,
    encoding: "utf8",
    env: env({
      PYTHONPATH: [drivers, process.env.PYTHONPATH].filter(Boolean).join(delimiter),
      PYTHONUTF8: "1",
      MSWEA_CONFIGURED: "true",
      SYMPHONY_BASH: bashExecutable(),
      SYMPHONY_STATE_DIR: stateDir,
      SYMPHONY_BRANCH: BRANCH,
      SYMPHONY_ISSUE: "SYM-1",
      SYMPHONY_TURN_ID: "turn-1",
    }),
  });
  const events = result.stdout
    .split("\n")
    .filter((line) => line.startsWith("@@SYMPHONY "))
    .map((line) => JSON.parse(line.slice("@@SYMPHONY ".length)));
  const summary = result.stdout
    .split("\n")
    .reverse()
    .find((line) => line.startsWith('{"exit_status"'));
  return { result, events, summary: summary ? JSON.parse(summary) : undefined, trajectory: script.replace(/\.json$/, ".traj.json") };
}

beforeAll(() => {
  if (!runnable) return;
  root = mkdtempSync(join(tmpdir(), "symphony-mini-"));
  home = join(root, "home");
  mkdirSync(home);
  origin = join(root, "origin.git");
  git(root, "init", "-q", "--bare", "-b", "main", posix(origin));
  const seed = join(root, "seed");
  git(root, "clone", "-q", posix(origin), posix(seed));
  writeFileSync(join(seed, "README.md"), "# widgets\n");
  git(seed, "add", "-A");
  git(seed, "commit", "-q", "-m", "init");
  git(seed, "push", "-q", "origin", "HEAD:refs/heads/main");
  git(seed, "push", "-q", "origin", `HEAD:refs/heads/${BRANCH}`);
});

afterAll(() => {
  if (root) rmSync(root, { recursive: true, force: true });
});

function checkout(name: string): string {
  const dir = join(root, name);
  git(root, "clone", "-q", posix(origin), posix(dir));
  git(dir, "checkout", "-q", BRANCH);
  return dir;
}

describe.skipIf(!runnable)("mini_durable_agent.DurableAgent", () => {
  it("pushes after every step that edited, reports tools and usage, and checkpoints", () => {
    const repo = checkout("run1");
    const state = join(root, "state1");
    const before = Number(git(repo, "rev-list", "--count", "HEAD"));

    const run = runAgent(repo, state, "run1", [
      { text: "Create a.", commands: ["echo a > a.txt"] },
      { text: "Create b and c.", commands: ["echo b > b.txt", "echo c > c.txt"] },
      { text: "Inspect only.", commands: ["ls"] },
      { text: "Done.", commands: ["echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT"] },
    ]);

    expect(run.result.status, run.result.stderr).toBe(0);
    expect(run.summary).toEqual({ exit_status: "Submitted" });

    // Two steps changed files, so exactly two pushes: one commit per editing step.
    const pushes = run.events.filter((e) => e.type === "pushed");
    expect(pushes).toHaveLength(2);
    expect(Number(git(repo, "rev-list", "--count", "HEAD"))).toBe(before + 2);
    expect(git(posix(origin), "rev-parse", `refs/heads/${BRANCH}`)).toBe(pushes[1].sha);
    expect(git(posix(origin), "ls-tree", "--name-only", `refs/heads/${BRANCH}`)).toMatch(/a\.txt[\s\S]*b\.txt[\s\S]*c\.txt/);
    expect(git(repo, "log", "-2", "--format=%s")).toBe("SYM-1: step 2\nSYM-1: step 1");

    // Events for the timeline: the commands, the model's text and cumulative usage.
    const tools = run.events.filter((e) => e.type === "tool");
    expect(tools.map((t) => t.input.command)).toEqual([
      "echo a > a.txt",
      "echo b > b.txt",
      "echo c > c.txt",
      "ls",
      "echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT",
    ]);
    expect(new Set(tools.map((t) => t.callId)).size).toBe(tools.length);
    expect(run.events.filter((e) => e.type === "output").map((e) => e.text)).toContain("Create b and c.");
    expect(run.events.some((e) => e.type === "usage")).toBe(true);

    // The checkpoint lives outside the repository and holds the conversation.
    const checkpoint = JSON.parse(readFileSync(join(state, "mini-checkpoint.json"), "utf8"));
    expect(checkpoint.messages.length).toBeGreaterThan(6);
    expect(existsSync(join(repo, ".symphony-state"))).toBe(false);
    expect(git(repo, "status", "--porcelain")).toBe("");
  });

  it("pushes edits made in the same step as the submit command", () => {
    const repo = checkout("run2");
    const run = runAgent(repo, join(root, "state2"), "run2", [
      { text: "Edit and submit together.", commands: ["echo z > z.txt", "echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT"] },
    ]);

    expect(run.summary).toEqual({ exit_status: "Submitted" });
    expect(git(posix(origin), "ls-tree", "--name-only", `refs/heads/${BRANCH}`)).toContain("z.txt");
  });

  it("tells the model what an interrupted run already did when it resumes", () => {
    const repo = checkout("run3");
    const state = join(root, "state3");
    // First run: interrupted after two steps (the script simply ends without submitting).
    const first = runAgent(repo, state, "run3a", [
      { text: "Step one.", commands: ["echo one > one.txt"] },
      { text: "Step two.", commands: ["echo two > two.txt"] },
    ]);
    expect(first.result.status).not.toBe(0); // the scripted model ran out of answers

    // Second run with the same state directory (the restored sandbox).
    const second = runAgent(repo, state, "run3b", [{ text: "Finish.", commands: ["echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT"] }]);
    expect(second.summary).toEqual({ exit_status: "Submitted" });

    const trajectory = JSON.parse(readFileSync(second.trajectory, "utf8"));
    const task = trajectory.messages.find((m: { role: string }) => m.role === "user").content as string;
    expect(task).toContain("fix the bug");
    expect(task).toContain("Resuming an interrupted run");
    expect(task).toContain("echo one > one.txt");
    expect(task).toContain("echo two > two.txt");
  });
});
