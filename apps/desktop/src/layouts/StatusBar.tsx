import React from "react";
import { formatMessage } from "@symphony/shared-types";
import { HardwareTelemetryStrip, StatusDot, formatCurrency, formatTokens } from "@symphony/ui";
import { shellMessages } from "../app/messages.js";
import { useLocale, useMessages } from "../lib/i18n.js";
import { useRuntimeState } from "../lib/queries.js";
import { useRuntime } from "../runtime/RuntimeProvider.js";
import { useConnectionSummary } from "./useConnectionSummary.js";

/** Compact footer: connection, scheduler, run counts, token totals and host telemetry. */
export const StatusBar: React.FC = () => {
  const m = useMessages(shellMessages);
  const locale = useLocale();
  const state = useRuntimeState();
  const { baseUrl } = useRuntime();
  const connection = useConnectionSummary();
  const s = state.data;
  return (
    <footer className="flex h-6 shrink-0 items-center gap-3 border-t border-border bg-surface px-2 text-xs text-muted-foreground" role="contentinfo">
      <span className="inline-flex items-center gap-1.5" title={baseUrl ?? undefined}>
        <StatusDot tone={connection.tone} label={connection.label} />
        {m.status.runtime}: {connection.short}
      </span>
      {s ? (
        <>
          <span className={s.scheduler_paused ? "text-warning" : undefined}>{s.scheduler_paused ? m.status.paused : m.status.active}</span>
          <span>{formatMessage(m.status.running, { count: s.counts.running })}</span>
          <span>{formatMessage(m.status.queued, { count: s.counts.queued })}</span>
          <span className="tabular-nums">
            {formatTokens(s.codex_totals.total_tokens, locale)} tok · {formatCurrency(s.codex_totals.estimated_cost_usd, locale)}
          </span>
          <HardwareTelemetryStrip hardware={s.hardware} className="ms-auto" />
        </>
      ) : null}
    </footer>
  );
};
