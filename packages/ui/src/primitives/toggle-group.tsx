import * as React from "react";
import { ToggleGroup as ToggleGroupPrimitive } from "radix-ui";
import { cn } from "../lib/utils";

/** Segmented control: a single-selection toggle group with a fixed, wrapping-safe layout. */
export function ToggleGroup({ className, ...props }: React.ComponentProps<typeof ToggleGroupPrimitive.Root>) {
  return (
    <ToggleGroupPrimitive.Root
      data-slot="toggle-group"
      className={cn("inline-flex shrink-0 items-center gap-0.5 rounded-md border border-border bg-muted p-0.5", className)}
      {...props}
    />
  );
}

export function ToggleGroupItem({ className, ...props }: React.ComponentProps<typeof ToggleGroupPrimitive.Item>) {
  return (
    <ToggleGroupPrimitive.Item
      data-slot="toggle-group-item"
      className={cn(
        "inline-flex h-6 items-center justify-center gap-1 whitespace-nowrap rounded-[5px] px-2.5 text-xs font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60 data-[state=on]:bg-surface data-[state=on]:text-foreground data-[state=on]:shadow-xs",
        className
      )}
      {...props}
    />
  );
}
