import { createInterface } from "node:readline";
import { execute } from "./entry.ts";
import { CodeSandboxGateway } from "./gateway.ts";
import { redact, type BridgeEvent } from "./protocol.ts";

/**
 * Entry point of `dist/bridge.mjs`: reads one request line from stdin, writes event lines to
 * stdout (nothing else goes there) and exits 0 when the request succeeded. The request handling
 * itself is `execute` in `entry.ts`.
 */
function write(event: BridgeEvent): void {
  process.stdout.write(`${JSON.stringify(event)}\n`);
}

async function main(): Promise<number> {
  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
  const first = await lines[Symbol.asyncIterator]().next();
  if (first.done || !first.value.trim()) {
    write({ type: "error", code: "empty_request", message: "No request was sent on stdin" });
    return 2;
  }
  // The host keeps stdin open while it waits; EOF means it is gone and the turn must stop.
  const hostGone = new Promise<void>((resolve) => {
    lines.once("close", resolve);
  });
  return execute(first.value, new CodeSandboxGateway(), write, hostGone);
}

main()
  .then((code) => {
    process.exitCode = code;
    // Open SDK connections must not keep the process alive after the request is done, but queued
    // events (the last one is `turn_completed`) must be written first: the callback runs once
    // everything written before it has been flushed.
    process.stdout.write("", () => process.exit(code));
    setTimeout(() => process.exit(code), 10_000).unref();
  })
  .catch((error: unknown) => {
    // No request credentials are in scope here; mask the token shapes that could still be in a message.
    write({
      type: "error",
      code: "bridge_crashed",
      message: redact(error instanceof Error ? error.message : String(error), []),
    });
    process.exit(1);
  });
