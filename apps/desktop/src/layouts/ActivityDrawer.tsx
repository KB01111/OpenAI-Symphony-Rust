import React, { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Button, CounterBadge, Tab, TabList, Tooltip } from "@fluentui/react-components";
import { ChevronDown, ChevronUp } from "lucide-react";
import { errorMessage } from "@symphony/api-client";
import { formatMessage, type AgentEvent, type ApprovalDecision, type ApprovalRequest } from "@symphony/shared-types";
import {
  AiApprovalBanner,
  AiEmptyView,
  AiEventStreamCard,
  ErrorState,
  LoadingRows,
  LogModeSwitch,
  SdkCodeBlock,
  VirtualList,
  formatAgentEvent,
  type AgentEventKind
} from "@symphony/ui";
import { shellMessages } from "../app/messages.js";
import { paths } from "../app/routes.js";
import { useMessages, useT } from "../lib/i18n.js";
import { useApprovals, useDecideApproval, useProviders, useRecentEvents, useRuntimeState, useTaskEventHistory, useTaskValidations, useTasks } from "../lib/queries.js";
import { mergeEventHistory, useEventStore } from "../runtime/eventStore.js";
import { useRuntime } from "../runtime/RuntimeProvider.js";
import { useFocusedTaskId } from "../hooks/useFocusedTaskId.js";
import { useUiStore, type DrawerTab } from "../store/uiStore.js";

const EventFeed: React.FC<{ events: AgentEvent[]; showIdentifier?: boolean; emptyTitle: string; emptyDetail?: string }> = ({
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

function useFocusedTaskEvents(kinds: ReadonlyArray<AgentEventKind>) {
  const taskId = useFocusedTaskId();
  const history = useTaskEventHistory(taskId);
  const live = useEventStore((s) => s.events);
  const events = useMemo(() => {
    if (!taskId) return null;
    const merged = mergeEventHistory(history.data, live.filter((e) => e.task_id === taskId));
    return merged.filter((e) => kinds.includes(formatAgentEvent(e).kind));
  }, [taskId, history.data, live, kinds]);
  return { events, history };
}

const AGENT_KINDS: AgentEventKind[] = ["message", "reasoning", "file_change", "tool_call", "error"];
const TERMINAL_KINDS: AgentEventKind[] = ["command"];

const ApprovalsTab: React.FC = () => {
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
  const list = approvals.data ?? [];
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
            <Button size="small" appearance="subtle" onClick={() => navigate(paths.run(a.task_id))}>
              {m.drawer.openTask}
            </Button>
          }
        />
      ))}
    </div>
  );
};

const ValidationsTab: React.FC = () => {
  const m = useMessages(shellMessages);
  const t = useT();
  const taskId = useFocusedTaskId();
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
          {v.stdout || v.stderr ? <SdkCodeBlock language="text">{[v.stdout, v.stderr].filter(Boolean).join("\n")}</SdkCodeBlock> : null}
        </li>
      ))}
    </ul>
  );
};

const ProblemsTab: React.FC = () => {
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
            <Button size="small" appearance="subtle" onClick={p.onOpen}>
              {t.common.open}
            </Button>
          ) : null}
        </li>
      ))}
    </ul>
  );
};

const FocusedFeed: React.FC<{ kinds: ReadonlyArray<AgentEventKind>; emptyTitle: string }> = ({ kinds, emptyTitle }) => {
  const m = useMessages(shellMessages);
  const t = useT();
  const { events, history } = useFocusedTaskEvents(kinds);
  if (events === null) return <AiEmptyView className="m-2" title={m.drawer.selectTask} />;
  // History not loaded (yet, or failed): never present that as "no events".
  if (!history.data && history.isLoading && events.length === 0) return <LoadingRows className="p-2" rows={3} />;
  if (!history.data && history.isError && events.length === 0) {
    return <ErrorState className="m-2" title={m.drawer.loadFailed} message={errorMessage(history.error)} onRetry={() => void history.refetch()} retryLabel={t.common.retry} />;
  }
  return <EventFeed events={events} emptyTitle={emptyTitle} />;
};

/** Bottom dock: live events, scoped agent/terminal output, validations, approvals, problems. */
export const ActivityDrawer: React.FC<{ collapsed: boolean; onToggle: () => void }> = ({ collapsed, onToggle }) => {
  const m = useMessages(shellMessages);
  const tab = useUiStore((s) => s.drawerTab);
  const setTab = useUiStore((s) => s.setDrawerTab);
  const logMode = useUiStore((s) => s.logMode);
  const setLogMode = useUiStore((s) => s.setLogMode);
  const live = useEventStore((s) => s.events);
  // Persisted history (survives reloads) merged with the live stream, deduplicated by id.
  const history = useRecentEvents(200);
  const events = useMemo(() => mergeEventHistory(history.data, live), [history.data, live]);
  const approvals = useApprovals(true);
  const pending = approvals.data?.length ?? 0;

  const tabs: { key: DrawerTab; label: string; badge?: number }[] = [
    { key: "events", label: m.drawer.events },
    { key: "agent", label: m.drawer.agent },
    { key: "terminal", label: m.drawer.terminal },
    { key: "validations", label: m.drawer.validations },
    { key: "approvals", label: m.drawer.approvals, badge: pending },
    { key: "problems", label: m.drawer.problems }
  ];

  return (
    <section aria-label={m.drawer.label} className="flex h-full min-h-0 flex-col border-t border-border bg-surface">
      <header className="flex h-9 shrink-0 items-center gap-2 overflow-hidden border-b border-border px-1">
        <TabList
          size="small"
          selectedValue={tab}
          onTabSelect={(_, d) => setTab(d.value as DrawerTab)}
          aria-label={m.drawer.label}
          className="min-w-0 flex-1 flex-nowrap overflow-x-auto [scrollbar-width:none]"
        >
          {tabs.map((x) => (
            <Tab key={x.key} value={x.key}>
              <span className="inline-flex items-center gap-1">
                {x.label}
                {x.badge ? <CounterBadge count={x.badge} size="extra-small" color="important" /> : null}
              </span>
            </Tab>
          ))}
        </TabList>
        <span className="flex shrink-0 items-center gap-2">
          {!collapsed && (tab === "events" || tab === "agent" || tab === "terminal") ? <LogModeSwitch value={logMode} onChange={setLogMode} /> : null}
          <Tooltip content={collapsed ? m.drawer.expand : m.drawer.collapse} relationship="label">
            <Button size="small" appearance="subtle" icon={collapsed ? <ChevronUp className="size-4" /> : <ChevronDown className="size-4" />} onClick={onToggle} />
          </Tooltip>
        </span>
      </header>
      {collapsed ? null : (
        <div className="min-h-0 flex-1 overflow-hidden" role="tabpanel">
          {tab === "events" ? (
            <EventFeed events={events} showIdentifier emptyTitle={m.drawer.noEvents} emptyDetail={m.drawer.noEventsDetail} />
          ) : tab === "agent" ? (
            <FocusedFeed kinds={AGENT_KINDS} emptyTitle={m.drawer.noEvents} />
          ) : tab === "terminal" ? (
            <FocusedFeed kinds={TERMINAL_KINDS} emptyTitle={m.drawer.noCommands} />
          ) : tab === "validations" ? (
            <ValidationsTab />
          ) : tab === "approvals" ? (
            <ApprovalsTab />
          ) : (
            <ProblemsTab />
          )}
        </div>
      )}
    </section>
  );
};
