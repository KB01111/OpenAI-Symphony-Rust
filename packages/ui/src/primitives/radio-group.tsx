import * as React from "react";
import { RadioGroup as RadioGroupPrimitive } from "radix-ui";
import { cn } from "../lib/utils";
import { Label } from "./label";

export function RadioGroup({ className, ...props }: React.ComponentProps<typeof RadioGroupPrimitive.Root>) {
  return <RadioGroupPrimitive.Root data-slot="radio-group" className={cn("flex flex-col gap-2", className)} {...props} />;
}

/** One radio with its label; `description` renders muted text under the label. */
export function RadioOption({
  value,
  label,
  description,
  disabled,
  className
}: {
  value: string;
  label: React.ReactNode;
  description?: React.ReactNode;
  disabled?: boolean;
  className?: string;
}) {
  const id = React.useId();
  return (
    <div className={cn("flex items-start gap-2", className)}>
      <RadioGroupPrimitive.Item
        id={id}
        value={value}
        disabled={disabled}
        className="mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-full border border-border-strong bg-surface outline-none focus-visible:ring-2 focus-visible:ring-ring/60 disabled:opacity-50 data-[state=checked]:border-accent"
      >
        <RadioGroupPrimitive.Indicator className="size-2 rounded-full bg-accent" />
      </RadioGroupPrimitive.Item>
      <div className="min-w-0">
        <Label htmlFor={id} className="text-sm font-normal">
          {label}
        </Label>
        {description ? <p className="text-xs text-muted-foreground">{description}</p> : null}
      </div>
    </div>
  );
}
