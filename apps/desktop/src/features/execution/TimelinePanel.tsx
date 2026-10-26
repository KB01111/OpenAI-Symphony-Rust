import React, { useCallback, useMemo, useRef, useState } from "react";
import { Button, ToggleButton } from "@fluentui/react-components";
import { ArrowDownToLine, History } from "lucide-react";
import { useShallow } from "zustand/react/shallow";
import { formatMessage, isActivePhase, type AgentEvent, type ApprovalDecision, type ApprovalRequest, type Task } from "@symphony/shared-types";
import {
  AiApprovalBanner,
  AiEmptyView,
  AiEventStreamCard,
  ErrorState,
  EVENT_KINDS,
  LoadingRows,
  LogModeSwitch,
  StreamingText,
  VirtualList,
  type AgentEventKind
} from "@symphony/ui";
import { useMessages, useT } from "../../lib/i18n.js";
import { useApprovals, useDecideApproval, useOlderTaskEvents, useTaskEventHistory } from "../../lib/queries.js";
import { useEventStore } from "../../runtime/eventStore.js";
import { useUiStore } from "../../store/uiStore.js";
import { executionMessages } from "./messages.js";
import { buildTaskTimeline, countByKind, filterByKinds, toggleKind, unseenSince, type PauseAnchor } from "./timeline.js";

/** Pending approvals for one task, pinned above the timeline. One decision in flight per approval. */
const TaskApprovals: React.FC<{ taskId: string }> = ({ taskId }) => {
  const m = useMessages(executionMessages);
  const approvals = useApprovals(true);
  const decide = useDecideApproval();
  const [pending, setPending] = useState<Record<string, ApprovalDecision>>({});
  const inFlight = useRef(new Set<string>());
  const list = useMemo(
    () => (approvals.data ?? []).filter((a) => a.task_id === taskId && a.status === "pending"),
    [approvals.data, taskId]
  );
  const onDecide = useCallback(
    (approval: ApprovalRequest, decision: ApprovalDecision) => {
      // Synchronous guard: a double click can fire before React re-renders the disabled banner.
      if (inFlight.current.has(approval.id)) return;
      inFlight.current.add(approval.id);
      setPending((p) => ({ ...p, [approval.id]: decision }));
      decide.mutate(
        { approval, decision },
        {
          onSettled: () => {
            inFlight.current.delete(approval.id);
            setPending(({ [approval.id]: _done, ...rest }) => rest);
          }
        }
      );
    },
    [decide]
  );
  if (approvals.isError && !approvals.data) {
    // Pending approvals could not be loaded: say so instead of silently showing none.
    return (
      <ErrorState
        className="m-2 shrink-0"
        intent="warning"
        title={m.timeline.approvalsFailed}
        message={approvals.error.message}
        onRetry={() => void approvals.refetch()}
      />
    );
  }
  if (list.length === 0) return null;
  return (
    <section aria-label={m.timeline.approvals} className="max-h-[60%] shrink-0 space-y-2 overflow-auto border-b border-border p-2">
      {list.map((a) => (
        <AiApprovalBanner key={a.id} approval={a} pendingDecision={pending[a.id] ?? null} onDecide={onDecide} />
      ))}
    </section>
  );
};

export interface TimelinePanelProps {
  task: Task;
  /** Latest operation reported by the live session, if any. */
  operation?: string | null;
}

