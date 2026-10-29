import { describe, expect, it } from "vitest";
import { driverEnv, LineSplitter, runControl, runRequest, runTurn, DRIVER_DIR } from "../src/bridge.ts";
import type { OpenOptions, RunningCommand, SandboxGateway, SandboxHandle } from "../src/gateway.ts";
import { parseDriverLine, parseRequest, redact, type BridgeEvent, type TurnRequest } from "../src/protocol.ts";

const GIT_TOKEN = "ghp_exampleexampleexampleexample1234";
const CSB_KEY = "csb_v1_exampleexampleexample12345678";
const MODEL_KEY = "sk-ant-exampleexampleexample1234";

function turnRequest(overrides: Partial<TurnRequest> = {}): TurnRequest {
  return {
    type: "turn",
    csbApiKey: CSB_KEY,
    templateId: "symphony-agents",
    sandboxId: null,
    taskId: "task-1",
    issueIdentifier: "SYM-1",
    turnId: "turn-1",
    prompt: "Fix the flaky test",
    agent: "mini_swe_agent",
    model: "anthropic/claude-sonnet-5-5",
    repoUrl: "https://github.com/acme/widgets.git",
    branch: "symphony/sym-1",
    baseBranch: "main",
    vmTier: "small",
    hibernationTimeoutSecs: 1800,
    costLimitUsd: 3,
    turnTimeoutMs: 60_000,
    git: { token: GIT_TOKEN, name: "Symphony Cloud Agent", email: "bot@example.com" },
    env: { ANTHROPIC_API_KEY: MODEL_KEY },
    ...overrides,
  };
}

class FakeCommand implements RunningCommand {
  killed = false;
  private listener: (chunk: string) => void = () => {};
  private finish: (code: number) => void = () => {};
  private readonly done = new Promise<number>((resolve) => {
    this.finish = resolve;
  });

  onOutput(listener: (chunk: string) => void): void {
    this.listener = listener;
  }
  wait(): Promise<number> {
    return this.done;
  }
  async kill(): Promise<void> {
    this.killed = true;
    this.finish(137);
  }
  print(chunk: string): void {
    this.listener(chunk);
  }
  exit(code: number): void {
    this.finish(code);
  }
}

interface RunCall {
  command: string;
  env?: Record<string, string>;
  cwd?: string;
  handle: FakeCommand;
}

class FakeHandle implements SandboxHandle {
  readonly id = "sb-123";
  readonly workspacePath = "/project/sandbox";
  readonly files = new Map<string, string>();
  readonly runs: RunCall[] = [];
  hibernated = 0;
  shutdowns = 0;
  disposed = 0;
  /** Called for every command that is not the driver run, to finish it. */
  script: (call: RunCall) => void = (call) => call.handle.exit(0);

  constructor(readonly resumed = false) {}

  async writeTextFile(path: string, content: string): Promise<void> {
    this.files.set(path, content);
  }
  async run(command: string, options: { cwd?: string; env?: Record<string, string> }): Promise<RunningCommand> {
    const call: RunCall = { command, env: options.env, cwd: options.cwd, handle: new FakeCommand() };
    this.runs.push(call);
    // After the caller had the chance to subscribe, as with the real SDK adapter.
    setTimeout(() => this.script(call), 0);
    return call.handle;
  }
  async hibernate(): Promise<void> {
    this.hibernated += 1;
  }
  async shutdown(): Promise<void> {
    this.shutdowns += 1;
  }
  dispose(): void {
    this.disposed += 1;
  }
}

class FakeGateway implements SandboxGateway {
  opened: OpenOptions[] = [];
  constructor(
    readonly handle: FakeHandle | null,
    private readonly openError?: Error,
  ) {}
  async open(options: OpenOptions): Promise<SandboxHandle> {
    this.opened.push(options);
    if (this.openError) throw this.openError;
    return this.handle as FakeHandle;
  }
  async attach(): Promise<SandboxHandle | null> {
    return this.handle;
  }
}

const never = new Promise<void>(() => {});

function collect() {
  const events: BridgeEvent[] = [];
  return { events, out: { emit: (event: BridgeEvent) => events.push(event) } };
}

/** The driver run is the command that starts the driver; everything else finishes immediately. */
function driverScript(handle: FakeHandle, behave: (driver: FakeCommand) => void) {
  handle.script = (call) => {
    if (call.command.includes("run.sh")) behave(call.handle);
    else call.handle.exit(0);
  };
}

