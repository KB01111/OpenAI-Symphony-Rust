import React from "react";
import { Alert } from "@openai/apps-sdk-ui/components/Alert";
import { Button } from "@openai/apps-sdk-ui/components/Button";
import { CodeBlock } from "@openai/apps-sdk-ui/components/CodeBlock";
import { formatMessage, type ApprovalDecision, type ApprovalRequest } from "@symphony/shared-types";
import { useUiStrings } from "../i18n.js";
import { ProviderBadge } from "./badges.js";

export interface AiApprovalBannerProps {
  approval: ApprovalRequest;
  /** Decision currently being submitted for this approval; disables all actions. */
  pendingDecision?: ApprovalDecision | null;
  onDecide: (approval: ApprovalRequest, decision: ApprovalDecision) => void;
  /** Hide the command/diff preview (e.g. in compact lists). */
  compact?: boolean;
  /** Optional extra action, e.g. navigation to the task. */
  extra?: React.ReactNode;
}

const DECISIONS: ReadonlyArray<{ decision: ApprovalDecision; color: "primary" | "discovery" | "secondary" | "danger"; variant: "solid" | "soft" | "outline" | "ghost" }> = [
  { decision: "accept", color: "primary", variant: "solid" },
  { decision: "accept_for_session", color: "discovery", variant: "soft" },
  { decision: "decline", color: "secondary", variant: "outline" },
  { decision: "cancel", color: "danger", variant: "ghost" }
];

/**
 * Approval surface for agent command/file-change requests. Decisions are sent to the runtime
 * approval gate; the banner never decides on its own and disables itself while a decision is in
 * flight so a request cannot be answered twice.
 */
export const AiApprovalBanner: React.FC<AiApprovalBannerProps> = ({
  approval,
  pendingDecision = null,
  onDecide,
  compact = false,
  extra
}) => {
  const { t, ui } = useUiStrings();
  const busy = pendingDecision !== null;
  return (
    <section
      aria-label={formatMessage(ui.approvalTitle, { identifier: approval.issue_identifier })}
      className="space-y-2 rounded-md border border-warning/40 bg-warning-subtle p-3"
    >
      <Alert
        color="warning"
        variant="soft"
        title={
          <span className="flex flex-wrap items-center gap-2">
            {formatMessage(ui.approvalTitle, { identifier: approval.issue_identifier })}
            <ProviderBadge provider={approval.provider} />
          </span>
        }
        description={approval.description || approval.title || ui.approvalFallback}
      />
      {!compact && approval.command_or_diff ? (
        <CodeBlock language={approval.kind.includes("file") ? "diff" : "bash"}>{approval.command_or_diff}</CodeBlock>
      ) : null}
      <div className="flex flex-wrap items-center gap-2" role="group">
        {DECISIONS.map(({ decision, color, variant }) => (
          <Button
            key={decision}
            size="sm"
            color={color}
            variant={variant}
            disabled={busy}
            loading={pendingDecision === decision}
            onClick={() => onDecide(approval, decision)}
          >
            {t.approvalDecision[decision]}
          </Button>
        ))}
        {busy ? (
          <span role="status" className="text-xs text-muted-foreground">
            {ui.approvalSubmitting}
          </span>
        ) : null}
        {extra ? <span className="ms-auto">{extra}</span> : null}
      </div>
    </section>
  );
};
