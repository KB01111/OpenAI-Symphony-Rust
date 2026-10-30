import React, { useMemo, useState } from "react";
import { GitCommitHorizontal, PencilLine } from "lucide-react";
import { formatMessage } from "@symphony/shared-types";
import type { CommitSummary, WorkingTreeChange } from "@symphony/shared-types";
import {
  AiEmptyView,
  Badge,
  ErrorState,
  Input,
  Label,
  LoadingRows,
  Panel,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
  TimeAgo,
  VirtualList,
  cn,
  Button
} from "@symphony/ui";
import { useMessages } from "../../lib/i18n.js";
import { useKnowledgeCommits, useKnowledgeWorktrees } from "../../lib/queries.js";
import { CommitPanel } from "./CommitPanel.js";
import { ShaChip } from "./parts.js";
import { filterCommits, shortSubject, worktreeLabel } from "./knowledgeLogic.js";
import { knowledgeMessages } from "./messages.js";
import { useKnowledgeSelection } from "./selection.js";

const ALL = "__all__";
const PAGE = 100;
const MAX_CHIPS = 3;

const CommitRow: React.FC<{ commit: CommitSummary; selected: boolean; onSelect: () => void }> = ({ commit, selected, onSelect }) => {
  const m = useMessages(knowledgeMessages);
  const chips = commit.worktrees.slice(0, MAX_CHIPS);
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-current={selected ? "true" : undefined}
      className={cn(
        "flex w-full min-w-0 items-start gap-2.5 border-b border-border px-3 py-2 text-start outline-none transition-colors hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/60",
        selected && "bg-surface-selected"
      )}
    >
      <GitCommitHorizontal aria-hidden className={cn("mt-0.5 size-4 shrink-0", commit.unpushed ? "text-warning" : "text-accent")} />
      <span className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="flex min-w-0 items-center gap-2">
          <ShaChip sha={commit.sha} />
          <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">{shortSubject(commit.subject)}</span>
        </span>
        <span className="flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          <span className="truncate">{commit.author_name}</span>
          <span aria-hidden>·</span>
          <TimeAgo iso={commit.committed_at} />
          {commit.is_merge ? <Badge tone="info">{m.commits.badges.merge}</Badge> : null}
          {commit.unpushed ? <Badge tone="warning">{m.commits.badges.unpushed}</Badge> : null}
          {commit.refs.slice(0, 2).map((ref) => (
            <Badge key={ref} tone="accent">
              {ref}
            </Badge>
          ))}
          {chips.map((wt) => (
            <Badge key={wt.worktree_id} tone="neutral">
              {wt.branch ?? m.commits.badges.detached}
              {wt.is_head ? " ●" : ""}
            </Badge>
          ))}
          {commit.worktrees.length > MAX_CHIPS ? <span>+{commit.worktrees.length - MAX_CHIPS}</span> : null}
        </span>
      </span>
    </button>
  );
};

const WorkingTreeRow: React.FC<{ change: WorkingTreeChange }> = ({ change }) => {
  const m = useMessages(knowledgeMessages);
  return (
    <div className="flex items-start gap-2.5 border-b border-border bg-warning-subtle/40 px-3 py-2 text-sm">
      <PencilLine aria-hidden className="mt-0.5 size-4 shrink-0 text-warning" />
      <div className="min-w-0 flex-1">
        <p className="font-medium text-foreground">{formatMessage(m.commits.workingTree, { count: change.changed_files, branch: change.branch ?? m.commits.badges.head })}</p>
        <p className="truncate font-mono text-xs text-muted-foreground" title={change.files.join(", ")}>
          {change.files.slice(0, 4).join(", ")}
        </p>
      </div>
    </div>
  );
};

