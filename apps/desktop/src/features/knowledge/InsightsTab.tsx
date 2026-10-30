import React, { useEffect, useRef, useState } from "react";
import { MessageSquarePlus, SendHorizontal, Sparkles } from "lucide-react";
import { formatMessage } from "@symphony/shared-types";
import type { ChatMessage } from "@symphony/shared-types";
import { AiEmptyView, Alert, Button, ErrorState, LoadingRows, Markdown, Panel, Spinner, Textarea } from "@symphony/ui";
import { useMessages } from "../../lib/i18n.js";
import { useAskKnowledge, useClearKnowledgeChat, useKnowledgeChat, useKnowledgeOverview } from "../../lib/queries.js";
import { AnalysisView } from "./AnalysisView.js";
import { EvidenceChip, ShaChip } from "./parts.js";
import { knowledgeMessages } from "./messages.js";
import { useKnowledgeSelection } from "./selection.js";

const MAX_QUESTION = 2000;

const Message: React.FC<{ message: ChatMessage }> = ({ message }) => {
  const m = useMessages(knowledgeMessages);
  const selection = useKnowledgeSelection();
  const mine = message.role === "user";
  return (
    <li className="flex flex-col gap-1.5">
      <span className="text-xs font-medium text-muted-foreground">{mine ? m.insights.you : m.insights.assistant}</span>
      {mine ? (
        <p className="whitespace-pre-wrap rounded-md bg-muted px-3 py-2 text-sm text-foreground">{message.content}</p>
      ) : message.status === "running" ? (
        <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
          <Spinner />
          {m.insights.thinking}
        </p>
      ) : message.status === "failed" ? (
        <ErrorState title={m.insights.failed} message={message.error ?? undefined} />
      ) : (
        <div className="flex flex-col gap-2 rounded-md border border-border bg-surface px-3 py-2">
          <Markdown className="text-sm text-foreground">{message.content}</Markdown>
          {message.evidence.length > 0 ? (
            <div className="flex flex-col gap-1">
              <span className="text-xs text-muted-foreground">{m.insights.evidence}</span>
              <div className="flex flex-wrap gap-1">
                {message.evidence.map((evidence) => (
                  <span key={evidence.id} className="inline-flex items-center gap-1">
                    <span className="font-mono text-xs text-subtle-foreground">[{evidence.id}]</span>
                    <EvidenceChip
                      evidence={evidence}
                      onOpenNode={(nodeId) => selection.focusNode(nodeId)}
                      onOpenCommit={(sha) => selection.selectCommit(sha, "commits")}
                    />
                  </span>
                ))}
              </div>
            </div>
          ) : (
            <p className="text-xs text-warning">{m.insights.insufficient}</p>
          )}
        </div>
      )}
    </li>
  );
};

/** Ask questions about the repository; answers cite the code and commits they rest on. */
export const InsightsTab: React.FC = () => {
  const m = useMessages(knowledgeMessages);
  const selection = useKnowledgeSelection();
  const { projectId, sha } = selection;
  const chat = useKnowledgeChat(projectId);
  const overview = useKnowledgeOverview(projectId);
  const ask = useAskKnowledge(projectId);
  const clear = useClearKnowledgeChat(projectId);
  const [text, setText] = useState("");
  const endRef = useRef<HTMLDivElement>(null);
  const messages = chat.data ?? [];
  const busy = ask.isPending || messages.some((message) => message.status === "running");
  const question = text.trim();
  const canSend = question.length > 0 && question.length <= MAX_QUESTION && !busy;

  useEffect(() => {
    endRef.current?.scrollIntoView?.({ block: "end" });
  }, [messages.length, messages[messages.length - 1]?.status]);

  const send = (value: string) => {
    if (!value || busy) return;
    ask.mutate(value, { onSuccess: () => setText("") });
  };

  return (
    <div className="@container flex h-full min-h-0 flex-col gap-2 p-2 @4xl:flex-row">
      <Panel title={m.insights.title} className="min-h-80 min-w-0 flex-[3]" bodyClassName="flex min-h-0 flex-col">
        <div className="min-h-0 flex-1 overflow-auto p-3">
          {overview.data && overview.data.index === null ? (
            <Alert intent="info" title={m.insights.noIndex} className="mb-3" />
          ) : null}
          {chat.isPending ? (
            <LoadingRows rows={3} />
          ) : chat.isError ? (
            <ErrorState title={m.unavailable.title} message={chat.error.message} onRetry={() => void chat.refetch()} retryLabel={m.unavailable.retry} />
          ) : messages.length === 0 ? (
            <AiEmptyView
              icon={<Sparkles className="size-5 text-accent" />}
              title={m.insights.empty}
              description={m.insights.description}
              action={m.insights.suggestions.map((suggestion) => (
                <Button key={suggestion} size="sm" variant="secondary" disabled={busy} onClick={() => send(suggestion)}>
                  {suggestion}
                </Button>
              ))}
            />
          ) : (
            <ul role="list" aria-live="polite" className="flex flex-col gap-4">
              {messages.map((message) => (
                <Message key={message.id} message={message} />
              ))}
            </ul>
          )}
          <div ref={endRef} />
        </div>
        <form
          className="flex shrink-0 flex-col gap-1.5 border-t border-border p-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (canSend) send(question);
          }}
        >
          <Textarea
            value={text}
            onChange={(event) => setText(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                event.preventDefault();
                if (canSend) send(question);
              }
            }}
            placeholder={m.insights.placeholder}
            aria-label={m.insights.title}
            maxLength={MAX_QUESTION + 200}
            rows={2}
          />
          <div className="flex items-center gap-2">
            <span className="text-xs tabular-nums text-subtle-foreground">{formatMessage(m.insights.length, { count: question.length, max: MAX_QUESTION })}</span>
            <span className="flex-1" />
            <Button type="button" size="sm" variant="subtle" disabled={clear.isPending || messages.length === 0 || busy} onClick={() => clear.mutate()}>
              <MessageSquarePlus className="size-4" />
              {m.insights.clear}
            </Button>
            <Button type="submit" size="sm" disabled={!canSend}>
              {ask.isPending ? <Spinner /> : <SendHorizontal className="size-4" />}
              {m.insights.ask}
            </Button>
          </div>
        </form>
      </Panel>
      {sha ? (
        <Panel
          title={
            <span className="flex items-center gap-2">
              {m.commits.panelTabs.review} <ShaChip sha={sha} />
            </span>
          }
          className="min-h-72 min-w-0 flex-[2]"
          bodyClassName="overflow-auto p-3"
        >
          <AnalysisView sha={sha} />
        </Panel>
      ) : null}
    </div>
  );
};