describe("parseRequest", () => {
  const valid = turnRequest();

  it("accepts a complete turn request", () => {
    const parsed = parseRequest(JSON.stringify(valid));
    expect(parsed).toMatchObject({ type: "turn", agent: "mini_swe_agent", vmTier: "small", sandboxId: null });
  });

  it("defaults the model-less case and keeps the sandbox id for a resume", () => {
    const parsed = parseRequest(JSON.stringify({ ...valid, model: "", sandboxId: "sb-9" })) as TurnRequest;
    expect(parsed.model).toBeNull();
    expect(parsed.sandboxId).toBe("sb-9");
  });

  it.each([
    ["an unknown agent", { agent: "gpt-engineer" }],
    ["an unknown VM tier", { vmTier: "huge" }],
    ["credentials inside the repository URL", { repoUrl: `https://x:${GIT_TOKEN}@github.com/acme/widgets.git` }],
    ["a non-https repository URL", { repoUrl: "git@github.com:acme/widgets.git" }],
    ["a branch that looks like an option", { branch: "--upload-pack=x" }],
    ["an env name that is not a variable name", { env: { "BAD NAME": "x" } }],
    ["a missing prompt", { prompt: "" }],
  ])("rejects %s without echoing the credentials", (_label, patch) => {
    let message = "";
    try {
      parseRequest(JSON.stringify({ ...valid, ...patch }));
    } catch (error) {
      message = (error as Error).message;
    }
    expect(message).not.toBe("");
    expect(message).not.toContain(GIT_TOKEN);
    expect(message).not.toContain(CSB_KEY);
  });

  it("rejects input that is not JSON", () => {
    expect(() => parseRequest("not json")).toThrow(/not valid JSON/);
  });

  it("parses control requests", () => {
    expect(parseRequest(JSON.stringify({ type: "stop", csbApiKey: CSB_KEY, sandboxId: "sb-1", mode: "hibernate" }))).toMatchObject({
      type: "stop",
      mode: "hibernate",
    });
    expect(() => parseRequest(JSON.stringify({ type: "stop", csbApiKey: CSB_KEY, sandboxId: "sb-1", mode: "delete" }))).toThrow(/mode/);
  });
});

describe("driver lines and splitting", () => {
  it("turns structured lines into events and anything else into output", () => {
    expect(parseDriverLine('@@SYMPHONY {"type":"pushed","sha":"deadbeef12","branch":"b"}')).toEqual({ type: "pushed", sha: "deadbeef12", branch: "b" });
    expect(parseDriverLine("just a line")).toEqual({ type: "output", text: "just a line" });
    // Any process in the sandbox can print this prefix, and a push makes the host update the worktree:
    // only a hex commit id counts as one.
    for (const sha of ['"ééééééééééééé"', '"not-a-sha"', '"abc"', "12345", "null", '"deadbeef; rm -rf /"']) {
      const line = `@@SYMPHONY {"type":"pushed","sha":${sha},"branch":"b"}`;
      expect(parseDriverLine(line), sha).toEqual({ type: "output", text: line });
    }
    expect(parseDriverLine('@@SYMPHONY {"type":"pushed","branch":"b"}')).toMatchObject({ type: "output" });
    // A structured prefix with a body the bridge does not accept stays visible as plain output.
    expect(parseDriverLine('@@SYMPHONY {"type":"turn_completed","succeeded":true}')).toMatchObject({ type: "output" });
    expect(parseDriverLine("@@SYMPHONY {broken")).toMatchObject({ type: "output" });
  });

  it("reassembles lines split across chunks", () => {
    const splitter = new LineSplitter();
    expect(splitter.push("one\ntw")).toEqual(["one"]);
    expect(splitter.push("o\r\nthree")).toEqual(["two"]);
    expect(splitter.flush()).toEqual(["three"]);
    expect(splitter.flush()).toEqual([]);
  });

  it("masks credentials, including token shapes that were not in the request", () => {
    const masked = redact(`a ${GIT_TOKEN} b ghp_otherotherotherother1234567 c`, [GIT_TOKEN]);
    expect(masked).toBe("a [REDACTED] b [REDACTED] c");
  });
});

