import * as React from "react";
import { Loader2Icon } from "lucide-react";
import { cn } from "../lib/utils";

/** Decorative by default (inside buttons); pass `label` for a standalone, announced spinner. */
export function Spinner({ className, label, ...props }: Omit<React.ComponentProps<"svg">, "ref"> & { label?: string }) {
  return (
    <Loader2Icon
      {...(label ? { role: "status", "aria-label": label } : { "aria-hidden": true })}
      className={cn("size-4 animate-spin text-muted-foreground", className)}
      {...props}
    />
  );
}
