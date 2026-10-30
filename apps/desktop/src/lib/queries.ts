/**
 * TanStack Query bindings for the runtime API. Components never fetch directly; they use these
 * hooks so caching, invalidation (driven by the event stream) and fallback polling are uniform.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useInfiniteQuery, useMutation, useQuery, useQueryClient, type UseQueryOptions } from "@tanstack/react-query";
import { errorMessage, isTauri, nativeHost, type SymphonyApiError } from "@symphony/api-client";
import type {
  AgentEvent,
  AppSettings,
  ApprovalDecision,
  ApprovalRequest,
  CloudAgentKind,
  CreateProjectRequest,
  CreateTaskRequest,
  GraphLevel,
  ProjectUpdate,
  Task,
  TaskStatus
} from "@symphony/shared-types";
import { DEFAULT_CLOUD_AGENT } from "@symphony/shared-types";
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

export const TASK_EVENT_PAGE = 500;

export function useTaskEventHistory(id: string | null | undefined) {
  const client = useClient();
  return useQuery({
    queryKey: qk.taskEvents(id ?? ""),
    queryFn: ({ signal }) => client.listTaskEvents(id!, TASK_EVENT_PAGE, signal),
    enabled: Boolean(id),
    staleTime: 10_000
  });
}

/**
 * Pages of a task's events older than the newest page (`useTaskEventHistory`), loaded on request.
 * The newest page as it was when paging started is kept as well (`older` begins with it), so events
 * that later move out of a refreshed newest page never fall into a gap. A response for a task that is
 * no longer shown is dropped. `hasMore` is true while the last page read was full.
 */
export function useOlderTaskEvents(id: string, newest: ReadonlyArray<AgentEvent> | undefined) {
  const client = useClient();
  const [state, setState] = useState<{ id: string; anchor: AgentEvent[]; pages: AgentEvent[]; lastPageFull: boolean | null }>(
    () => ({ id, anchor: [], pages: [], lastPageFull: null })
  );
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const current = useRef(id);
  current.current = id;
  // State of another task is never shown.
  const own = state.id === id ? state : { id, anchor: [], pages: [], lastPageFull: null };
  useEffect(() => {
    setState({ id, anchor: [], pages: [], lastPageFull: null });
    setError(null);
    setLoading(false);
  }, [id]);
  const hasMore = own.lastPageFull ?? (newest?.length ?? 0) >= TASK_EVENT_PAGE;
  const loadOlder = useCallback(async () => {
    if (loading) return;
    const anchor = own.anchor.length > 0 ? own.anchor : [...(newest ?? [])];
    const cursor = own.pages.length > 0 ? own.pages[own.pages.length - 1]!.id : anchor[anchor.length - 1]?.id;
    if (!cursor) return;
    const forTask = id;
    setLoading(true);
    setError(null);
    try {
      const page = await client.listTaskEvents(forTask, TASK_EVENT_PAGE, undefined, cursor);
      if (current.current !== forTask) return;
      setState((prev) =>
        prev.id === forTask
          ? { id: forTask, anchor, pages: [...prev.pages, ...page], lastPageFull: page.length >= TASK_EVENT_PAGE }
          : prev
      );
    } catch (e) {
      if (current.current === forTask) setError(e);
    } finally {
      if (current.current === forTask) setLoading(false);
    }
  }, [client, id, loading, newest, own.anchor, own.pages]);
  const older = useMemo(() => [...own.anchor, ...own.pages], [own.anchor, own.pages]);
  return { older, pageCount: own.pages.length, hasMore, loading, error, loadOlder };
}

export const TOOL_CALL_PAGE = 100;

