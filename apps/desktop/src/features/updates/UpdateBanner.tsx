import React from "react";
import { Button, MessageBar, MessageBarActions, MessageBarBody } from "@fluentui/react-components";
import { X } from "lucide-react";
import { formatMessage } from "@symphony/shared-types";
import { useMessages } from "../../lib/i18n.js";
import { useUiStore } from "../../store/uiStore.js";
import { UpdateActions } from "./UpdateActions.js";
import { UpdateConfirmDialog } from "./UpdateConfirmDialog.js";
import { updateMessages } from "./messages.js";
import { useAppUpdate } from "./useAppUpdate.js";

/**
 * Shell-level notice for a newer release. Runs the automatic check (when enabled), offers the
 * in-app install and can be dismissed per version. Stays visible while an install is running.
 */
export const UpdateBanner: React.FC = () => {
  const m = useMessages(updateMessages);
  const update = useAppUpdate({ auto: true }).data;
  const dismissed = useUiStore((s) => s.dismissedUpdate);
  const dismiss = useUiStore((s) => s.dismissUpdate);
  const phase = useUiStore((s) => s.updateInstall.phase);
  const setUpdateInstall = useUiStore((s) => s.setUpdateInstall);

  const available = update?.update_available && update.latest !== null ? update : null;
  const show = available !== null && (dismissed !== available.latest || phase !== "idle");
  return (
    <>
      <UpdateConfirmDialog />
      {show ? (
        <section aria-label={m.bannerLabel}>
          <MessageBar intent="info" layout="multiline" shape="square">
            <MessageBarBody>
              <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
                <span>
                  {formatMessage(available.prerelease ? m.availablePre : m.available, {
                    version: available.latest ?? "",
                    current: available.current
                  })}
                </span>
                <UpdateActions update={available} />
              </div>
            </MessageBarBody>
            {phase === "idle" || phase === "error" ? (
              <MessageBarActions
                containerAction={
                  <Button appearance="transparent" size="small" icon={<X className="size-4" />} aria-label={m.later} title={m.later} onClick={() => {
                      dismiss(available.latest);
                      setUpdateInstall({ phase: "idle" });
                    }}
                  />
                }
              />
            ) : null}
          </MessageBar>
        </section>
      ) : null}
    </>
  );
};
