import React, { useState } from "react";
import {
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  Spinner,
  Textarea
} from "@fluentui/react-components";
import { formatMessage, type Task } from "@symphony/shared-types";
import { useMessages } from "../../lib/i18n.js";
import { useContinueTask } from "../../lib/queries.js";
import { executionMessages } from "./messages.js";

/** Same bound as the runtime (`MAX_FOLLOW_UP_CHARS`); the runtime re-validates. */
const MAX_INSTRUCTIONS = 20_000;

/**
 * Follow-up instructions for a task whose run ended: the runtime resumes the agent's thread in the
 * same worktree with them as the next prompt.
 */
export const ContinueDialog: React.FC<{ task: Task; open: boolean; onOpenChange: (open: boolean) => void }> = ({
  task,
  open,
  onOpenChange
}) => {
  const m = useMessages(executionMessages);
  const continueTask = useContinueTask();
  const [instructions, setInstructions] = useState("");
  const trimmed = instructions.trim();
  // Counted in code points, as the runtime counts (`chars()`), not UTF-16 units: an emoji is one.
  const tooLong = [...instructions].length > MAX_INSTRUCTIONS;
  const close = () => {
    if (!continueTask.isPending) onOpenChange(false);
  };
  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (!trimmed || tooLong || continueTask.isPending) return;
    continueTask.mutate(
      { id: task.id, instructions: trimmed },
      {
        onSuccess: () => {
          setInstructions("");
          onOpenChange(false);
        }
      }
    );
  };
  return (
    <Dialog open={open} onOpenChange={(_, data) => !data.open && close()}>
      <DialogSurface className="!max-w-[600px]">
        <form onSubmit={submit}>
          <DialogBody>
            <DialogTitle>{formatMessage(m.continueDialog.title, { identifier: task.issue.identifier })}</DialogTitle>
            <DialogContent className="space-y-3">
              <p className="text-sm text-muted-foreground">{m.continueDialog.description}</p>
              <Field
                label={m.continueDialog.label}
                required
                validationState={tooLong ? "error" : "none"}
                validationMessage={tooLong ? formatMessage(m.continueDialog.tooLong, { max: MAX_INSTRUCTIONS }) : undefined}
              >
                <Textarea
                  autoFocus
                  resize="vertical"
                  rows={6}
                  value={instructions}
                  placeholder={m.continueDialog.placeholder}
                  onChange={(_, data) => setInstructions(data.value)}
                />
              </Field>
            </DialogContent>
            <DialogActions>
              <Button appearance="secondary" disabled={continueTask.isPending} onClick={close}>
                {m.continueDialog.cancel}
              </Button>
              <Button
                appearance="primary"
                type="submit"
                disabled={!trimmed || tooLong || continueTask.isPending}
                icon={continueTask.isPending ? <Spinner size="tiny" /> : undefined}
              >
                {m.continueDialog.confirm}
              </Button>
            </DialogActions>
          </DialogBody>
        </form>
      </DialogSurface>
    </Dialog>
  );
};
