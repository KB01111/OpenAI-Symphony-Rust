import React, { useState } from "react";
import type { ApprovalDecision, ApprovalRequest } from "@symphony/shared-types";
import { AiApprovalBanner, AiEmptyView, ErrorState, LoadingRows, useNotify } from "@symphony/ui";
import { useStrings } from "../i18n.js";
import { useDecideApproval, usePendingApprovals } from "./queries.js";

export const ApprovalsScreen: React.FC = () => {
  const { m, t } = useStrings();
  const approvals = usePendingApprovals();
  const decide = useDecideApproval();
  const notify = useNotify();
  const [pending, setPending] = useState<Record<string, ApprovalDecision>>({});

  const onDecide = (approval: ApprovalRequest, decision: ApprovalDecision) => {
    if (pending[approval.id]) return; // one decision per approval
    setPending((p) => ({ ...p, [approval.id]: decision }));
    // Per-call promise: each concurrent decision settles (and unlocks its banner) independently.
    decide
      .mutateAsync({ approval, decision })
      .then(() => notify("success", m.decisionSent))
      .catch((error: Error) => notify("error", m.decisionFailed, error.message))
      .finally(() => setPending(({ [approval.id]: _done, ...rest }) => rest));
  };

  if (approvals.isLoading) return <LoadingRows rows={3} label={t.common.loading} />;
  if (approvals.isError) {
    return <ErrorState title={m.loadApprovalsFailed} message={approvals.error.message} onRetry={() => void approvals.refetch()} retryLabel={t.common.retry} />;
  }
  const list = approvals.data ?? [];
  if (list.length === 0) return <AiEmptyView title={t.mobile.noApprovals} description={m.noApprovalsBody} />;
  return (
    <ul className="space-y-2">
      {list.map((approval) => (
        <li key={approval.id}>
          <AiApprovalBanner approval={approval} pendingDecision={pending[approval.id] ?? null} onDecide={onDecide} />
        </li>
      ))}
    </ul>
  );
};