describe("runTurn", () => {
  it("runs the driver, streams its events in order and completes", async () => {
    const handle = new FakeHandle();
    driverScript(handle, (driver) => {
      driver.print('@@SYMPHONY {"type":"tool","callId":"c1","name":"bash","kind":"command","status":"succeeded","input":{"command":"ls"}}\n');
      driver.print('@@SYMPHONY {"type":"pu');
      driver.print('shed","sha":"deadbeef","branch":"symphony/sym-1"}\n');
      driver.print("thinking out loud\n");
      driver.exit(0);
    });
    const { events, out } = collect();

    const ok = await runTurn(turnRequest(), new FakeGateway(handle), out, { hostGone: never });

    expect(ok).toBe(true);
    expect(events.map((e) => e.type)).toEqual(["started", "tool", "pushed", "output", "turn_completed"]);
    expect(events[0]).toEqual({ type: "started", sandboxId: "sb-123", resumed: false });
    expect(events[2]).toEqual({ type: "pushed", sha: "deadbeef", branch: "symphony/sym-1" });
    expect(events.at(-1)).toEqual({ type: "turn_completed", succeeded: true });
    expect(handle.disposed).toBe(1);
  });

  it("hands the prompt over in a file and the settings in the driver environment", async () => {
    const handle = new FakeHandle();
    driverScript(handle, (driver) => driver.exit(0));
    const { out } = collect();

    await runTurn(turnRequest({ agent: "pi_durable" }), new FakeGateway(handle), out, { hostGone: never });

    expect(handle.files.get("/tmp/symphony/prompt-turn-1.txt")).toBe("Fix the flaky test");
    const driver = handle.runs.find((r) => r.command.includes("run.sh"))!;
    expect(driver.command).toBe(`bash ${DRIVER_DIR}/run.sh`);
    expect(driver.env).toMatchObject({
      SYMPHONY_AGENT: "pi_durable",
      SYMPHONY_BRANCH: "symphony/sym-1",
      SYMPHONY_REPO_URL: "https://github.com/acme/widgets.git",
      SYMPHONY_REPO_DIR: "/project/sandbox/symphony-repo",
      SYMPHONY_PROMPT_FILE: "/tmp/symphony/prompt-turn-1.txt",
      SYMPHONY_MODEL: "anthropic/claude-sonnet-5-5",
      ANTHROPIC_API_KEY: MODEL_KEY,
    });
    // The prompt and the git token never travel in the environment.
    expect(Object.values(driver.env ?? {})).not.toContain("Fix the flaky test");
    expect(Object.values(driver.env ?? {})).not.toContain(GIT_TOKEN);
  });

  it("opens the sandbox with the template, tier and git credentials, and resumes by id", async () => {
    const handle = new FakeHandle(true);
    driverScript(handle, (driver) => driver.exit(0));
    const gateway = new FakeGateway(handle);
    const { events, out } = collect();

    await runTurn(turnRequest({ sandboxId: "sb-123", vmTier: "medium" }), gateway, out, { hostGone: never });

    expect(gateway.opened[0]).toMatchObject({
      templateId: "symphony-agents",
      sandboxId: "sb-123",
      vmTier: "medium",
      hibernationTimeoutSecs: 1800,
      git: { token: GIT_TOKEN },
    });
    expect(events[0]).toEqual({ type: "started", sandboxId: "sb-123", resumed: true });
  });

  it("never lets a credential reach an event, even when the agent prints it", async () => {
    const handle = new FakeHandle();
    driverScript(handle, (driver) => {
      driver.print(`remote: https://x-access-token:${GIT_TOKEN}@github.com\n`);
      driver.print(`@@SYMPHONY {"type":"tool","callId":"c","name":"bash","kind":"command","status":"succeeded","output":"${MODEL_KEY}"}\n`);
      driver.exit(0);
    });
    const { events, out } = collect();

    await runTurn(turnRequest(), new FakeGateway(handle), out, { hostGone: never });

    const serialized = JSON.stringify(events);
    for (const secret of [GIT_TOKEN, MODEL_KEY, CSB_KEY]) expect(serialized).not.toContain(secret);
    expect(serialized).toContain("[REDACTED]");
  });

  it("reports a failing agent with its exit code", async () => {
    const handle = new FakeHandle();
    driverScript(handle, (driver) => driver.exit(10));
    const { events, out } = collect();

    const ok = await runTurn(turnRequest(), new FakeGateway(handle), out, { hostGone: never });

    expect(ok).toBe(false);
    expect(events.at(-1)).toMatchObject({ type: "turn_completed", succeeded: false, errorCode: "agent_exit_10" });
  });

  it("reports a sandbox that cannot be started, without the API key", async () => {
    const gateway = new FakeGateway(null, new Error(`401 for ${CSB_KEY}`));
    const { events, out } = collect();

    const ok = await runTurn(turnRequest(), gateway, out, { hostGone: never });

    expect(ok).toBe(false);
    expect(events.map((e) => e.type)).toEqual(["error", "turn_completed"]);
    expect(JSON.stringify(events)).not.toContain(CSB_KEY);
    expect(events[0]).toMatchObject({ code: "codesandbox_open_failed" });
  });

  it("kills the agent when the turn times out", async () => {
    const handle = new FakeHandle();
    let driver!: FakeCommand;
    driverScript(handle, (d) => {
      driver = d; // never exits by itself
    });
    const { events, out } = collect();

    // 5 s is the floor of the bridge's own limit, so use fake timers for the wait.
    const { vi } = await import("vitest");
    vi.useFakeTimers();
    try {
      const turn = runTurn(turnRequest({ turnTimeoutMs: 1000 }), new FakeGateway(handle), out, { hostGone: never });
      await vi.advanceTimersByTimeAsync(5001);
      const ok = await turn;
      expect(ok).toBe(false);
      expect(driver.killed).toBe(true);
      expect(events.at(-1)).toMatchObject({ type: "turn_completed", succeeded: false, errorCode: "turn_timeout" });
    } finally {
      vi.useRealTimers();
    }
  });

  it("kills the agent when the host goes away", async () => {
    const handle = new FakeHandle();
    let driver!: FakeCommand;
    let gone!: () => void;
    const hostGone = new Promise<void>((resolve) => {
      gone = resolve;
    });
    driverScript(handle, (d) => {
      driver = d;
    });
    const { events, out } = collect();

    const turn = runTurn(turnRequest(), new FakeGateway(handle), out, { hostGone });
    // Wait until the agent is really running, then lose the host.
    while (!driver) await new Promise((resolve) => setTimeout(resolve, 5));
    await new Promise((resolve) => setTimeout(resolve, 5));
    gone();
    const ok = await turn;

    expect(ok).toBe(false);
    expect(driver.killed).toBe(true);
    expect(events.at(-1)).toMatchObject({ type: "turn_completed", errorCode: "host_closed" });
  });
});

