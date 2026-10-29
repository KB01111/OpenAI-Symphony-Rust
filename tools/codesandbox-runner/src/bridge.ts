import type { RunningCommand, SandboxGateway, SandboxHandle } from "./gateway.ts";
import {
  parseDriverLine,
  redact,
  secretsOf,
  type BridgeEvent,
  type BridgeRequest,
  type ControlRequest,
  type TurnRequest,
} from "./protocol.ts";

/** Where the template installs the drivers (see `template/.codesandbox/Dockerfile`). */
export const DRIVER_DIR = "/opt/symphony/drivers";
/** Marker in the driver's command line, so `abort` can find it (`[s]` keeps pkill from matching itself). */
const DRIVER_PROCESS_PATTERN = "[s]ymphony-run-agent";
const SCRATCH_DIR = "/tmp/symphony";

export interface BridgeOutput {
  emit(event: BridgeEvent): void;
}

export interface TurnControl {
  /** Resolves when the host closed its side: the turn must stop. */
  readonly hostGone: Promise<void>;
}

/** Splits streamed chunks into lines; `flush` returns the unterminated rest. */
export class LineSplitter {
  private rest = "";

  push(chunk: string): string[] {
    const text = this.rest + chunk;
    const lines = text.split(/\r?\n/);
    this.rest = lines.pop() ?? "";
    return lines;
  }

  flush(): string[] {
    const rest = this.rest;
    this.rest = "";
    return rest ? [rest] : [];
  }
}

/** `value` with every string in it masked (see {@link redact}). */
export function redactDeep<T>(value: T, secrets: readonly string[]): T {
  if (typeof value === "string") return redact(value, secrets) as T;
  if (Array.isArray(value)) return value.map((v) => redactDeep(v, secrets)) as T;
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([k, v]) => [k, redactDeep(v, secrets)]),
    ) as T;
  }
  return value;
}

/** Environment of the in-sandbox driver; the prompt travels in a file, credentials in the session. */
export function driverEnv(request: TurnRequest, workspacePath: string, promptFile: string): Record<string, string> {
  const env: Record<string, string> = {
    ...request.env,
    SYMPHONY_AGENT: request.agent,
    SYMPHONY_TASK_ID: request.taskId,
    SYMPHONY_ISSUE: request.issueIdentifier,
    SYMPHONY_TURN_ID: request.turnId,
    SYMPHONY_PROMPT_FILE: promptFile,
    SYMPHONY_REPO_URL: request.repoUrl,
    SYMPHONY_BRANCH: request.branch,
    SYMPHONY_BASE_BRANCH: request.baseBranch,
    SYMPHONY_REPO_DIR: `${workspacePath}/symphony-repo`,
    SYMPHONY_COST_LIMIT_USD: String(request.costLimitUsd),
    SYMPHONY_GIT_NAME: request.git.name,
    SYMPHONY_GIT_EMAIL: request.git.email,
  };
  if (request.model) env.SYMPHONY_MODEL = request.model;
  return env;
}

export async function runTurn(
  request: TurnRequest,
  gateway: SandboxGateway,
  out: BridgeOutput,
  control: TurnControl,
): Promise<boolean> {
  const secrets = secretsOf(request);
  const emit = (event: BridgeEvent) => out.emit(redactDeep(event, secrets));

  let handle: SandboxHandle | undefined;
  try {
    handle = await gateway.open({
      apiKey: request.csbApiKey,
      templateId: request.templateId,
      sandboxId: request.sandboxId,
      vmTier: request.vmTier,
      hibernationTimeoutSecs: request.hibernationTimeoutSecs,
      title: `Symphony ${request.issueIdentifier}`,
      git: request.git,
      env: request.env,
    });
  } catch (error) {
    emit({
      type: "error",
      code: "codesandbox_open_failed",
      message: `Could not start the CodeSandbox sandbox: ${(error as Error).message}`,
    });
    emit({ type: "turn_completed", succeeded: false, errorCode: "codesandbox_open_failed" });
    return false;
  }

  let command: RunningCommand | undefined;
  let timedOut = false;
  let hostGone = false;
  try {
    emit({ type: "started", sandboxId: handle.id, resumed: handle.resumed });

    const promptFile = `${SCRATCH_DIR}/prompt-${request.turnId}.txt`;
    await (await handle.run(`mkdir -p ${SCRATCH_DIR}`, { name: "prepare" })).wait();
    await handle.writeTextFile(promptFile, request.prompt);

    command = await handle.run(`bash ${DRIVER_DIR}/run.sh`, {
      env: driverEnv(request, handle.workspacePath, promptFile),
      name: `symphony-${request.turnId}`,
    });

    const splitter = new LineSplitter();
    command.onOutput((chunk) => {
      for (const line of splitter.push(chunk)) emit(parseDriverLine(line));
    });

    const timer = setTimeout(() => {
      timedOut = true;
      void command?.kill();
    }, Math.max(request.turnTimeoutMs, 5000));
    void control.hostGone.then(() => {
      hostGone = true;
      void command?.kill();
    });

    let exitCode: number;
    try {
      exitCode = await command.wait();
    } finally {
      clearTimeout(timer);
    }
    for (const line of splitter.flush()) emit(parseDriverLine(line));

    if (timedOut) {
      emit({ type: "error", code: "turn_timeout", message: `The cloud agent did not finish within ${request.turnTimeoutMs} ms` });
      emit({ type: "turn_completed", succeeded: false, errorCode: "turn_timeout" });
      return false;
    }
    if (hostGone) {
      emit({ type: "turn_completed", succeeded: false, errorCode: "host_closed", message: "Symphony stopped waiting for the turn" });
      return false;
    }
    if (exitCode !== 0) {
      emit({
        type: "turn_completed",
        succeeded: false,
        errorCode: `agent_exit_${exitCode}`,
        message: `The cloud agent exited with code ${exitCode}`,
      });
      return false;
    }
    emit({ type: "turn_completed", succeeded: true });
    return true;
  } catch (error) {
    emit({ type: "error", code: "codesandbox_run_failed", message: (error as Error).message });
    emit({ type: "turn_completed", succeeded: false, errorCode: "codesandbox_run_failed" });
    return false;
  } finally {
    handle.dispose();
  }
}

/** `abort`: stop the agent process in a running sandbox (dropping the bridge does not). `stop`: hibernate / shut down. */
export async function runControl(request: ControlRequest, gateway: SandboxGateway): Promise<boolean> {
  const handle = await gateway.attach(request.csbApiKey, request.sandboxId);
  if (!handle) return false;
  try {
    if (request.type === "abort") {
      const kill = await handle.run(`pkill -f '${DRIVER_PROCESS_PATTERN}' || true`, { name: "abort" });
      await kill.wait();
      return true;
    }
    if (request.mode === "shutdown") await handle.shutdown();
    else await handle.hibernate();
    return true;
  } finally {
    handle.dispose();
  }
}

export async function runRequest(
  request: BridgeRequest,
  gateway: SandboxGateway,
  out: BridgeOutput,
  control: TurnControl,
): Promise<boolean> {
  if (request.type === "turn") return runTurn(request, gateway, out, control);
  const secrets = secretsOf(request);
  try {
    return await runControl(request, gateway);
  } catch (error) {
    out.emit({ type: "error", code: `${request.type}_failed`, message: redact((error as Error).message, secrets) });
    return false;
  }
}
