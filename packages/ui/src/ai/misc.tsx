import React from "react";
import { Button } from "@openai/apps-sdk-ui/components/Button";
import { EmptyMessage } from "@openai/apps-sdk-ui/components/EmptyMessage";
import { SegmentedControl } from "@openai/apps-sdk-ui/components/SegmentedControl";
import { ShimmerText } from "@openai/apps-sdk-ui/components/ShimmerText";
import { Textarea } from "@openai/apps-sdk-ui/components/Textarea";
import { Sparkles } from "lucide-react";
import { cn } from "../lib/utils.js";
import { useUiStrings } from "../i18n.js";

export interface AiEmptyViewProps {
  title: string;
  description?: string;
  icon?: React.ReactNode;
  action?: React.ReactNode;
  className?: string;
}

/** Empty state for agent-related surfaces (no runs, no events, no approvals…). */
export const AiEmptyView: React.FC<AiEmptyViewProps> = ({ title, description, icon, action, className }) => (
  <div className={cn("flex min-h-40 items-center justify-center rounded-md border border-dashed border-border p-6", className)}>
    <EmptyMessage fill="none">
      <EmptyMessage.Icon>{icon ?? <Sparkles className="size-6 text-accent" aria-hidden />}</EmptyMessage.Icon>
      <EmptyMessage.Title>{title}</EmptyMessage.Title>
      {description ? <EmptyMessage.Description>{description}</EmptyMessage.Description> : null}
      {action ? <EmptyMessage.ActionRow>{action}</EmptyMessage.ActionRow> : null}
    </EmptyMessage>
  </div>
);

export interface LogModeSwitchProps {
  value: "concise" | "technical";
  onChange: (mode: "concise" | "technical") => void;
}

export const LogModeSwitch: React.FC<LogModeSwitchProps> = ({ value, onChange }) => {
  const { ui } = useUiStrings();
  return (
    <SegmentedControl
      aria-label={`${ui.concise} / ${ui.technical}`}
      value={value}
      onChange={(next) => onChange(next as "concise" | "technical")}
      size="sm"
    >
      <SegmentedControl.Option value="concise">{ui.concise}</SegmentedControl.Option>
      <SegmentedControl.Option value="technical">{ui.technical}</SegmentedControl.Option>
    </SegmentedControl>
  );
};

/** Subtle live-activity text for running agents (static when reduced motion is preferred). */
export const StreamingText: React.FC<{ children: React.ReactNode; active?: boolean; className?: string }> = ({
  children,
  active = true,
  className
}) => (active ? <ShimmerText as="span" className={className}>{children}</ShimmerText> : <span className={className}>{children}</span>);

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
        if (onSubmit && (e.ctrlKey || e.metaKey) && e.key === "Enter" && value.trim()) {
          e.preventDefault();
          onSubmit();
        }
      }}
      placeholder={placeholder}
      rows={rows}
      autoResize
      maxRows={14}
      disabled={disabled}
    />
    {onSubmit && submitLabel ? (
      <div className="flex justify-end">
        <Button color="primary" size="sm" onClick={onSubmit} disabled={disabled || submitting || !value.trim()} loading={submitting}>
          {submitLabel}
        </Button>
      </div>
    ) : null}
  </div>
);
