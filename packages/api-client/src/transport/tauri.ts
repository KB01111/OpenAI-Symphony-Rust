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
  /** Why the host could not publish `runtime.json` (CLI tools cannot attach), if it failed. */
  discovery_error?: string | null;
}

/** Result of an explicit "check for updates" (`check_for_updates`). */
export interface UpdateCheck {
  current: string;
  /** Newest published version; `null` when no release is published yet. */
  latest: string | null;
  update_available: boolean;
  prerelease: boolean;
  published_at: string | null;
  notes: string | null;
  /**
   * The app can download, verify and install this update itself. `false` for a release without a
   * signed manifest: the user installs it from the releases page.
   */
  installable: boolean;
}

/** Answer of `install_update`. Without `confirmed`, running agents are reported first. */
export type InstallOutcome = { status: "needs_confirmation"; running: number } | { status: "installed" };

/** Download progress pushed by the host on `symphony://update-progress`. */
export interface UpdateProgress {
  phase: "downloading" | "installing";
  downloaded: number;
  total: number | null;
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
  /**
   * Returns whether autostart is supported on this platform. Rejects when Windows refused or did
   * not persist the registry change, so callers never report an entry that does not exist.
   */
  setAutostart: (enabled: boolean) => invoke<boolean>("set_autostart", { enabled }),
  exitApplication: () => invoke<void>("exit_application"),
  /** Language (`en` | `sv`) of the tray menu and native notifications. */
  setNativeLocale: (locale: string) => invoke<void>("set_native_locale", { locale }),
  /** Opens the application data directory in Explorer. Takes no path. Returns the opened path. */
  openDataDirectory: () => invoke<string>("open_data_directory"),
  /** Opens the diagnostic log directory in Explorer. Takes no path. Returns the opened path. */
  openLogDirectory: () => invoke<string>("open_log_directory"),
  /** Asks GitHub for the newest release. Only runs on request; nothing is downloaded. */
  checkForUpdates: () => invoke<UpdateCheck>("check_for_updates"),
  /**
   * Downloads, verifies and installs the update found by the last `checkForUpdates`; the host
   * takes no URL. With running agents it answers `needs_confirmation` until `confirmed` is true.
   * On Windows the app exits for the installer, so the promise may never settle.
   */
  installUpdate: (confirmed: boolean) => invoke<InstallOutcome>("install_update", { confirmed }),
  /** Opens the project's releases page in the default browser (a fixed URL). */
  openReleasesPage: () => invoke<void>("open_releases_page")
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

/** Subscribe to update download progress. Resolves an unsubscribe function. */
export async function listenUpdateProgress(onProgress: (progress: UpdateProgress) => void): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<UpdateProgress>("symphony://update-progress", (e) => onProgress(e.payload));
}