describe("controls", () => {
  it("abort stops the driver and everything it started, not just the driver", async () => {
    const handle = new FakeHandle(true);
    const ok = await runControl({ type: "abort", csbApiKey: CSB_KEY, sandboxId: "sb-123" }, new FakeGateway(handle));
    expect(ok).toBe(true);
    expect(handle.runs[0].command).toBe(`bash ${DRIVER_DIR}/abort.sh`);
    expect(handle.disposed).toBe(1);
  });

  it("stop hibernates by default and can shut down", async () => {
    const handle = new FakeHandle(true);
    await runControl({ type: "stop", csbApiKey: CSB_KEY, sandboxId: "sb-123", mode: null }, new FakeGateway(handle));
    await runControl({ type: "stop", csbApiKey: CSB_KEY, sandboxId: "sb-123", mode: "shutdown" }, new FakeGateway(handle));
    expect([handle.hibernated, handle.shutdowns]).toEqual([1, 1]);
  });

  it("treats a sandbox that no longer exists as already stopped", async () => {
    const { events, out } = collect();
    const ok = await runRequest({ type: "abort", csbApiKey: CSB_KEY, sandboxId: "gone" }, new FakeGateway(null), out, { hostGone: never });
    expect(ok).toBe(true);
    expect(events).toEqual([]);
  });

  it("reports an abort that could not reach the sandbox instead of looking like a success", async () => {
    const unreachable: SandboxGateway = {
      open: async () => {
        throw new Error("unused");
      },
      attach: async () => {
        throw new Error(`Failed to start VM sb-1: Bad gateway (${CSB_KEY})`);
      },
    };
    const { events, out } = collect();
    const ok = await runRequest({ type: "abort", csbApiKey: CSB_KEY, sandboxId: "sb-1" }, unreachable, out, { hostGone: never });
    // The agent may still be running and pushing: the caller has to be told.
    expect(ok).toBe(false);
    expect(events).toEqual([{ type: "error", code: "abort_failed", message: expect.stringContaining("Bad gateway") }]);
    expect(JSON.stringify(events)).not.toContain(CSB_KEY);
  });
});

describe("driverEnv", () => {
  it("omits the model when none was chosen", () => {
    const env = driverEnv(turnRequest({ model: null }), "/project/sandbox", "/tmp/p.txt");
    expect("SYMPHONY_MODEL" in env).toBe(false);
    expect(env.SYMPHONY_COST_LIMIT_USD).toBe("3");
  });
});
