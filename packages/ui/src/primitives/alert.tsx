import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { AlertCircleIcon, AlertTriangleIcon, CheckCircle2Icon, InfoIcon } from "lucide-react";
import { cn } from "../lib/utils";

const alertVariants = cva("flex items-start gap-2 rounded-lg border px-3 py-2 text-sm", {
  variants: {
    intent: {
      info: "border-transparent bg-info-subtle text-foreground [&>svg]:text-info",
      success: "border-transparent bg-success-subtle text-foreground [&>svg]:text-success",
      warning: "border-transparent bg-warning-subtle text-foreground [&>svg]:text-warning",
      error: "border-transparent bg-destructive-subtle text-foreground [&>svg]:text-destructive"
    }
  },
  defaultVariants: { intent: "info" }
});

const icons = { info: InfoIcon, success: CheckCircle2Icon, warning: AlertTriangleIcon, error: AlertCircleIcon } as const;

export function Alert({ className, intent = "info", title, children, ...props }: React.ComponentProps<"div"> & VariantProps<typeof alertVariants> & { title?: React.ReactNode }) {
  const Icon = icons[intent ?? "info"];
  return (
    <div role={intent === "error" ? "alert" : "status"} data-slot="alert" className={cn(alertVariants({ intent }), className)} {...props}>
      <Icon className="mt-0.5 size-4 shrink-0" aria-hidden />
      <div className="min-w-0 flex-1">
        {title ? <p className="font-medium">{title}</p> : null}
        <div className={cn(title && "text-muted-foreground")}>{children}</div>
      </div>
    </div>
  );
}
