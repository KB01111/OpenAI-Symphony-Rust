import React, { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Button } from "@fluentui/react-components";
import { Activity, Bot, CheckCircle2, Coins, FolderGit2, HeartPulse, Hourglass, ShieldQuestion, XCircle, Zap } from "lucide-react";
import { formatMessage, getTranslations, type ApprovalDecision, type ApprovalRequest } from "@symphony/shared-types";
import {
  AiApprovalBanner,
  AiEmptyView,
  AiEventStreamCard,
  Duration,
  ErrorState,
  LoadingRows,
  MetricTile,
  PageHeader,
  Panel,
  PhaseBadge,
  ProviderBadge,
  StreamingText,
  TimeAgo,
  formatCurrency,
  formatDuration,
  formatTokens
} from "@symphony/ui";
import { paths } from "../../app/routes.js";
import { useLocale, useMessages } from "../../lib/i18n.js";
import { useApprovals, useDecideApproval, useProjects, useRecentEvents, useRuntimeState, useTasks } from "../../lib/queries.js";
import { useEventStore } from "../../runtime/eventStore.js";
import { useRuntime } from "../../runtime/RuntimeProvider.js";
import { useUiStore } from "../../store/uiStore.js";
import { overviewMessages } from "./messages.js";
import { computeOverviewMetrics, recentTimeline } from "./overviewLogic.js";

