/**
 * The in-sandbox drivers, run for real against a local bare repository: the commit-and-push-after-
 * every-edit guarantee lives in `autocommit.sh`, and `run.sh` puts the task branch in the sandbox.
 * Skipped where bash or git is not available. HOME is isolated so no git config of the machine is
 * touched.
 */
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

const drivers = resolve(dirname(fileURLToPath(import.meta.url)), "..", "template", "drivers");
const posix = (p: string) => p.replace(/\\/g, "/");

function available(command: string, args: string[]): boolean {
  return spawnSync(command, args, { stdio: "ignore" }).status === 0;
}
const haveTools = available("bash", ["-c", "true"]) && available("git", ["--version"]);

const BRANCH = "symphony/sym-1";
let root: string;
let home: string;
let origin: string;

function env(extra: Record<string, string> = {}): NodeJS.ProcessEnv {
  return {
    ...process.env,
    HOME: posix(home),
    USERPROFILE: home,
    // Set explicitly: an inherited GIT_CONFIG_GLOBAL would send `git config --global` (run.sh) to the
    // developer's real configuration instead of the temporary HOME.
    GIT_CONFIG_GLOBAL: posix(join(home, ".gitconfig")),
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_TERMINAL_PROMPT: "0",
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

function autocommit(cwd: string, message: string) {
  return spawnSync("bash", [posix(join(drivers, "autocommit.sh")), message], {
    cwd,
    env: env({ SYMPHONY_BRANCH: BRANCH }),
    encoding: "utf8",
  });
}

/** A working clone of the origin with the task branch checked out (what run.sh produces). */
function clone(name: string): string {
  const dir = join(root, name);
  git(root, "clone", "-q", posix(origin), posix(dir));
  git(dir, "checkout", "-q", BRANCH);
  return dir;
}

beforeAll(() => {
  if (!haveTools) return;
  root = mkdtempSync(join(tmpdir(), "symphony-drivers-"));
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
  // The local worktree published the task branch before the sandbox started.
  git(seed, "push", "-q", "origin", `HEAD:refs/heads/${BRANCH}`);
});

afterAll(() => {
  if (root) rmSync(root, { recursive: true, force: true });
});

describe.skipIf(!haveTools)("autocommit.sh", () => {
  it("does nothing when nothing changed", () => {
    const work = clone("noop");
    const result = autocommit(work, "SYM-1: nothing");
    expect(result.status).toBe(0);
    expect(result.stdout).toBe("");
  });

  it("commits and pushes every edit as its own commit, reporting each push", () => {
    const work = clone("edits");
    const before = Number(git(work, "rev-list", "--count", "HEAD"));
    const pushed: string[] = [];
    for (const [file, message] of [
      ["a.txt", "SYM-1: edit a.txt"],
      ["b.txt", "SYM-1: edit b.txt"],
    ]) {
      writeFileSync(join(work, file), `${file}\n`);
      const result = autocommit(work, message);
      expect(result.status, result.stderr).toBe(0);
      const line = result.stdout.trim();
      expect(line).toMatch(/^@@SYMPHONY \{"type":"pushed"/);
      const event = JSON.parse(line.slice("@@SYMPHONY ".length));
      expect(event.branch).toBe(BRANCH);
      // The pushed commit is what the remote branch now points at.
      expect(git(posix(origin), "rev-parse", `refs/heads/${BRANCH}`)).toBe(event.sha);
      pushed.push(event.sha);
    }
    expect(new Set(pushed).size).toBe(2);
    expect(Number(git(work, "rev-list", "--count", "HEAD"))).toBe(before + 2);
    expect(git(work, "log", "-1", "--format=%s")).toBe("SYM-1: edit b.txt");
  });

  it("pushes commits the agent made itself even when the tree is clean", () => {
    const work = clone("own-commit");
    writeFileSync(join(work, "own.txt"), "own\n");
    git(work, "add", "-A");
    git(work, "commit", "-q", "-m", "agent commit");
    const result = autocommit(work, "unused");
    expect(result.status).toBe(0);
    expect(git(posix(origin), "log", "-1", "--format=%s", `refs/heads/${BRANCH}`)).toBe("agent commit");
  });

  it("never runs repository hooks", () => {
    const work = clone("hooks");
    const marker = join(root, "hook-ran");
    for (const hook of ["pre-commit", "pre-push", "commit-msg"]) {
      const path = join(work, ".git", "hooks", hook);
      writeFileSync(path, `#!/bin/sh\ntouch '${posix(marker)}'\nexit 1\n`, { mode: 0o755 });
    }
    writeFileSync(join(work, "h.txt"), "h\n");
    const result = autocommit(work, "SYM-1: hooks");
    expect(result.status, result.stderr).toBe(0);
    expect(existsSync(marker)).toBe(false);
  });

  it("rebases onto a remote that moved and pushes", () => {
    const work = clone("rebase");
    const other = clone("rebase-other");
    writeFileSync(join(other, "theirs.txt"), "theirs\n");
    git(other, "add", "-A");
    git(other, "commit", "-q", "-m", "theirs");
    git(other, "push", "-q", "origin", `HEAD:refs/heads/${BRANCH}`);

    writeFileSync(join(work, "mine.txt"), "mine\n");
    const result = autocommit(work, "SYM-1: mine");
    expect(result.status, result.stderr).toBe(0);
    const files = git(posix(origin), "ls-tree", "--name-only", `refs/heads/${BRANCH}`);
    expect(files).toContain("theirs.txt");
    expect(files).toContain("mine.txt");
  });

  it("fails instead of reporting success when git add cannot run", () => {
    const work = clone("add-fails");
    const remoteBefore = git(posix(origin), "rev-parse", `refs/heads/${BRANCH}`);
    // A stale index lock makes `git add` fail, as a concurrent git process would.
    writeFileSync(join(work, ".git", "index.lock"), "");
    writeFileSync(join(work, "never-committed.txt"), "x\n");

    const result = autocommit(work, "SYM-1: add fails");

    expect(result.status).toBe(1);
    expect(result.stderr).toMatch(/git add failed/);
    expect(result.stdout).toBe("");
    expect(git(posix(origin), "rev-parse", `refs/heads/${BRANCH}`)).toBe(remoteBefore);
    rmSync(join(work, ".git", "index.lock"));
  });

  it("never forces: a conflicting remote change fails and leaves the remote alone", () => {
    const work = clone("conflict");
    const other = clone("conflict-other");
    writeFileSync(join(other, "README.md"), "# theirs\n");
    git(other, "add", "-A");
    git(other, "commit", "-q", "-m", "theirs readme");
    git(other, "push", "-q", "origin", `HEAD:refs/heads/${BRANCH}`);
    const remoteBefore = git(posix(origin), "rev-parse", `refs/heads/${BRANCH}`);

    writeFileSync(join(work, "README.md"), "# mine\n");
    const result = autocommit(work, "SYM-1: conflicting");
    expect(result.status).toBe(1);
    expect(result.stderr).toMatch(/rejected|failed/);
    expect(git(posix(origin), "rev-parse", `refs/heads/${BRANCH}`)).toBe(remoteBefore);
    // The working tree is left usable (no rebase in progress).
    expect(existsSync(join(work, ".git", "rebase-merge"))).toBe(false);
  });
});

describe.skipIf(!haveTools)("run.sh", () => {
  function runSh(repoDir: string, extra: Record<string, string> = {}) {
    const promptFile = join(root, "prompt.txt");
    writeFileSync(promptFile, "do the thing");
    return spawnSync("bash", [posix(join(drivers, "run.sh"))], {
      env: env({
        SYMPHONY_REPO_URL: posix(origin),
        SYMPHONY_BRANCH: BRANCH,
        SYMPHONY_REPO_DIR: posix(repoDir),
        SYMPHONY_TASK_ID: "task-1",
        SYMPHONY_ISSUE: "SYM-1",
        SYMPHONY_TURN_ID: "turn-1",
        SYMPHONY_PROMPT_FILE: posix(promptFile),
        // Not a real agent: the driver reports it and still pushes what is there.
        SYMPHONY_AGENT: "no_such_agent",
        ...extra,
      }),
      encoding: "utf8",
    });
  }

  it("clones the task branch, dispatches the driver and leaves the checkout on the branch", () => {
    const repo = join(root, "sandbox", "symphony-repo");
    const result = runSh(repo);
    // 10 = the agent failed (here: there is no such agent), not a git failure.
    expect(result.status, result.stderr).toBe(10);
    expect(result.stdout).toContain("unknown agent no_such_agent");
    expect(git(repo, "rev-parse", "--abbrev-ref", "HEAD")).toBe(BRANCH);
    // The branch's content (earlier cases pushed to it), checked out in the sandbox.
    expect(existsSync(join(repo, "README.md"))).toBe(true);
    expect(git(repo, "rev-parse", "HEAD")).toBe(git(posix(origin), "rev-parse", `refs/heads/${BRANCH}`));
  });

  it("configures git for the sandbox user only (the global config of the home it runs in)", () => {
    const repo = join(root, "sandbox3", "symphony-repo");
    const result = runSh(repo, { SYMPHONY_GIT_NAME: "Cloud Bot" });
    expect(result.status, result.stderr).toBe(10);
    // These tests point GIT_CONFIG_GLOBAL at a file in the temporary home, so nothing of the
    // developer's own configuration is read or written.
    expect(readFileSync(join(home, ".gitconfig"), "utf8")).toContain("Cloud Bot");
  });

  it("keeps unpushed commits of a resumed sandbox and pushes them first", () => {
    const repo = join(root, "sandbox2", "symphony-repo");
    expect(runSh(repo).status).toBe(10);
    // The sandbox was interrupted after an edit but before the push.
    writeFileSync(join(repo, "left-over.txt"), "work in progress\n");

    const result = runSh(repo);
    expect(result.status, result.stderr).toBe(10);
    expect(git(posix(origin), "ls-tree", "--name-only", `refs/heads/${BRANCH}`)).toContain("left-over.txt");
    expect(result.stdout).toContain('"type":"pushed"');
  });
});
