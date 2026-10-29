import { describe, expect, it } from "vitest";
import { execute } from "../src/entry.ts";
import type { OpenOptions, SandboxGateway, SandboxHandle } from "../src/gateway.ts";
import type { BridgeEvent } from "../src/protocol.ts";

const GIT_TOKEN = "ghp_exampleexampleexampleexample1234";
const CSB_KEY = "csb_v1_exampleexampleexample12345678";
const MODEL_KEY = "sk-ant-exampleexampleexample1234";

const request = (extra: Record<string, unknown> = {}) =>
  JSON.stringify({
    type: "turn",
    csbApiKey: CSB_KEY,
    templateId: "symphony-agents",
    taskId: "task-1",
    issueIdentifier: "SYM-1",
    turnId: "turn-1",
    prompt: "Fix it",
    agent: "mini_swe_agent",
    repoUrl: "https://github.com/acme/widgets.git",
    branch: "symphony/sym-1",
    baseBranch: "main",
    vmTier: "small",
    git: { token: GIT_TOKEN, name: "Bot", email: "bot@example.com" },
    env: { ANTHROPIC_API_KEY: MODEL_KEY },
    ...extra,
  });

const never = new Promise<void>(() => {});

function run(line: string, gateway: SandboxGateway) {
  const events: BridgeEvent[] = [];
  return execute(line, gateway, (e) => events.push(e), never).then((code) => ({ code, events }));
}

/** A sandbox whose `dispose` throws, which `runTurn` does not catch: the unexpected-error path. */
function explodingHandle(message: string): SandboxHandle {
  return {
    id: "sb-1",
    resumed: false,
    workspacePath: "/project/sandbox",
    writeTextFile: async () => {},
    run: async () => ({
      onOutput: () => {},
      wait: async () => 0,
      kill: async () => {},
    }),
    hibernate: async () => {},
    shutdown: async () => {},
    dispose: () => {
      throw new Error(message);
    },
  };
}

const gatewayFor = (handle: SandboxHandle): SandboxGateway => ({
  open: async (_options: OpenOptions) => handle,
  attach: async () => handle,
});

describe("execute", () => {
  it("masks credentials in an unexpected error instead of printing it raw", async () => {
    const leak = `connection failed for ${CSB_KEY}, pushing with ${GIT_TOKEN} and model key ${MODEL_KEY}`;

    const { code, events } = await run(request(), gatewayFor(explodingHandle(leak)));

    expect(code).toBe(1);
    const error = events.find((e) => e.type === "error");
    expect(error).toMatchObject({ code: "bridge_failed" });
    const printed = JSON.stringify(events);
    for (const secret of [CSB_KEY, GIT_TOKEN, MODEL_KEY]) expect(printed).not.toContain(secret);
    expect(printed).toContain("[REDACTED]");
  });

  it("rejects an invalid request with exit code 2 and without echoing credentials", async () => {
    const { code, events } = await run(request({ agent: "gpt-engineer" }), gatewayFor(explodingHandle("unused")));

    expect(code).toBe(2);
    expect(events).toEqual([{ type: "error", code: "invalid_request", message: expect.stringContaining("agent") }]);
    for (const secret of [CSB_KEY, GIT_TOKEN, MODEL_KEY]) expect(JSON.stringify(events)).not.toContain(secret);
  });

  it("reports a control request that fails instead of crashing", async () => {
    const stop = JSON.stringify({ type: "stop", csbApiKey: CSB_KEY, sandboxId: "sb-1", mode: "hibernate" });
    const { code, events } = await run(stop, gatewayFor(explodingHandle(`dispose failed for ${CSB_KEY}`)));
    // `dispose` throws after the hibernate: the failure is reported (masked), not swallowed or crashed on.
    expect(JSON.stringify(events)).not.toContain(CSB_KEY);
    expect(code).toBe(1);
    expect(events.at(-1)).toMatchObject({ type: "error" });
  });
});
