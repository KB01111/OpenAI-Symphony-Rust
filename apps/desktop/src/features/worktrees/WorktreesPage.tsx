import React, { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { FileDiff, FolderOpen, MonitorPlay, ShieldCheck } from "lucide-react";
import { formatMessage, type WorktreeInfo } from "@symphony/shared-types";
import {
  AiEmptyView,
  Badge,
  Button,
  DataTable,
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  ErrorState,
  Hint,
  Label,
  LoadingRows,
  PageHeader,
  ProviderBadge,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  StatusDot,
  Switch,
  TaskStatusBadge,
  TimeAgo,
  type ColumnDef
} from "@symphony/ui";
import { paths } from "../../app/routes.js";
import { useMessages, useT } from "../../lib/i18n.js";
import { useWorktrees } from "../../lib/queries.js";
import { TaskDiffView } from "../execution/ChangesPanel.js";
import { useWorkspaceOpener } from "../execution/useWorkspaceOpener.js";
import { worktreesMessages } from "./messages.js";

const ALL = "__all__";

/** Read-only state chips: missing worktree, uncommitted changes, live agent session. */
const WorktreeState: React.FC<{ worktree: WorktreeInfo }> = ({ worktree: w }) => {
  const m = useMessages(worktreesMessages);
  return (
    <span className="inline-flex items-center gap-1.5">
      {!w.exists ? (
        <Badge tone="danger">
          {m.state.missing}
        </Badge>
      ) : w.dirty ? (
        <Badge tone="warning">
          {m.state.dirty}
        </Badge>
      ) : (
        <Badge tone="outline">
          {m.state.clean}
        </Badge>
      )}
      {w.active_session_id ? <StatusDot tone="accent" pulse label={m.state.activeSession} /> : null}
    </span>
  );
};

const WorktreeActions: React.FC<{
  worktree: WorktreeInfo;
  explorer: boolean;
  onDiff: (w: WorktreeInfo) => void;
  onOpenExplorer: (w: WorktreeInfo) => void;
  onOpenExecution: (w: WorktreeInfo) => void;
}> = ({ worktree: w, explorer, onDiff, onOpenExplorer, onOpenExecution }) => {
  const m = useMessages(worktreesMessages);
  const button = (label: string, icon: React.ReactElement, onClick: () => void, disabled = false) => (
    <Hint label={label} asLabel>
      <Button size="sm" variant="subtle" disabled={disabled} onClick={onClick}>{icon}</Button>
    </Hint>
  );
  return (
    // Keep clicks/keys on the buttons from also activating (selecting) the table row.
    <div
      role="toolbar"
      aria-label={formatMessage(m.actions.label, { identifier: w.issue_identifier })}
      className="flex items-center justify-end gap-0.5"
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => e.stopPropagation()}
    >
      {button(m.actions.viewDiff, <FileDiff className="size-4" />, () => onDiff(w), !w.exists)}
      {explorer && w.exists ? button(m.actions.openExplorer, <FolderOpen className="size-4" />, () => onOpenExplorer(w)) : null}
      {button(m.actions.openExecution, <MonitorPlay className="size-4" />, () => onOpenExecution(w))}
    </div>
  );
};

