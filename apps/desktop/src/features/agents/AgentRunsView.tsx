import React, { useCallback, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { MonitorPlay, Play, RotateCcw, Square } from "lucide-react";
import { ALL_PROVIDERS, formatMessage, isActivePhase, type ProviderCapabilities, type ProviderKind } from "@symphony/shared-types";
import {
  AiEmptyView,
  Badge,
  Button,
  ConfirmDialog,
  Duration,
  ErrorState,
  Hint,
  LoadingRows,
  PhaseBadge,
  ProviderBadge,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  StreamingText,
  Tabs,
  TabsList,
  TabsTrigger,
  TaskStatusBadge,
  TimeAgo,
  VirtualList,
  cn,
  formatCurrency,
  formatTokens,
  useNow
} from "@symphony/ui";
import { paths } from "../../app/routes.js";
import { useLocale, useMessages, useT } from "../../lib/i18n.js";
import {
  useApprovals,
  useCancelTask,
  useDispatchTask,
  useProjects,
  useProviders,
  useRetryTask,
  useRuntimeState,
  useTasks
} from "../../lib/queries.js";
import {
  AGENT_GROUP_BY,
  AGENT_SEGMENTS,
  FINISHED_SEGMENTS,
  buildAgentRows,
  groupAgentRows,
  groupRowsBy,
  rowActions,
  type AgentGroupBy,
  type AgentRow,
  type AgentSegment
} from "./grouping.js";
import { agentsMessages } from "./messages.js";

type SegmentFilter = AgentSegment | "all";

type ListItem = { kind: "header"; id: string; label: string; count: number } | { kind: "row"; id: string; row: AgentRow };

interface RowProps {
  row: AgentRow;
  projectName: string | null;
  capabilities: ProviderCapabilities | undefined;
  busy: boolean;
  onOpen: (row: AgentRow) => void;
  onDispatch: (row: AgentRow) => void;
  onRetry: (row: AgentRow) => void;
  onCancel: (row: AgentRow) => void;
}

const AgentRunRow = React.memo(function AgentRunRow({
  row,
  projectName,
  capabilities,
  busy,
  onOpen,
  onDispatch,
  onRetry,
  onCancel
}: RowProps) {
  const m = useMessages(agentsMessages);
  const locale = useLocale();
  const allowed = rowActions(row, capabilities);
  const finished = FINISHED_SEGMENTS.includes(row.segment);
  const iconButton = (label: string, icon: React.ReactElement, onClick: () => void, primary = false) => (
    <Hint label={label} asLabel>
      <Button
        size="sm"
        variant={primary ? "default" : "subtle"}
        disabled={busy}
        onClick={(e) => {
          e.stopPropagation();
          onClick();
        }}>{icon}</Button>
    </Hint>
  );

  return (
    <div
      role="group"
      aria-label={`${row.identifier} ${row.title ?? ""}`.trim()}
      onClick={() => onOpen(row)}
      className="flex cursor-pointer items-start gap-3 border-b border-border px-4 py-2 hover:bg-surface-hover focus-within:bg-surface-hover"
    >
      <div className="min-w-0 flex-1 space-y-1">
        <div className="flex min-w-0 flex-wrap items-center gap-1.5">
          <span className="font-mono text-xs text-muted-foreground">{row.identifier}</span>
          {/* The title is the keyboard and screen-reader target; clicking elsewhere on the row is a mouse shortcut. */}
          <button
            type="button"
            className="min-w-0 max-w-[32rem] truncate rounded-sm text-start text-sm font-medium text-foreground outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent"
            title={row.title ?? undefined}
            onClick={(e) => {
              e.stopPropagation();
              onOpen(row);
            }}
          >
            {row.title ?? m.row.unknownTitle}
          </button>
          {row.status ? <TaskStatusBadge status={row.status} /> : null}
          {row.phase && (row.live || !finished) ? <PhaseBadge phase={row.phase} /> : null}
          {row.provider ? <ProviderBadge provider={row.provider} model={row.model} /> : null}
          {row.pendingApprovals > 0 ? (
            <Badge tone="warning">
              {formatMessage(m.row.pendingApprovals, { count: row.pendingApprovals })}
            </Badge>
          ) : null}
        </div>
        <dl className="flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-muted-foreground">
          <div className="flex gap-1">
            <dt className="sr-only">{m.groupBy.project}</dt>
            <dd className="truncate">{projectName ?? m.row.unknownProject}</dd>
          </div>
          {row.live || row.durationSeconds > 0 ? (
            <div className="flex gap-1">
              <dt className="sr-only">{m.row.duration}</dt>
              <dd>{row.live && row.startedAt ? <Duration since={row.startedAt} /> : <Duration seconds={row.durationSeconds} />}</dd>
            </div>
          ) : null}
          {row.totalTokens > 0 ? (
            <div className="flex gap-1">
              <dt className="sr-only">{m.row.tokensLabel}</dt>
              <dd className="tabular-nums">{formatMessage(m.row.tokens, { count: formatTokens(row.totalTokens, locale) })}</dd>
            </div>
          ) : null}
          {row.costUsd !== null && row.costUsd > 0 ? (
            <div className="flex gap-1">
              <dt className="sr-only">{m.row.cost}</dt>
              <dd className="tabular-nums">{formatCurrency(row.costUsd, locale)}</dd>
            </div>
          ) : null}
          {row.turnCount !== null ? (
            <div className="flex gap-1">
              <dt className="sr-only">{m.row.turnsLabel}</dt>
              <dd className="tabular-nums">{formatMessage(m.row.turns, { count: row.turnCount })}</dd>
            </div>
          ) : null}
          {row.attempt ? (
            <div className="flex gap-1">
              <dt className="sr-only">{m.row.attemptLabel}</dt>
              <dd className="tabular-nums">{formatMessage(m.row.attempt, { number: row.attempt })}</dd>
            </div>
          ) : null}
          {row.segment === "retrying" && row.dueAt ? (
            <div className="flex gap-1 text-warning">
              <dt>{m.row.retryDue}</dt>
              <dd>
                <time dateTime={row.dueAt}>{new Date(row.dueAt).toLocaleTimeString(locale)}</time>
              </dd>
            </div>
          ) : null}
          {row.live && row.lastEventAt ? (
            <div className="flex gap-1">
              <dt>{m.row.lastEvent}</dt>
              <dd>
                <TimeAgo iso={row.lastEventAt} />
              </dd>
            </div>
          ) : null}
          {finished && row.finishedAt ? (
            <div className="flex gap-1">
              <dt>{m.row.finished}</dt>
              <dd>
                <TimeAgo iso={row.finishedAt} />
              </dd>
            </div>
          ) : null}
        </dl>
        {row.live ? (
          <p className="truncate text-xs text-foreground" title={row.operation ?? undefined}>
            <span className="sr-only">{m.row.operation}: </span>
            {row.operation ? <StreamingText active={isActivePhase(row.phase)}>{row.operation}</StreamingText> : <span className="text-subtle-foreground">{m.row.idle}</span>}
          </p>
        ) : row.error && (row.segment === "failed" || row.segment === "retrying") ? (
          <p className="truncate text-xs text-destructive" title={row.error}>
            <span className="sr-only">{m.row.error}: </span>
            {row.error}
          </p>
        ) : null}
      </div>
      {/* Clicks here never open the run: a disabled button (pointer-events: none) would otherwise let them fall through to the row. */}
      <div
        role="toolbar"
        aria-label={formatMessage(m.actions.label, { identifier: row.identifier })}
        className="flex shrink-0 items-center gap-0.5"
        onClick={(e) => e.stopPropagation()}
      >
        {allowed.dispatch ? iconButton(m.actions.dispatch, <Play className="size-4" />, () => onDispatch(row), true) : null}
        {allowed.retry ? iconButton(m.actions.retry, <RotateCcw className="size-4" />, () => onRetry(row)) : null}
        {allowed.cancel ? iconButton(m.actions.cancel, <Square className="size-4" />, () => onCancel(row)) : null}
        {iconButton(m.actions.openExecution, <MonitorPlay className="size-4" />, () => onOpen(row))}
      </div>
    </div>
  );
});

/** The Runs view of the Tasks page: live sessions, retry queue, queued and recently finished runs. */
export default function AgentRunsView() {
  const m = useMessages(agentsMessages);
  const t = useT();
  const navigate = useNavigate();
  const now = useNow();
  const state = useRuntimeState();
  const tasks = useTasks();
  const approvals = useApprovals(true);
  const projects = useProjects();
  const providers = useProviders();
  const dispatch = useDispatchTask();
  const retry = useRetryTask();
  const cancel = useCancelTask();

  const [filter, setFilter] = useState<SegmentFilter>("all");
  const [groupBy, setGroupBy] = useState<AgentGroupBy>("status");
  const [cancelTarget, setCancelTarget] = useState<AgentRow | null>(null);

  const rows = useMemo(
    () => buildAgentRows({ snapshot: state.data, tasks: tasks.data, approvals: approvals.data, projects: projects.data, now }),
    [state.data, tasks.data, approvals.data, projects.data, now]
  );
  const counts = useMemo(() => {
    const groups = groupAgentRows(rows);
    return Object.fromEntries(AGENT_SEGMENTS.map((s) => [s, groups[s].length])) as Record<AgentSegment, number>;
  }, [rows]);
  const projectNames = useMemo(() => new Map((projects.data ?? []).map((p) => [p.id, p.name])), [projects.data]);
  const capabilities = useMemo(
    () => new Map<ProviderKind, ProviderCapabilities>((providers.data ?? []).map((p) => [p.provider, p.capabilities])),
    [providers.data]
  );

  const items = useMemo<ListItem[]>(() => {
    const visible = filter === "all" ? rows : rows.filter((r) => r.segment === filter);
    const groups = groupRowsBy(visible, groupBy, { providerOrder: ALL_PROVIDERS, projectNames });
    const label = (key: string | null) =>
      groupBy === "status"
        ? m.segments[key as AgentSegment]
        : groupBy === "provider"
          ? key
            ? (t.provider[key as ProviderKind] ?? key)
            : m.unknownProvider
          : key
            ? (projectNames.get(key) ?? m.row.unknownProject)
            : m.row.unknownProject;
    return groups.flatMap((g) => [
      { kind: "header" as const, id: `h:${g.key ?? "none"}`, label: label(g.key), count: g.rows.length },
      ...g.rows.map((row) => ({ kind: "row" as const, id: row.taskId, row }))
    ]);
  }, [rows, filter, groupBy, projectNames, m, t]);

  const pendingTaskId =
    (dispatch.isPending && dispatch.variables) || (retry.isPending && retry.variables) || (cancel.isPending && cancel.variables) || null;

  const onOpen = useCallback((row: AgentRow) => navigate(paths.run(row.taskId)), [navigate]);
  const onDispatch = useCallback((row: AgentRow) => !dispatch.isPending && dispatch.mutate(row.taskId), [dispatch]);
  const onRetry = useCallback((row: AgentRow) => !retry.isPending && retry.mutate(row.taskId), [retry]);
  const onCancel = useCallback((row: AgentRow) => setCancelTarget(row), []);

  const loading = (state.isLoading && !state.data) || (tasks.isLoading && !tasks.data);
  // Approvals decide which rows wait for a decision, so a failed approvals load is a failure too.
  const failure =
    (state.isError && !state.data ? state.error : null) ??
    (tasks.isError && !tasks.data ? tasks.error : null) ??
    (approvals.isError && !approvals.data ? approvals.error : null);

  let body: React.ReactNode;
  if (failure) {
    body = (
      <div className="p-4">
        <ErrorState
          title={m.loadFailed}
          message={failure.message}
          onRetry={() => {
            void state.refetch();
            void tasks.refetch();
            void approvals.refetch();
          }}
        />
      </div>
    );
  } else if (loading) {
    body = (
      <div className="p-4">
        <LoadingRows rows={6} label={t.common.loading} />
      </div>
    );
  } else if (rows.length === 0) {
    body = (
      <div className="p-4">
        <AiEmptyView
          title={m.empty.title}
          description={m.empty.description}
          action={
            <Button size="sm" variant="default" onClick={() => navigate(paths.tasks())}>
              {m.empty.cta}
            </Button>
          }
        />
      </div>
    );
  } else if (items.length === 0) {
    body = (
      <div className="p-4">
        <AiEmptyView title={m.empty.filtered} />
      </div>
    );
  } else {
    body = (
      <VirtualList<ListItem>
        items={items}
        getKey={(item) => item.id}
        estimateSize={72}
        className="h-full"
        ariaLabel={m.list}
        renderItem={(item) =>
          item.kind === "header" ? (
            <h2 className="flex h-8 items-center gap-2 border-b border-border bg-surface-sunken px-4 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
              <span className="truncate">{item.label}</span>
              <span className="font-normal normal-case tabular-nums text-subtle-foreground">
                {formatMessage(m.groupCount, { count: item.count })}
              </span>
            </h2>
          ) : (
            <AgentRunRow
              row={item.row}
              projectName={item.row.projectId ? (projectNames.get(item.row.projectId) ?? null) : null}
              capabilities={item.row.provider ? capabilities.get(item.row.provider) : undefined}
              busy={pendingTaskId === item.row.taskId}
              onOpen={onOpen}
              onDispatch={onDispatch}
              onRetry={onRetry}
              onCancel={onCancel}
            />
          )
        }
      />
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border bg-surface px-2 py-1">
        <Tabs value={filter} onValueChange={(v) => setFilter(v as SegmentFilter)} className="min-w-0 flex-1">
          <TabsList aria-label={m.segmentTabs} className="flex-wrap border-b-0 px-0">
            <TabsTrigger value="all">
              {m.all} <span className="tabular-nums text-subtle-foreground">{rows.length}</span>
            </TabsTrigger>
            {/* Empty segments are noise; the selected one stays so the filter can be left. */}
            {AGENT_SEGMENTS.filter((s) => counts[s] > 0 || s === filter).map((s) => (
              <TabsTrigger key={s} value={s}>
                {m.segments[s]} <span className="tabular-nums text-subtle-foreground">{counts[s]}</span>
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
        <div className={cn("flex items-center gap-2 text-xs text-muted-foreground", rows.length === 0 && "hidden")}>
          <span id="agents-group-by">{m.groupByLabel}</span>
          <Select value={groupBy} onValueChange={(v) => setGroupBy(v as AgentGroupBy)}>
            <SelectTrigger className="w-36" aria-labelledby="agents-group-by">
              <SelectValue>{m.groupBy[groupBy]}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {AGENT_GROUP_BY.map((g) => (
                <SelectItem key={g} value={g}>
                  {m.groupBy[g]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>
      {FINISHED_SEGMENTS.includes(filter as AgentSegment) ? (
        <p className="shrink-0 border-b border-border bg-surface px-4 py-1 text-xs text-muted-foreground">{m.recentHint}</p>
      ) : null}
      <div className="min-h-0 flex-1 bg-background">{body}</div>
      <ConfirmDialog
        open={cancelTarget !== null}
        title={formatMessage(m.cancelDialog.title, { identifier: cancelTarget?.identifier ?? "" })}
        description={m.cancelDialog.description}
        confirmLabel={m.cancelDialog.confirm}
        destructive
        pending={cancel.isPending}
        onOpenChange={(open) => !open && setCancelTarget(null)}
        onConfirm={() => {
          if (!cancelTarget || cancel.isPending) return;
          cancel.mutate(cancelTarget.taskId, { onSettled: () => setCancelTarget(null) });
        }}
      />
    </div>
  );
}
