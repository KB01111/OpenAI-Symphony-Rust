import React, { useCallback, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { GitPullRequest, SearchX } from "lucide-react";
import { formatMessage, type Project } from "@symphony/shared-types";
import { AiEmptyView, Badge, Button, ErrorState, LoadingRows, PageHeader, ResizableHandle, ResizablePanel, ResizablePanelGroup, useDefaultLayout } from "@symphony/ui";
import { errorMessage } from "@symphony/api-client";
import { paths } from "../../app/routes.js";
import { useMessages } from "../../lib/i18n.js";
import { useProjects, usePullRequests } from "../../lib/queries.js";
import { safeStateStorage } from "../../store/safeStorage.js";
import { useUiStore } from "../../store/uiStore.js";
import { reviewMessages } from "./messages.js";
import { DEFAULT_REVIEW_FILTERS, filterReviews, type ReviewFilters } from "./reviewHelpers.js";
import { ReviewFiltersBar, ReviewList } from "./ReviewList.js";
import { ReviewDetail } from "./ReviewDetail.js";

/** Pull Request Review Center (`/reviews`, `/reviews/:reviewId`). */
export default function ReviewsPage() {
  const m = useMessages(reviewMessages);
  const { reviewId } = useParams<{ reviewId?: string }>();
  const navigate = useNavigate();
  const activeProjectId = useUiStore((s) => s.activeProjectId);
  const prs = usePullRequests(activeProjectId);
  const projects = useProjects();
  const [filters, setFilters] = useState<ReviewFilters>(DEFAULT_REVIEW_FILTERS);
  const layout = useDefaultLayout({ id: "symphony-reviews-h", storage: safeStateStorage, onlySaveAfterUserInteractions: true });

  const projectById = useMemo(() => new Map<string, Project>((projects.data ?? []).map((p) => [p.id, p])), [projects.data]);
  const projectNames = useMemo(() => new Map((projects.data ?? []).map((p) => [p.id, p.name])), [projects.data]);
  const all = prs.data;
  const visible = useMemo(() => filterReviews(all ?? [], filters, projectNames), [all, filters, projectNames]);
  const selectedFromList = useMemo(() => all?.find((r) => r.id === reviewId), [all, reviewId]);

  const select = useCallback(
    (id: string) => {
      navigate(paths.review(id));
    },
    [navigate]
  );

  const header = (
    <PageHeader
      title={m.title}
      description={m.description}
      meta={
        all ? (
          <Badge tone="neutral">
            {formatMessage(m.count, { count: all.length })}
          </Badge>
        ) : null
      }
    />
  );

  let body: React.ReactNode;
  if (prs.isLoading) {
    body = <LoadingRows className="p-4" rows={8} />;
  } else if (prs.isError) {
    body = (
      <div className="p-4">
        <ErrorState title={m.errors.list} message={errorMessage(prs.error)} onRetry={() => void prs.refetch()} />
      </div>
    );
  } else if ((all?.length ?? 0) === 0 && !reviewId) {
    body = (
      <div className="p-4">
        <AiEmptyView
          title={m.empty.noReviewsTitle}
          description={m.empty.noReviewsBody}
          icon={<GitPullRequest className="size-6 text-accent" aria-hidden />}
          action={
            <Button size="sm" onClick={() => navigate(paths.runs())}>
              {m.empty.openRuns}
            </Button>
          }
        />
      </div>
    );
  } else {
    body = (
      <ResizablePanelGroup orientation="horizontal" id="symphony-reviews-h" {...layout}>
        <ResizablePanel id="reviews-list" defaultSize="34" minSize={260} maxSize="60">
          <div className="flex h-full min-h-0 flex-col border-e border-border bg-surface">
            <ReviewFiltersBar filters={filters} onChange={setFilters} />
            {visible.length === 0 ? (
              <div className="p-3">
                <AiEmptyView
                  title={m.empty.noMatchesTitle}
                  description={m.empty.noMatchesBody}
                  icon={<SearchX className="size-6 text-muted-foreground" aria-hidden />}
                  action={
                    <Button size="sm" onClick={() => setFilters(DEFAULT_REVIEW_FILTERS)}>
                      {m.filters.clear}
                    </Button>
                  }
                />
              </div>
            ) : (
              <ReviewList reviews={visible} projects={projectById} selectedId={reviewId} onSelect={select} />
            )}
          </div>
        </ResizablePanel>
        <ResizableHandle />
        <ResizablePanel id="review-detail" minSize="35">
          <div className="h-full min-h-0 overflow-auto bg-background">
            {reviewId ? (
              <ReviewDetail key={reviewId} id={reviewId} fallback={selectedFromList} projects={projectById} />
            ) : (
              <div className="p-4">
                <AiEmptyView
                  title={m.empty.selectTitle}
                  description={m.empty.selectBody}
                  icon={<GitPullRequest className="size-6 text-muted-foreground" aria-hidden />}
                />
              </div>
            )}
          </div>
        </ResizablePanel>
      </ResizablePanelGroup>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      {header}
      <div className="min-h-0 flex-1">{body}</div>
    </div>
  );
}
