import React, { useRef, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import {
  Button,
  CounterBadge,
  MessageBar,
  MessageBarBody,
  MessageBarTitle,
  Tab,
  TabList
} from "@fluentui/react-components";
import { ExternalLink, GitBranch, GitPullRequest, ShieldAlert } from "lucide-react";
import { formatMessage, type Project, type PullRequestReview } from "@symphony/shared-types";
import { AiEmptyView, ConfirmDialog, ErrorState, KeyValueList, LoadingRows, MergeReadinessCard, SdkBadge, SdkMarkdown, TimeAgo, useNotify } from "@symphony/ui";
import { errorMessage, SymphonyApiError } from "@symphony/api-client";
import { paths } from "../../app/routes.js";
import { useMessages } from "../../lib/i18n.js";
import { useApprovePullRequest, usePullRequest } from "../../lib/queries.js";
import { reviewMessages } from "./messages.js";
import { isWebUrl } from "./reviewHelpers.js";
import { CommentsTab, DiffTab, FilesChangedTab, SecurityTab, ValidationTab, type DiffSource } from "./ReviewTabs.js";

type ReviewTab = "files" | "diff" | "validation" | "comments" | "security";

const TabLabel: React.FC<{ label: string; count?: number; tone?: "danger" | "informative" }> = ({ label, count, tone = "informative" }) => (
  <span className="inline-flex items-center gap-1.5">
    {label}
    {count ? <CounterBadge size="small" appearance={tone === "danger" ? "filled" : "ghost"} color={tone === "danger" ? "danger" : "informative"} count={count} /> : null}
  </span>
);

const ReviewUrl: React.FC<{ url: string }> = ({ url }) => {
  const m = useMessages(reviewMessages);
  if (isWebUrl(url)) {
    return (
      <a href={url} target="_blank" rel="noreferrer" className="inline-flex max-w-full items-center gap-1 text-accent underline-offset-2 hover:underline">
        <span className="truncate">{url}</span>
        <ExternalLink className="size-3.5 shrink-0" aria-hidden />
      </a>
    );
  }
  return (
    <span className="inline-flex max-w-full flex-col">
      <span className="truncate">{url}</span>
      <span className="font-sans text-xs text-subtle-foreground">{m.detail.localUrlHint}</span>
    </span>
  );
};

const ReviewContent: React.FC<{ review: PullRequestReview; project: Project | undefined }> = ({ review, project }) => {
  const m = useMessages(reviewMessages);
  const notify = useNotify();
  const approve = useApprovePullRequest();
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [tab, setTab] = useState<ReviewTab>("files");
  const [file, setFile] = useState<string | null>(null);
  const [diffSource, setDiffSource] = useState<DiffSource>("snapshot");

  // Synchronous guard: `isPending` only updates on the next render, so a fast double click
  // could otherwise submit twice.
  const submitting = useRef(false);

  const confirmApproval = () => {
    if (submitting.current || approve.isPending || review.human_approved) return;
    submitting.current = true;
    approve.mutate(review.id, {
      onSuccess: () => notify("success", m.approve.success, m.approve.successBody),
      onSettled: () => {
        submitting.current = false;
        setConfirmOpen(false);
      }
    });
  };

  const blockers = review.merge_readiness.blockers;
  const warnings = review.security_warnings;

  return (
    <div className="space-y-3 p-4">
      <section aria-labelledby={`review-title-${review.id}`} className="space-y-2">
        <div className="flex flex-wrap items-start gap-2">
          <GitPullRequest className="mt-0.5 size-5 shrink-0 text-accent" aria-hidden />
          <h2 id={`review-title-${review.id}`} className="min-w-0 flex-1 font-display text-lg font-semibold text-foreground">
            <span className="text-muted-foreground">#{review.pr_number}</span> {review.title}
          </h2>
          <SdkBadge size="sm" variant="outline" color="secondary" pill>
            {review.state}
          </SdkBadge>
          <TimeAgo iso={review.updated_at} className="text-xs text-subtle-foreground" />
        </div>
        <KeyValueList
          items={[
            {
              label: m.detail.repository,
              value: (
                <span className="flex min-w-0 flex-col">
                  <span className="font-medium">{project?.name ?? review.project_id}</span>
                  {project ? <span className="truncate font-mono text-xs text-muted-foreground">{project.repo_path}</span> : null}
                </span>
              )
            },
            { label: m.detail.url, value: <ReviewUrl url={review.url} />, mono: true },
            {
              label: m.detail.branches,
              value: (
                <span className="inline-flex min-w-0 items-center gap-1">
                  <GitBranch className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
                  <span className="truncate">{review.branch_name}</span>
                  <span aria-hidden>→</span>
                  <span className="truncate">{review.base_branch}</span>
                </span>
              ),
              mono: true
            },
            {
              label: m.detail.task,
              value: (
                <Link to={paths.run(review.task_id)} className="text-accent underline-offset-2 hover:underline">
                  {m.detail.openTask} <span className="font-mono text-xs text-muted-foreground">({review.task_id})</span>
                </Link>
              )
            }
          ]}
        />
      </section>

      <MergeReadinessCard
        review={review}
        approving={approve.isPending}
        onApprove={review.human_approved ? undefined : () => setConfirmOpen(true)}
      />

      {review.has_conflicts ? (
        <MessageBar intent="error">
          <MessageBarBody>{m.detail.conflicts}</MessageBarBody>
        </MessageBar>
      ) : null}
      {warnings.length > 0 ? (
        <MessageBar intent="warning" layout="multiline" icon={<ShieldAlert className="size-4" aria-hidden />}>
          <MessageBarBody>
            <MessageBarTitle>{m.detail.securityTitle}</MessageBarTitle>
            <ul className="list-disc ps-4">
              {warnings.map((w, i) => (
                <li key={`${i}-${w}`}>{w}</li>
              ))}
            </ul>
          </MessageBarBody>
        </MessageBar>
      ) : null}

      {review.description.trim() ? (
        <details className="rounded-md border border-border bg-surface">
          <summary className="cursor-pointer px-3 py-1.5 text-sm font-medium text-foreground hover:bg-surface-hover">{m.detail.descriptionTitle}</summary>
          <div className="border-t border-border px-3 py-2 text-sm text-foreground">
            <SdkMarkdown skipHtml>{review.description}</SdkMarkdown>
          </div>
        </details>
      ) : null}

      <div className="rounded-md border border-border bg-surface">
        <TabList
          size="small"
          aria-label={m.tabs.label}
          selectedValue={tab}
          onTabSelect={(_, data) => setTab(data.value as ReviewTab)}
          className="border-b border-border px-1"
        >
          <Tab value="files">
            <TabLabel label={m.tabs.files} count={review.changed_files.length} />
          </Tab>
          <Tab value="diff">
            <TabLabel label={m.tabs.diff} />
          </Tab>
          <Tab value="validation">
            <TabLabel
              label={m.tabs.validation}
              count={review.validation_results.length}
              tone={review.validation_results.some((v) => !v.passed) ? "danger" : "informative"}
            />
          </Tab>
          <Tab value="comments">
            <TabLabel label={m.tabs.comments} count={review.review_comments.length} />
          </Tab>
          <Tab value="security">
            <TabLabel label={m.tabs.security} count={warnings.length} tone="danger" />
          </Tab>
        </TabList>
        <div role="tabpanel" aria-label={m.tabs[tab]} className="p-3">
          {tab === "files" ? (
            <FilesChangedTab
              review={review}
              onSelectFile={(path) => {
                setFile(path);
                setTab("diff");
              }}
            />
          ) : tab === "diff" ? (
            <DiffTab review={review} source={diffSource} onSourceChange={setDiffSource} file={file} onClearFile={() => setFile(null)} />
          ) : tab === "validation" ? (
            <ValidationTab results={review.validation_results} />
          ) : tab === "comments" ? (
            <CommentsTab review={review} />
          ) : (
            <SecurityTab warnings={warnings} />
          )}
        </div>
      </div>

      <ConfirmDialog
        open={confirmOpen}
        onOpenChange={setConfirmOpen}
        title={m.approve.title}
        confirmLabel={m.approve.confirm}
        pending={approve.isPending}
        onConfirm={confirmApproval}
        description={formatMessage(m.approve.body, { title: review.title })}
      >
        <p className="font-semibold">{m.approve.noMerge}</p>
        {blockers.length > 0 ? (
          <MessageBar intent="warning" layout="multiline">
            <MessageBarBody>
              <MessageBarTitle>{formatMessage(m.approve.lowScore, { score: review.merge_readiness.score })}</MessageBarTitle>
              <ul className="list-disc ps-4">
                {blockers.map((b) => (
                  <li key={b}>{b}</li>
                ))}
              </ul>
            </MessageBarBody>
          </MessageBar>
        ) : null}
      </ConfirmDialog>
    </div>
  );
};

/** Detail pane for one pull request review (`/reviews/:reviewId`). */
export const ReviewDetail: React.FC<{
  id: string;
  /** List entry used while the detail request is in flight. */
  fallback: PullRequestReview | undefined;
  projects: ReadonlyMap<string, Project>;
}> = ({ id, fallback, projects }) => {
  const m = useMessages(reviewMessages);
  const navigate = useNavigate();
  const detail = usePullRequest(id);
  // The list entry only stands in while the detail request is in flight; a failed request shows
  // its error (with retry) instead of possibly stale list data.
  const review = detail.data ?? (detail.isError ? undefined : fallback);

  if (!review) {
    if (detail.isLoading) return <LoadingRows className="p-4" rows={8} />;
    const notFound = detail.error instanceof SymphonyApiError && detail.error.kind === "not_found";
    if (detail.isError && !notFound) {
      return (
        <div className="p-4">
          <ErrorState title={m.errors.detail} message={errorMessage(detail.error)} onRetry={() => void detail.refetch()} />
        </div>
      );
    }
    return (
      <div className="p-4">
        <AiEmptyView
          title={m.empty.notFoundTitle}
          description={m.empty.notFoundBody}
          icon={<GitPullRequest className="size-6 text-muted-foreground" aria-hidden />}
          action={
            <Button size="small" onClick={() => navigate(paths.reviews())}>
              {m.empty.backToList}
            </Button>
          }
        />
      </div>
    );
  }
  return <ReviewContent review={review} project={projects.get(review.project_id)} />;
};
