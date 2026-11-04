import { spawn } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/** Prefix of the structured lines the bridge understands (see `src/protocol.ts`). */
export const PREFIX = "@@SYMPHONY ";

export const DRIVER_DIR = dirname(fileURLToPath(import.meta.url));

export interface DriverEvent {
  type: "output" | "tool" | "usage" | "pushed";
  [key: string]: unknown;
}

export function emit(event: DriverEvent): void {
  process.stdout.write(`${PREFIX}${JSON.stringify(event)}\n`);
}

function required(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is not set`);
  return value;
}

export const config = {
  agent: process.env.SYMPHONY_AGENT ?? "mini_swe_agent",
  taskId: required("SYMPHONY_TASK_ID"),
  issue: process.env.SYMPHONY_ISSUE ?? "task",
  turnId: process.env.SYMPHONY_TURN_ID ?? "turn-1",
  promptFile: required("SYMPHONY_PROMPT_FILE"),
  repoDir: required("SYMPHONY_REPO_DIR"),
  branch: required("SYMPHONY_BRANCH"),
  model: process.env.SYMPHONY_MODEL || undefined,
  costLimitUsd: Number(process.env.SYMPHONY_COST_LIMIT_USD ?? "0") || 0,
  /** Outside the repository, so checkpoints are never committed. */
  stateDir: join(dirname(required("SYMPHONY_REPO_DIR")), ".symphony-state"),
};

let queue: Promise<boolean> = Promise.resolve(true);
let lastResult = true;

/**
 * Commit and push whatever changed, one at a time (`autocommit.sh` prints the `pushed` event).
 * Resolves with whether the push worked; a failure is reported as output and remembered, so the
 * driver can fail the turn instead of losing the work silently.
 */
export function autocommit(message: string): Promise<boolean> {
  queue = queue.then(
    () =>
      new Promise<boolean>((resolve) => {
        const child = spawn("bash", [join(DRIVER_DIR, "autocommit.sh"), message], {
          cwd: config.repoDir,
          env: { ...process.env, SYMPHONY_BRANCH: config.branch },
          stdio: ["ignore", "inherit", "pipe"],
        });
        let stderr = "";
        child.stderr.on("data", (chunk: Buffer) => {
          stderr += chunk.toString();
        });
        child.on("close", (code) => {
          lastResult = code === 0;
          if (code !== 0) emit({ type: "output", text: `commit/push failed: ${stderr.trim().slice(0, 2000)}` });
          resolve(lastResult);
        });
        child.on("error", () => {
          lastResult = false;
          resolve(false);
        });
      }),
  );
  return queue;
}

export function lastAutocommitSucceeded(): boolean {
  return lastResult;
}

export function clip(text: string, limit = 4000): string {
  return text.length <= limit ? text : `${text.slice(0, limit)}…`;
}

/** Exit codes of the driver, mapped to `agent_exit_<code>` by the bridge. */
export const EXIT = { ok: 0, agentFailed: 10, pushFailed: 11 } as const;
