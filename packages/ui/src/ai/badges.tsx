import React from "react";
import { Badge } from "@openai/apps-sdk-ui/components/Badge";
import { ShimmerText } from "@openai/apps-sdk-ui/components/ShimmerText";
import {
  isActivePhase,
  type ApprovalStatus,
  type ProviderKind,
  type RunAttemptPhase,
  type TaskStatus
} from "@symphony/shared-types";
import { useUiStrings } from "../i18n.js";

type BadgeColor = "secondary" | "success" | "danger" | "warning" | "info" | "discovery";

export const TASK_STATUS_COLORS: Record<TaskStatus, BadgeColor> = {
  backlog: "secondary",
  ready: "info",
  queued: "discovery",
  running: "info",
  blocked: "warning",
  human_review: "discovery",
  completed: "success",
  failed: "danger",
  cancelled: "secondary"
};

const PROVIDER_COLORS: Record<ProviderKind, BadgeColor> = {
  codex: "discovery",
  claude_code: "warning",
  opencode: "success",
  simulated: "secondary"
};

export const TaskStatusBadge = React.memo(function TaskStatusBadge({ status }: { status: TaskStatus }) {
  const { t } = useUiStrings();
  const label = t.status[status];
  return (
    <Badge variant="soft" color={TASK_STATUS_COLORS[status]} size="sm" pill>
      {status === "running" ? <ShimmerText as="span">{label}</ShimmerText> : label}
    </Badge>
  );
});

export const PhaseBadge = React.memo(function PhaseBadge({ phase }: { phase: RunAttemptPhase }) {
  const { t } = useUiStrings();
  const active = isActivePhase(phase);
  const color: BadgeColor =
    phase === "succeeded"
      ? "success"
      : phase === "failed" || phase === "timed_out" || phase === "stalled"
        ? "danger"
        : active
          ? "info"
          : "secondary";
  return (
    <Badge variant="outline" color={color} size="sm" pill>
      {active ? <ShimmerText as="span">{t.phase[phase]}</ShimmerText> : t.phase[phase]}
    </Badge>
  );
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
      <Badge variant="soft" color={PROVIDER_COLORS[provider]} size="sm" pill>
        {t.provider[provider]}
      </Badge>
      {model ? (
        <span className="inline-flex min-w-0" title={model}>
          <Badge variant="outline" color="secondary" size="sm" className="min-w-0 max-w-full font-mono">
            <span className="min-w-0 truncate">{model}</span>
          </Badge>
        </span>
      ) : null}
    </span>
  );
});

export const ApprovalStatusBadge = React.memo(function ApprovalStatusBadge({ status }: { status: ApprovalStatus }) {
  const { t } = useUiStrings();
  const color: BadgeColor =
    status === "pending" ? "warning" : status === "approved" ? "success" : status === "denied" ? "danger" : "secondary";
  return (
    <Badge variant="soft" color={color} size="sm" pill>
      {t.approvalStatus[status]}
    </Badge>
  );
});
