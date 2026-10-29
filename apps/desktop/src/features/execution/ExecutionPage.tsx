import React, { useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { isSymphonyApiError } from "@symphony/api-client";
import { ArrowLeft, FolderOpen, PanelRight, Play, RotateCcw, Square } from "lucide-react";
import { canCancel, canDispatch, canRetry, formatMessage, providerActions, type Task } from "@symphony/shared-types";
import {
  Button,
  ConfirmDialog,
  Duration,
  ErrorState,
  Hint,
  LoadingRows,
  Markdown,
  PageHeader,
  PhaseBadge,
  ProviderBadge,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  Skeleton,
  TaskStatusBadge
} from "@symphony/ui";
import { paths } from "../../app/routes.js";
import { useElementWidth } from "../../hooks/useElementWidth.js";
import { useMessages, useT } from "../../lib/i18n.js";
import { useCancelTask, useDispatchTask, useProjects, useProviders, useRetryTask, useRuntimeState, useTask } from "../../lib/queries.js";
import { workbenchMessages } from "../../layouts/messages.js";
import { useUiStore } from "../../store/uiStore.js";
import { FollowUpComposer } from "./FollowUpComposer.js";
import { executionMessages } from "./messages.js";
import { RunDetailsPanel } from "./RunDetailsPanel.js";
import { TimelinePanel } from "./TimelinePanel.js";
import { useWorkspaceOpener } from "./useWorkspaceOpener.js";

/** Status- and capability-gated run controls. The runtime re-validates every request. */
const ExecutionControls: React.FC<{ task: Task }> = ({ task }) => {
  const m = useMessages(executionMessages);
  const providers = useProviders();
  const dispatch = useDispatchTask();
  const retry = useRetryTask();
  const cancel = useCancelTask();
  const workspace = useWorkspaceOpener();
  // The dialog belongs to the task it was opened for: navigating to another thread closes it, so a
  // cancellation can never reach a different task.
  const [cancelFor, setCancelFor] = useState<string | null>(null);
  const confirmCancel = cancelFor === task.id;
  const capabilities = providers.data?.find((p) => p.provider === task.provider)?.capabilities;
  const actions = providerActions(capabilities);
  const busy = dispatch.isPending || retry.isPending || cancel.isPending;

  return (
    <div role="toolbar" aria-label={m.actions.label} className="flex items-center gap-1.5">
      {canDispatch(task.status) ? (
        <Button size="sm" variant="default" disabled={busy} onClick={() => dispatch.mutate(task.id)}>
          <Play />
          {m.actions.start}
        </Button>
      ) : null}
      {/* A task that has not run yet has nothing to retry: Start is the one way forward. */}
      {canRetry(task.status) && task.status !== "ready" ? (
        <Button size="sm" disabled={busy} onClick={() => retry.mutate(task.id)}>
          <RotateCcw />
          {m.actions.retry}
        </Button>
      ) : null}
      {canCancel(task.status) && actions.cancel ? (
        <Button size="sm" disabled={busy} onClick={() => setCancelFor(task.id)}>
          <Square />
          {m.actions.cancel}
        </Button>
      ) : null}
      {workspace.available && task.workspace_path ? (
        <Button size="sm" variant="subtle" onClick={() => void workspace.open(task.id)}>
          <FolderOpen />
          {m.actions.openWorkspace}
        </Button>
      ) : null}
      <ConfirmDialog
        open={confirmCancel}
        title={formatMessage(m.cancelDialog.title, { identifier: task.issue.identifier })}
        description={m.cancelDialog.description}
        confirmLabel={m.cancelDialog.confirm}
        destructive
        pending={cancel.isPending}
        onOpenChange={(open) => setCancelFor(open ? task.id : null)}
        onConfirm={() => {
          if (cancel.isPending) return;
          cancel.mutate(task.id, { onSettled: () => setCancelFor(null) });
        }}
      />
    </div>
  );
};

/** The task prompt as the first message of the thread. */
const PromptCard: React.FC<{ task: Task }> = ({ task }) => {
  const w = useMessages(workbenchMessages).thread;
  const m = useMessages(executionMessages);
  const text = task.issue.description?.trim() || task.issue.title;
  const [expanded, setExpanded] = useState(false);
  const long = text.length > 280 || text.split("\n").length > 5;
  return (
    <section aria-label={w.promptHeading} className="shrink-0 border-b border-border bg-surface-sunken px-4 py-3">
      <div className="mx-auto max-w-3xl">
        <p className="mb-1 text-xs font-medium text-muted-foreground">{w.promptBy}</p>
        <div
          tabIndex={expanded ? 0 : undefined}
          className={
            expanded
              ? "max-h-[40vh] overflow-y-auto text-sm"
              : long
                ? "max-h-24 overflow-hidden text-sm [mask-image:linear-gradient(to_bottom,black_60%,transparent)]"
                : "text-sm"
          }
        >
          <Markdown breakNewLines>{text}</Markdown>
        </div>
        {long ? (
          <Button variant="link" size="sm" className="mt-1 h-auto p-0 text-xs" aria-expanded={expanded} onClick={() => setExpanded((v) => !v)}>
            {expanded ? m.changes.hideOutput : m.changes.showOutput}
          </Button>
        ) : null}
      </div>
    </section>
  );
};

/** Width of the thread view at which the run-details panel can dock beside it (35rem + 26rem panel). */
const DOCK_MIN_WIDTH = 976;

const ThreadWorkspace: React.FC<{ task: Task }> = ({ task }) => {
  const m = useMessages(executionMessages);
  const t = useT();
  const w = useMessages(workbenchMessages).thread;
  const projects = useProjects();
  const providers = useProviders();
  const state = useRuntimeState();
  const panelOpen = useUiStore((s) => s.threadPanelOpen);
  const setPanelOpen = useUiStore((s) => s.setThreadPanelOpen);
  // The panel is docked only when the thread keeps at least ~35rem beside it (26rem panel); the view's
  // own width is used, not the window's, because the sidebar and the shell take their share first.
  const [frame, frameWidth] = useElementWidth<HTMLDivElement>();
  const wide = frameWidth >= DOCK_MIN_WIDTH;
  const [sheetOpen, setSheetOpen] = useState(false);
  const session = state.data?.running.find((r) => r.task_id === task.id);
  const project = projects.data?.find((p) => p.id === task.project_id);
  const providerStatus = providers.data?.find((p) => p.provider === task.provider);
  const inline = wide && panelOpen;

  // Ctrl+J shows or hides the run details: the docked panel when there is room, else the sheet.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      // Holding the keys auto-repeats keydown: toggle once, not on every repeat.
      if (e.repeat || !(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey || e.key.toLowerCase() !== "j") return;
      const target = e.target as HTMLElement | null;
      if (target && (target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName))) return;
      e.preventDefault();
      if (wide) setPanelOpen(!panelOpen);
      else setSheetOpen((open) => !open);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [wide, panelOpen, setPanelOpen]);

  const details = <RunDetailsPanel task={task} project={project} providerStatus={providerStatus} liveSince={session?.started_at ?? null} />;

  return (
    <div ref={frame} className="flex h-full min-h-0">
      <div className="flex min-w-0 flex-1 flex-col">
        <PageHeader
          title={`${task.issue.identifier} · ${task.issue.title}`}
          description={formatMessage(m.pageDescription, {
            project: project?.name ?? m.unknownProject,
            branch: task.branch_name ?? m.noBranch
          })}
          meta={
            <span className="flex min-w-0 max-w-full flex-wrap items-center gap-1.5">
              <TaskStatusBadge status={task.status} />
              {task.current_phase ? <PhaseBadge phase={task.current_phase} /> : null}
              <ProviderBadge provider={task.provider} model={session?.model ?? task.model} />
              <span className="text-xs text-muted-foreground" aria-label={session ? m.started : m.totalDuration}>
                {session ? <Duration since={session.started_at} /> : <Duration seconds={task.duration_seconds} />}
              </span>
            </span>
          }
          actions={
            <>
              <ExecutionControls task={task} />
              <Hint label={inline ? w.hidePanel : w.showPanel}>
                <Button
                  variant="subtle"
                  size="icon"
                  aria-label={inline ? w.hidePanel : w.showPanel}
                  aria-pressed={wide ? panelOpen : sheetOpen}
                  onClick={() => (wide ? setPanelOpen(!panelOpen) : setSheetOpen(true))}
                >
                  <PanelRight />
                </Button>
              </Hint>
            </>
          }
        />
        <PromptCard task={task} />
        <div className="min-h-0 flex-1" role="region" aria-label={m.panels.timeline}>
          <TimelinePanel task={task} operation={session ? (session.last_message || session.last_event) : null} />
        </div>
        <FollowUpComposer key={task.id} task={task} />
      </div>
      {inline ? (
        <aside aria-label={w.panel} className="flex w-[26rem] max-w-[40%] shrink-0 flex-col border-s border-border bg-surface">
          {details}
        </aside>
      ) : null}
      {!wide ? (
        <Sheet open={sheetOpen} onOpenChange={setSheetOpen}>
          <SheetContent side="right" closeLabel={t.common.close} className="w-[min(32rem,92vw)] gap-0 bg-surface p-0 sm:max-w-none">
            <SheetHeader className="border-b border-border">
              <SheetTitle>{w.panel}</SheetTitle>
              <SheetDescription className="sr-only">{task.issue.identifier}</SheetDescription>
            </SheetHeader>
            <div className="min-h-0 flex-1">{details}</div>
          </SheetContent>
        </Sheet>
      ) : null}
    </div>
  );
};

/** A task as a thread (`/runs/:taskId`): prompt, live timeline, follow-ups and run details. */
export default function ExecutionPage() {
  const m = useMessages(executionMessages);
  const t = useT();
  const navigate = useNavigate();
  const { taskId } = useParams<{ taskId: string }>();
  const task = useTask(taskId);

  const backButton = useMemo(
    () => (
      <Button size="sm" onClick={() => navigate(paths.runs())}>
        <ArrowLeft />
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
        <div className="min-h-0 flex-1 p-4">
          <LoadingRows rows={10} label={t.common.loading} />
        </div>
      </div>
    );
  }
  return <ThreadWorkspace task={task.data} />;
}
