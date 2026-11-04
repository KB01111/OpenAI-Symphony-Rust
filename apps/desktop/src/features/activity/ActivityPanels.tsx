import React, { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { errorMessage } from "@symphony/api-client";
import { formatMessage, type AgentEvent, type ApprovalDecision, type ApprovalRequest } from "@symphony/shared-types";
import {
  AiApprovalBanner,
  AiEmptyView,
  AiEventStreamCard,
  ErrorState,
  LoadingRows,
  Button,
  CodeBlock,
  VirtualList,
  formatAgentEvent,
  type AgentEventKind
} from "@symphony/ui";
import { shellMessages } from "../../app/messages.js";
import { paths } from "../../app/routes.js";
import { useMessages, useT } from "../../lib/i18n.js";
import { useApprovals, useDecideApproval, useProviders, useRuntimeState, useTaskEventHistory, useTaskValidations, useTasks } from "../../lib/queries.js";
import { mergeEventHistory, useEventStore } from "../../runtime/eventStore.js";
import { useRuntime } from "../../runtime/RuntimeProvider.js";
import { useFocusedTaskId } from "../../hooks/useFocusedTaskId.js";
import { useUiStore } from "../../store/uiStore.js";

export const EventFeed: React.FC<{ events: AgentEvent[]; showIdentifier?: boolean; emptyTitle: string; emptyDetail?: string }> = ({
  events,
  showIdentifier = false,
  emptyTitle,
  emptyDetail
}) => {
  const logMode = useUiStore((s) => s.logMode);
  if (events.length === 0) return <AiEmptyView className="m-2" title={emptyTitle} description={emptyDetail} />;
  return (
    <VirtualList
      items={events}
      getKey={(e) => e.id}
      renderItem={(e) => <AiEventStreamCard event={e} mode={logMode} showIdentifier={showIdentifier} />}
      estimateSize={48}
      followTail
      className="h-full px-1"
      ariaLabel={emptyTitle}
    />
  );
};

function useScopedTaskId(taskId?: string | null): string | null {
  const focused = useFocusedTaskId();
  return taskId === undefined ? focused : taskId;
}

function useFocusedTaskEvents(kinds: ReadonlyArray<AgentEventKind>, scoped?: string | null) {
  const taskId = useScopedTaskId(scoped);
  const history = useTaskEventHistory(taskId);
  const live = useEventStore((s) => s.events);
  const events = useMemo(() => {
    if (!taskId) return null;
    const merged = mergeEventHistory(history.data, live.filter((e) => e.task_id === taskId));
    return merged.filter((e) => kinds.includes(formatAgentEvent(e).kind));
  }, [taskId, history.data, live, kinds]);
  return { events, history };
}

export const AGENT_KINDS: AgentEventKind[] = ["message", "reasoning", "file_change", "tool_call", "error"];
export const TERMINAL_KINDS: AgentEventKind[] = ["command"];

export const ApprovalsTab: React.FC<{ taskId?: string | null }> = ({ taskId }) => {
  const m = useMessages(shellMessages);
  const t = useT();
  const approvals = useApprovals(true);
  const decide = useDecideApproval();
  const navigate = useNavigate();
  const [pending, setPending] = useState<Record<string, ApprovalDecision>>({});
  const onDecide = (approval: ApprovalRequest, decision: ApprovalDecision) => {
    if (pending[approval.id]) return;
    setPending((p) => ({ ...p, [approval.id]: decision }));
    decide.mutate(
      { approval, decision },
      { onSettled: () => setPending(({ [approval.id]: _removed, ...rest }) => rest) }
    );
  };
  if (approvals.isLoading) return <LoadingRows className="p-2" rows={3} />;
  // A failed background refresh keeps showing the cached list; only block when nothing loaded.
  if (approvals.isError && !approvals.data) {
    // Never show "no approvals" when the list could not be fetched: requests may be waiting.
    return <ErrorState className="m-2" title={m.drawer.loadFailed} message={errorMessage(approvals.error)} onRetry={() => void approvals.refetch()} retryLabel={t.common.retry} />;
  }
  const list = (approvals.data ?? []).filter((a) => !taskId || a.task_id === taskId);
  if (list.length === 0) return <AiEmptyView className="m-2" title={m.drawer.noApprovals} />;
  return (
    <div className="h-full space-y-2 overflow-auto p-2">
      {list.map((a) => (
        <AiApprovalBanner
          key={a.id}
          approval={a}
          pendingDecision={pending[a.id] ?? null}
          onDecide={onDecide}
          extra={
            <Button size="sm" variant="subtle" onClick={() => navigate(paths.run(a.task_id))}>
              {m.drawer.openTask}
            </Button>
          }
        />
      ))}
    </div>
  );
};

export const ValidationsTab: React.FC<{ taskId?: string | null }> = ({ taskId: scoped }) => {
  const m = useMessages(shellMessages);
  const t = useT();
  const taskId = useScopedTaskId(scoped);
  const validations = useTaskValidations(taskId);
  if (!taskId) return <AiEmptyView className="m-2" title={m.drawer.selectTask} />;
  if (validations.isLoading) return <LoadingRows className="p-2" rows={3} />;
  if (validations.isError && !validations.data) {
    return <ErrorState className="m-2" title={m.drawer.loadFailed} message={errorMessage(validations.error)} onRetry={() => void validations.refetch()} retryLabel={t.common.retry} />;
  }
  const list = validations.data ?? [];
  if (list.length === 0) return <AiEmptyView className="m-2" title={m.drawer.noValidations} />;
  return (
    <ul className="h-full space-y-2 overflow-auto p-2">
      {list.map((v) => (
        <li key={v.id} className="space-y-1 rounded-md border border-border p-2">
          <div className="flex items-center gap-2 text-xs">
            <span className={v.passed ? "text-success" : "text-destructive"}>{v.passed ? m.drawer.passed : m.drawer.failed}</span>
            <code className="flex-1 truncate font-mono text-foreground">{v.command}</code>
            <span className="tabular-nums text-muted-foreground">{formatMessage(m.drawer.validationMeta, { ms: v.duration_ms, code: v.exit_code })}</span>
          </div>
          {v.stdout || v.stderr ? <CodeBlock language="text">{[v.stdout, v.stderr].filter(Boolean).join("\n")}</CodeBlock> : null}
        </li>
      ))}
    </ul>
  );
};

export const ProblemsTab: React.FC = () => {
  const m = useMessages(shellMessages);
  const t = useT();
  const navigate = useNavigate();
  const tasks = useTasks(null);
  const state = useRuntimeState();
  const providers = useProviders();
  const { streamStatus } = useRuntime();
  const problems: { key: string; text: string; onOpen?: () => void }[] = [];
  if (streamStatus !== "open") problems.push({ key: "stream", text: m.drawer.problemStream });
  const failedQueries: [string, { isError: boolean; error: unknown; refetch: () => unknown }][] = [
    [m.nav.overview, state],
    [m.nav.tasks, tasks],
    [m.nav.providers, providers]
  ];
  for (const [resource, query] of failedQueries) {
    if (query.isError) {
      problems.push({
        key: `q-${resource}`,
        text: formatMessage(m.drawer.problemQuery, { resource, error: errorMessage(query.error) }),
        onOpen: () => void query.refetch()
      });
    }
  }
  if (state.data?.workflow_error) problems.push({ key: "wf", text: formatMessage(m.drawer.problemWorkflow, { error: state.data.workflow_error }), onOpen: () => navigate(paths.workflow()) });
  for (const p of providers.data ?? []) {
    if (!p.installed) problems.push({ key: `p-${p.provider}`, text: formatMessage(m.drawer.problemProvider, { provider: t.provider[p.provider] }), onOpen: () => navigate(paths.providers()) });
  }
  for (const task of (tasks.data ?? []).filter((x) => x.status === "failed")) {
    problems.push({
      key: task.id,
      text: formatMessage(m.drawer.problemFailed, { identifier: task.issue.identifier, error: task.last_error ?? "" }),
      onOpen: () => navigate(paths.run(task.id))
    });
  }
  if (problems.length === 0) return <AiEmptyView className="m-2" title={m.drawer.noProblems} />;
  return (
    <ul className="h-full overflow-auto p-1 text-sm">
      {problems.map((p) => (
        <li key={p.key} className="flex items-center gap-2 rounded px-2 py-1 hover:bg-surface-hover">
          <span className="size-1.5 shrink-0 rounded-full bg-warning" aria-hidden />
          <span className="min-w-0 flex-1 break-words">{p.text}</span>
          {p.onOpen ? (
            <Button size="sm" variant="subtle" onClick={p.onOpen}>
              {t.common.open}
            </Button>
          ) : null}
        </li>
      ))}
    </ul>
  );
};

export const FocusedFeed: React.FC<{ kinds: ReadonlyArray<AgentEventKind>; emptyTitle: string; taskId?: string | null }> = ({ kinds, emptyTitle, taskId }) => {
  const m = useMessages(shellMessages);
  const t = useT();
  const { events, history } = useFocusedTaskEvents(kinds, taskId);
  if (events === null) return <AiEmptyView className="m-2" title={m.drawer.selectTask} />;
  // History not loaded (yet, or failed): never present that as "no events".
  if (!history.data && history.isLoading && events.length === 0) return <LoadingRows className="p-2" rows={3} />;
  if (!history.data && history.isError && events.length === 0) {
    return <ErrorState className="m-2" title={m.drawer.loadFailed} message={errorMessage(history.error)} onRetry={() => void history.refetch()} retryLabel={t.common.retry} />;
  }
  return <EventFeed events={events} emptyTitle={emptyTitle} />;
};

