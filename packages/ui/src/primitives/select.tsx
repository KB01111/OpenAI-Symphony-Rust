import * as React from "react";
import { Select as SelectPrimitive } from "radix-ui";
import { CheckIcon, ChevronDownIcon } from "lucide-react";
import { cn } from "../lib/utils";
import { controlClasses } from "./input";

interface TriggerA11y {
  id?: string;
  "aria-describedby"?: string;
  "aria-invalid"?: boolean | "true" | "false";
  "aria-required"?: boolean | "true" | "false";
}

const TriggerA11yContext = React.createContext<TriggerA11y>({});

/**
 * Radix `Select.Root` renders no element, so `id` and the `aria-*` props a `Field` injects are
 * forwarded to the trigger instead: the field's label then names the select.
 */
export function Select({
  id,
  "aria-describedby": describedBy,
  "aria-invalid": invalid,
  "aria-required": required,
  ...props
}: React.ComponentProps<typeof SelectPrimitive.Root> & TriggerA11y) {
  const value = React.useMemo(
    () => ({ id, "aria-describedby": describedBy, "aria-invalid": invalid, "aria-required": required }),
    [id, describedBy, invalid, required]
  );
  return (
    <TriggerA11yContext.Provider value={value}>
      <SelectPrimitive.Root {...props} />
    </TriggerA11yContext.Provider>
  );
}

export const SelectGroup = SelectPrimitive.Group;
export const SelectValue = SelectPrimitive.Value;

/** The trigger never imposes a minimum width, so it always fits the grid cell it is placed in. */
export function SelectTrigger({ className, children, ...props }: React.ComponentProps<typeof SelectPrimitive.Trigger>) {
  const a11y = React.useContext(TriggerA11yContext);
  return (
    <SelectPrimitive.Trigger
      data-slot="select-trigger"
      className={cn(controlClasses, "flex h-8 items-center justify-between gap-2 text-left data-[placeholder]:text-subtle-foreground [&>span]:truncate", className)}
      {...a11y}
      {...props}
    >
      {children}
      <SelectPrimitive.Icon asChild>
        <ChevronDownIcon className="size-4 shrink-0 text-muted-foreground" />
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  );
}

export function SelectContent({ className, children, position = "popper", ...props }: React.ComponentProps<typeof SelectPrimitive.Content>) {
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Content
        data-slot="select-content"
        position={position}
        sideOffset={4}
        className={cn(
          "z-50 max-h-72 min-w-[var(--radix-select-trigger-width)] overflow-hidden rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-elevated data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0",
          className
        )}
        {...props}
      >
        <SelectPrimitive.Viewport>{children}</SelectPrimitive.Viewport>
      </SelectPrimitive.Content>
    </SelectPrimitive.Portal>
  );
}

export function SelectItem({ className, children, ...props }: React.ComponentProps<typeof SelectPrimitive.Item>) {
  return (
    <SelectPrimitive.Item
      data-slot="select-item"
      className={cn(
        "relative flex w-full cursor-default select-none items-center gap-2 rounded-md py-1.5 pl-2 pr-8 text-sm outline-none data-[disabled]:pointer-events-none data-[disabled]:opacity-50 data-[highlighted]:bg-surface-hover",
        className
      )}
      {...props}
    >
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
      <SelectPrimitive.ItemIndicator className="absolute right-2 inline-flex">
        <CheckIcon className="size-4" />
      </SelectPrimitive.ItemIndicator>
    </SelectPrimitive.Item>
  );
}

export function SelectLabel({ className, ...props }: React.ComponentProps<typeof SelectPrimitive.Label>) {
  return <SelectPrimitive.Label className={cn("px-2 py-1 text-xs text-muted-foreground", className)} {...props} />;
}
