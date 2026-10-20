/**
 * Minimal Tauri bridge. Only the narrowly scoped commands registered by the desktop host
 * (`apps/desktop/src-tauri/src/main.rs`) are exposed; there is no generic command passthrough.
 */
import type { AgentEvent } from "@symphony/shared-types";

export interface LocalApiConfig {
  base_url: string;
  token: string;
  api_version: number;
  mode: "embedded" | "attached";
}

interface TauriInternals {
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
}

function internals(): TauriInternals | null {
  const w = globalThis as unknown as { __TAURI_INTERNALS__?: TauriInternals };
  return w.__TAURI_INTERNALS__?.invoke ? w.__TAURI_INTERNALS__ : null;
}

export function isTauri(): boolean {
  return internals() !== null;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const bridge = internals();
  if (!bridge) throw new Error(`Native command '${cmd}' requires the Symphony desktop host`);
  return bridge.invoke<T>(cmd, args);
}

export const nativeHost = {
  getLocalApiConfig: () => invoke<LocalApiConfig>("get_local_api_config"),
  /** Opens a task's validated worktree in Explorer. Returns the opened path. */
  openTaskWorkspace: (taskId: string) => invoke<string>("open_task_workspace", { taskId }),
  /** Native folder picker; resolves `null` when cancelled. */
  pickRepositoryFolder: () => invoke<string | null>("pick_repository_folder"),
  /** Returns whether autostart is supported on this platform. */
  setAutostart: (enabled: boolean) => invoke<boolean>("set_autostart", { enabled }),
  exitApplication: () => invoke<void>("exit_application")
};

export type NativeHost = typeof nativeHost;

/** Subscribe to runtime events forwarded by the desktop host on `symphony://event`. */
export async function listenNativeEvents(
  onEvent: (event: AgentEvent) => void,
  onResync: () => void
): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  const unlistenEvent = await listen<AgentEvent>("symphony://event", (e) => onEvent(e.payload));
  const unlistenResync = await listen<string>("symphony://resync", () => onResync());
  return () => {
    unlistenEvent();
    unlistenResync();
  };
}
