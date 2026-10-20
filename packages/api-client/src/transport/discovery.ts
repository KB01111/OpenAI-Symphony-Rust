import { isTauri, nativeHost } from "./tauri.js";

export interface RuntimeEndpoint {
  baseUrl: string;
  token: string | null;
  /** How the endpoint was found. */
  source: "tauri" | "dev-server" | "configured";
  /** `embedded`/`attached` in the desktop host; `remote` for explicitly configured endpoints. */
  mode: "embedded" | "attached" | "remote" | "dev";
}

/** Path served by the Vite dev-server plugin (development only) that exposes `runtime.json`. */
export const DEV_DISCOVERY_PATH = "/__symphony/runtime";

/**
 * Header the dev-server route requires. Navigations cannot send it and cross-origin scripts need
 * a CORS preflight for it, so only same-origin scripts can read the discovery record.
 */
export const DEV_DISCOVERY_HEADER = "X-Symphony-Discovery";

/**
 * Discover the local runtime endpoint:
 * 1. Desktop host IPC (`get_local_api_config`) — handles ephemeral fallback ports and the
 *    per-launch token.
 * 2. The Vite dev-server discovery route (browser development and E2E tests).
 * Throws when neither is available; remote clients must configure the endpoint explicitly.
 */
export async function discoverRuntimeEndpoint(fetchImpl: typeof fetch = fetch): Promise<RuntimeEndpoint> {
  if (isTauri()) {
    const cfg = await nativeHost.getLocalApiConfig();
    return {
      baseUrl: cfg.base_url.replace(/\/+$/, ""),
      token: cfg.token,
      source: "tauri",
      mode: cfg.mode
    };
  }
  const response = await fetchImpl(DEV_DISCOVERY_PATH, { cache: "no-store", headers: { [DEV_DISCOVERY_HEADER]: "1" } });
  if (!response.ok) {
    throw new Error(`Runtime discovery failed (HTTP ${response.status})`);
  }
  const body = (await response.json()) as { base_url?: string; token?: string };
  if (!body.base_url) throw new Error("Runtime discovery returned no endpoint");
  return {
    baseUrl: body.base_url.replace(/\/+$/, ""),
    token: body.token ?? null,
    source: "dev-server",
    mode: "dev"
  };
}
