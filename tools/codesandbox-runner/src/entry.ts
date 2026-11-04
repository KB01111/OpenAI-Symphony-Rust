import { redactDeep, runRequest } from "./bridge.ts";
import type { SandboxGateway } from "./gateway.ts";
import { parseRequest, RequestError, secretsOf, type BridgeEvent } from "./protocol.ts";

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Handle one request line and return the process exit code (0 = done, 1 = the request failed,
 * 2 = the request itself was invalid). `emit` receives every event; whatever the request's own
 * credentials are, none of them can reach it, not even through an unexpected error.
 */
export async function execute(
  line: string,
  gateway: SandboxGateway,
  emit: (event: BridgeEvent) => void,
  hostGone: Promise<void>,
): Promise<number> {
  let request;
  try {
    request = parseRequest(line);
  } catch (error) {
    const message = error instanceof RequestError ? error.message : "invalid request";
    emit({ type: "error", code: "invalid_request", message });
    return 2;
  }

  const secrets = secretsOf(request);
  const safeEmit = (event: BridgeEvent) => emit(redactDeep(event, secrets));
  try {
    const ok = await runRequest(request, gateway, { emit: safeEmit }, { hostGone });
    return ok ? 0 : 1;
  } catch (error) {
    safeEmit({ type: "error", code: "bridge_failed", message: describe(error) });
    return 1;
  }
}
