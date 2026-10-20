/**
 * Task action controller shared by the board, table, Fluent action menus and shadcn context
 * menus. One provider owns the mutations and the cancel confirmation so every surface offers the
 * same state-machine-gated actions.
 */
import React, { createContext, useCallback, useContext, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import {
  Button,
  Menu,
  MenuDivider,
  MenuGroup,
  MenuGroupHeader,
  MenuItem,
  MenuList,
  MenuPopover,
  MenuTrigger,
  Tooltip
} from "@fluentui/react-components";
import {
  canCancel,
  canDispatch,
  canRetry,
  formatMessage,
  providerActions,
  type ProviderCapabilities,
  type ProviderKind,
  type Task,
  type TaskStatus
} from "@symphony/shared-types";
import {
  ConfirmDialog,
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
  useNotify
} from "@symphony/ui";
import { ArrowRightLeft, History, Info, MoreHorizontal, Play, RotateCcw, ScanSearch, XCircle } from "lucide-react";
import { paths } from "../../app/routes.js";
import { useMessages, useT } from "../../lib/i18n.js";
import {
  useCancelTask,
  useDispatchTask,
  useMoveTaskStatus,
  useProviders,
  useRetryTask,
  useRuntimeState
} from "../../lib/queries.js";
import { useUiStore } from "../../store/uiStore.js";
import { taskMessages } from "./messages.js";
import { activeTaskIds, allowedMoveTargets, applyDrop } from "./taskLogic.js";

export interface TaskActionsApi {
  activeIds: ReadonlySet<string>;
  /** Pending mutation for this task (prevents duplicate submissions). */
  isBusy(id: string): boolean;
  start(task: Task): void;
  retry(task: Task): void;
  requestCancel(task: Task): void;
  move(task: Task, status: TaskStatus): void;
  /** Drop handler for the board: validates client-side, warns, or moves. */
  drop(task: Task, status: TaskStatus | null): void;
  inspect(task: Task): void;
  openRun(task: Task): void;
  openHistory(task: Task): void;
  capabilities(provider: ProviderKind): ProviderCapabilities | undefined;
}

const TaskActionsContext = createContext<TaskActionsApi | null>(null);

export function useTaskActions(): TaskActionsApi {
  const ctx = useContext(TaskActionsContext);
  if (!ctx) throw new Error("useTaskActions must be used inside <TaskActionsProvider>");
  return ctx;
}

export const TaskActionsProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const m = useMessages(taskMessages);
  const t = useT();
  const notify = useNotify();
  const navigate = useNavigate();
  const inspect = useUiStore((s) => s.inspect);
  const state = useRuntimeState();
  const providers = useProviders();
  const dispatchTask = useDispatchTask();
  const retryTask = useRetryTask();
  const cancelTask = useCancelTask();
  const moveTask = useMoveTaskStatus();
  const [cancelTarget, setCancelTarget] = useState<Task | null>(null);

  const activeIds = useMemo(() => activeTaskIds(state.data), [state.data]);
  const capsByProvider = useMemo(
    () => new Map((providers.data ?? []).map((p) => [p.provider, p.capabilities] as const)),
    [providers.data]
  );

  const busy = useCallback(
    (id: string) =>
      (dispatchTask.isPending && dispatchTask.variables === id) ||
      (retryTask.isPending && retryTask.variables === id) ||
      (cancelTask.isPending && cancelTask.variables === id) ||
      (moveTask.isPending && moveTask.variables?.id === id),
    [dispatchTask.isPending, dispatchTask.variables, retryTask.isPending, retryTask.variables, cancelTask.isPending, cancelTask.variables, moveTask.isPending, moveTask.variables]
  );

  const api = useMemo<TaskActionsApi>(() => {
    const move = (task: Task, status: TaskStatus) =>
      moveTask.mutate(
        { id: task.id, status },
        { onSuccess: () => notify("success", formatMessage(m.actions.moved, { identifier: task.issue.identifier, status: t.status[status] })) }
      );
    return {
      activeIds,
      isBusy: busy,
      start: (task) =>
        dispatchTask.mutate(task.id, {
          onSuccess: () => notify("success", formatMessage(m.actions.started, { identifier: task.issue.identifier }))
        }),
      retry: (task) =>
        retryTask.mutate(task.id, {
          onSuccess: () => notify("success", formatMessage(m.actions.retried, { identifier: task.issue.identifier }))
        }),
      requestCancel: (task) => setCancelTarget(task),
      move,
      drop: (task, status) =>
        void applyDrop(task, status, activeIds, t, m.actions.moveRejected, {
          move: (_id, to) => move(task, to),
          warn: (title, body) => notify("warning", title, body)
        }),
      inspect: (task) => inspect({ kind: "task", id: task.id }),
      openRun: (task) => navigate(paths.run(task.id)),
      openHistory: (task) => navigate(`${paths.run(task.id)}?tab=history`),
      capabilities: (provider) => capsByProvider.get(provider)
    };
  }, [activeIds, busy, capsByProvider, dispatchTask, inspect, m, moveTask, navigate, notify, retryTask, t]);

  const confirmCancel = () => {
    if (!cancelTarget) return;
    const target = cancelTarget;
    cancelTask.mutate(target.id, {
      onSuccess: () => notify("success", formatMessage(m.actions.cancelled, { identifier: target.issue.identifier })),
      onSettled: () => setCancelTarget(null)
    });
  };

  return (
    <TaskActionsContext.Provider value={api}>
      {children}
      <ConfirmDialog
        open={cancelTarget !== null}
        title={formatMessage(m.actions.confirmCancelTitle, { identifier: cancelTarget?.issue.identifier ?? "" })}
        description={m.actions.confirmCancelBody}
        confirmLabel={m.actions.confirmCancelAction}
        destructive
        pending={cancelTask.isPending}
        onConfirm={confirmCancel}
        onOpenChange={(open) => !open && setCancelTarget(null)}
      >
        {cancelTarget && activeIds.has(cancelTarget.id) ? <p className="text-warning">{m.actions.confirmCancelRunning}</p> : null}
      </ConfirmDialog>
    </TaskActionsContext.Provider>
  );
};

