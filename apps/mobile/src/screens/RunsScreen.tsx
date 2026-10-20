import React, { useState } from "react";
import { Button } from "@fluentui/react-components";
import { formatMessage, type RunningSessionSummary } from "@symphony/shared-types";
import {
  AiEmptyView,
  AiEventStreamCard,
  ConfirmDialog,
  Duration,
  ErrorState,
  LoadingRows,
  PhaseBadge,
  ProviderBadge,
  StreamingText,
  TimeAgo,
  formatTokens,
  useNotify
} from "@symphony/ui";
import { useLocale, useStrings } from "../i18n.js";
import { useCancelTask, useRuntimeState, useTaskEvents } from "./queries.js";

const SessionEvents: React.FC<{ taskId: string }> = ({ taskId }) => {
  const { m, t } = useStrings();
  const events = useTaskEvents(taskId, true);
  if (events.isLoading) return <LoadingRows rows={3} label={t.common.loading} />;
  if (events.isError) return <ErrorState title={m.loadEventsFailed} message={events.error.message} onRetry={() => void events.refetch()} retryLabel={t.common.retry} />;
  // The runtime returns newest first; show the latest 20 in chronological order.
  const list = [...(events.data ?? [])].reverse();
  if (list.length === 0) return <p className="text-xs text-muted-foreground">{m.noEvents}</p>;
  return (
    <ol className="space-y-0.5" aria-label={t.mobile.events}>
      {list.map((event) => (
        <li key={event.id}>
          <AiEventStreamCard event={event} mode="technical" />
        </li>
      ))}
    </ol>
  );
};

const SessionCard: React.FC<{ session: RunningSessionSummary; onCancel: (s: RunningSessionSummary) => void }> = ({ session, onCancel }) => {
  const { m, t } = useStrings();
  const locale = useLocale();
  const [open, setOpen] = useState(false);
  return (
    <li className="rounded-md border border-border bg-surface p-3">
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono text-xs font-medium text-foreground">{session.issue_identifier}</span>
        <PhaseBadge phase={session.phase} />
      </div>
      <StreamingText className="mt-1 block text-sm text-foreground">{session.last_message || session.last_event || "…"}</StreamingText>
      <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <ProviderBadge provider={session.provider} model={session.model} />
        <span>
          {m.runningFor} <Duration since={session.started_at} />
        </span>
        <span>{formatMessage(m.turns, { count: session.turn_count })}</span>
        <span>{formatMessage(m.tokenCount, { count: formatTokens(session.tokens.total_tokens, locale) })}</span>
        {session.last_event_at ? <TimeAgo iso={session.last_event_at} /> : null}
      </div>
      <div className="mt-2 flex gap-2">
        <Button size="small" appearance="subtle" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
          {open ? m.hideEvents : m.showEvents}
        </Button>
        <Button size="small" appearance="secondary" onClick={() => onCancel(session)}>
          {t.mobile.cancelRun}
        </Button>
      </div>
      {open ? (
        <div className="mt-2">
          <SessionEvents taskId={session.task_id} />
        </div>
      ) : null}
    </li>
  );
};

export const RunsScreen: React.FC = () => {
  const { m, t } = useStrings();
  const state = useRuntimeState();
  const cancel = useCancelTask();
  const notify = useNotify();
  const [target, setTarget] = useState<RunningSessionSummary | null>(null);

  if (state.isLoading) return <LoadingRows rows={4} label={t.common.loading} />;
  if (state.isError || !state.data) {
    return <ErrorState title={m.loadStateFailed} message={state.error?.message} onRetry={() => void state.refetch()} retryLabel={t.common.retry} />;
  }
  const { running, retrying } = state.data;

  return (
    <div className="space-y-3">
      {running.length === 0 ? (
        <AiEmptyView title={t.mobile.noAgents} description={m.noAgentsBody} />
      ) : (
        <ul className="space-y-2">
          {running.map((s) => (
            <SessionCard key={s.task_id} session={s} onCancel={setTarget} />
          ))}
        </ul>
      )}
      {retrying.length > 0 ? (
        <section aria-label={m.waitingToRetry} className="rounded-md border border-border bg-surface p-3">
          <h2 className="mb-1 text-xs font-medium uppercase text-muted-foreground">{m.waitingToRetry}</h2>
          <ul className="space-y-1 text-sm">
            {retrying.map((r) => (
              <li key={r.task_id} className="flex items-center justify-between gap-2">
                <span className="font-mono text-xs text-foreground">{r.identifier}</span>
                <span className="text-xs text-muted-foreground">
                  {formatMessage(m.retryAttempt, { attempt: r.attempt })} · <TimeAgo iso={r.due_at} />
                </span>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      <ConfirmDialog
        open={target !== null}
        title={target ? formatMessage(t.mobile.confirmCancel, { identifier: target.issue_identifier }) : ""}
        description={m.cancelRunBody}
        confirmLabel={t.mobile.cancelRun}
        cancelLabel={m.keepRunning}
        destructive
        pending={cancel.isPending}
        onOpenChange={(open) => {
          if (!open) setTarget(null);
        }}
        onConfirm={() => {
          if (!target || cancel.isPending) return;
          cancel.mutate(target.task_id, {
            onSuccess: () => notify("success", m.cancelRequested),
            onError: (error) => notify("error", m.cancelFailed, error.message),
            onSettled: () => setTarget(null)
          });
        }}
      />
    </div>
  );
};
