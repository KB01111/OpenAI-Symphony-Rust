import React, { useState } from "react";
import { ArrowUp } from "lucide-react";
import { formatMessage, type Task } from "@symphony/shared-types";
import { Button, Hint, Spinner, Textarea, cn, useNotify } from "@symphony/ui";
import { errorMessage } from "@symphony/api-client";
import { useMessages } from "../../lib/i18n.js";
import { useContinueTask } from "../../lib/queries.js";
import { workbenchMessages } from "../../layouts/messages.js";
import { executionMessages } from "./messages.js";

/** Same bound as the runtime (`MAX_FOLLOW_UP_CHARS`); the runtime re-validates. */
const MAX_INSTRUCTIONS = 20_000;

/** A run that ended can be continued with instructions (its thread and worktree are kept). */
export function isContinuable(task: Pick<Task, "status">): boolean {
  return task.status === "human_review" || task.status === "failed" || task.status === "completed";
}

/**
 * Follow-up box under a thread. The runtime resumes the agent's thread in the same worktree with the
 * text as the next prompt; while a turn is running it explains why sending is not possible yet.
 */
export const FollowUpComposer: React.FC<{ task: Task }> = ({ task }) => {
  const wb = useMessages(workbenchMessages);
  const w = wb.thread;
  const hint = wb.composer.shortcut;
  const m = useMessages(executionMessages);
  const notify = useNotify();
  const continueTask = useContinueTask();
  const [text, setText] = useState("");
  const trimmed = text.trim();
  // Counted in code points, as the runtime counts (`chars()`), not UTF-16 units: an emoji is one. The
  // runtime trims first, so surrounding whitespace does not count.
  const tooLong = [...trimmed].length > MAX_INSTRUCTIONS;
  const enabled = isContinuable(task);

  const reason =
    task.status === "running"
      ? w.followUpRunning
      : task.status === "queued" || task.status === "ready"
        ? w.followUpQueued
        : task.status === "cancelled"
          ? w.followUpCancelled
          : task.status === "backlog" || task.status === "blocked"
            ? w.followUpBacklog
            : null;

  const submit = () => {
    if (!enabled || !trimmed || tooLong || continueTask.isPending) return;
    continueTask.mutate(
      { id: task.id, instructions: trimmed },
      {
        onSuccess: () => {
          setText("");
          notify("success", w.followUpSent);
        }
      }
    );
  };

  return (
    <form
      aria-label={m.actions.continue}
      className="shrink-0 border-t border-border bg-surface px-4 py-3"
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <div className={cn("mx-auto max-w-3xl", !enabled && "opacity-80")}>
        <div className="rounded-2xl border border-border-strong bg-surface focus-within:border-ring focus-within:ring-2 focus-within:ring-ring/30">
          <Textarea
            value={text}
            disabled={!enabled}
            aria-label={m.continueDialog.label}
            placeholder={w.followUpPlaceholder}
            rows={2}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
                e.preventDefault();
                submit();
              }
            }}
            className="max-h-48 min-h-14 resize-none rounded-2xl border-0 bg-transparent px-4 pb-1 pt-3 shadow-none ring-0 [field-sizing:content] focus-visible:ring-0"
          />
          <div className="flex items-center gap-2 px-3 pb-2.5">
            <p className="min-w-0 flex-1 truncate text-xs text-muted-foreground" role={tooLong || continueTask.isError ? "alert" : undefined}>
              {tooLong
                ? formatMessage(m.continueDialog.tooLong, { max: MAX_INSTRUCTIONS })
                : continueTask.isError
                  ? `${w.followUpError}: ${errorMessage(continueTask.error)}`
                  : enabled
                    ? hint
                    : reason}
            </p>
            <Hint label={w.followUpSend}>
              <Button
                type="submit"
                variant="default"
                size="icon-sm"
                className="rounded-full"
                disabled={!enabled || !trimmed || tooLong || continueTask.isPending}
                aria-label={continueTask.isPending ? w.followUpSending : w.followUpSend}
              >
                {continueTask.isPending ? <Spinner className="text-current" /> : <ArrowUp />}
              </Button>
            </Hint>
          </div>
        </div>
      </div>
    </form>
  );
};
