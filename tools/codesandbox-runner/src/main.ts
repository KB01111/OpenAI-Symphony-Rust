import { createInterface } from "node:readline";
import { runRequest } from "./bridge.ts";
import { CodeSandboxGateway } from "./gateway.ts";
import { parseRequest, RequestError, type BridgeEvent } from "./protocol.ts";

/**
 * Entry point of `dist/bridge.mjs`: reads one request line from stdin, writes event lines to
 * stdout (nothing else goes there) and exits 0 when the request succeeded.
 */
function write(event: BridgeEvent): void {
  process.stdout.write(`${JSON.stringify(event)}\n`);
}

async function main(): Promise<number> {
  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
  const iterator = lines[Symbol.asyncIterator]();
  const first = await iterator.next();
  if (first.done || !first.value.trim()) {
    write({ type: "error", code: "empty_request", message: "No request was sent on stdin" });
    return 2;
  }

  let request;
  try {
    request = parseRequest(first.value);
  } catch (error) {
    const message = error instanceof RequestError ? error.message : "invalid request";
    write({ type: "error", code: "invalid_request", message });
    return 2;
  }

  // The host keeps stdin open while it waits; EOF means it is gone and the turn must stop.
  const hostGone = new Promise<void>((resolve) => {
    lines.once("close", resolve);
  });

  const ok = await runRequest(request, new CodeSandboxGateway(), { emit: write }, { hostGone });
  return ok ? 0 : 1;
}

main()
  .then((code) => {
    process.exitCode = code;
    // Open SDK connections must not keep the process alive after the request is done.
    setTimeout(() => process.exit(code), 100).unref();
  })
  .catch((error: unknown) => {
    write({ type: "error", code: "bridge_crashed", message: (error as Error).message });
    process.exit(1);
  });