/** A task's tool-call history, newest first, one page at a time (`fetchNextPage` reads older calls). */
export function useTaskToolCalls(id: string | null | undefined) {
  const client = useClient();
  // Without the event stream (fallback polling) nothing invalidates this query: poll like the
  // other task queries.
  const refetchInterval = usePoll();
  return useInfiniteQuery({
    queryKey: qk.taskToolCalls(id ?? ""),
    queryFn: ({ signal, pageParam }) => client.listTaskToolCalls(id!, TOOL_CALL_PAGE, signal, pageParam),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => (last.length >= TOOL_CALL_PAGE ? last[last.length - 1]!.id : undefined),
    enabled: Boolean(id),
    staleTime: 10_000,
    refetchInterval
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

// ---------------------------------------------------------------------------------------------
// Repository Knowledge Base. Every query is scoped to one project and kept current by the
// `knowledge/*` events (see runtime/invalidation.ts); polling is only the fallback.
// ---------------------------------------------------------------------------------------------

export function useKnowledgeEngines() {
  const client = useClient();
  return useQuery({ queryKey: qk.knowledgeEngines, queryFn: ({ signal }) => client.getKnowledgeEngines(signal), staleTime: 60_000 });
}

export function useKnowledgeOverview(projectId: string | null) {
  const client = useClient();
  const refetchInterval = usePoll(30_000);
  return useQuery({
    queryKey: qk.knowledgeOverview(projectId ?? ""),
    queryFn: ({ signal }) => client.getKnowledgeOverview(projectId!, signal),
    enabled: projectId !== null,
    refetchInterval
  });
}

export function useKnowledgeWorktrees(projectId: string | null) {
  const client = useClient();
  const refetchInterval = usePoll(30_000);
  return useQuery({
    queryKey: qk.knowledgeWorktrees(projectId ?? ""),
    queryFn: ({ signal }) => client.listKnowledgeWorktrees(projectId!, signal),
    enabled: projectId !== null,
    refetchInterval
  });
}

/**
 * Keep the previous result on screen while a query for the same repository refetches (a longer
 * list, another graph level), but never show one repository's rows under another's name: the
 * project id is the second element of every Knowledge Base query key.
 */
function keepWithinProject(projectId: string | null) {
  return <T>(previous: T | undefined, previousQuery?: { queryKey: readonly unknown[] }): T | undefined =>
    previousQuery?.queryKey[1] === projectId ? previous : undefined;
}

export function useKnowledgeCommits(projectId: string | null, limit = 100) {
  const client = useClient();
  const refetchInterval = usePoll(30_000);
  return useQuery({
    queryKey: qk.knowledgeCommits(projectId ?? "", limit),
    queryFn: ({ signal }) => client.listKnowledgeCommits(projectId!, limit, signal),
    enabled: projectId !== null,
    refetchInterval,
    placeholderData: keepWithinProject(projectId)
  });
}

export function useKnowledgeCommit(projectId: string | null, sha: string | null) {
  const client = useClient();
  return useQuery({
    queryKey: qk.knowledgeCommit(projectId ?? "", sha ?? ""),
    queryFn: ({ signal }) => client.getKnowledgeCommit(projectId!, sha!, signal),
    enabled: projectId !== null && sha !== null,
    staleTime: 5 * 60_000
  });
}

export function useKnowledgeCommitDiff(projectId: string | null, sha: string | null, enabled = true) {
  const client = useClient();
  return useQuery({
    queryKey: qk.knowledgeDiff(projectId ?? "", sha ?? ""),
    queryFn: ({ signal }) => client.getKnowledgeCommitDiff(projectId!, sha!, signal),
    enabled: enabled && projectId !== null && sha !== null,
    staleTime: 5 * 60_000
  });
}

export function useKnowledgeCommitImpact(projectId: string | null, sha: string | null, depth = 2, includeInferred = true) {
  const client = useClient();
  return useQuery({
    queryKey: qk.knowledgeImpact(projectId ?? "", sha ?? "", depth, includeInferred),
    queryFn: ({ signal }) => client.getKnowledgeCommitImpact(projectId!, sha!, depth, includeInferred, signal),
    enabled: projectId !== null && sha !== null,
    staleTime: 5 * 60_000
  });
}

/** The newest AI analysis of a commit; polls while one is running in case the event was missed. */
export function useKnowledgeCommitAnalysis(projectId: string | null, sha: string | null) {
  const client = useClient();
  return useQuery({
    queryKey: qk.knowledgeAnalysis(projectId ?? "", sha ?? ""),
    queryFn: ({ signal }) => client.getKnowledgeCommitAnalysis(projectId!, sha!, signal),
    enabled: projectId !== null && sha !== null,
    refetchInterval: (query) => (query.state.data?.analysis?.status === "running" ? 4_000 : false)
  });
}

export interface KnowledgeGraphParams {
  level: GraphLevel;
  module?: string | null;
  focus?: string | null;
  depth?: number;
  includeInferred?: boolean;
  includeExternal?: boolean;
}

export function useKnowledgeGraph(projectId: string | null, params: KnowledgeGraphParams, enabled = true) {
  const client = useClient();
  return useQuery({
    queryKey: qk.knowledgeGraph(projectId ?? "", params),
    queryFn: ({ signal }) => client.getKnowledgeGraph(projectId!, params, signal),
    enabled: enabled && projectId !== null && (params.level !== "symbols" || Boolean(params.focus)),
    staleTime: 60_000,
    placeholderData: keepWithinProject(projectId)
  });
}

export function useKnowledgeSearch(projectId: string | null, text: string) {
  const client = useClient();
  const trimmed = text.trim();
  return useQuery({
    queryKey: qk.knowledgeSearch(projectId ?? "", trimmed),
    queryFn: ({ signal }) => client.searchKnowledge(projectId!, trimmed, 25, signal),
    enabled: projectId !== null && trimmed.length >= 2,
    staleTime: 60_000
  });
}

export function useKnowledgeNode(projectId: string | null, nodeId: string | null) {
  const client = useClient();
  return useQuery({
    queryKey: qk.knowledgeNode(projectId ?? "", nodeId ?? ""),
    queryFn: ({ signal }) => client.getKnowledgeNode(projectId!, nodeId!, signal),
    enabled: projectId !== null && nodeId !== null,
    staleTime: 60_000
  });
}

/** Precise references of one symbol; only fetched once `enabled` (it can take a minute to start). */
export function useKnowledgeNodeReferences(projectId: string | null, nodeId: string | null, enabled: boolean) {
  const client = useClient();
  return useQuery({
    queryKey: qk.knowledgeReferences(projectId ?? "", nodeId ?? ""),
    queryFn: ({ signal }) => client.getKnowledgeNodeReferences(projectId!, nodeId!, signal),
    enabled: enabled && projectId !== null && nodeId !== null,
    staleTime: 5 * 60_000,
    retry: false
  });
}

export function useKnowledgeChat(projectId: string | null) {
  const client = useClient();
  return useQuery({
    queryKey: qk.knowledgeChat(projectId ?? ""),
    queryFn: ({ signal }) => client.listKnowledgeChat(projectId!, signal),
    enabled: projectId !== null,
    refetchInterval: (query) => (query.state.data?.some((m) => m.status === "running") ? 4_000 : false)
  });
}

export function useStartKnowledgeIndex(projectId: string | null) {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: (worktreeId?: string | null) => client.startKnowledgeIndex(projectId!, worktreeId),
    onSuccess: () => void qc.invalidateQueries({ queryKey: qk.knowledgeOverview(projectId ?? "") }),
    onError
  });
}

export function useStartKnowledgeAnalysis(projectId: string | null) {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: ({ sha, force }: { sha: string; force?: boolean }) => client.startKnowledgeCommitAnalysis(projectId!, sha, force ?? false),
    onSuccess: (_analysis, { sha }) => void qc.invalidateQueries({ queryKey: qk.knowledgeAnalysis(projectId ?? "", sha) }),
    onError
  });
}