/** Changed files + unified diff of one worktree in a Fluent dialog. */
const WorktreeDiffDialog: React.FC<{ worktree: WorktreeInfo | null; onClose: () => void; onOpenExecution: (w: WorktreeInfo) => void }> = ({
  worktree,
  onClose,
  onOpenExecution
}) => {
  const m = useMessages(worktreesMessages);
  const t = useT();
  return (
    <Dialog open={worktree !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="w-[min(60rem,calc(100vw-3rem))] max-w-none" closeLabel={t.common.close}>
        {worktree ? (
          <>
            <DialogHeader>
              <DialogTitle>{formatMessage(m.diffDialog.title, { identifier: worktree.issue_identifier })}</DialogTitle>
              <DialogDescription className="truncate font-mono text-xs" title={worktree.workspace_path}>
                {formatMessage(m.diffDialog.description, { branch: worktree.branch_name ?? m.noBranch, path: worktree.workspace_path })}
              </DialogDescription>
            </DialogHeader>
            <DialogBody>
              <div className="max-h-[60vh] overflow-auto">
                <TaskDiffView taskId={worktree.task_id} />
              </div>
            </DialogBody>
            <DialogFooter>
              <Button variant="secondary" onClick={() => onOpenExecution(worktree)}>
                <MonitorPlay />
                {m.actions.openExecution}
              </Button>
              <Button variant="default" onClick={onClose}>
                {t.common.close}
              </Button>
            </DialogFooter>
          </>
        ) : null}
      </DialogContent>
    </Dialog>
  );
};

/** Task worktrees (`/worktrees`): read-only inspection; the runtime owns creation and cleanup. */
export default function WorktreesPage() {
  const m = useMessages(worktreesMessages);
  const t = useT();
  const navigate = useNavigate();
  const worktrees = useWorktrees();
  const workspace = useWorkspaceOpener();
  const [projectId, setProjectId] = useState<string | null>(null);
  const [dirtyOnly, setDirtyOnly] = useState(false);
  const [diffTarget, setDiffTarget] = useState<WorktreeInfo | null>(null);

  const all = useMemo(() => worktrees.data ?? [], [worktrees.data]);
  const projects = useMemo(() => {
    const map = new Map<string, string>();
    for (const w of all) if (!map.has(w.project_id)) map.set(w.project_id, w.project_name ?? m.unknownProject);
    return [...map].map(([id, name]) => ({ id, name })).sort((a, b) => a.name.localeCompare(b.name));
  }, [all, m]);
  const visible = useMemo(
    () => all.filter((w) => (!projectId || w.project_id === projectId) && (!dirtyOnly || (w.exists && w.dirty))),
    [all, projectId, dirtyOnly]
  );
  const dirtyCount = useMemo(() => all.filter((w) => w.exists && w.dirty).length, [all]);
  const filtersActive = projectId !== null || dirtyOnly;

  const columns = useMemo<ColumnDef<WorktreeInfo, unknown>[]>(() => {
    const openExecution = (w: WorktreeInfo) => navigate(paths.run(w.task_id));
    return [
      {
        id: "task",
        header: m.columns.task,
        accessorFn: (w) => w.issue_identifier,
        cell: ({ row }) => (
          <span className="flex min-w-0 max-w-[22rem] items-center gap-2">
            <span className="shrink-0 font-mono text-xs text-muted-foreground">{row.original.issue_identifier}</span>
            <span className="truncate text-sm text-foreground" title={row.original.issue_title}>
              {row.original.issue_title}
            </span>
          </span>
        )
      },
      {
        id: "project",
        header: m.columns.project,
        accessorFn: (w) => w.project_name ?? "",
        cell: ({ row }) => <span className="truncate text-xs text-muted-foreground">{row.original.project_name ?? m.unknownProject}</span>
      },
      {
        id: "branch",
        header: m.columns.branch,
        accessorFn: (w) => w.branch_name ?? "",
        cell: ({ row }) => (
          <span className="block max-w-[14rem] truncate font-mono text-xs text-foreground" title={row.original.branch_name ?? undefined}>
            {row.original.branch_name ?? m.noBranch}
          </span>
        )
      },
      {
        id: "path",
        header: m.columns.path,
        accessorFn: (w) => w.workspace_path,
        cell: ({ row }) => (
          <Hint label={<span className="font-mono text-xs">{row.original.workspace_path}</span>}>
            <span className="block max-w-[16rem] truncate font-mono text-xs text-muted-foreground">{row.original.workspace_path}</span>
          </Hint>
        )
      },
      {
        id: "state",
        header: m.columns.state,
        accessorFn: (w) => (!w.exists ? 0 : w.dirty ? 1 : 2),
        cell: ({ row }) => <WorktreeState worktree={row.original} />
      },
      {
        id: "lastModified",
        header: m.columns.lastModified,
        accessorFn: (w) => w.last_activity_at,
        cell: ({ row }) => <TimeAgo iso={row.original.last_activity_at} className="text-xs text-muted-foreground" />
      },
      {
        id: "status",
        header: m.columns.status,
        accessorFn: (w) => w.task_status,
        cell: ({ row }) => (
          <span className="inline-flex items-center gap-1.5">
            <TaskStatusBadge status={row.original.task_status} />
            <ProviderBadge provider={row.original.provider} />
          </span>
        )
      },
      {
        id: "actions",
        header: () => <span className="sr-only">{m.columns.actions}</span>,
        enableSorting: false,
        cell: ({ row }) => (
          <WorktreeActions
            worktree={row.original}
            explorer={workspace.available}
            onDiff={setDiffTarget}
            onOpenExplorer={(w) => void workspace.open(w.task_id)}
            onOpenExecution={openExecution}
          />
        )
      }
    ];
  }, [m, navigate, workspace]);

  let body: React.ReactNode;
  if (worktrees.isError && !worktrees.data) {
    body = (
      <div className="p-4">
        <ErrorState title={m.loadFailed} message={worktrees.error.message} onRetry={() => void worktrees.refetch()} />
      </div>
    );
  } else if (worktrees.isLoading) {
    body = (
      <div className="p-4">
        <LoadingRows rows={6} label={t.common.loading} />
      </div>
    );
  } else if (all.length === 0) {
    body = (
      <div className="p-4">
        <AiEmptyView
          title={m.empty.title}
          description={m.empty.description}
          action={
            <Button size="sm" variant="default" onClick={() => navigate(paths.tasks())}>
              {m.empty.cta}
            </Button>
          }
        />
      </div>
    );
  } else if (visible.length === 0) {
    body = (
      <div className="p-4">
        <AiEmptyView
          title={m.empty.filtered}
          action={
            <Button
              size="sm"
              onClick={() => {
                setProjectId(null);
                setDirtyOnly(false);
              }}
            >
              {m.empty.clear}
            </Button>
          }
        />
      </div>
    );
  } else {
    body = (
      <DataTable<WorktreeInfo>
        data={visible}
        columns={columns}
        getRowId={(w) => w.task_id}
        onRowActivate={(w) => navigate(paths.run(w.task_id))}
        initialSorting={[{ id: "lastModified", desc: true }]}
        ariaLabel={m.tableLabel}
        className="h-full"
      />
    );
  }

  const activeProjectName = projects.find((p) => p.id === projectId)?.name;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader title={m.title} description={formatMessage(m.description, { total: all.length, dirty: dirtyCount })} />
      <div role="search" aria-label={m.filters} className="flex shrink-0 flex-wrap items-center gap-3 border-b border-border bg-surface px-4 py-2">
        <Select value={projectId ?? ALL} onValueChange={(v) => setProjectId(v === ALL ? null : v)}>
          <SelectTrigger className="w-48" aria-label={m.project}>
            <SelectValue>{activeProjectName ?? m.allProjects}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ALL}>{m.allProjects}</SelectItem>
            {projects.map((p) => (
              <SelectItem key={p.id} value={p.id}>
                {p.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <span className="inline-flex items-center gap-2">
          <Switch id="worktrees-dirty-only" checked={dirtyOnly} onCheckedChange={setDirtyOnly} />
          <Label htmlFor="worktrees-dirty-only" className="text-sm font-normal">
            {m.dirtyOnly}
          </Label>
        </span>
        {filtersActive ? (
          <span className="text-xs tabular-nums text-muted-foreground">{formatMessage(m.shown, { shown: visible.length, total: all.length })}</span>
        ) : null}
        <span className="ms-auto flex items-center gap-1.5 text-xs text-muted-foreground">
          <ShieldCheck className="size-3.5 text-success" aria-hidden />
          {m.protectedHint}
        </span>
      </div>
      <div className="min-h-0 flex-1 bg-background">{body}</div>
      <WorktreeDiffDialog
        worktree={diffTarget}
        onClose={() => setDiffTarget(null)}
        onOpenExecution={(w) => {
          setDiffTarget(null);
          navigate(paths.run(w.task_id));
        }}
      />
    </div>
  );
}