// ---------------------------------------------------------------------------------------------
// Action descriptors (shared by the Fluent menu and the shadcn context menu)
// ---------------------------------------------------------------------------------------------

interface ActionItem {
  key: string;
  label: string;
  icon: React.ReactNode;
  disabled: boolean;
  hint?: string;
  onSelect: () => void;
  destructive?: boolean;
}

interface TaskMenuModel {
  primary: ActionItem[];
  moveTargets: TaskStatus[];
  navigation: ActionItem[];
}

function useTaskMenuModel(task: Task): TaskMenuModel {
  const m = useMessages(taskMessages);
  const t = useT();
  const actions = useTaskActions();
  const busy = actions.isBusy(task.id);
  const active = actions.activeIds.has(task.id);
  const caps = actions.capabilities(task.provider);
  // Stopping a live session depends on the provider; non-running cancels are a pure status change.
  const cancelSupported = !active || !caps || providerActions(caps).cancel;
  const icon = "size-4";
  return {
    primary: [
      {
        key: "start",
        label: m.actions.start,
        icon: <Play className={icon} aria-hidden />,
        disabled: busy || !canDispatch(task.status),
        onSelect: () => actions.start(task)
      },
      {
        key: "retry",
        label: m.actions.retry,
        icon: <RotateCcw className={icon} aria-hidden />,
        disabled: busy || !canRetry(task.status),
        onSelect: () => actions.retry(task)
      },
      {
        key: "cancel",
        label: m.actions.cancel,
        icon: <XCircle className={icon} aria-hidden />,
        disabled: busy || !canCancel(task.status) || !cancelSupported,
        hint: cancelSupported ? undefined : formatMessage(m.actions.cancelUnsupported, { provider: t.provider[task.provider] }),
        onSelect: () => actions.requestCancel(task),
        destructive: true
      }
    ],
    moveTargets: busy ? [] : allowedMoveTargets(task.status, active),
    navigation: [
      {
        key: "inspect",
        label: m.actions.inspect,
        icon: <Info className={icon} aria-hidden />,
        disabled: false,
        onSelect: () => actions.inspect(task)
      },
      {
        key: "open",
        label: m.actions.openExecution,
        icon: <ScanSearch className={icon} aria-hidden />,
        disabled: false,
        onSelect: () => actions.openRun(task)
      },
      {
        key: "history",
        label: m.actions.viewHistory,
        icon: <History className={icon} aria-hidden />,
        disabled: false,
        onSelect: () => actions.openHistory(task)
      }
    ]
  };
}

