/**
 * Runs one Symphony agent turn inside the sandbox. `run.sh` has already put the task branch in
 * `SYMPHONY_REPO_DIR`. The file name is what `abort` looks for (`pkill -f '[s]ymphony-run-agent'`).
 *
 * Exit codes: 0 done, 10 the agent failed, 11 the work could not be pushed.
 */
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync } from "node:fs";
import { delimiter, join } from "node:path";
import { createInterface } from "node:readline";
import { autocommit, clip, config, DRIVER_DIR, EXIT, emit, lastAutocommitSucceeded, PREFIX } from "./lib.ts";

const MAX_FAILURE_TAIL = 40;

function toolKind(name: string): "command" | "file_change" | "provider_tool" {
  if (name === "bash") return "command";
  if (name === "edit" || name === "write") return "file_change";
  return "provider_tool";
}

/** Lines of a child's stdout/stderr: structured lines pass through, the rest is kept for failures. */
function pump(stream: NodeJS.ReadableStream, tail: string[]): void {
  const lines = createInterface({ input: stream, crlfDelay: Infinity });
  lines.on("line", (line) => {
    if (line.startsWith(PREFIX)) {
      process.stdout.write(`${line}\n`);
      return;
    }
    tail.push(line);
    if (tail.length > MAX_FAILURE_TAIL) tail.shift();
  });
}

function exitCodeOf(child: ReturnType<typeof spawn>): Promise<number> {
  return new Promise((resolve) => {
    child.on("close", (code) => resolve(code ?? 1));
    child.on("error", () => resolve(127));
  });
}

