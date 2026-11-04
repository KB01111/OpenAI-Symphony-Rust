import React from "react";
import { Alert } from "../primitives/alert.js";
import { Badge } from "../primitives/badge.js";
import { Button } from "../primitives/button.js";
import { Spinner } from "../primitives/spinner.js";
import { formatMessage, type PullRequestReview } from "@symphony/shared-types";
import { CheckCircle2, CircleDashed, GitMerge, ShieldAlert, ShieldCheck, XCircle } from "lucide-react";
import { cn } from "../lib/utils.js";
import { useUiStrings } from "../i18n.js";

type EvidenceState = "pass" | "fail" | "pending";

export interface ReadinessEvidence {
  ci: EvidenceState;
  validation: EvidenceState;
  conflicts: EvidenceState;
  security: EvidenceState;
  reviewedComments: number;
}

/** Derive the evidence breakdown from the runtime's PR review record. */
export function readinessEvidence(review: PullRequestReview): ReadinessEvidence {
  const ci = review.ci_status.toLowerCase();
  const validations = review.validation_results;
  return {
    ci: ci === "success" || ci === "passed" ? "pass" : ci === "failure" || ci === "failed" ? "fail" : "pending",
    validation:
      validations.length === 0 ? "pending" : validations.every((v) => v.passed) ? "pass" : "fail",
    // `mergeable` is only true after a simulated merge succeeded: unknown is not "clean".
    conflicts: review.has_conflicts ? "fail" : review.mergeable ? "pass" : "pending",
    security: review.security_warnings.length > 0 ? "fail" : "pass",
    reviewedComments: review.review_comments.length
  };
}

const SCORE_TONE = (score: number) =>
  score >= 5 ? "text-success" : score >= 4 ? "text-success" : score === 3 ? "text-warning" : "text-destructive";

const SCORE_BAR = (score: number) =>
  score >= 4 ? "bg-success" : score === 3 ? "bg-warning" : "bg-destructive";

export interface MergeReadinessCardProps {
  review: PullRequestReview;
  approving?: boolean;
  /** Request explicit human approval. The caller confirms before calling the API. */
  onApprove?: (review: PullRequestReview) => void;
  className?: string;
}

/**
 * Evidence-backed 1–5 merge readiness summary. The score is informational: approval is a separate
 * explicit human action and never merges a pull request.
 */
export const MergeReadinessCard: React.FC<MergeReadinessCardProps> = ({ review, approving = false, onApprove, className }) => {
  const { ui } = useUiStrings();
  const report = review.merge_readiness;
  const evidence = readinessEvidence(review);

  const rows: { key: string; label: string; state: EvidenceState; value: string }[] = [
    {
      key: "ci",
      label: ui.ci,
      state: evidence.ci,
      value: review.ci_status === "none" || !review.ci_status ? ui.noRemoteCi : review.ci_status
    },
    {
      key: "validation",
      label: ui.validation,
      state: evidence.validation,
      value:
        evidence.validation === "pending"
          ? ui.noneRun
          : `${review.validation_results.filter((v) => v.passed).length}/${review.validation_results.length}`
    },
    { key: "conflicts", label: ui.conflicts, state: evidence.conflicts, value: evidence.conflicts === "pass" ? ui.clean : evidence.conflicts === "fail" ? ui.detected : ui.notChecked },
    {
      key: "security",
      label: ui.security,
      state: evidence.security,
      value: evidence.security === "pass" ? ui.clean : formatMessage(ui.warnings, { count: review.security_warnings.length })
    },
    { key: "comments", label: ui.reviewedComments, state: "pass", value: String(evidence.reviewedComments) }
  ];

  return (
    <section aria-label={ui.readiness} className={cn("@container space-y-3 rounded-xl border border-border bg-surface p-4", className)}>
      <div className="flex flex-wrap items-start gap-4">
        <div className="flex items-baseline gap-2">
          <span className={cn("font-display text-4xl font-semibold tabular-nums", SCORE_TONE(report.score))}>
            {report.score}
          </span>
          <span className="text-sm text-muted-foreground">/ 5</span>
        </div>
        <div className="min-w-0 flex-1 space-y-1.5">
          <div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">{ui.readiness}</div>
          <div className="text-sm font-medium text-foreground">{report.label}</div>
          <div
            className="flex h-1.5 gap-0.5"
            role="meter"
            aria-valuemin={1}
            aria-valuemax={5}
            aria-valuenow={report.score}
            aria-label={ui.readiness}
          >
            {[1, 2, 3, 4, 5].map((step) => (
              <span key={step} className={cn("flex-1 rounded-full", step <= report.score ? SCORE_BAR(report.score) : "bg-muted")} />
            ))}
          </div>
        </div>
        <div className="flex flex-col items-end gap-2">
          {review.human_approved ? (
            <Badge tone="success">{ui.humanApproved}</Badge>
          ) : (
            <Badge tone="warning">{ui.awaitingApproval}</Badge>
          )}
          {onApprove && !review.human_approved ? (
            <Button size="sm" variant="default" disabled={approving} onClick={() => onApprove(review)}>
              {approving ? <Spinner className="size-3.5 text-current" /> : <GitMerge className="size-4" aria-hidden />}
              {ui.approveForMerge}
            </Button>
          ) : null}
        </div>
      </div>

      <dl className="grid grid-cols-2 gap-2 @xl:grid-cols-5">
        {rows.map((row) => (
          <div key={row.key} className="rounded-md border border-border bg-surface-sunken px-2.5 py-2">
            <dt className="flex items-center gap-1.5 text-xs text-muted-foreground">
              {row.key === "security" ? (
                row.state === "pass" ? (
                  <ShieldCheck className="size-3.5 text-success" aria-hidden />
                ) : (
                  <ShieldAlert className="size-3.5 text-destructive" aria-hidden />
                )
              ) : row.state === "pass" ? (
                <CheckCircle2 className="size-3.5 text-success" aria-hidden />
              ) : row.state === "fail" ? (
                <XCircle className="size-3.5 text-destructive" aria-hidden />
              ) : (
                <CircleDashed className="size-3.5 text-warning" aria-hidden />
              )}
              {row.label}
            </dt>
            <dd className="mt-0.5 line-clamp-2 text-sm font-medium [overflow-wrap:anywhere] text-foreground" title={typeof row.value === "string" ? row.value : undefined}>{row.value}</dd>
          </div>
        ))}
      </dl>

      {report.blockers.length > 0 ? (
        <Alert intent="warning" title={ui.blockers}>
          <ul className="list-disc ps-4">
            {report.blockers.map((b) => (
              <li key={b}>{b}</li>
            ))}
          </ul>
        </Alert>
      ) : null}
      {report.evidence.length > 0 ? (
        <div>
          <div className="mb-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">{ui.evidence}</div>
          <ul className="space-y-0.5 text-sm text-foreground">
            {report.evidence.map((e) => (
              <li key={e} className="flex gap-2">
                <CheckCircle2 className="mt-0.5 size-3.5 shrink-0 text-success" aria-hidden />
                {e}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {!review.human_approved ? <p className="text-xs text-muted-foreground">{ui.approvalNeverMerges}</p> : null}
    </section>
  );
};