/** One timeline of every commit in every worktree, and the panel for the selected one. */
export const CommitsTab: React.FC = () => {
  const m = useMessages(knowledgeMessages);
  const selection = useKnowledgeSelection();
  const { projectId, sha, worktreeId } = selection;
  const [limit, setLimit] = useState(PAGE);
  const [text, setText] = useState("");
  const [onlyUnpushed, setOnlyUnpushed] = useState(false);
  const timeline = useKnowledgeCommits(projectId, limit);
  const worktrees = useKnowledgeWorktrees(projectId);

  const visible = useMemo(
    () => filterCommits(timeline.data?.commits ?? [], { text, worktreeId, onlyUnpushed }),
    [timeline.data, text, worktreeId, onlyUnpushed]
  );
  const workingTrees = useMemo(
    () => (timeline.data?.working_trees ?? []).filter((change) => !worktreeId || change.worktree_id === worktreeId),
    [timeline.data, worktreeId]
  );

  if (timeline.isPending) return <LoadingRows className="p-4" rows={8} />;
  if (timeline.isError) {
    return (
      <div className="p-4">
        <ErrorState title={m.unavailable.title} message={timeline.error.message} onRetry={() => void timeline.refetch()} retryLabel={m.unavailable.retry} />
      </div>
    );
  }

  return (
    <div className="@container flex h-full min-h-0 flex-col gap-2 p-2 @4xl:flex-row">
      <Panel className="min-h-72 min-w-0 flex-1 @4xl:max-w-[34rem]" bodyClassName="flex min-h-0 flex-col" aria-label={m.commits.listLabel}>
        <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border p-2">
          <Input
            value={text}
            onChange={(event) => setText(event.target.value)}
            placeholder={m.commits.filter}
            aria-label={m.commits.filterLabel}
            className="min-w-40 flex-1"
          />
          <Select value={worktreeId ?? ALL} onValueChange={(value) => selection.selectWorktree(value === ALL ? null : value)}>
            <SelectTrigger aria-label={m.commits.allWorktrees} className="w-44">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={ALL}>{m.commits.allWorktrees}</SelectItem>
              {(worktrees.data ?? []).map((wt) => (
                <SelectItem key={wt.id} value={wt.id}>
                  {worktreeLabel(wt, m.commits.badges.detached)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {timeline.data.has_remote_refs ? (
            <div className="flex items-center gap-1.5">
              <Switch id="kb-unpushed" checked={onlyUnpushed} onCheckedChange={setOnlyUnpushed} />
              <Label htmlFor="kb-unpushed" className="text-xs">
                {m.commits.onlyUnpushed}
              </Label>
            </div>
          ) : null}
        </div>
        <div className="flex min-h-0 flex-1 flex-col">
          {workingTrees.length > 0 ? (
            <div className="max-h-40 shrink-0 overflow-auto">
              {workingTrees.map((change) => (
                <WorkingTreeRow key={change.worktree_id} change={change} />
              ))}
            </div>
          ) : null}
          {visible.length === 0 ? (
            <p className="p-4 text-sm text-muted-foreground">{m.commits.empty}</p>
          ) : (
            // The list scrolls itself inside the space left over, which is what lets it render
            // only the rows in view.
            <VirtualList
              items={visible}
              getKey={(commit) => commit.sha}
              estimateSize={64}
              className="min-h-0 flex-1"
              ariaLabel={m.commits.listLabel}
              renderItem={(commit) => <CommitRow commit={commit} selected={commit.sha === sha} onSelect={() => selection.selectCommit(commit.sha)} />}
            />
          )}
        </div>
        <footer className="flex shrink-0 items-center gap-2 border-t border-border px-3 py-1.5 text-xs text-muted-foreground">
          <span className="flex-1">{formatMessage(m.commits.shown, { shown: visible.length, worktrees: timeline.data.worktrees_total })}</span>
          {timeline.data.truncated ? (
            <Button size="sm" variant="subtle" onClick={() => setLimit((current) => current + PAGE)}>
              {m.commits.loadMore}
            </Button>
          ) : null}
        </footer>
      </Panel>
      <Panel className="min-h-80 min-w-0 flex-[2]" bodyClassName="min-h-0 overflow-hidden">
        {sha ? (
          <CommitPanel key={sha} sha={sha} />
        ) : (
          <div className="p-4">
            <AiEmptyView icon={<GitCommitHorizontal className="size-5 text-accent" />} title={m.commits.selectPrompt} />
          </div>
        )}
      </Panel>
    </div>
  );
};
