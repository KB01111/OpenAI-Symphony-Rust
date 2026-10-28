import * as React from "react";
import { cn } from "../lib/utils";
import { Label } from "./label";

export interface FieldProps {
  label?: React.ReactNode;
  hint?: React.ReactNode;
  error?: React.ReactNode;
  required?: boolean;
  /** Explicit control id; generated when omitted. */
  htmlFor?: string;
  /** The control is a composite (e.g. a radio group): the label names the group instead of an input. */
  group?: boolean;
  className?: string;
  /** The control. When it is a single element, it receives id/aria wiring automatically. */
  children: React.ReactElement<Record<string, unknown>>;
}

/**
 * Label + control + hint/error. The control gets `id`, `aria-describedby` and `aria-invalid`.
 * The field is a plain block that shrinks with its container (`min-w-0`): it never forces a width.
 */
export function Field({ label, hint, error, required, htmlFor, group, className, children }: FieldProps) {
  const generated = React.useId();
  const id = htmlFor ?? (children.props.id as string | undefined) ?? generated;
  const describedBy = error ? `${id}-error` : hint ? `${id}-hint` : undefined;
  const control = React.cloneElement(children, {
    id,
    "aria-describedby": describedBy,
    "aria-invalid": error ? true : undefined,
    ...(required ? { "aria-required": true } : {}),
    ...(group ? { role: "group", "aria-labelledby": `${id}-label` } : {})
  });
  return (
    <div data-slot="field" className={cn("flex min-w-0 flex-col gap-1.5", className)}>
      {label ? (
        group ? (
          <span id={`${id}-label`} className="text-xs font-medium text-foreground">
            {label}
          </span>
        ) : (
          <Label htmlFor={id}>
            {label}
            {required ? <span aria-hidden className="text-destructive">*</span> : null}
          </Label>
        )
      ) : null}
      {control}
      {error ? (
        <p id={`${id}-error`} role="alert" className="text-xs text-destructive">
          {error}
        </p>
      ) : hint ? (
        <p id={`${id}-hint`} className="text-xs text-muted-foreground">
          {hint}
        </p>
      ) : null}
    </div>
  );
}

/** A titled group of fields. Container-query aware: two columns only when the *container* is wide. */
export function FormSection({ title, children, className }: { title?: React.ReactNode; children: React.ReactNode; className?: string }) {
  return (
    <section className={cn("flex min-w-0 flex-col gap-3 border-t border-border pt-4 first:border-t-0 first:pt-0", className)}>
      {title ? <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">{title}</h3> : null}
      {children}
    </section>
  );
}

/** Responsive field grid driven by the width of its container (never the viewport). */
export function FieldGrid({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <div className="@container min-w-0">
      <div className={cn("grid grid-cols-1 gap-3 @md:grid-cols-2", className)}>{children}</div>
    </div>
  );
}
