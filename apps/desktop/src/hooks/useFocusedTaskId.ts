import { useMatch } from "react-router-dom";
import { useUiStore } from "../store/uiStore.js";

/** Task the operator is focused on: the open execution workspace, else the inspected task. */
export function useFocusedTaskId(): string | null {
  const runMatch = useMatch("/runs/:taskId");
  const inspector = useUiStore((s) => s.inspector);
  if (runMatch?.params.taskId) return decodeURIComponent(runMatch.params.taskId);
  return inspector?.kind === "task" ? inspector.id : null;
}
