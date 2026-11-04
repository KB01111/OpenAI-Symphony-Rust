import { describe, expect, it } from "vitest";
import { SdkHandle } from "../src/gateway.ts";

/** A command the way the SDK's `Command` behaves: it emits chunks to whoever subscribed. */
function fakeCommand() {
  const listeners: Array<(chunk: string) => void> = [];
  let finish: (code: number) => void = () => {};
  const done = new Promise<number>((resolve) => {
    finish = resolve;
  });
  return {
    onOutput: (listener: (chunk: string) => void) => {
      listeners.push(listener);
    },
    // What the SDK really returns: every chunk it recorded, joined with "\n".
    recorded: [] as string[],
    waitUntilComplete: async function (this: { recorded: string[] }) {
      const code = await done;
      if (code !== 0) {
        throw Object.assign(new Error(`Command failed with exit code ${code}`), {
          exitCode: code,
          output: this.recorded.join("\n"),
        });
      }
      return this.recorded.join("\n");
    },
    kill: async () => {},
    emit(chunk: string) {
      this.recorded.push(chunk);
      for (const listener of listeners) listener(chunk);
    },
    exit: (code: number) => finish(code),
  };
}

function handleFor(command: ReturnType<typeof fakeCommand>) {
  const client = {
    workspacePath: "/project/sandbox",
    commands: { runBackground: async () => command },
    dispose: () => {},
  };
  // The SDK object and sandbox are not used by `run`.
  return new SdkHandle({} as never, { id: "sb-1" } as never, client as never, false);
}

describe("SdkHandle.run output", () => {
  it("hands over output that arrived before the caller subscribed first, then the live output, in order", async () => {
    const command = fakeCommand();
    const running = await handleFor(command).run("bash run.sh", {});

    // The SDK delivered these while the bridge had not subscribed yet...
    command.emit('@@SYMPHONY {"type":"usage","inputTokens":1,"outputTokens":1}\n');
    command.emit("second line\n");
    const received: string[] = [];
    running.onOutput((chunk) => received.push(chunk));
    // ...and this one afterwards.
    command.emit('@@SYMPHONY {"type":"pushed","sha":"deadbeef12","branch":"b"}\n');
    command.exit(0);

    expect(await running.wait()).toBe(0);
    // Every chunk exactly once and in the order the SDK produced them; the SDK's own record (which would
    // add newlines of its own between chunks) is not spliced in.
    expect(received).toEqual([
      '@@SYMPHONY {"type":"usage","inputTokens":1,"outputTokens":1}\n',
      "second line\n",
      '@@SYMPHONY {"type":"pushed","sha":"deadbeef12","branch":"b"}\n',
    ]);
  });

  it("reports a failing command by its exit code without repeating any output", async () => {
    const command = fakeCommand();
    const running = await handleFor(command).run("bash run.sh", {});
    const received: string[] = [];
    running.onOutput((chunk) => received.push(chunk));
    command.emit("boom\n");
    command.exit(10);

    expect(await running.wait()).toBe(10);
    expect(received).toEqual(["boom\n"]);
  });

  it("passes on an error that is not a command failure", async () => {
    const command = fakeCommand();
    command.waitUntilComplete = async () => {
      throw new Error("connection lost");
    };
    const running = await handleFor(command).run("bash run.sh", {});
    await expect(running.wait()).rejects.toThrow("connection lost");
  });
});
