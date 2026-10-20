/**
 * TanStack Query bindings for the runtime API. Components never fetch directly; they use these
 * hooks so caching, invalidation (driven by the event stream) and fallback polling are uniform.
 */
import { useMutation, useQuery, useQueryClient, type UseQueryOptions } from "@tanstack/react-query";
import { errorMessage, isTauri, nativeHost, type SymphonyApiError } from "@symphony/api-client";
import type {
  AppSettings,
  ApprovalDecision,
  ApprovalRequest,
  CreateProjectRequest,
  CreateTaskRequest,
  ProjectUpdate,
  Task,
  TaskStatus
} from "@symphony/shared-types";
import { useNotify } from "@symphony/ui";
import { useClient, useRuntime } from "../runtime/RuntimeProvider.js";
import { qk } from "./queryKeys.js";
import { useT } from "./i18n.js";

type Opts<T> = Omit<UseQueryOptions<T, SymphonyApiError>, "queryKey" | "queryFn">;

function usePoll(base?: number): number | false {
  const { pollInterval, documentHidden } = useRuntime();
  if (pollInterval === false) return base && !documentHidden ? base : false;
  return pollInterval;
}

// ---------------------------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------------------------

/** Orchestrator snapshot (counts, live sessions, hardware). Refreshed by events; hardware ticks every 15 s while visible. */
export function useRuntimeState(opts?: Opts<Awaited<ReturnType<ReturnType<typeof useClient>["getState"]>>>) {
  const client = useClient();
  const refetchInterval = usePoll(15_000);
  return useQuery({ queryKey: qk.state, queryFn: ({ signal }) => client.getState(signal), refetchInterval, ...opts });
}

/** Persisted recent activity (survives app restarts); live events are appended by the stream. */
export function useRecentEvents(limit = 100) {
  const client = useClient();
  return useQuery({
    queryKey: [...qk.recentEvents, limit],
    queryFn: ({ signal }) => client.listRecentEvents(limit, signal),
    staleTime: 60_000
  });
}

export function useProjects() {
  const client = useClient();
  return useQuery({ queryKey: qk.projects, queryFn: ({ signal }) => client.listProjects(signal), staleTime: 30_000 });
}

export function useTasks(projectId?: string | null) {
  const client = useClient();
  const refetchInterval = usePoll();
  return useQuery({
    queryKey: qk.tasks(projectId),
    queryFn: ({ signal }) => client.listTasks({ projectId }, signal),
    refetchInterval
  });
}

export function useTask(id: string | null | undefined) {
  const client = useClient();
  const refetchInterval = usePoll();
  return useQuery({
    queryKey: qk.task(id ?? ""),
    queryFn: ({ signal }) => client.getTask(id!, signal),
    enabled: Boolean(id),
    refetchInterval
  });
}

export function useTaskEventHistory(id: string | null | undefined) {
  const client = useClient();
  return useQuery({
    queryKey: qk.taskEvents(id ?? ""),
    queryFn: ({ signal }) => client.listTaskEvents(id!, 500, signal),
    enabled: Boolean(id),
    staleTime: 10_000
  });
}

export function useTaskAttempts(id: string | null | undefined) {
  const client = useClient();
  return useQuery({ queryKey: qk.taskAttempts(id ?? ""), queryFn: ({ signal }) => client.listTaskAttempts(id!, signal), enabled: Boolean(id) });
}

export function useTaskValidations(id: string | null | undefined) {
  const client = useClient();
  return useQuery({
    queryKey: qk.taskValidations(id ?? ""),
    queryFn: ({ signal }) => client.listTaskValidations(id!, signal),
    enabled: Boolean(id)
  });
}

export function useTaskDiff(id: string | null | undefined) {
  const client = useClient();
  return useQuery({ queryKey: qk.taskDiff(id ?? ""), queryFn: ({ signal }) => client.getTaskDiff(id!, signal), enabled: Boolean(id), staleTime: 5_000 });
}

export function useApprovals(pendingOnly = true) {
  const client = useClient();
  const refetchInterval = usePoll();
  return useQuery({
    queryKey: qk.approvals(pendingOnly),
    queryFn: ({ signal }) => client.listApprovals(pendingOnly, signal),
    refetchInterval
  });
}

export function usePullRequests(projectId?: string | null) {
  const client = useClient();
  return useQuery({ queryKey: qk.prs(projectId), queryFn: ({ signal }) => client.listPullRequests(projectId, signal) });
}

export function usePullRequest(id: string | null | undefined) {
  const client = useClient();
  return useQuery({ queryKey: qk.pr(id ?? ""), queryFn: ({ signal }) => client.getPullRequest(id!, signal), enabled: Boolean(id) });
}

