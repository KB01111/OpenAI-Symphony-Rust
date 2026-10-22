import React from "react";
import { Tooltip } from "@fluentui/react-components";
import { formatMessage, type Task } from "@symphony/shared-types";
import { PhaseBadge, ProviderBadge, TimeAgo, cn, formatTokens } from "@symphony/ui";
import { Coins, RotateCcw } from "lucide-react";
import { useLocale, useMessages } from "../../lib/i18n.js";
import { taskMessages } from "./messages.js";
import { normalizePriority } from "./taskLogic.js";

const PRIORITY_TONE: Record<number, string> = {
  1: "bg-destructive-subtle text-destructive",
  2: "bg-warning-subtle text-warning",
  3: "bg-info-subtle text-info",
  4: "bg-surface-sunken text-muted-foreground"
};

export const PriorityChip: React.FC<{ priority: number | null | undefined }> = ({ priority }) => {
  const m = useMessages(taskMessages);
  const p = normalizePriority(priority);
  if (p === null) return <span className="text-xs text-subtle-foreground">—</span>;
  const name = m.priority[`p${p}` as "p1" | "p2" | "p3" | "p4"];
  return (
    <Tooltip content={`${formatMessage(m.priority.full, { value: p })} · ${name}`} relationship="description">
      <span className={cn("inline-flex h-5 items-center rounded px-1.5 font-mono text-[11px] font-semibold", PRIORITY_TONE[p])}>
        {formatMessage(m.priority.label, { value: p })}
      </span>
    </Tooltip>
  );
};

/** Visual body of a board card (shared by the in-column card and the drag overlay). */
export const TaskCardBody = React.memo(function TaskCardBody({ task }: { task: Task }) {
  const m = useMessages(taskMessages);
  const locale = useLocale();
  const tokensLabel = formatMessage(task.usage_estimated ? m.card.tokensEstimated : m.card.tokens, {
    count: formatTokens(task.total_tokens, locale)
  });
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <div className="flex items-center gap-1.5 pe-7">
        <span className="truncate font-mono text-xs text-muted-foreground">{task.issue.identifier}</span>
        <PriorityChip priority={task.issue.priority} />
      </div>
      <p className="line-clamp-2 text-sm font-medium leading-snug text-foreground">{task.issue.title}</p>
      <div className="flex min-w-0 flex-wrap items-center gap-1">
        <ProviderBadge provider={task.provider} model={task.model} />
        {task.status === "running" && task.current_phase ? <PhaseBadge phase={task.current_phase} /> : null}
      </div>
      <div className="flex items-center gap-3 text-xs text-muted-foreground tabular-nums">
        <span className="inline-flex items-center gap-1" title={formatMessage(m.card.attempts, { count: task.attempt_count })}>
          <RotateCcw className="size-3" aria-hidden />
          <span className="sr-only">{formatMessage(m.card.attempts, { count: task.attempt_count })}</span>
          <span aria-hidden>{task.attempt_count}</span>
        </span>
        <span className="inline-flex items-center gap-1" title={tokensLabel}>
          <Coins className="size-3" aria-hidden />
          <span className="sr-only">{tokensLabel}</span>
          <span aria-hidden>
            {task.usage_estimated ? "≈ " : ""}
            {formatTokens(task.total_tokens, locale)}
          </span>
        </span>
        <TimeAgo iso={task.updated_at} className="ms-auto" />
      </div>
    </div>
  );
});
