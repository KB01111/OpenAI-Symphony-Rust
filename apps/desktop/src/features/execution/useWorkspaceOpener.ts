import { useCallback } from "react";
import { errorMessage, isTauri, nativeHost } from "@symphony/api-client";
import { useNotify } from "@symphony/ui";
import { useT } from "../../lib/i18n.js";
import { useRuntime } from "../../runtime/RuntimeProvider.js";

/**
 * Native "open the task's worktree in Explorer". Only available inside the Tauri desktop host
 * with the embedded runtime (the host validates the path against the runtime's workspace root);
 * failures are reported through a toast.
 */
export function useWorkspaceOpener(): { available: boolean; open: (taskId: string) => Promise<void> } {
  const { mode } = useRuntime();
  const notify = useNotify();
  const t = useT();
  const available = isTauri() && mode === "embedded";
  const open = useCallback(
    async (taskId: string) => {
      try {
        await nativeHost.openTaskWorkspace(taskId);
      } catch (e) {
        notify("error", t.errors.generic, errorMessage(e));
      }
    },
    [notify, t]
  );
  return { available, open };
}
