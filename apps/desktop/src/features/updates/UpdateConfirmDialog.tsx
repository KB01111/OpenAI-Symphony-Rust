import React from "react";
import { formatMessage } from "@symphony/shared-types";
import { ConfirmDialog } from "@symphony/ui";
import { useMessages } from "../../lib/i18n.js";
import { useUiStore } from "../../store/uiStore.js";
import { startInstall } from "./install.js";
import { updateMessages } from "./messages.js";

/** Asks before installing while agents are running, since the installer ends their processes. */
export const UpdateConfirmDialog: React.FC = () => {
  const m = useMessages(updateMessages);
  const install = useUiStore((s) => s.updateInstall);
  const setUpdateInstall = useUiStore((s) => s.setUpdateInstall);
  return (
    <ConfirmDialog
      open={install.phase === "confirm"}
      onOpenChange={(open) => {
        if (!open) setUpdateInstall({ phase: "idle" });
      }}
      title={m.confirmTitle}
      description={formatMessage(m.confirmDescription, { count: install.phase === "confirm" ? install.running : 0 })}
      confirmLabel={m.confirmInstall}
      destructive
      onConfirm={() => void startInstall(true)}
    />
  );
};
