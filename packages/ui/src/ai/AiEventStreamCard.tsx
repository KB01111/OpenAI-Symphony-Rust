import React, { useMemo, useState } from "react";
import { formatMessage, type AgentEvent } from "@symphony/shared-types";
import {
  AlertTriangle,
  Bot,
  CheckCircle2,
  CircleDot,
  FileDiff,
  Lightbulb,
  ShieldQuestion,
  SquareTerminal,
  Wrench,
  XCircle
} from "lucide-react";
import { cn } from "../lib/utils.js";
import { useUiStrings } from "../i18n.js";
import { formatAgentEvent, runtimeEventTemplate, type AgentEventKind, type EventTone } from "./eventFormat.js";
import { PhaseBadge } from "./badges.js";
import { CodeBlock, CopyButton, Markdown } from "./codeBlocks.js";
import { Button } from "../primitives/button.js";

const KIND_ICONS: Record<AgentEventKind, React.ComponentType<{ className?: string }>> = {
  message: Bot,
  reasoning: Lightbulb,
  command: SquareTerminal,
  file_change: FileDiff,
  tool_call: Wrench,
  approval: ShieldQuestion,
  error: XCircle,
  session: CircleDot,
  lifecycle: CheckCircle2,
  other: AlertTriangle
};

const TONE_CLASSES: Record<EventTone, string> = {
  neutral: "text-muted-foreground",
  info: "text-info",
  success: "text-success",
  warning: "text-warning",
  danger: "text-destructive",
  accent: "text-accent"
};

export interface AiEventStreamCardProps {
  event: AgentEvent;
  /** Technical mode shows event types, ids and raw JSON-RPC payload toggles. */
  mode?: "concise" | "technical";
  /** Show the issue identifier (useful in cross-task feeds). */
  showIdentifier?: boolean;
  className?: string;
}

/**
 * One timeline entry. Memoized on event identity so streaming new events never re-renders (or
 * re-parses Markdown for) existing entries.
 */
export const AiEventStreamCard = React.memo(function AiEventStreamCard({
  event,
  mode = "concise",
  showIdentifier = false,
  className
}: AiEventStreamCardProps) {
  const { ui } = useUiStrings();
  const [showRaw, setShowRaw] = useState(false);
  const formatted = useMemo(() => {
    const base = formatAgentEvent(event);
    // Runtime lifecycle events with a known shape are rendered in the UI locale.
    const template = runtimeEventTemplate(event);
    return template ? { ...base, text: formatMessage(ui[template.key], template.values) } : base;
  }, [event, ui]);
  const Icon = KIND_ICONS[formatted.kind];
  const technical = mode === "technical";
  const time = useMemo(() => new Date(event.timestamp).toLocaleTimeString(), [event.timestamp]);

  // What the agent says reads as a chat message; everything it does is a quieter activity row.
  const spoken = formatted.kind === "message";

  return (
    <article
      className={cn(
        "group relative grid grid-cols-[28px_minmax(0,1fr)] gap-x-3 rounded-xl px-2 transition-colors",
        spoken ? "py-3" : "py-1.5 hover:bg-surface-hover",
        formatted.kind === "error" && "bg-destructive-subtle hover:bg-destructive-subtle",
        formatted.kind === "approval" && formatted.tone === "warning" && "bg-warning-subtle hover:bg-warning-subtle",
        className
      )}
      aria-label={`${event.event_type} ${time}`}
    >
      {spoken ? (
        <span aria-hidden className="grid size-7 place-items-center rounded-full bg-accent-subtle text-accent">
          <Icon className="size-4" />
        </span>
      ) : (
        <span aria-hidden className="grid h-5 w-7 place-items-center">
          <Icon className={cn("size-3.5 shrink-0", TONE_CLASSES[formatted.tone])} />
        </span>
      )}
      <div className={cn("min-w-0", spoken ? "space-y-1.5" : "space-y-1")}>
        <header className={cn("flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-muted-foreground", !spoken && !technical && !showIdentifier && "float-end ms-2 h-5")}>
          {showIdentifier && event.issue_identifier ? (
            <span className="font-mono font-medium text-foreground">{event.issue_identifier}</span>
          ) : null}
          {technical ? <span className="font-mono">{event.event_type}</span> : null}
          {technical && event.phase ? <PhaseBadge phase={event.phase} /> : null}
          <time dateTime={event.timestamp} className={cn("ms-auto tabular-nums", spoken ? "order-first ms-0" : "text-subtle-foreground")}>
            {time}
          </time>
          <span className="opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
            <CopyButton value={formatted.text} label={ui.copyEvent} />
          </span>
        </header>

        {formatted.kind === "message" ? (
          <div className="text-[14px] leading-relaxed text-foreground">
            <Markdown breakNewLines>
              {formatted.text}
            </Markdown>
          </div>
        ) : (
          <p className={cn("break-words text-sm leading-5", formatted.kind === "error" || formatted.kind === "approval" ? "text-foreground" : "text-muted-foreground")}>{formatted.text}</p>
        )}

        {formatted.command ? (
          <div className="space-y-1">
            <CodeBlock language="bash">{formatted.command}</CodeBlock>
            {formatted.exitCode !== null ? (
              <span className={cn("text-xs font-mono", formatted.exitCode === 0 ? "text-success" : "text-destructive")}>
                {formatMessage(ui.exitCode, { code: formatted.exitCode })}
              </span>
            ) : null}
            {formatted.output && technical ? <CodeBlock language="text">{formatted.output}</CodeBlock> : null}
          </div>
        ) : null}

        {formatted.files.length > 0 ? (
          <ul className="space-y-1">
            {formatted.files.map((file) => (
              <li key={file.path} className="space-y-1">
                <span className="font-mono text-xs text-foreground">
                  {file.kind} · {file.path}
                </span>
                {technical && file.diff ? <CodeBlock language="diff">{file.diff}</CodeBlock> : null}
              </li>
            ))}
          </ul>
        ) : null}

        {technical && event.payload != null ? (
          <div>
            <Button size="sm" variant="subtle" className="h-6 px-1.5" onClick={() => setShowRaw((v) => !v)} aria-expanded={showRaw}>
              {showRaw ? ui.hidePayload : ui.rawPayload}
            </Button>
            {showRaw ? <CodeBlock language="json">{JSON.stringify(event.payload, null, 2)}</CodeBlock> : null}
          </div>
        ) : null}
      </div>
    </article>
  );
});