/** Fluent "…" menu: actions + "Move to…" group listing only allowed targets (keyboard alternative to DnD). */
export const TaskActionMenu: React.FC<{ task: Task; className?: string }> = ({ task, className }) => {
  const m = useMessages(taskMessages);
  const t = useT();
  const actions = useTaskActions();
  const model = useTaskMenuModel(task);
  const label = formatMessage(m.card.actions, { identifier: task.issue.identifier });
  return (
    <Menu positioning="below-end">
      <MenuTrigger disableButtonEnhancement>
        <Tooltip content={label} relationship="label">
          <Button
            size="small"
            appearance="subtle"
            icon={<MoreHorizontal className="size-4" aria-hidden />}
            className={className}
            data-card-control=""
          />
        </Tooltip>
      </MenuTrigger>
      <MenuPopover>
        <MenuList>
          {model.primary.map((item) => (
            <MenuItem key={item.key} icon={item.icon as React.ReactElement} disabled={item.disabled} onClick={item.onSelect} title={item.hint}>
              {item.label}
            </MenuItem>
          ))}
          <MenuDivider />
          <MenuGroup>
            <MenuGroupHeader>
              <span className="inline-flex items-center gap-1.5">
                <ArrowRightLeft className="size-3.5" aria-hidden />
                {m.actions.moveTo}
              </span>
            </MenuGroupHeader>
            {model.moveTargets.length === 0 ? (
              <MenuItem disabled>{m.actions.noMoves}</MenuItem>
            ) : (
              model.moveTargets.map((status) => (
                <MenuItem key={status} onClick={() => actions.move(task, status)}>
                  {t.status[status]}
                </MenuItem>
              ))
            )}
          </MenuGroup>
          <MenuDivider />
          {model.navigation.map((item) => (
            <MenuItem key={item.key} icon={item.icon as React.ReactElement} onClick={item.onSelect}>
              {item.label}
            </MenuItem>
          ))}
        </MenuList>
      </MenuPopover>
    </Menu>
  );
};

const TaskContextMenuBody: React.FC<{ task: Task }> = ({ task }) => {
  const m = useMessages(taskMessages);
  const t = useT();
  const actions = useTaskActions();
  const model = useTaskMenuModel(task);
  return (
    <ContextMenuContent className="min-w-48">
      {model.primary.map((item) => (
        <ContextMenuItem
          key={item.key}
          disabled={item.disabled}
          variant={item.destructive ? "destructive" : "default"}
          onSelect={item.onSelect}
          title={item.hint}
        >
          {item.icon}
          {item.label}
        </ContextMenuItem>
      ))}
      <ContextMenuSeparator />
      <ContextMenuSub>
        <ContextMenuSubTrigger disabled={model.moveTargets.length === 0}>
          <ArrowRightLeft className="size-4" aria-hidden />
          <span className="ms-2">{m.actions.moveTo}</span>
        </ContextMenuSubTrigger>
        <ContextMenuSubContent>
          {model.moveTargets.map((status) => (
            <ContextMenuItem key={status} onSelect={() => actions.move(task, status)}>
              {t.status[status]}
            </ContextMenuItem>
          ))}
        </ContextMenuSubContent>
      </ContextMenuSub>
      <ContextMenuSeparator />
      {model.navigation.map((item) => (
        <ContextMenuItem key={item.key} onSelect={item.onSelect}>
          {item.icon}
          {item.label}
        </ContextMenuItem>
      ))}
    </ContextMenuContent>
  );
};

/** Right-click menu (shadcn ContextMenu) with the same actions as the Fluent menu. */
export const TaskContextMenu: React.FC<{ task: Task; children: React.ReactElement }> = ({ task, children }) => {
  const [open, setOpen] = useState(false);
  return (
    <ContextMenu onOpenChange={setOpen}>
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      {open ? <TaskContextMenuBody task={task} /> : null}
    </ContextMenu>
  );
};