const LiveSessions: React.FC = () => {
  const m = useMessages(overviewMessages);
  const locale = useLocale();
  const navigate = useNavigate();
  const state = useRuntimeState();
  const projects = useProjects();
  const setNewTaskOpen = useUiStore((s) => s.setNewTaskOpen);
  const projectName = useMemo(() => new Map((projects.data ?? []).map((p) => [p.id, p.name])), [projects.data]);
  const sessions = state.data?.running ?? [];

  if (sessions.length === 0) {
    return (
      <AiEmptyView
        className="m-3"
        icon={<Bot className="size-6 text-accent" aria-hidden />}
        title={m.live.empty}
        description={m.live.emptyDetail}
        action={
          <Button appearance="primary" size="small" onClick={() => setNewTaskOpen(true)}>
            {m.live.newTask}
          </Button>
        }
      />
    );
  }

  return (
    <div className="overflow-auto">
      <table className="w-full text-sm">
        <thead className="sticky top-0 bg-surface text-xs text-muted-foreground">
          <tr className="border-b border-border text-start">
            {[m.live.task, m.live.project, m.live.provider, m.live.phase, m.live.operation, m.live.duration, m.live.tokens, m.live.lastActivity].map((h) => (
              <th key={h} scope="col" className="h-8 whitespace-nowrap px-3 text-start font-medium">
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {sessions.map((s) => (
            <tr
              key={s.task_id}
              tabIndex={0}
              aria-label={formatMessage(m.live.open, { identifier: s.issue_identifier })}
              onClick={() => navigate(paths.run(s.task_id))}
              onKeyDown={(e) => {
                if (e.key === "Enter") navigate(paths.run(s.task_id));
              }}
              className="h-row cursor-pointer border-b border-border last:border-0 hover:bg-surface-hover"
            >
              <td className="whitespace-nowrap px-3 font-mono text-xs font-medium text-foreground">{s.issue_identifier}</td>
              <td className="max-w-40 truncate px-3 text-muted-foreground">{projectName.get(s.project_id) ?? "—"}</td>
              <td className="px-3">
                <ProviderBadge provider={s.provider} model={s.model} />
              </td>
              <td className="px-3">
                <PhaseBadge phase={s.phase} />
              </td>
              <td className="max-w-72 truncate px-3 text-foreground">
                <StreamingText>{s.last_message || s.last_event || "…"}</StreamingText>
              </td>
              <td className="whitespace-nowrap px-3 text-muted-foreground">
                <Duration since={s.started_at} />
              </td>
              <td className="px-3 tabular-nums text-muted-foreground">{formatTokens(s.tokens.total_tokens, locale)}</td>
              <td className="px-3 text-muted-foreground">
                <TimeAgo iso={s.last_event_at} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
};

const PendingApprovals: React.FC = () => {
  const m = useMessages(overviewMessages);
  const navigate = useNavigate();
  const approvals = useApprovals(true);
  const decide = useDecideApproval();
  const [pending, setPending] = useState<Record<string, ApprovalDecision>>({});
  const list = approvals.data ?? [];
  const onDecide = (approval: ApprovalRequest, decision: ApprovalDecision) => {
    if (pending[approval.id]) return;
    setPending((p) => ({ ...p, [approval.id]: decision }));
    decide.mutate({ approval, decision }, { onSettled: () => setPending(({ [approval.id]: _done, ...rest }) => rest) });
  };
  if (approvals.isLoading) return <LoadingRows className="p-3" rows={2} />;
  if (approvals.isError && !approvals.data) {
    return <ErrorState className="m-3" title={m.approvals.loadFailed} message={approvals.error.message} onRetry={() => void approvals.refetch()} />;
  }
  if (list.length === 0) {
    return <AiEmptyView className="m-3 min-h-28" icon={<ShieldQuestion className="size-6 text-muted-foreground" aria-hidden />} title={m.approvals.empty} />;
  }
  return (
    <div className="space-y-2 overflow-auto p-3">
      {list.map((a) => (
        <AiApprovalBanner
          key={a.id}
          approval={a}
          compact
          pendingDecision={pending[a.id] ?? null}
          onDecide={onDecide}
          extra={
            <Button size="small" appearance="subtle" onClick={() => navigate(paths.run(a.task_id))}>
              {m.approvals.openTask}
            </Button>
          }
        />
      ))}
    </div>
  );
};

const RecentActivity: React.FC = () => {
  const m = useMessages(overviewMessages);
  const history = useRecentEvents(100);
  const live = useEventStore((s) => s.events);
  const logMode = useUiStore((s) => s.logMode);
  const items = useMemo(() => recentTimeline(history.data, live.slice(-200)), [history.data, live]);
  if (history.isLoading) return <LoadingRows className="p-3" rows={6} />;
  if (items.length === 0) {
    return <AiEmptyView className="m-3" icon={<Activity className="size-6 text-muted-foreground" aria-hidden />} title={m.timeline.empty} description={m.timeline.emptyDetail} />;
  }
  return (
    <ol className="space-y-0.5 overflow-auto p-1.5">
      {items.map((e) => (
        <li key={e.id}>
          <AiEventStreamCard event={e} mode={logMode} showIdentifier />
        </li>
      ))}
    </ol>
  );
};

/** Real-time orchestration overview. Every value comes from the runtime snapshot, tasks and events. */
export default function OverviewPage() {
  const m = useMessages(overviewMessages);
  const locale = useLocale();
  const navigate = useNavigate();
  const { streamStatus } = useRuntime();
  const state = useRuntimeState();
  const tasks = useTasks(null);
  const projects = useProjects();
  const approvals = useApprovals(true);
  const setAddProjectOpen = useUiStore((s) => s.setAddProjectOpen);
  const setTaskFilters = useUiStore((s) => s.setTaskFilters);

  const metrics = useMemo(
    () => (state.data ? computeOverviewMetrics(state.data, tasks.data ?? [], streamStatus === "open") : null),
    [state.data, tasks.data, streamStatus]
  );

  const showTasksWith = (status: "queued" | "failed" | "completed") => {
    setTaskFilters({ statuses: [status] });
    navigate(paths.tasks());
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader title={m.title} description={m.description} />
      <div className="min-h-0 flex-1 space-y-3 overflow-auto p-3">
        {state.isError ? <ErrorState title={m.metrics.health} message={state.error.message} onRetry={() => void state.refetch()} /> : null}
        {projects.data && projects.data.length === 0 ? (
          <AiEmptyView
            icon={<FolderGit2 className="size-6 text-accent" aria-hidden />}
            title={m.onboarding.title}
            description={m.onboarding.detail}
            action={
              <Button appearance="primary" size="small" onClick={() => setAddProjectOpen(true)}>
                {m.onboarding.action}
              </Button>
            }
          />
        ) : null}

        {!metrics ? (
          <LoadingRows rows={2} />
        ) : (
          <section aria-label={m.title} className="grid grid-cols-2 gap-2 md:grid-cols-3 xl:grid-cols-5 2xl:grid-cols-9">
            <MetricTile
              label={m.metrics.activeAgents}
              icon={<Bot className="size-3.5" aria-hidden />}
              value={metrics.activeAgents}
              tone={metrics.activeAgents > 0 ? "accent" : "neutral"}
              hint={formatMessage(m.metrics.capacity, { count: state.data?.hardware.effective_max_concurrency ?? 0 })}
              onClick={() => navigate(paths.runs())}
            />
            <MetricTile
              label={m.metrics.queued}
              icon={<Hourglass className="size-3.5" aria-hidden />}
              value={metrics.queued}
              hint={metrics.retrying > 0 ? formatMessage(m.metrics.retrying, { count: metrics.retrying }) : undefined}
              onClick={() => showTasksWith("queued")}
            />
            <MetricTile
              label={m.metrics.approvals}
              icon={<ShieldQuestion className="size-3.5" aria-hidden />}
              value={approvals.data ? approvals.data.length : "—"}
              tone={(approvals.data?.length ?? 0) > 0 ? "warning" : "neutral"}
              onClick={() => useUiStore.getState().setDrawerTab("approvals")}
            />
            <MetricTile
              label={m.metrics.failed}
              icon={<XCircle className="size-3.5" aria-hidden />}
              value={metrics.failed}
              tone={metrics.failed > 0 ? "danger" : "neutral"}
              onClick={() => showTasksWith("failed")}
            />
            <MetricTile
              label={m.metrics.completed}
              icon={<CheckCircle2 className="size-3.5" aria-hidden />}
              value={metrics.completed}
              tone={metrics.completed > 0 ? "success" : "neutral"}
              hint={metrics.humanReview > 0 ? formatMessage(m.metrics.reviewWaiting, { count: metrics.humanReview }) : undefined}
              onClick={() => showTasksWith("completed")}
            />
            <MetricTile
              label={m.metrics.repositories}
              icon={<FolderGit2 className="size-3.5" aria-hidden />}
              value={metrics.activeRepositories}
              hint={formatMessage(m.metrics.ofRepositories, { count: projects.data?.length ?? 0 })}
              onClick={() => navigate(paths.projects())}
            />
            <MetricTile
              label={m.metrics.tokens}
              icon={<Zap className="size-3.5" aria-hidden />}
              value={formatTokens(metrics.totalTokens, locale)}
              hint={formatMessage(m.metrics.runningFor, { duration: formatDuration(metrics.secondsRunning, getTranslations(locale)) })}
            />
            <MetricTile
              label={m.metrics.cost}
              icon={<Coins className="size-3.5" aria-hidden />}
              value={formatCurrency(metrics.estimatedCost, locale)}
            />
            <MetricTile
              label={m.metrics.health}
              icon={<HeartPulse className="size-3.5" aria-hidden />}
              value={m.metrics[metrics.health]}
              tone={metrics.health === "healthy" ? "success" : "warning"}
              hint={state.data?.hardware.dispatch_delayed_reason ?? state.data?.workflow_error ?? undefined}
            />
          </section>
        )}

        <div className="grid min-h-0 gap-3 xl:grid-cols-[minmax(0,2fr)_minmax(320px,1fr)]">
          <Panel title={m.live.title} className="min-h-56">
            <LiveSessions />
          </Panel>
          <Panel title={m.approvals.title} className="max-h-[420px] min-h-56">
            <PendingApprovals />
          </Panel>
        </div>
        <Panel title={m.timeline.title} className="max-h-[520px] min-h-56">
          <RecentActivity />
        </Panel>
      </div>
    </div>
  );
}
