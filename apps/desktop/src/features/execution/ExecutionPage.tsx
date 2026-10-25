import React, { useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { Button } from "@fluentui/react-components";
import { isSymphonyApiError } from "@symphony/api-client";
import { ArrowLeft, FolderOpen, MessageSquarePlus, Play, RotateCcw, Square } from "lucide-react";
import { canCancel, canDispatch, canRetry, formatMessage, providerActions, type Task } from "@symphony/shared-types";
import {
  ConfirmDialog,
  Duration,
  ErrorState,
  LoadingRows,
  PageHeader,
  PhaseBadge,
  ProviderBadge,
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
  Skeleton,
  TaskStatusBadge,
  useDefaultLayout
} from "@symphony/ui";
import { paths } from "../../app/routes.js";
import { useMessages, useT } from "../../lib/i18n.js";
import { useCancelTask, useDispatchTask, useProjects, useProviders, useRetryTask, useRuntimeState, useTask } from "../../lib/queries.js";
import { safeStateStorage } from "../../store/safeStorage.js";
import { useUiStore } from "../../store/uiStore.js";
import { ChangesPanel } from "./ChangesPanel.js";
import { ContinueDialog } from "./ContinueDialog.js";
import { ContextPanel } from "./ContextPanel.js";
import { executionMessages } from "./messages.js";
import { TimelinePanel } from "./TimelinePanel.js";
import { useWorkspaceOpener } from "./useWorkspaceOpener.js";

const PanelFrame: React.FC<{ title: string; children: React.ReactNode }> = ({ title, children }) => (
  <section aria-label={title} className="flex h-full min-h-0 flex-col bg-surface">
    <h2 className="flex h-8 shrink-0 items-center border-b border-border px-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
      {title}
    </h2>
    <div className="min-h-0 flex-1">{children}</div>
  </section>
);

/** Status- and capability-gated run controls. The runtime re-validates every request. */
const ExecutionControls: React.FC<{ task: Task }> = ({ task }) => {
  const m = useMessages(executionMessages);
  const providers = useProviders();
  const dispatch = useDispatchTask();
  const retry = useRetryTask();
  const cancel = useCancelTask();
  const workspace = useWorkspaceOpener();
  // Dialogs belong to the task they were opened for: navigating to another run closes them, so
  // neither a cancellation nor typed instructions can reach a different task.
  const [dialog, setDialog] = useState<{ taskId: string; kind: "cancel" | "continue" } | null>(null);
  const confirmCancel = dialog?.taskId === task.id && dialog.kind === "cancel";
  const continueOpen = dialog?.taskId === task.id && dialog.kind === "continue";
  const setConfirmCancel = (open: boolean) => setDialog(open ? { taskId: task.id, kind: "cancel" } : null);
  const setContinueOpen = (open: boolean) => setDialog(open ? { taskId: task.id, kind: "continue" } : null);
  // A run that ended can be continued with instructions (its thread and worktree are kept).
  const continuable = task.status === "human_review" || task.status === "failed" || task.status === "completed";
  const capabilities = providers.data?.find((p) => p.provider === task.provider)?.capabilities;
  const actions = providerActions(capabilities);
  const busy = dispatch.isPending || retry.isPending || cancel.isPending;

  return (
    <div role="toolbar" aria-label={m.actions.label} className="flex items-center gap-1.5">
      {canDispatch(task.status) ? (
        <Button size="small" appearance="primary" icon={<Play className="size-4" />} disabled={busy} onClick={() => dispatch.mutate(task.id)}>
          {m.actions.start}
        </Button>
      ) : null}
      {canRetry(task.status) ? (
        <Button size="small" icon={<RotateCcw className="size-4" />} disabled={busy} onClick={() => retry.mutate(task.id)}>
          {m.actions.retry}
        </Button>
      ) : null}
      {continuable ? (
        <Button size="small" icon={<MessageSquarePlus className="size-4" />} disabled={busy} onClick={() => setContinueOpen(true)}>
          {m.actions.continue}
        </Button>
      ) : null}
      {canCancel(task.status) && actions.cancel ? (
        <Button size="small" icon={<Square className="size-4" />} disabled={busy} onClick={() => setConfirmCancel(true)}>
          {m.actions.cancel}
        </Button>
      ) : null}
      {workspace.available && task.workspace_path ? (
        <Button size="small" appearance="subtle" icon={<FolderOpen className="size-4" />} onClick={() => void workspace.open(task.id)}>
          {m.actions.openWorkspace}
        </Button>
      ) : null}
      {continuable ? <ContinueDialog key={task.id} task={task} open={continueOpen} onOpenChange={setContinueOpen} /> : null}
      <ConfirmDialog
        open={confirmCancel}
        title={formatMessage(m.cancelDialog.title, { identifier: task.issue.identifier })}
        description={m.cancelDialog.description}
        confirmLabel={m.cancelDialog.confirm}
        destructive
        pending={cancel.isPending}
        onOpenChange={setConfirmCancel}
        onConfirm={() => {
          if (cancel.isPending) return;
          cancel.mutate(task.id, { onSettled: () => setConfirmCancel(false) });
        }}
      />
    </div>
  );
};

const ExecutionWorkspace: React.FC<{ task: Task }> = ({ task }) => {
  const m = useMessages(executionMessages);
  const projects = useProjects();
  const providers = useProviders();
  const state = useRuntimeState();
  const layout = useDefaultLayout({ id: "symphony-execution", storage: safeStateStorage });
  const session = state.data?.running.find((r) => r.task_id === task.id);
  const project = projects.data?.find((p) => p.id === task.project_id);
  const providerStatus = providers.data?.find((p) => p.provider === task.provider);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        title={`${task.issue.identifier} · ${task.issue.title}`}
        description={formatMessage(m.pageDescription, {
          project: project?.name ?? m.unknownProject,
          branch: task.branch_name ?? m.noBranch
        })}
        meta={
          <span className="flex shrink-0 flex-wrap items-center gap-1.5">
            <TaskStatusBadge status={task.status} />
            {task.current_phase ? <PhaseBadge phase={task.current_phase} /> : null}
            <ProviderBadge provider={task.provider} model={session?.model ?? task.model} />
            <span className="text-xs text-muted-foreground" aria-label={session ? m.started : m.totalDuration}>
              {session ? <Duration since={session.started_at} /> : <Duration seconds={task.duration_seconds} />}
            </span>
          </span>
        }
        actions={<ExecutionControls task={task} />}
      />
      <div className="min-h-0 flex-1">
        <ResizablePanelGroup orientation="horizontal" id="symphony-execution" {...layout} aria-label={m.panels.layout}>
          <ResizablePanel id="context" defaultSize="24" minSize="15">
            <PanelFrame title={m.panels.context}>
              <ContextPanel task={task} project={project} providerStatus={providerStatus} />
            </PanelFrame>
          </ResizablePanel>
          <ResizableHandle />
          <ResizablePanel id="timeline" defaultSize="46" minSize="25">
            <PanelFrame title={m.panels.timeline}>
              <TimelinePanel task={task} operation={session ? (session.last_message || session.last_event) : null} />
            </PanelFrame>
          </ResizablePanel>
          <ResizableHandle />
          <ResizablePanel id="changes" defaultSize="30" minSize="18">
            <PanelFrame title={m.panels.changes}>
              <ChangesPanel task={task} liveSince={session?.started_at ?? null} />
            </PanelFrame>
          </ResizablePanel>
        </ResizablePanelGroup>
      </div>
    </div>
  );
};

