import React, { useCallback, useEffect, useRef } from "react";
import { toast } from "sonner";
import { getTranslations } from "@symphony/shared-types";
import { Skeleton } from "../primitives/skeleton.js";
import { Button } from "../primitives/button.js";
import { Spinner } from "../primitives/spinner.js";
import { Alert } from "../primitives/alert.js";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "../primitives/dialog.js";
import { useDesign } from "../providers/SymphonyDesignProvider.js";
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

/** Confirmation dialog used for every destructive or irreversible action. */
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
    <Dialog open={open} onOpenChange={(next) => !pending && onOpenChange(next)}>
      <DialogContent role="alertdialog" showClose={false} className="max-w-md" onInteractOutside={(e) => pending && e.preventDefault()}>
        <DialogHeader className="pr-5">
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription asChild>
            <div>{description}</div>
          </DialogDescription>
        </DialogHeader>
        {children ? <DialogBody className="space-y-3 text-sm">{children}</DialogBody> : null}
        <DialogFooter>
          <Button disabled={pending} onClick={() => onOpenChange(false)}>
            {cancelLabel ?? t.common.cancel}
          </Button>
          <Button variant={destructive ? "destructive" : "default"} disabled={pending} onClick={confirm}>
            {pending ? <Spinner className="text-current" /> : null}
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
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

/** Inline error/warning surface with optional retry. */
export const ErrorState: React.FC<ErrorStateProps> = ({ title, message, onRetry, retryLabel, intent = "error", className }) => {
  const { locale } = useDesign();
  const t = getTranslations(locale);
  return (
    <Alert intent={intent} title={title} className={className}>
      <div className="flex flex-wrap items-center gap-2">
        {message ? <span className="min-w-0 flex-1 break-words">{message}</span> : null}
        {onRetry ? (
          <Button size="sm" onClick={onRetry}>
            {retryLabel ?? t.common.retry}
          </Button>
        ) : null}
      </div>
    </Alert>
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

/** App-wide toast notifications through the single sonner toaster mounted by the design provider. */
export function useNotify() {
  return useCallback((intent: NotifyIntent, title: string, body?: string) => {
    const options = { description: body, duration: intent === "error" ? 8000 : 4000 };
    if (intent === "success") toast.success(title, options);
    else if (intent === "error") toast.error(title, options);
    else if (intent === "warning") toast.warning(title, options);
    else toast.info(title, options);
  }, []);
}

/** Visually hidden live region for screen-reader announcements. */
export const LiveAnnouncer: React.FC<{ message: string; politeness?: "polite" | "assertive" }> = ({ message, politeness = "polite" }) => (
  <div aria-live={politeness} role="status" className="sr-only">
    {message}
  </div>
);
