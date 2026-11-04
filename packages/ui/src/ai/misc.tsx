import React from "react";
import { Sparkles } from "lucide-react";
import { cn } from "../lib/utils.js";
import { useUiStrings } from "../i18n.js";
import { Button } from "../primitives/button.js";
import { Spinner } from "../primitives/spinner.js";
import { Textarea } from "../primitives/input.js";
import { ToggleGroup, ToggleGroupItem } from "../primitives/toggle-group.js";

export interface AiEmptyViewProps {
  title: string;
  description?: string;
  icon?: React.ReactNode;
  action?: React.ReactNode;
  className?: string;
}

/** Empty state for agent-related surfaces (no runs, no events, no approvals…). */
export const AiEmptyView: React.FC<AiEmptyViewProps> = ({ title, description, icon, action, className }) => (
  <div className={cn("flex min-h-40 items-center justify-center rounded-xl border border-dashed border-border p-6", className)}>
    <div className="flex max-w-sm flex-col items-center gap-2 text-center">
      <span className="flex size-10 items-center justify-center rounded-full bg-accent-subtle" aria-hidden>
        {icon ?? <Sparkles className="size-5 text-accent" />}
      </span>
      <p className="text-sm font-medium text-foreground">{title}</p>
      {description ? <p className="text-sm text-muted-foreground">{description}</p> : null}
      {action ? <div className="mt-1 flex flex-wrap justify-center gap-2">{action}</div> : null}
    </div>
  </div>
);

export interface LogModeSwitchProps {
  value: "concise" | "technical";
  onChange: (mode: "concise" | "technical") => void;
}

/** Concise / Technical toggle: a fixed-width segmented control that never overlaps its labels. */
export const LogModeSwitch: React.FC<LogModeSwitchProps> = ({ value, onChange }) => {
  const { ui } = useUiStrings();
  return (
    <ToggleGroup
      type="single"
      aria-label={`${ui.concise} / ${ui.technical}`}
      value={value}
      onValueChange={(next) => next && onChange(next as "concise" | "technical")}
    >
      <ToggleGroupItem value="concise">{ui.concise}</ToggleGroupItem>
      <ToggleGroupItem value="technical">{ui.technical}</ToggleGroupItem>
    </ToggleGroup>
  );
};

/** Subtle live-activity text for running agents (static when reduced motion is preferred). */
export const StreamingText: React.FC<{ children: React.ReactNode; active?: boolean; className?: string }> = ({
  children,
  active = true,
  className
}) => <span className={cn(active && "animate-pulse", className)}>{children}</span>;

export interface PromptComposerBoxProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  label?: string;
  onSubmit?: () => void;
  submitLabel?: string;
  disabled?: boolean;
  submitting?: boolean;
  rows?: number;
}

/** Task specification / prompt composer. Ctrl+Enter submits. */
export const PromptComposerBox: React.FC<PromptComposerBoxProps> = ({
  value,
  onChange,
  placeholder,
  label,
  onSubmit,
  submitLabel,
  disabled = false,
  submitting = false,
  rows = 5
}) => (
  <div className="space-y-2">
    <Textarea
      aria-label={label ?? placeholder}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      onKeyDown={(e) => {
        if (onSubmit && !submitting && !disabled && (e.ctrlKey || e.metaKey) && e.key === "Enter" && value.trim()) {
          e.preventDefault();
          onSubmit();
        }
      }}
      placeholder={placeholder}
      rows={rows}
      disabled={disabled}
      className="max-h-80 [field-sizing:content]"
    />
    {onSubmit && submitLabel ? (
      <div className="flex justify-end">
        <Button variant="default" size="sm" onClick={onSubmit} disabled={disabled || submitting || !value.trim()}>
          {submitting ? <Spinner className="size-3.5 text-current" /> : null}
          {submitLabel}
        </Button>
      </div>
    ) : null}
  </div>
);