/** Live Agent Execution Workspace (`/runs/:taskId`). */
export default function ExecutionPage() {
  const m = useMessages(executionMessages);
  const t = useT();
  const navigate = useNavigate();
  const { taskId } = useParams<{ taskId: string }>();
  const inspect = useUiStore((s) => s.inspect);
  const task = useTask(taskId);

  // Focus the shell inspector and activity drawer on this task.
  useEffect(() => {
    if (taskId) inspect({ kind: "task", id: taskId });
  }, [taskId, inspect]);

  const backButton = useMemo(
    () => (
      <Button size="small" icon={<ArrowLeft className="size-4" />} onClick={() => navigate(paths.runs())}>
        {m.notFound.back}
      </Button>
    ),
    [m, navigate]
  );

  if (!taskId || (task.isError && isSymphonyApiError(task.error) && task.error.kind === "not_found")) {
    return (
      <div className="flex h-full min-h-0 flex-col">
        <PageHeader title={m.notFound.title} />
        <div className="space-y-3 p-4">
          <ErrorState title={m.notFound.title} message={m.notFound.message} intent="warning" />
          {backButton}
        </div>
      </div>
    );
  }
  if (task.isError && !task.data) {
    return (
      <div className="flex h-full min-h-0 flex-col">
        <PageHeader title={m.loadFailed} />
        <div className="space-y-3 p-4">
          <ErrorState title={m.loadFailed} message={task.error.message} onRetry={() => void task.refetch()} />
          {backButton}
        </div>
      </div>
    );
  }
  if (!task.data) {
    return (
      <div className="flex h-full min-h-0 flex-col" aria-busy="true">
        <header className="flex shrink-0 items-center gap-3 border-b border-border bg-surface px-4 py-3">
          <Skeleton className="h-5 w-72" />
          <Skeleton className="h-5 w-24" />
        </header>
        <div className="grid min-h-0 flex-1 grid-cols-[1fr_2fr_1.3fr] gap-3 p-3">
          <LoadingRows rows={6} label={t.common.loading} />
          <LoadingRows rows={10} label={t.common.loading} />
          <LoadingRows rows={6} label={t.common.loading} />
        </div>
      </div>
    );
  }
  return <ExecutionWorkspace task={task.data} />;
}