export function useWorktrees() {
  const client = useClient();
  return useQuery({ queryKey: qk.worktrees, queryFn: ({ signal }) => client.listWorktrees(signal) });
}

export function useWorkflow() {
  const client = useClient();
  return useQuery({ queryKey: qk.workflow, queryFn: ({ signal }) => client.getWorkflow(signal), staleTime: Infinity });
}

export function useSettings() {
  const client = useClient();
  return useQuery({ queryKey: qk.settings, queryFn: ({ signal }) => client.getSettings(signal), staleTime: Infinity });
}

export function useProviders() {
  const client = useClient();
  return useQuery({ queryKey: qk.providers, queryFn: ({ signal }) => client.listProviders(false, signal), staleTime: 60_000 });
}

export function useCredentials() {
  const client = useClient();
  return useQuery({ queryKey: qk.credentials, queryFn: ({ signal }) => client.listCredentials(signal) });
}

// ---------------------------------------------------------------------------------------------
// Mutations — every mutation reports failures through a toast and refreshes affected caches.
// ---------------------------------------------------------------------------------------------

function useFailureToast() {
  const notify = useNotify();
  const t = useT();
  return (error: unknown) => notify("error", t.errors.generic, errorMessage(error));
}

function useTaskInvalidation() {
  const qc = useQueryClient();
  return (task?: Task) => {
    void qc.invalidateQueries({ queryKey: qk.tasksAll });
    void qc.invalidateQueries({ queryKey: qk.state });
    void qc.invalidateQueries({ queryKey: qk.worktrees });
    if (task) {
      qc.setQueryData(qk.task(task.id), task);
      void qc.invalidateQueries({ queryKey: qk.taskAttempts(task.id) });
    }
  };
}

export function useCreateProject() {
  const client = useClient();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: CreateProjectRequest) => client.createProject(input),
    onSuccess: () => void qc.invalidateQueries({ queryKey: qk.projects })
  });
}

export function useValidateRepository() {
  const client = useClient();
  return useMutation({ mutationFn: (path: string) => client.validateRepository(path) });
}

export function useUpdateProject() {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: ({ id, update }: { id: string; update: ProjectUpdate }) => client.updateProject(id, update),
    onSuccess: () => void qc.invalidateQueries({ queryKey: qk.projects }),
    onError
  });
}

export function useDeleteProject() {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: (id: string) => client.deleteProject(id),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: qk.projects });
      void qc.invalidateQueries({ queryKey: qk.tasksAll });
    },
    onError
  });
}

export function useCreateTask() {
  const client = useClient();
  const invalidate = useTaskInvalidation();
  return useMutation({
    mutationFn: (input: CreateTaskRequest) => client.createTask(input),
    onSuccess: (task) => invalidate(task)
  });
}

/**
 * Manual status move. Optimistically moves the card (board drag-and-drop) and rolls back if the
 * runtime's state machine rejects the transition; the runtime response is authoritative.
 */
export function useMoveTaskStatus() {
  const client = useClient();
  const qc = useQueryClient();
  const invalidate = useTaskInvalidation();
  const onFailure = useFailureToast();
  return useMutation({
    mutationFn: ({ id, status }: { id: string; status: TaskStatus }) => client.moveTaskStatus(id, status),
    onMutate: async ({ id, status }) => {
      await qc.cancelQueries({ queryKey: qk.tasksAll });
      const snapshots = qc.getQueriesData<Task[]>({ queryKey: qk.tasksAll });
      qc.setQueriesData<Task[]>({ queryKey: qk.tasksAll }, (list) =>
        list?.map((t) => (t.id === id ? { ...t, status } : t))
      );
      return { snapshots };
    },
    onError: (error, _vars, context) => {
      context?.snapshots.forEach(([key, data]) => qc.setQueryData(key, data));
      onFailure(error);
    },
    onSuccess: (task) => invalidate(task)
  });
}

function useTaskAction(action: (id: string) => Promise<Task>) {
  const invalidate = useTaskInvalidation();
  const onError = useFailureToast();
  return useMutation({ mutationFn: action, onSuccess: (task) => invalidate(task), onError });
}

export function useDispatchTask() {
  const client = useClient();
  return useTaskAction((id) => client.dispatchTask(id));
}

export function useRetryTask() {
  const client = useClient();
  return useTaskAction((id) => client.retryTask(id));
}

export function useCancelTask() {
  const client = useClient();
  return useTaskAction((id) => client.cancelTask(id));
}

