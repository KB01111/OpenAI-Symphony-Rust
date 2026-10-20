import React from "react";
import { MessageBar, MessageBarBody } from "@fluentui/react-components";
import { formatMessage } from "@symphony/shared-types";
import { ErrorState, LoadingRows, MetricTile, formatCurrency, formatTokens } from "@symphony/ui";
import { useLocale, useStrings } from "../i18n.js";
import { usePendingApprovals, useRuntimeState } from "./queries.js";

export const OverviewScreen: React.FC<{ onOpen: (tab: "runs" | "approvals" | "tasks") => void }> = ({ onOpen }) => {
  const { m, t } = useStrings();
  const locale = useLocale();
  const state = useRuntimeState();
  const approvals = usePendingApprovals();

  if (state.isLoading) return <LoadingRows rows={5} label={t.common.loading} />;
  if (state.isError || !state.data) {
    return <ErrorState title={m.loadStateFailed} message={state.error?.message} onRetry={() => void state.refetch()} retryLabel={t.common.retry} />;
  }
  const s = state.data;
  const hw = s.hardware;
  const power = hw.on_battery
    ? hw.battery_percent !== null
      ? formatMessage(m.onBattery, { percent: Math.round(hw.battery_percent) })
      : m.onBatteryUnknown
    : m.acPower;

  return (
    <div className="space-y-3">
      {s.workflow_error ? (
        <MessageBar intent="error">
          <MessageBarBody>
            {m.workflowError}: {s.workflow_error}
          </MessageBarBody>
        </MessageBar>
      ) : null}
      {hw.dispatch_delayed_reason ? (
        <MessageBar intent="warning">
          <MessageBarBody>{formatMessage(m.dispatchDelayed, { reason: hw.dispatch_delayed_reason })}</MessageBarBody>
        </MessageBar>
      ) : null}
      <section className="grid grid-cols-2 gap-2" aria-label={m.tabOverview}>
        <MetricTile label={t.mobile.activeAgents} value={s.counts.running} tone={s.counts.running > 0 ? "accent" : "neutral"} onClick={() => onOpen("runs")} />
        <MetricTile
          label={t.mobile.pendingApprovals}
          value={approvals.data?.length ?? "—"}
          tone={(approvals.data?.length ?? 0) > 0 ? "warning" : "neutral"}
          onClick={() => onOpen("approvals")}
        />
        <MetricTile label={m.queued} value={s.counts.queued} onClick={() => onOpen("tasks")} />
        <MetricTile label={m.needsReview} value={s.counts.human_review} onClick={() => onOpen("tasks")} />
        <MetricTile label={m.retrying} value={s.counts.retrying} tone={s.counts.retrying > 0 ? "warning" : "neutral"} />
        <MetricTile label={t.status.failed} value={s.counts.failed} tone={s.counts.failed > 0 ? "danger" : "neutral"} />
        <MetricTile label={m.tokensUsed} value={formatTokens(s.codex_totals.total_tokens, locale)} />
        <MetricTile label={m.estimatedCost} value={formatCurrency(s.codex_totals.estimated_cost_usd, locale)} />
      </section>
      <section className="rounded-md border border-border bg-surface p-3 text-sm" aria-label={m.host}>
        <h2 className="mb-1 text-xs font-medium uppercase text-muted-foreground">{m.host}</h2>
        <p className="text-foreground">{hw.cpu_brand}</p>
        <p className="text-muted-foreground">
          {formatMessage(m.cpuUsage, { percent: Math.round(hw.cpu_usage_percent) })} ·{" "}
          {formatMessage(m.memoryUsage, { percent: Math.round(hw.memory_usage_percent) })} · {power}
          {hw.thermal_throttling ? ` · ${m.thermalThrottling}` : ""}
        </p>
        <p className="text-muted-foreground">{formatMessage(m.concurrency, { count: hw.effective_max_concurrency })}</p>
      </section>
    </div>
  );
};