async function runMini(): Promise<number> {
  const prompt = readFileSync(config.promptFile, "utf8");
  mkdirSync(config.stateDir, { recursive: true });
  const trajectory = join(config.stateDir, "mini-trajectory.json");
  const args = [
    "-y",
    "--exit-immediately",
    "--agent-class",
    "mini_durable_agent.DurableAgent",
    "-t",
    prompt,
    "-o",
    trajectory,
    "-l",
    String(config.costLimitUsd),
  ];
  if (config.model) args.push("-m", config.model);

  const child = spawn("mini", args, {
    cwd: config.repoDir,
    env: {
      ...process.env,
      PYTHONPATH: [DRIVER_DIR, process.env.PYTHONPATH].filter(Boolean).join(delimiter),
      PYTHONUNBUFFERED: "1",
      MSWEA_CONFIGURED: "true",
      SYMPHONY_STATE_DIR: config.stateDir,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const tail: string[] = [];
  pump(child.stdout, tail);
  pump(child.stderr, tail);
  const code = await exitCodeOf(child);

  let exitStatus = "";
  try {
    exitStatus = JSON.parse(readFileSync(trajectory, "utf8"))?.info?.exit_status ?? "";
  } catch {
    // no trajectory: mini did not get far enough to write one
  }
  if (code === 0 && exitStatus === "Submitted") return EXIT.ok;
  emit({
    type: "output",
    text: clip(
      `mini-swe-agent ended with ${exitStatus || `exit code ${code}`}${tail.length ? `\n${tail.join("\n")}` : ""}`,
    ),
  });
  return EXIT.agentFailed;
}

async function runPi(): Promise<number> {
  const sessions = join(config.stateDir, "pi-sessions");
  mkdirSync(sessions, { recursive: true });
  const args = [
    "--mode",
    "json",
    // Repository-local Pi configuration is untrusted input.
    "--no-approve",
    "--session-dir",
    sessions,
    // The same session on every turn and after a restore: Pi continues the conversation.
    "--session-id",
    `symphony-${config.taskId}`,
  ];
  if (config.model) args.push("--model", config.model);
  args.push(`@${config.promptFile}`, "Carry out the task described in the attached file.");

  const child = spawn("pi", args, {
    cwd: config.repoDir,
    env: { ...process.env, PI_OFFLINE: "1" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const tail: string[] = [];
  pump(child.stderr, tail);

  let inputTokens = 0;
  let outputTokens = 0;
  let failed = false;
  const commits: Promise<boolean>[] = [];
  const started = new Map<string, { name: string; args: Record<string, unknown> }>();

  // Pi's JSONL framing is LF only; readline also splits on Unicode separators, which the docs warn
  // about, but a JSON string never contains an unescaped LF, so line-splitting on `\n` is what we do.
  let buffer = "";
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk: string) => {
    buffer += chunk;
    let newline = buffer.indexOf("\n");
    while (newline >= 0) {
      const line = buffer.slice(0, newline).replace(/\r$/, "");
      buffer = buffer.slice(newline + 1);
      newline = buffer.indexOf("\n");
      if (!line) continue;
      let event: Record<string, any>;
      try {
        event = JSON.parse(line);
      } catch {
        tail.push(line);
        continue;
      }
      switch (event.type) {
        case "tool_execution_start":
          started.set(event.toolCallId, { name: event.toolName, args: event.args ?? {} });
          emit({
            type: "tool",
            callId: event.toolCallId,
            name: event.toolName,
            kind: toolKind(event.toolName),
            status: "started",
            input: event.args,
          });
          break;
        case "tool_execution_end": {
          const call = started.get(event.toolCallId);
          const name: string = event.toolName ?? call?.name ?? "tool";
          const output = (event.result?.content ?? [])
            .map((part: { text?: string }) => part.text ?? "")
            .join("");
          emit({
            type: "tool",
            callId: event.toolCallId,
            name,
            kind: toolKind(name),
            status: event.isError ? "failed" : "succeeded",
            output: clip(output),
            durationMs: event.durationMs,
          });
          // Every edit reaches the branch: commit and push as soon as a changing tool finished.
          if (!event.isError && (name === "bash" || name === "edit" || name === "write")) {
            const path = typeof call?.args.path === "string" ? ` ${call.args.path}` : "";
            commits.push(autocommit(`${config.issue}: ${name}${path}`));
          }
          break;
        }
        case "message_end":
          if (event.message?.role === "assistant") {
            const text = (Array.isArray(event.message.content) ? event.message.content : [])
              .filter((part: { type?: string }) => part.type === "text")
              .map((part: { text?: string }) => part.text ?? "")
              .join("\n")
              .trim();
            if (text) emit({ type: "output", text: clip(text) });
            const usage = event.message.usage;
            if (usage) {
              inputTokens += Number(usage.input ?? 0);
              outputTokens += Number(usage.output ?? 0);
              emit({ type: "usage", inputTokens, outputTokens });
            }
            if (event.message.stopReason === "error") failed = true;
          }
          break;
        default:
          break;
      }
    }
  });

  const code = await exitCodeOf(child);
  await Promise.all(commits);
  if (code === 0 && !failed) return EXIT.ok;
  emit({ type: "output", text: clip(`pi ended with exit code ${code}${tail.length ? `\n${tail.join("\n")}` : ""}`) });
  return EXIT.agentFailed;
}

async function runPiDurable(): Promise<number> {
  const { runPiDurableAgent } = await import("./pi-durable-agent.ts");
  return (await runPiDurableAgent()) ? EXIT.ok : EXIT.agentFailed;
}

async function main(): Promise<number> {
  // Work left by an interrupted run (a restored sandbox) goes to the branch first.
  await autocommit(`${config.issue}: save work from the interrupted run`);

  let code: number;
  switch (config.agent) {
    case "mini_swe_agent":
      code = await runMini();
      break;
    case "pi":
      code = await runPi();
      break;
    case "pi_durable":
      code = await runPiDurable();
      break;
    default:
      emit({ type: "output", text: `unknown agent ${config.agent}` });
      code = EXIT.agentFailed;
  }

  // Whatever the agent left behind is pushed, successful or not.
  await autocommit(`${config.issue}: ${config.agent} ${code === EXIT.ok ? "finished" : "stopped"}`);
  if (code === EXIT.ok && !lastAutocommitSucceeded()) return EXIT.pushFailed;
  return code;
}

main().then(
  (code) => process.exit(code),
  (error: unknown) => {
    emit({ type: "output", text: `driver crashed: ${(error as Error).message}` });
    process.exit(EXIT.agentFailed);
  },
);
