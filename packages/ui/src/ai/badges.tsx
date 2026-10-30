import React from "react";
import {
  isActivePhase,
  type ApprovalStatus,
  type ProviderKind,
  type RunAttemptPhase,
  type TaskStatus
} from "@symphony/shared-types";
import { useUiStrings } from "../i18n.js";
import { Badge, type BadgeTone } from "../primitives/badge.js";

export const TASK_STATUS_COLORS: Record<TaskStatus, BadgeTone> = {
  backlog: "neutral",
  ready: "info",
  queued: "accent",
  running: "info",
  blocked: "warning",
  human_review: "accent",
  completed: "success",
  failed: "danger",
  cancelled: "neutral"
};

const PROVIDER_COLORS: Record<ProviderKind, BadgeTone> = {
  codex: "accent",
  claude_code: "warning",
  opencode: "success",
  codesandbox: "info",
  simulated: "neutral"
};

/** Text that gently pulses while work is in progress (static under reduced motion). */
const Live: React.FC<{ children: React.ReactNode }> = ({ children }) => <span className="animate-pulse">{children}</span>;

export const TaskStatusBadge = React.memo(function TaskStatusBadge({ status }: { status: TaskStatus }) {
  const { t } = useUiStrings();
  const label = t.status[status];
  return <Badge tone={TASK_STATUS_COLORS[status]}>{status === "running" ? <Live>{label}</Live> : label}</Badge>;
});

export const PhaseBadge = React.memo(function PhaseBadge({ phase }: { phase: RunAttemptPhase }) {
  const { t } = useUiStrings();
  const active = isActivePhase(phase);
  const tone: BadgeTone =
    phase === "succeeded"
      ? "success"
      : phase === "failed" || phase === "timed_out" || phase === "stalled"
        ? "danger"
        : active
          ? "info"
          : "neutral";
  return <Badge tone={tone}>{active ? <Live>{t.phase[phase]}</Live> : t.phase[phase]}</Badge>;
});

export const ProviderBadge = React.memo(function ProviderBadge({
  provider,
  model
}: {
  provider: ProviderKind;
  model?: string | null;
}) {
  const { t } = useUiStrings();
  return (
    <span className="inline-flex min-w-0 items-center gap-1">
      <Badge tone={PROVIDER_COLORS[provider]}>{t.provider[provider]}</Badge>
      {model ? (
        <Badge tone="outline" className="min-w-0 max-w-full font-mono" title={model}>
          <span className="min-w-0 truncate">{model}</span>
        </Badge>
      ) : null}
    </span>
  );
});

export const ApprovalStatusBadge = React.memo(function ApprovalStatusBadge({ status }: { status: ApprovalStatus }) {
  const { t } = useUiStrings();
  const tone: BadgeTone = status === "pending" ? "warning" : status === "approved" ? "success" : status === "denied" ? "danger" : "neutral";
  return <Badge tone={tone}>{t.approvalStatus[status]}</Badge>;
});
