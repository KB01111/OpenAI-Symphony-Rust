import * as React from "react";
import { Slot } from "radix-ui";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../lib/utils";

export const buttonVariants = cva(
  "inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-md text-sm font-medium transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring/60 disabled:pointer-events-none disabled:opacity-50 aria-disabled:pointer-events-none aria-disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*=size-])]:size-4",
  {
    variants: {
      variant: {
        default: "bg-accent text-accent-foreground hover:bg-accent-hover",
        secondary: "border border-border bg-surface text-foreground hover:bg-surface-hover",
        outline: "border border-border-strong bg-transparent text-foreground hover:bg-surface-hover",
        ghost: "text-foreground hover:bg-surface-hover",
        subtle: "text-muted-foreground hover:bg-surface-hover hover:text-foreground",
        destructive: "bg-destructive text-background hover:opacity-90",
        "destructive-ghost": "text-destructive hover:bg-destructive-subtle",
        link: "text-accent underline-offset-4 hover:underline"
      },
      size: {
        sm: "h-7 px-2.5 text-xs",
        default: "h-8 px-3",
        lg: "h-10 px-4 text-base",
        icon: "size-8",
        "icon-sm": "size-7"
      }
    },
    defaultVariants: { variant: "secondary", size: "default" }
  }
);

export interface ButtonProps extends React.ComponentProps<"button">, VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

export function Button({ className, variant, size, asChild = false, type, ...props }: ButtonProps) {
  const Comp = asChild ? Slot.Root : "button";
  return (
    <Comp
      data-slot="button"
      {...(asChild ? {} : { type: type ?? "button" })}
      className={cn(buttonVariants({ variant, size }), className)}
      {...props}
    />
  );
}
