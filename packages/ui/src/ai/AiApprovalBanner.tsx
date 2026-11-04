import React from "react";
import { ShieldQuestion } from "lucide-react";
import { formatMessage, type ApprovalDecision, type ApprovalRequest } from "@symphony/shared-types";
import { useUiStrings } from "../i18n.js";
import { Button, type ButtonProps } from "../primitives/button.js";
import { Spinner } from "../primitives/spinner.js";
import { ProviderBadge } from "./badges.js";
import { CodeBlock } from "./codeBlocks.js";

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

const DECISIONS: ReadonlyArray<{ decision: ApprovalDecision; variant: NonNullable<ButtonProps["variant"]> }> = [
  { decision: "accept", variant: "default" },
  { decision: "accept_for_session", variant: "secondary" },
  { decision: "decline", variant: "outline" },
  { decision: "cancel", variant: "destructive-ghost" }
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
      className="space-y-2.5 rounded-xl border border-warning/40 bg-warning-subtle p-3"
    >
      <div className="flex items-start gap-2">
        <ShieldQuestion className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden />
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-2 text-sm font-medium text-foreground">
            {formatMessage(ui.approvalTitle, { identifier: approval.issue_identifier })}
            <ProviderBadge provider={approval.provider} />
          </p>
          <p className="break-words text-sm text-muted-foreground">{approval.description || approval.title || ui.approvalFallback}</p>
        </div>
      </div>
      {!compact && approval.command_or_diff ? (
        <CodeBlock language={approval.kind.includes("file") ? "diff" : "bash"}>{approval.command_or_diff}</CodeBlock>
      ) : null}
      <div className="flex flex-wrap items-center gap-2" role="group">
        {DECISIONS.map(({ decision, variant }) => (
          <Button key={decision} size="sm" variant={variant} disabled={busy} onClick={() => onDecide(approval, decision)}>
            {pendingDecision === decision ? <Spinner className="size-3.5 text-current" /> : null}
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
