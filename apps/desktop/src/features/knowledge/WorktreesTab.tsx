import React, { useMemo } from "react";
import { GitBranch, History, Network } from "lucide-react";
import { formatMessage } from "@symphony/shared-types";
import type { KnowledgeWorktree } from "@symphony/shared-types";
import {
  AiEmptyView,
  Badge,
  Button,
  DataTable,
  ErrorState,
  Hint,
  LoadingRows,
  ProviderBadge,
  StatusDot,
  TaskStatusBadge,
  TimeAgo,
  type ColumnDef
} from "@symphony/ui";
import { useNavigate } from "react-router-dom";
import { paths } from "../../app/routes.js";
import { useMessages } from "../../lib/i18n.js";
import { useKnowledgeOverview, useKnowledgeWorktrees, useStartKnowledgeIndex } from "../../lib/queries.js";
import { ShaChip } from "./parts.js";
import { shortSubject, worktreeLabel } from "./knowledgeLogic.js";
import { knowledgeMessages } from "./messages.js";
import { useKnowledgeSelection } from "./selection.js";

const State: React.FC<{ worktree: KnowledgeWorktree }> = ({ worktree: w }) => {
  const m = useMessages(knowledgeMessages);
  return (
    <span className="inline-flex flex-wrap items-center gap-1">
      {!w.exists ? (
        <Badge tone="danger">{m.worktrees.state.missing}</Badge>
      ) : w.dirty ? (
        <Badge tone="warning">{formatMessage(m.worktrees.state.dirty, { count: w.changed_files })}</Badge>
      ) : (
        <Badge tone="outline">{m.worktrees.state.clean}</Badge>
      )}
      {w.detached ? <Badge tone="neutral">{m.worktrees.state.detached}</Badge> : null}
      {w.locked ? <Badge tone="neutral">{m.worktrees.state.locked}</Badge> : null}
      {w.prunable ? <Badge tone="danger">{m.worktrees.state.prunable}</Badge> : null}
    </span>
  );
};

