import React from "react";
import { cn } from "../lib/utils.js";

/** Bordered surface section with an optional header row (title + actions). */
export const Panel: React.FC<{
  title?: React.ReactNode;
  actions?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  bodyClassName?: string;
  as?: "section" | "div";
  "aria-label"?: string;
}> = ({ title, actions, children, className, bodyClassName, as = "section", ...rest }) => {
  const Comp = as;
  return (
    <Comp className={cn("flex min-h-0 flex-col rounded-md border border-border bg-surface", className)} aria-label={rest["aria-label"]}>
      {title || actions ? (
        <header className="flex h-9 shrink-0 items-center gap-2 border-b border-border px-3">
          {title ? <h2 className="min-w-0 flex-1 truncate text-sm font-semibold text-foreground">{title}</h2> : <span className="flex-1" />}
          {actions}
        </header>
      ) : null}
      <div className={cn("min-h-0 flex-1", bodyClassName)}>{children}</div>
    </Comp>
  );
};

/** Page title row used at the top of every workspace view. */
export const PageHeader: React.FC<{
  title: string;
  description?: React.ReactNode;
  actions?: React.ReactNode;
  meta?: React.ReactNode;
  className?: string;
}> = ({ title, description, actions, meta, className }) => (
  <header className={cn("flex shrink-0 flex-wrap items-center gap-3 border-b border-border bg-surface px-4 py-2.5", className)}>
    <div className="min-w-0 flex-1">
      <div className="flex items-center gap-2">
        <h1 className="truncate font-display text-base font-semibold text-foreground">{title}</h1>
        {meta}
      </div>
      {description ? <p className="truncate text-xs text-muted-foreground">{description}</p> : null}
    </div>
    {actions ? <div className="flex shrink-0 items-center gap-1.5">{actions}</div> : null}
  </header>
);

/** Small status dot with accessible label. */
export const StatusDot: React.FC<{ tone: "success" | "warning" | "danger" | "neutral" | "accent"; pulse?: boolean; label: string }> = ({
  tone,
  pulse = false,
  label
}) => (
  <span className="relative inline-flex size-2" role="img" aria-label={label}>
    {pulse ? (
      <span
        className={cn(
          "absolute inline-flex size-full animate-ping rounded-full opacity-60",
          tone === "success" && "bg-success",
          tone === "warning" && "bg-warning",
          tone === "danger" && "bg-destructive",
          tone === "accent" && "bg-accent",
          tone === "neutral" && "bg-muted-foreground"
        )}
      />
    ) : null}
    <span
      className={cn(
        "relative inline-flex size-2 rounded-full",
        tone === "success" && "bg-success",
        tone === "warning" && "bg-warning",
        tone === "danger" && "bg-destructive",
        tone === "accent" && "bg-accent",
        tone === "neutral" && "bg-muted-foreground"
      )}
    />
  </span>
);
