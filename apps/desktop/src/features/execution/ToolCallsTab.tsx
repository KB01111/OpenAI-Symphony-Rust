import React, { useState } from "react";
import { formatMessage, getTranslations, type ToolCallKind, type ToolCallRecord, type ToolCallStatus } from "@symphony/shared-types";
import { AiEmptyView, Badge, Button, CodeBlock, ErrorState, LoadingRows, TimeAgo, formatDuration, type BadgeTone } from "@symphony/ui";
import { useLocale, useMessages, useT } from "../../lib/i18n.js";
import { useTaskToolCalls } from "../../lib/queries.js";
import { executionMessages } from "./messages.js";

const STATUS_COLOR: Record<ToolCallStatus, BadgeTone> = {
  started: "info",
  succeeded: "success",
  failed: "danger"
};

function argumentsText(value: unknown): string | null {
  if (value === null || value === undefined) return null;
  return typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

const ToolCallItem = React.memo(function ToolCallItem({ call }: { call: ToolCallRecord }) {
  const m = useMessages(executionMessages);
  const locale = useLocale();
  const [open, setOpen] = useState(call.status === "failed");
  const args = argumentsText(call.arguments);
  const kind: Record<ToolCallKind, string> = m.toolCalls.kinds;
  return (
    <li className="space-y-1.5 rounded-md border border-border p-2">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <Badge tone={STATUS_COLOR[call.status]}>{m.toolCalls.status[call.status]}</Badge>
        <code className="min-w-0 flex-1 truncate font-mono text-foreground" title={call.tool_name}>
          {call.tool_name || m.toolCalls.unnamed}
        </code>
        <span className="text-muted-foreground">{kind[call.kind]}</span>
      </div>
      <div className="flex flex-wrap items-center gap-x-3 text-xs tabular-nums text-muted-foreground">
        {call.duration_ms !== null ? <span>{formatDuration(call.duration_ms / 1000, getTranslations(locale))}</span> : null}
        <TimeAgo iso={call.started_at} />
        <Button className="ms-auto" size="sm" variant="subtle" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
          {open ? m.toolCalls.hideDetails : m.toolCalls.showDetails}
        </Button>
      </div>
      {open ? (
        <div className="space-y-2">
          <section className="space-y-1" aria-label={m.toolCalls.arguments}>
            <h4 className="text-xs font-semibold text-muted-foreground">{m.toolCalls.arguments}</h4>
            {args ? <CodeBlock language="json">{args}</CodeBlock> : <p className="text-xs text-muted-foreground">{m.toolCalls.none}</p>}
          </section>
          <section className="space-y-1" aria-label={m.toolCalls.output}>
            <h4 className="text-xs font-semibold text-muted-foreground">{m.toolCalls.output}</h4>
            {call.output ? (
              <CodeBlock language="text">{call.output}</CodeBlock>
            ) : (
              <p className="text-xs text-muted-foreground">{call.status === "started" ? m.toolCalls.running : m.toolCalls.none}</p>
            )}
            {call.output_truncated ? <p className="text-xs text-warning">{m.toolCalls.truncated}</p> : null}
          </section>
        </div>
      ) : null}
    </li>
  );
});

/** Every tool call of a task (commands, file changes, MCP and host tools), newest first. */
export const ToolCallsTab: React.FC<{ taskId: string }> = ({ taskId }) => {
  const m = useMessages(executionMessages);
  const t = useT();
  const calls = useTaskToolCalls(taskId);
  if (calls.isLoading) return <LoadingRows rows={3} label={t.common.loading} />;
  // Only a failed first page replaces the list; pages already loaded stay when a later one fails.
  if (calls.isError && !calls.data) {
    return <ErrorState title={m.toolCalls.loadFailed} message={calls.error.message} onRetry={() => void calls.refetch()} />;
  }
  const list = calls.data?.pages.flat() ?? [];
  // A failed refresh keeps what was loaded but says it may be out of date.
  const refreshFailed = calls.isRefetchError ? (
    <ErrorState title={m.toolCalls.refreshFailed} message={calls.error?.message ?? ""} onRetry={() => void calls.refetch()} />
  ) : null;
  if (list.length === 0) {
    return refreshFailed ?? <AiEmptyView title={m.toolCalls.empty} description={m.toolCalls.emptyDetail} />;
  }
  return (
    <div className="space-y-2">
      {refreshFailed}
      <p className="text-xs text-muted-foreground">{formatMessage(m.toolCalls.count, { count: list.length })}</p>
      <ul className="space-y-2" aria-label={m.toolCalls.listLabel}>
        {list.map((call) => (
          <ToolCallItem key={call.id} call={call} />
        ))}
      </ul>
      {calls.isFetchNextPageError ? (
        <ErrorState title={m.toolCalls.loadOlderFailed} message={calls.error?.message ?? ""} onRetry={() => void calls.fetchNextPage()} />
      ) : null}
      {calls.hasNextPage && !calls.isFetchNextPageError ? (
        <Button size="sm" variant="secondary" disabled={calls.isFetchingNextPage} onClick={() => void calls.fetchNextPage()}>
          {calls.isFetchingNextPage ? t.common.loading : m.toolCalls.loadOlder}
        </Button>
      ) : null}
    </div>
  );
};
