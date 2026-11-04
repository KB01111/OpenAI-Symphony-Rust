import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@symphony/ui";
import { useMessages } from "../../lib/i18n.js";
import { useUiStore } from "../../store/uiStore.js";
import { workbenchMessages } from "../../layouts/messages.js";
import { TaskComposer } from "./TaskComposer.js";

/** The composer in a modal, for "New task" entry points outside the home view. Mounted by the shell. */
export default function NewTaskDialog() {
  const m = useMessages(workbenchMessages);
  const setNewTaskOpen = useUiStore((s) => s.setNewTaskOpen);
  return (
    <Dialog open onOpenChange={(open) => !open && setNewTaskOpen(false)}>
      <DialogContent className="max-w-2xl" closeLabel={m.composer.close}>
        <DialogHeader>
          <DialogTitle>{m.composer.dialogTitle}</DialogTitle>
          <DialogDescription className="sr-only">{m.home.subtitle}</DialogDescription>
        </DialogHeader>
        <div className="px-5 pb-5 pt-2">
          <TaskComposer variant="dialog" onCreated={() => setNewTaskOpen(false)} />
        </div>
      </DialogContent>
    </Dialog>
  );
}
