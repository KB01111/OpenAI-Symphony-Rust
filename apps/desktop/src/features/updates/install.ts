import { errorMessage, listenUpdateProgress, nativeHost } from "@symphony/api-client";
import { useUiStore } from "../../store/uiStore.js";

/**
 * Starts installing the staged update. The host downloads it, verifies the signature against the
 * key compiled into the app and runs the installer. With agents running it answers first and
 * waits for `startInstall(true)`. On Windows the app exits for the installer, so success is
 * "installing" and the promise may never settle.
 */
export async function startInstall(confirmed: boolean): Promise<void> {
  const { setUpdateInstall } = useUiStore.getState();
  const phase = useUiStore.getState().updateInstall.phase;
  if (phase === "downloading" || phase === "installing") return;

  setUpdateInstall({ phase: "downloading", downloaded: 0, total: null });
  let unlisten: (() => void) | undefined;
  try {
    unlisten = await listenUpdateProgress((p) =>
      setUpdateInstall(p.phase === "installing" ? { phase: "installing" } : { phase: "downloading", downloaded: p.downloaded, total: p.total })
    );
  } catch {
    // Progress is cosmetic: the install still works without it.
  }
  try {
    const outcome = await nativeHost.installUpdate(confirmed);
    setUpdateInstall(outcome.status === "needs_confirmation" ? { phase: "confirm", running: outcome.running } : { phase: "installing" });
  } catch (error) {
    setUpdateInstall({ phase: "error", message: errorMessage(error) });
  } finally {
    unlisten?.();
  }
}

/** Percent downloaded, or `null` while the size is unknown. */
export function downloadPercent(downloaded: number, total: number | null): number | null {
  if (!total || total <= 0) return null;
  return Math.min(100, Math.floor((downloaded / total) * 100));
}
