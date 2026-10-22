import React from "react";
import { Button, MessageBar, MessageBarBody, MessageBarTitle, ProgressBar, Spinner } from "@fluentui/react-components";
import { Download, ExternalLink, RotateCcw } from "lucide-react";
import { errorMessage, nativeHost, type UpdateCheck } from "@symphony/api-client";
import { formatMessage } from "@symphony/shared-types";
import { useNotify } from "@symphony/ui";
import { useMutation } from "@tanstack/react-query";
import { useMessages } from "../../lib/i18n.js";
import { useUiStore } from "../../store/uiStore.js";
import { downloadPercent, startInstall } from "./install.js";
import { updateMessages } from "./messages.js";

/**
 * What to do about an available update: install it in-app (with progress and errors), or, for a
 * release the app cannot install itself, open the releases page.
 */
export const UpdateActions: React.FC<{ update: UpdateCheck }> = ({ update }) => {
  const m = useMessages(updateMessages);
  const notify = useNotify();
  const install = useUiStore((s) => s.updateInstall);
  const openReleases = useMutation({
    mutationFn: () => nativeHost.openReleasesPage(),
    onError: (error) => notify("error", m.openReleasesFailed, errorMessage(error))
  });
  const busy = install.phase === "downloading" || install.phase === "installing";

  if (!update.installable) {
    return (
      <div className="space-y-1">
        <Button appearance="primary" icon={<ExternalLink className="size-4" />} disabled={openReleases.isPending} onClick={() => openReleases.mutate()}>
          {m.openReleases}
        </Button>
        <p className="text-xs text-muted-foreground">{m.manualOnly}</p>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {install.phase === "downloading" || install.phase === "installing" ? (
        <div className="space-y-1" aria-live="polite">
          {install.phase === "downloading" ? (
            <DownloadProgress downloaded={install.downloaded} total={install.total} />
          ) : (
            <p className="flex items-center gap-2 text-sm">
              <Spinner size="tiny" />
              {m.installing}
            </p>
          )}
        </div>
      ) : null}
      {install.phase === "error" ? (
        <MessageBar intent="error" layout="multiline">
          <MessageBarBody>
            <MessageBarTitle>{m.installFailed}</MessageBarTitle>
            {install.message}
          </MessageBarBody>
        </MessageBar>
      ) : null}
      {!busy ? (
        <Button
          appearance="primary"
          icon={install.phase === "error" ? <RotateCcw className="size-4" /> : <Download className="size-4" />}
          onClick={() => void startInstall(false)}
        >
          {install.phase === "error" ? m.retry : m.install}
        </Button>
      ) : null}
    </div>
  );
};

const DownloadProgress: React.FC<{ downloaded: number; total: number | null }> = ({ downloaded, total }) => {
  const m = useMessages(updateMessages);
  const percent = downloadPercent(downloaded, total);
  return (
    <>
      <p className="text-sm">{percent === null ? m.downloadingUnknown : formatMessage(m.downloading, { percent })}</p>
      <ProgressBar aria-label={m.progressLabel} value={percent === null ? undefined : percent / 100} thickness="large" />
    </>
  );
};