export function useDecideApproval() {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: ({ approval, decision }: { approval: ApprovalRequest; decision: ApprovalDecision }) =>
      client.decideApproval(approval.id, decision),
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: qk.approvalsAll });
      void qc.invalidateQueries({ queryKey: qk.state });
    },
    onError
  });
}

export function useApprovePullRequest() {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: (id: string) => client.approvePullRequest(id),
    onSuccess: (pr) => {
      qc.setQueryData(qk.pr(pr.id), pr);
      void qc.invalidateQueries({ queryKey: qk.prsAll });
      void qc.invalidateQueries({ queryKey: qk.tasksAll });
    },
    onError
  });
}

export function useSaveWorkflow() {
  const client = useClient();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (raw: string) => client.saveWorkflow(raw),
    onSuccess: () => void qc.invalidateQueries({ queryKey: qk.workflow })
  });
}

/**
 * Login-autostart value the native host last applied in this session: `null` until the first
 * change, `"unknown"` after a failure whose outcome could not be read back. Compared instead of
 * the cached settings so a failed native update is retried by the next settings save even though
 * the runtime already stored a value (`"unknown"` never equals a wanted value, so it always
 * retries).
 */
let appliedAutostart: boolean | null | "unknown" = null;

/** Registry state reported with a failed `set_autostart` (`autostart_state=<state>; …`). */
export function observedAutostart(error: unknown): boolean | null {
  const match = /autostart_state=(enabled|disabled|unknown)/.exec(String(error));
  if (!match || match[1] === "unknown") return null;
  return match[1] === "enabled";
}

/** A full settings object, or an update applied to the latest saved settings when the write runs. */
export type SettingsChange = AppSettings | ((latest: AppSettings) => AppSettings);

/**
 * Persist settings to the runtime and apply native side effects (login autostart).
 * Writes are serialized (`scope`), and functional changes are applied to the latest cached
 * settings at execution time, so a preference write cannot replace a newer save with a stale
 * snapshot.
 */
export function useSaveSettings() {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    scope: { id: "settings" },
    mutationFn: async (change: SettingsChange) => {
      const previous = qc.getQueryData<AppSettings>(qk.settings);
      const next = typeof change === "function" ? (previous ? change(previous) : null) : change;
      if (!next) throw new Error("Settings are not loaded yet");
      const saved = await client.saveSettings(next);
      // The runtime has stored the change: reflect it before any native side effect can fail.
      qc.setQueryData(qk.settings, saved);
      const wanted = saved.start_background_runtime_on_login;
      const applied = appliedAutostart ?? previous?.start_background_runtime_on_login;
      if (isTauri() && applied !== wanted) {
        try {
          await nativeHost.setAutostart(wanted);
          appliedAutostart = wanted;
        } catch (error) {
          // The host reports the registry state it observed after the failure. Store that state
          // so the switch never shows an autostart that is not registered. When it is unknown,
          // restore the last confirmed value (the one before this change) and mark the native
          // state unknown so the next save retries.
          const observed = observedAutostart(error);
          appliedAutostart = observed ?? "unknown";
          const restore = observed ?? !wanted;
          if (restore !== wanted) {
            const reverted = await client
              .saveSettings({ ...saved, start_background_runtime_on_login: restore })
              .catch(() => null);
            if (reverted) qc.setQueryData(qk.settings, reverted);
            // The revert itself failed: refetch rather than keep showing the rejected value.
            else void qc.invalidateQueries({ queryKey: qk.settings });
          }
          throw error;
        }
      }
      return saved;
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: qk.providers });
      // The runtime applies remote-access changes to the running listener on save.
      void qc.invalidateQueries({ queryKey: qk.remoteAccess });
    },
    onError
  });
}

export function useSaveCredential() {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: ({ key, value }: { key: string; value: string }) => client.saveCredential(key, value),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: qk.credentials });
      void qc.invalidateQueries({ queryKey: qk.providers });
    },
    onError
  });
}

export function useDeleteCredential() {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: (key: string) => client.deleteCredential(key),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: qk.credentials });
      void qc.invalidateQueries({ queryKey: qk.providers });
    },
    onError
  });
}

export function useSchedulerToggle() {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: (paused: boolean) => client.setSchedulerPaused(paused),
    onSuccess: () => void qc.invalidateQueries({ queryKey: qk.state }),
    onError
  });
}

export function useRefreshRuntime() {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: () => client.refresh(),
    onSuccess: () => void qc.invalidateQueries(),
    onError
  });
}

export function useRefreshProviders() {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: () => client.listProviders(true),
    onSuccess: (data) => qc.setQueryData(qk.providers, data),
    onError
  });
}
