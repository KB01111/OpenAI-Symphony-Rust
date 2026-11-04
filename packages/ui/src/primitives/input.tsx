import * as React from "react";
import { cn } from "../lib/utils";

export const controlClasses =
  "w-full min-w-0 rounded-md border border-border-strong bg-surface px-2.5 text-sm text-foreground shadow-xs outline-none transition-colors placeholder:text-subtle-foreground focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/40 disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-destructive/30";

/** Text input; `icon` renders a leading decoration inside the field (the `input` keeps the id/aria props). */
export function Input({ className, type = "text", icon, ...props }: React.ComponentProps<"input"> & { icon?: React.ReactNode }) {
  if (!icon) return <input type={type} data-slot="input" className={cn(controlClasses, "h-8", className)} {...props} />;
  return (
    <div className="relative min-w-0">
      <span
        aria-hidden
        className="pointer-events-none absolute inset-y-0 start-2.5 flex items-center text-subtle-foreground [&_svg]:size-3.5"
      >
        {icon}
      </span>
      <input type={type} data-slot="input" className={cn(controlClasses, "h-8 ps-8", className)} {...props} />
    </div>
  );
}

export function Textarea({ className, ...props }: React.ComponentProps<"textarea">) {
  return <textarea data-slot="textarea" className={cn(controlClasses, "min-h-16 resize-y py-1.5 leading-relaxed", className)} {...props} />;
}
