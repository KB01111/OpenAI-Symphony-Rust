import { useMatch } from "react-router-dom";

/** Task the operator is focused on: the open thread (`/runs/:taskId`). */
export function useFocusedTaskId(): string | null {
  const runMatch = useMatch("/runs/:taskId");
  return runMatch?.params.taskId ? decodeURIComponent(runMatch.params.taskId) : null;
}