/** Every git worktree of the repository: its branch, HEAD, uncommitted work and the agent that owns it. */
export const WorktreesTab: React.FC = () => {
  const m = useMessages(knowledgeMessages);
  const selection = useKnowledgeSelection();
  const navigate = useNavigate();
  const worktrees = useKnowledgeWorktrees(selection.projectId);
  const overview = useKnowledgeOverview(selection.projectId);
  const index = useStartKnowledgeIndex(selection.projectId);
  const defaultBranch = overview.data?.default_branch ?? "main";
  const canIndex = overview.data?.engines.some((e) => e.id === "graphify" && e.available) ?? false;

  const columns = useMemo<ColumnDef<KnowledgeWorktree, unknown>[]>(
    () => [
      {
        id: "branch",
        header: m.worktrees.columns.branch,
        accessorFn: (w) => worktreeLabel(w),
        cell: ({ row }) => (
          <span className="flex min-w-0 max-w-[16rem] items-center gap-2">
            <GitBranch aria-hidden className="size-4 shrink-0 text-accent" />
            <span className="truncate font-mono text-xs text-foreground" title={row.original.path}>
              {worktreeLabel(row.original)}
            </span>
            <Badge tone={row.original.kind === "main" ? "accent" : row.original.kind === "symphony" ? "info" : "neutral"}>
              {m.worktrees.kinds[row.original.kind]}
            </Badge>
          </span>
        )
      },
      {
        id: "head",
        header: m.worktrees.columns.head,
        accessorFn: (w) => w.head_committed_at ?? "",
        cell: ({ row }) =>
          row.original.head_sha ? (
            <span className="flex min-w-0 max-w-[22rem] items-center gap-2">
              <button
                type="button"
                className="rounded outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
                onClick={(event) => {
                  event.stopPropagation();
                  selection.selectCommit(row.original.head_sha, "commits");
                }}
              >
                <ShaChip sha={row.original.head_sha} />
              </button>
              <span className="truncate text-sm text-foreground" title={row.original.head_subject ?? undefined}>
                {shortSubject(row.original.head_subject ?? "")}
              </span>
              <TimeAgo iso={row.original.head_committed_at} className="shrink-0 text-xs text-muted-foreground" />
            </span>
          ) : (
            <span className="text-xs text-muted-foreground">—</span>
          )
      },
      {
        id: "state",
        header: m.worktrees.columns.state,
        accessorFn: (w) => (!w.exists ? 0 : w.dirty ? 1 : 2),
        cell: ({ row }) => <State worktree={row.original} />
      },
      {
        id: "agent",
        header: m.worktrees.columns.agent,
        accessorFn: (w) => w.task?.issue_identifier ?? "",
        cell: ({ row }) =>
          row.original.task ? (
            <span className="inline-flex min-w-0 max-w-[18rem] items-center gap-1.5">
              {row.original.task.active_session_id ? <StatusDot tone="accent" pulse label={m.worktrees.agentActive} /> : null}
              <button
                type="button"
                className="min-w-0 truncate rounded text-start text-sm text-accent outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring/60"
                title={m.worktrees.openRun}
                onClick={(event) => {
                  event.stopPropagation();
                  navigate(paths.run(row.original.task!.task_id));
                }}
              >
                {row.original.task.issue_identifier} · {row.original.task.issue_title}
              </button>
              <TaskStatusBadge status={row.original.task.task_status} />
              <ProviderBadge provider={row.original.task.provider} />
            </span>
          ) : (
            <span className="text-xs text-muted-foreground">{m.worktrees.noAgent}</span>
          )
      },
      {
        id: "ahead",
        header: m.worktrees.columns.ahead,
        accessorFn: (w) => w.ahead_of_default ?? -1,
        cell: ({ row }) =>
          row.original.ahead_of_default ? (
            <Hint label={formatMessage(m.worktrees.ahead, { count: row.original.ahead_of_default, branch: defaultBranch })}>
              <span className="font-mono text-xs tabular-nums text-foreground">+{row.original.ahead_of_default}</span>
            </Hint>
          ) : (
            <span className="text-xs text-muted-foreground">0</span>
          ),
        meta: { className: "hidden @3xl:table-cell" }
      },
      {
        id: "actions",
        header: () => <span className="sr-only">{m.worktrees.columns.actions}</span>,
        enableSorting: false,
        cell: ({ row }) => (
          <div role="toolbar" className="flex items-center justify-end gap-0.5" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
            <Hint label={m.worktrees.showCommits} asLabel>
              <Button size="sm" variant="subtle" onClick={() => selection.selectWorktree(row.original.id, "commits")}>
                <History className="size-4" />
              </Button>
            </Hint>
            <Hint label={m.worktrees.indexThis} asLabel>
              <Button
                size="sm"
                variant="subtle"
                disabled={!canIndex || !row.original.head_sha || !row.original.exists || index.isPending}
                onClick={() => index.mutate(row.original.id, { onSuccess: () => selection.setTab("overview") })}
              >
                <Network className="size-4" />
              </Button>
            </Hint>
          </div>
        )
      }
    ],
    [m, selection, navigate, defaultBranch, canIndex, index]
  );

  if (worktrees.isPending) return <LoadingRows className="p-4" rows={5} />;
  if (worktrees.isError) {
    return (
      <div className="p-4">
        <ErrorState title={m.unavailable.title} message={worktrees.error.message} onRetry={() => void worktrees.refetch()} retryLabel={m.unavailable.retry} />
      </div>
    );
  }
  if (worktrees.data.length === 0) {
    return (
      <div className="p-4">
        <AiEmptyView title={m.worktrees.empty} />
      </div>
    );
  }
  return (
    <div className="@container h-full min-h-0 p-2">
      <DataTable<KnowledgeWorktree>
        data={worktrees.data}
        columns={columns}
        getRowId={(w) => w.id}
        selectedId={selection.worktreeId}
        onRowActivate={(w) => selection.selectWorktree(w.id, "commits")}
        initialSorting={[{ id: "head", desc: true }]}
        ariaLabel={m.worktrees.label}
        className="h-full"
      />
    </div>
  );
};
