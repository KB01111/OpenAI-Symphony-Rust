import React, { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import {
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Dropdown,
  Option,
  Switch,
  Tooltip
} from "@fluentui/react-components";
import { FileDiff, FolderOpen, MonitorPlay, ShieldCheck } from "lucide-react";
import { formatMessage, type WorktreeInfo } from "@symphony/shared-types";
import {
  AiEmptyView,
  DataTable,
  ErrorState,
  LoadingRows,
  PageHeader,
  ProviderBadge,
  SdkBadge,
  StatusDot,
  TaskStatusBadge,
  TimeAgo,
  type ColumnDef
} from "@symphony/ui";
import { paths } from "../../app/routes.js";
import { useMessages, useT } from "../../lib/i18n.js";
import { useWorktrees } from "../../lib/queries.js";
import { useUiStore } from "../../store/uiStore.js";
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
        <SdkBadge variant="soft" color="danger" size="sm" pill>
          {m.state.missing}
        </SdkBadge>
      ) : w.dirty ? (
        <SdkBadge variant="soft" color="warning" size="sm" pill>
          {m.state.dirty}
        </SdkBadge>
      ) : (
        <SdkBadge variant="outline" color="secondary" size="sm" pill>
          {m.state.clean}
        </SdkBadge>
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
    <Tooltip content={label} relationship="label">
      <Button size="small" appearance="subtle" icon={icon} disabled={disabled} onClick={onClick} />
    </Tooltip>
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
    <Dialog open={worktree !== null} onOpenChange={(_, d) => !d.open && onClose()}>
      <DialogSurface className="!w-[min(960px,calc(100vw-48px))] !max-w-none">
        {worktree ? (
          <DialogBody>
            <DialogTitle>{formatMessage(m.diffDialog.title, { identifier: worktree.issue_identifier })}</DialogTitle>
            <DialogContent>
              <p className="mb-2 truncate font-mono text-xs text-muted-foreground" title={worktree.workspace_path}>
                {formatMessage(m.diffDialog.description, { branch: worktree.branch_name ?? m.noBranch, path: worktree.workspace_path })}
              </p>
              <div className="max-h-[60vh] overflow-auto">
                <TaskDiffView taskId={worktree.task_id} />
              </div>
            </DialogContent>
            <DialogActions>
              <Button appearance="secondary" icon={<MonitorPlay className="size-4" />} onClick={() => onOpenExecution(worktree)}>
                {m.actions.openExecution}
              </Button>
              <Button appearance="primary" onClick={onClose}>
                {t.common.close}
              </Button>
            </DialogActions>
          </DialogBody>
        ) : null}
      </DialogSurface>
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
  const inspector = useUiStore((s) => s.inspector);
  const inspect = useUiStore((s) => s.inspect);
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
  const selectedId = inspector?.kind === "worktree" ? inspector.id : null;
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
          <Tooltip content={<span className="font-mono text-xs">{row.original.workspace_path}</span>} relationship="description">
            <span className="block max-w-[16rem] truncate font-mono text-xs text-muted-foreground">{row.original.workspace_path}</span>
          </Tooltip>
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
            <Button size="small" appearance="primary" onClick={() => navigate(paths.tasks())}>
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
              size="small"
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
        selectedId={selectedId}
        onRowActivate={(w) => inspect({ kind: "worktree", id: w.task_id })}
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
        <Dropdown
          size="small"
          className="!min-w-0 w-48"
          aria-label={m.project}
          value={activeProjectName ?? m.allProjects}
          selectedOptions={[projectId ?? ALL]}
          onOptionSelect={(_, d) => setProjectId(d.optionValue && d.optionValue !== ALL ? d.optionValue : null)}
        >
          <Option value={ALL}>{m.allProjects}</Option>
          {projects.map((p) => (
            <Option key={p.id} value={p.id} text={p.name}>
              {p.name}
            </Option>
          ))}
        </Dropdown>
        <Switch checked={dirtyOnly} onChange={(_, d) => setDirtyOnly(d.checked)} label={m.dirtyOnly} />
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