/** Center column: merged persisted + live event timeline with kind filters and tail-following. */
export const TimelinePanel: React.FC<TimelinePanelProps> = ({ task, operation }) => {
  const m = useMessages(executionMessages);
  const t = useT();
  const logMode = useUiStore((s) => s.logMode);
  const setLogMode = useUiStore((s) => s.setLogMode);
  const history = useTaskEventHistory(task.id);
  const older = useOlderTaskEvents(task.id, history.data);
  // Shallow-compared selection: batches for other tasks do not re-render this timeline.
  const live = useEventStore(useShallow((s) => s.events.filter((e) => e.task_id === task.id)));
  const [hidden, setHidden] = useState<ReadonlySet<AgentEventKind>>(() => new Set());
  const [following, setFollowing] = useState(true);
  // The newest event when the operator stopped following; everything after it is new. An event, not
  // a position, so loading older events (prepended) does not count as new activity.
  const [pausedAfter, setPausedAfter] = useState<PauseAnchor | "" | null>(null);
  const [epoch, setEpoch] = useState(0);

  const persisted = useMemo(
    () => (older.older.length > 0 ? [...(history.data ?? []), ...older.older] : history.data),
    [history.data, older.older]
  );
  const all = useMemo(() => buildTaskTimeline(task.id, persisted, live), [task.id, persisted, live]);
  const counts = useMemo(() => countByKind(all), [all]);
  const visible = useMemo(() => filterByKinds(all, hidden), [all, hidden]);
  // The pause baseline is taken from the unfiltered timeline, so changing kind filters while
  // paused neither invents nor hides "new" events.
  const newest = useRef<PauseAnchor | null>(null);
  const last = all[all.length - 1];
  newest.current = last ? { id: last.id, timestamp: last.timestamp } : null;

  const onFollowChange = useCallback((next: boolean) => {
    setFollowing(next);
    setPausedAfter(next ? null : newest.current ?? "");
  }, []);
  const jumpToLatest = useCallback(() => {
    // Remounting the list restores tail-following and scrolls to the newest event.
    setEpoch((e) => e + 1);
    setFollowing(true);
    setPausedAfter(null);
  }, []);

  const renderItem = useCallback((e: AgentEvent) => <AiEventStreamCard event={e} mode={logMode} />, [logMode]);
  const getKey = useCallback((e: AgentEvent) => e.id, []);

  const chips = EVENT_KINDS.filter((k) => counts[k] > 0 || hidden.has(k));
  const unseen = useMemo(() => unseenSince(all, pausedAfter, hidden), [all, hidden, pausedAfter]);
  const active = isActivePhase(task.current_phase);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-border px-2 py-1.5">
        <LogModeSwitch value={logMode} onChange={setLogMode} />
        <span className="text-xs tabular-nums text-muted-foreground">{formatMessage(m.timeline.events, { count: all.length })}</span>
        <div role="group" aria-label={m.timeline.filters} className="ms-auto flex flex-wrap items-center gap-1">
          {chips.map((kind) => (
            <ToggleButton
              key={kind}
              size="small"
              shape="circular"
              appearance={hidden.has(kind) ? "outline" : "secondary"}
              checked={!hidden.has(kind)}
              onClick={() => setHidden((h) => toggleKind(h, kind))}
            >
              {formatMessage(m.timeline.kindCount, { label: m.timeline.kinds[kind], count: counts[kind] })}
            </ToggleButton>
          ))}
          {hidden.size > 0 ? (
            <Button size="small" appearance="subtle" onClick={() => setHidden(new Set())}>
              {m.timeline.showAll}
            </Button>
          ) : null}
        </div>
      </div>

      <TaskApprovals taskId={task.id} />

      {active && task.current_phase ? (
        <div role="status" className="flex shrink-0 items-center gap-2 border-b border-border bg-info-subtle px-3 py-1 text-xs text-info">
          <StreamingText>{formatMessage(m.timeline.streaming, { phase: t.phase[task.current_phase] })}</StreamingText>
          {operation ? <span className="min-w-0 flex-1 truncate text-muted-foreground">{operation}</span> : null}
        </div>
      ) : null}

      {older.hasMore && history.data ? (
        <div className="flex shrink-0 items-center gap-2 border-b border-border px-2 py-1 text-xs text-muted-foreground">
          <Button
            size="small"
            appearance="subtle"
            icon={<History className="size-4" />}
            disabled={older.loading}
            onClick={() => void older.loadOlder()}
          >
            {older.loading ? m.timeline.loadingOlder : m.timeline.loadOlder}
          </Button>
          {older.error ? (
            <span role="alert" className="text-destructive">
              {m.timeline.olderFailed}
            </span>
          ) : null}
        </div>
      ) : null}

      <div className="relative min-h-0 flex-1">
        {history.isLoading && all.length === 0 ? (
          <LoadingRows className="p-3" rows={6} label={t.common.loading} />
        ) : history.isError && all.length === 0 ? (
          <ErrorState className="m-3" title={t.errors.generic} message={history.error.message} onRetry={() => void history.refetch()} />
        ) : all.length === 0 ? (
          <AiEmptyView className="m-3" title={m.timeline.empty} description={m.timeline.emptyDetail} />
        ) : visible.length === 0 ? (
          <p className="p-3 text-sm text-muted-foreground">{m.timeline.filteredEmpty}</p>
        ) : (
          <VirtualList
            key={epoch}
            items={visible}
            getKey={getKey}
            renderItem={renderItem}
            estimateSize={56}
            followTail
            onFollowChange={onFollowChange}
            className="h-full px-1 py-1"
            ariaLabel={m.panels.timeline}
          />
        )}
        {!following && visible.length > 0 ? (
          <div className="pointer-events-none absolute inset-x-0 bottom-3 flex justify-center">
            <Button
              className="pointer-events-auto shadow-md"
              size="small"
              appearance="primary"
              shape="circular"
              icon={<ArrowDownToLine className="size-4" />}
              onClick={jumpToLatest}
            >
              {unseen > 0 ? formatMessage(m.timeline.jumpNew, { count: unseen }) : m.timeline.jump}
            </Button>
          </div>
        ) : null}
      </div>
    </div>
  );
};
