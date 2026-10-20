import { discoverRuntimeEndpoint, type RuntimeEndpoint } from "../transport/discovery.js";
import { isTauri, nativeHost } from "../transport/tauri.js";

/** Path prefix of every runtime route (`/api/v1/*`). The OpenAPI document uses absolute paths. */
export const API_BASE_PATH = "/api/v1";

/** Default per-request timeout. Streaming requests are not subject to it. */
export const DEFAULT_TIMEOUT_MS = 20_000;

/**
 * Where a client should connect:
 * - `desktop`: the runtime owned by the Tauri host (embedded or attached), via IPC.
 * - `local`: a runtime on this machine; an explicit `baseUrl`/`token`, otherwise the Vite
 *   dev-server discovery route (browser development and E2E tests).
 * - `remote`: an explicitly configured runtime, e.g. the mobile companion over remote access.
 */
export type RuntimeTarget =
  | { kind: "desktop" }
  | { kind: "local"; baseUrl?: string; token?: string | null }
  | { kind: "remote"; baseUrl: string; token?: string | null };

/** The single connection description every `SymphonyClient` is built from. */
export interface ApiConfig {
  /** Origin of the runtime, without `/api/v1` and without a trailing slash. */
  baseUrl: string;
  token: string | null;
  /** How the endpoint was obtained; selects the event transport (Tauri channel vs. SSE). */
  endpoint: Pick<RuntimeEndpoint, "source" | "mode">;
  timeoutMs: number;
  fetchImpl?: typeof fetch;
}

export interface ApiConfigOptions {
  fetchImpl?: typeof fetch;
  timeoutMs?: number;
}

/** Trim whitespace, trailing slashes and an accidental `/api/v1` suffix. */
export function normalizeBaseUrl(url: string): string {
  let out = url.trim().replace(/\/+$/, "");
  if (out.endsWith(API_BASE_PATH)) out = out.slice(0, -API_BASE_PATH.length).replace(/\/+$/, "");
  return out;
}

function fromEndpoint(endpoint: RuntimeEndpoint, options: ApiConfigOptions): ApiConfig {
  return {
    baseUrl: normalizeBaseUrl(endpoint.baseUrl),
    token: endpoint.token || null,
    endpoint: { source: endpoint.source, mode: endpoint.mode },
    timeoutMs: options.timeoutMs ?? DEFAULT_TIMEOUT_MS,
    fetchImpl: options.fetchImpl
  };
}

/** Configuration for an explicitly addressed runtime (no discovery). */
export function configuredApiConfig(
  baseUrl: string,
  token: string | null | undefined,
  mode: "attached" | "remote",
  options: ApiConfigOptions = {}
): ApiConfig {
  return fromEndpoint({ baseUrl, token: token ?? null, source: "configured", mode }, options);
}

/** Resolve a target to a connection, discovering desktop and local runtimes when needed. */
export async function resolveApiConfig(target: RuntimeTarget, options: ApiConfigOptions = {}): Promise<ApiConfig> {
  switch (target.kind) {
    case "desktop": {
      if (!isTauri()) throw new Error("The desktop runtime is only reachable from the Symphony desktop app");
      const cfg = await nativeHost.getLocalApiConfig();
      return fromEndpoint({ baseUrl: cfg.base_url, token: cfg.token, source: "tauri", mode: cfg.mode }, options);
    }
    case "local":
      return target.baseUrl
        ? configuredApiConfig(target.baseUrl, target.token, "attached", options)
        : fromEndpoint(await discoverRuntimeEndpoint(options.fetchImpl), options);
    case "remote":
      return configuredApiConfig(target.baseUrl, target.token, "remote", options);
  }
}

/** The target an app running in this environment should use by default. */
export function defaultRuntimeTarget(): RuntimeTarget {
  return isTauri() ? { kind: "desktop" } : { kind: "local" };
}