export function useAskKnowledge(projectId: string | null) {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: (question: string) => client.askKnowledge(projectId!, question),
    onSettled: () => void qc.invalidateQueries({ queryKey: qk.knowledgeChat(projectId ?? "") }),
    onError
  });
}

export function useClearKnowledgeChat(projectId: string | null) {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: () => client.clearKnowledgeChat(projectId!),
    onSuccess: () => void qc.invalidateQueries({ queryKey: qk.knowledgeChat(projectId ?? "") }),
    onError
  });
}

export function useWorkflow() {
  const client = useClient();
  return useQuery({ queryKey: qk.workflow, queryFn: ({ signal }) => client.getWorkflow(signal), staleTime: Infinity });
}

/**
 * The agent a CodeSandbox task runs when it picks none: the application setting if one was ever
 * chosen, else `codesandbox.default_agent` of the active workflow, else mini-swe-agent. Mirrors the
 * runtime's own order, so the picker marks the agent that would really run.
 */
export function useDefaultCloudAgent(): CloudAgentKind {
  const settings = useSettings();
  const workflow = useWorkflow();
  return settings.data?.default_cloud_agent ?? workflow.data?.config?.codesandbox.default_agent ?? DEFAULT_CLOUD_AGENT;
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

export function useContinueTask() {
  const client = useClient();
  const invalidate = useTaskInvalidation();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: ({ id, instructions }: { id: string; instructions: string }) => client.continueTask(id, instructions),
    onSuccess: (task) => invalidate(task),
    onError
  });
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

/** Commit, push and open a pull request for a review (an explicit human action; never merges). */
export function useCreatePullRequest() {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: (id: string) => client.createPullRequest(id),
    onSuccess: (pr) => {
      qc.setQueryData(qk.pr(pr.id), pr);
      void qc.invalidateQueries({ queryKey: qk.prsAll });
      void qc.invalidateQueries({ queryKey: qk.tasksAll });
    },
    onError
  });
}

/** Re-read a review's pull request status from GitHub. */
export function useRefreshPullRequest() {
  const client = useClient();
  const qc = useQueryClient();
  const onError = useFailureToast();
  return useMutation({
    mutationFn: (id: string) => client.refreshPullRequest(id),
    onSuccess: (pr) => {
      qc.setQueryData(qk.pr(pr.id), pr);
      void qc.invalidateQueries({ queryKey: qk.prsAll });
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
          // restore the last confirmed native state (else the value before this change) and mark
          // the native state unknown so the next save retries.
          const observed = observedAutostart(error);
          appliedAutostart = observed ?? "unknown";
          const restore = observed ?? (typeof applied === "boolean" ? applied : previous?.start_background_runtime_on_login);
          if (restore !== undefined && restore !== wanted) {
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
