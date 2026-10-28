import React from "react";
import { AlertTriangle, CheckCircle2, CircleDashed, GitBranch, Search, XCircle } from "lucide-react";
import { formatMessage, type Project, type PullRequestReview } from "@symphony/shared-types";
import {
  ApprovalStatusBadge,
  Button,
  Checkbox,
  Field,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  TimeAgo,
  cn
} from "@symphony/ui";
import { useMessages } from "../../lib/i18n.js";
import { reviewMessages } from "./messages.js";
import {
  SCORE_CHIP_CLASS,
  ciTone,
  hasActiveReviewFilters,
  hasPullRequest,
  scoreTone,
  DEFAULT_REVIEW_FILTERS,
  type ApprovalFilter,
  type ReviewFilters
} from "./reviewHelpers.js";

const SCORE_OPTIONS = [2, 3, 4, 5] as const;

export const ReviewFiltersBar: React.FC<{ filters: ReviewFilters; onChange: (next: ReviewFilters) => void }> = ({ filters, onChange }) => {
  const m = useMessages(reviewMessages);
  const approvalLabels: Record<ApprovalFilter, string> = {
    all: m.filters.all,
    awaiting: m.filters.awaiting,
    approved: m.filters.approved
  };
  const scoreLabel = filters.minScore === null ? m.filters.anyScore : formatMessage(m.filters.scoreAtLeast, { score: filters.minScore });
  return (
    <div className="space-y-2 border-b border-border p-2">
      <Field label={m.search}>
        <Input
          type="search"
          icon={<Search />}
          value={filters.search}
          placeholder={m.searchPlaceholder}
          onChange={(e) => onChange({ ...filters, search: e.target.value })}
        />
      </Field>
      <div className="@container">
        <div className="grid grid-cols-1 gap-2 @xs:grid-cols-2">
          <Field label={m.filters.approval}>
            <Select value={filters.approval} onValueChange={(value) => onChange({ ...filters, approval: (value as ApprovalFilter) ?? "all" })}>
              <SelectTrigger>
                <SelectValue>{approvalLabels[filters.approval]}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                {(Object.keys(approvalLabels) as ApprovalFilter[]).map((key) => (
                  <SelectItem key={key} value={key}>
                    {approvalLabels[key]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
          <Field label={m.filters.minScore}>
            <Select
              value={filters.minScore === null ? "any" : String(filters.minScore)}
              onValueChange={(value) => onChange({ ...filters, minScore: value === "any" ? null : Number(value) })}
            >
              <SelectTrigger>
                <SelectValue>{scoreLabel}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="any">{m.filters.anyScore}</SelectItem>
                {SCORE_OPTIONS.map((score) => (
                  <SelectItem key={score} value={String(score)}>
                    {formatMessage(m.filters.scoreAtLeast, { score })}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
        </div>
      </div>
      <div className="flex items-center justify-between gap-2">
        <span className="inline-flex items-center gap-2">
          <Checkbox
            id="review-has-blockers"
            checked={filters.hasBlockers}
            onCheckedChange={(checked) => onChange({ ...filters, hasBlockers: checked === true })}
          />
          <Label htmlFor="review-has-blockers" className="text-sm font-normal">
            {m.filters.hasBlockers}
          </Label>
        </span>
        {hasActiveReviewFilters(filters) ? (
          <Button size="sm" variant="subtle" onClick={() => onChange(DEFAULT_REVIEW_FILTERS)}>
            {m.filters.clear}
          </Button>
        ) : null}
      </div>
    </div>
  );
};

export const ScoreChip: React.FC<{ score: number; className?: string }> = ({ score, className }) => {
  const m = useMessages(reviewMessages);
  const label = formatMessage(m.list.score, { score });
  return (
    <span
      className={cn("inline-flex shrink-0 items-center rounded-md px-1.5 py-0.5 text-xs font-semibold tabular-nums", SCORE_CHIP_CLASS[scoreTone(score)], className)}
      title={label}
      aria-label={label}
    >
      {score}/5
    </span>
  );
};

export const CiStatus: React.FC<{ status: string }> = ({ status }) => {
  const m = useMessages(reviewMessages);
  const tone = ciTone(status);
  const Icon = tone === "success" ? CheckCircle2 : tone === "danger" ? XCircle : CircleDashed;
  return (
    <span className={cn("inline-flex items-center gap-1", tone === "success" ? "text-success" : tone === "danger" ? "text-destructive" : "text-warning")}>
      <Icon className="size-3.5" aria-hidden />
      <span className="text-muted-foreground">
        {status === "none" ? m.list.ciNone : status ? formatMessage(m.list.ci, { status }) : m.list.ciUnknown}
      </span>
    </span>
  );
};

const ReviewListItem = React.memo(function ReviewListItem({
  review,
  project,
  selected,
  onSelect
}: {
  review: PullRequestReview;
  project: Project | undefined;
  selected: boolean;
  onSelect: (id: string) => void;
}) {
  const m = useMessages(reviewMessages);
  const blockers = review.merge_readiness.blockers.length;
  return (
    <li>
      <button
        type="button"
        aria-current={selected ? "page" : undefined}
        onClick={() => onSelect(review.id)}
        className={cn(
          "flex w-full flex-col gap-1 border-b border-border px-3 py-2 text-start transition-colors hover:bg-surface-hover focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-accent",
          selected && "bg-surface-selected"
        )}
      >
        <span className="flex items-start gap-2">
          <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
            <span className="text-muted-foreground">{hasPullRequest(review) ? `#${review.pr_number}` : m.list.localReview}</span> {review.title}
          </span>
          <ScoreChip score={review.merge_readiness.score} />
        </span>
        <span className="flex min-w-0 items-center gap-1 font-mono text-xs text-muted-foreground">
          <GitBranch className="size-3 shrink-0" aria-hidden />
          <span className="truncate">{review.branch_name}</span>
          <span aria-hidden>→</span>
          <span className="truncate">{review.base_branch}</span>
        </span>
        <span className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs">
          <span className="max-w-40 truncate text-muted-foreground">{project?.name ?? m.list.unknownProject}</span>
          <CiStatus status={review.ci_status} />
          <ApprovalStatusBadge status={review.human_approved ? "approved" : "pending"} />
          {blockers > 0 ? (
            <span className="inline-flex items-center gap-1 text-warning">
              <AlertTriangle className="size-3.5" aria-hidden />
              {formatMessage(m.list.blockers, { count: blockers })}
            </span>
          ) : null}
          <TimeAgo iso={review.updated_at} className="ms-auto text-subtle-foreground" />
        </span>
      </button>
    </li>
  );
});

export const ReviewList: React.FC<{
  reviews: ReadonlyArray<PullRequestReview>;
  projects: ReadonlyMap<string, Project>;
  selectedId: string | undefined;
  onSelect: (id: string) => void;
}> = ({ reviews, projects, selectedId, onSelect }) => {
  const m = useMessages(reviewMessages);
  return (
    <ul aria-label={m.listLabel} className="min-h-0 flex-1 overflow-auto">
      {reviews.map((review) => (
        <ReviewListItem
          key={review.id}
          review={review}
          project={projects.get(review.project_id)}
          selected={review.id === selectedId}
          onSelect={onSelect}
        />
      ))}
    </ul>
  );
};
