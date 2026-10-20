import React, { useCallback, useEffect, useRef } from "react";
import {
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  MessageBar,
  MessageBarActions,
  MessageBarBody,
  MessageBarTitle,
  Spinner,
  Toast,
  ToastBody,
  ToastTitle,
  useToastController
} from "@fluentui/react-components";
import { getTranslations } from "@symphony/shared-types";
import { Skeleton } from "../primitives/skeleton.js";
import { SYMPHONY_TOASTER_ID, useDesign } from "../providers/SymphonyDesignProvider.js";
import { cn } from "../lib/utils.js";

export interface ConfirmDialogProps {
  open: boolean;
  title: string;
  description: React.ReactNode;
  confirmLabel: string;
  cancelLabel?: string;
  destructive?: boolean;
  pending?: boolean;
  onConfirm: () => void;
  onOpenChange: (open: boolean) => void;
  children?: React.ReactNode;
}

/** Fluent confirmation dialog used for every destructive or irreversible action. */
export const ConfirmDialog: React.FC<ConfirmDialogProps> = ({
  open,
  title,
  description,
  confirmLabel,
  cancelLabel,
  destructive = false,
  pending = false,
  onConfirm,
  onOpenChange,
  children
}) => {
  const { locale } = useDesign();
  const t = getTranslations(locale);
  // A confirmation fires once: a second click in the same frame (before `pending` re-renders)
  // is ignored. The lock resets when the dialog closes or a pending action settles (retry).
  const fired = useRef(false);
  const wasPending = useRef(pending);
  useEffect(() => {
    if (!open) fired.current = false;
  }, [open]);
  useEffect(() => {
    if (wasPending.current && !pending) fired.current = false;
    wasPending.current = pending;
  }, [pending]);
  const confirm = () => {
    if (fired.current || pending) return;
    fired.current = true;
    onConfirm();
  };
  return (
    <Dialog open={open} onOpenChange={(_, data) => !pending && onOpenChange(data.open)} modalType="alert">
      <DialogSurface aria-describedby={undefined}>
        <DialogBody>
          <DialogTitle>{title}</DialogTitle>
          <DialogContent>
            <div className="space-y-3 text-sm text-foreground">
              <div>{description}</div>
              {children}
            </div>
          </DialogContent>
          <DialogActions>
            <Button appearance="secondary" disabled={pending} onClick={() => onOpenChange(false)}>
              {cancelLabel ?? t.common.cancel}
            </Button>
            <Button
              appearance="primary"
              disabled={pending}
              icon={pending ? <Spinner size="tiny" /> : undefined}
              onClick={confirm}
              style={destructive ? { background: "var(--destructive)", color: "var(--accent-foreground)" } : undefined}
            >
              {confirmLabel}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
};

export interface ErrorStateProps {
  title: string;
  message?: string;
  onRetry?: () => void;
  retryLabel?: string;
  intent?: "error" | "warning" | "info";
  className?: string;
}

/** Inline error/warning surface with optional retry (Fluent MessageBar). */
export const ErrorState: React.FC<ErrorStateProps> = ({ title, message, onRetry, retryLabel, intent = "error", className }) => {
  const { locale } = useDesign();
  const t = getTranslations(locale);
  return (
    <MessageBar intent={intent} className={className} layout="multiline">
      <MessageBarBody>
        <MessageBarTitle>{title}</MessageBarTitle>
        {message}
      </MessageBarBody>
      {onRetry ? (
        <MessageBarActions>
          <Button size="small" onClick={onRetry}>
            {retryLabel ?? t.common.retry}
          </Button>
        </MessageBarActions>
      ) : null}
    </MessageBar>
  );
};

/** Skeleton rows for list/table loading states. */
export const LoadingRows: React.FC<{ rows?: number; className?: string; label?: string }> = ({ rows = 6, className, label }) => (
  <div role="status" aria-busy="true" aria-label={label} className={cn("space-y-2", className)}>
    {Array.from({ length: rows }, (_, i) => (
      <Skeleton key={i} className="h-row w-full" style={{ opacity: 1 - i * 0.1 }} />
    ))}
  </div>
);

export type NotifyIntent = "success" | "error" | "warning" | "info";

/** App-wide toast notifications through the single Fluent toaster. */
export function useNotify() {
  const { dispatchToast } = useToastController(SYMPHONY_TOASTER_ID);
  return useCallback(
    (intent: NotifyIntent, title: string, body?: string) => {
      dispatchToast(
        <Toast>
          <ToastTitle>{title}</ToastTitle>
          {body ? <ToastBody>{body}</ToastBody> : null}
        </Toast>,
        { intent, timeout: intent === "error" ? 8000 : 4000, politeness: intent === "error" ? "assertive" : "polite" }
      );
    },
    [dispatchToast]
  );
}

/** Visually hidden live region for screen-reader announcements. */
export const LiveAnnouncer: React.FC<{ message: string; politeness?: "polite" | "assertive" }> = ({ message, politeness = "polite" }) => (
  <div aria-live={politeness} role="status" className="sr-only">
    {message}
  </div>
);
