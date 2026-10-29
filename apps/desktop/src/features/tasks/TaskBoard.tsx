import React, { useCallback, useMemo, useState } from "react";
import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
  pointerWithin,
  rectIntersection,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
  type Announcements,
  type CollisionDetection,
  type DragEndEvent,
  type DragStartEvent,
  type KeyboardCoordinateGetter,
  type UniqueIdentifier
} from "@dnd-kit/core";
import {
  ALL_TASK_STATUSES,
  explainManualTransition,
  formatMessage,
  type Task,
  type TaskStatus
} from "@symphony/shared-types";
import { Button, cn } from "@symphony/ui";
import { useMessages, useT } from "../../lib/i18n.js";
import { taskMessages } from "./messages.js";
import { TaskActionMenu, TaskContextMenu, useTaskActions } from "./taskActions.js";
import { TaskCardBody } from "./TaskCard.js";
import { groupByStatus } from "./taskLogic.js";

const PAGE_SIZE = 60;
const STATUS_SET = new Set<string>(ALL_TASK_STATUSES);

/**
 * Parking states that are usually empty. Their columns are shown only while they hold a task or a card
 * is being dragged (so they remain drop targets); the card menu can always move a task there.
 */
const OCCASIONAL_STATUSES = new Set<TaskStatus>(["backlog", "blocked", "cancelled"]);

/**
 * Pointer: drop only on the column under the cursor (a gutter is no drop); keyboard drags have no
 * pointer coordinates, so they use rectangle overlap.
 */
const collisionDetection: CollisionDetection = (args) => (args.pointerCoordinates ? pointerWithin(args) : rectIntersection(args));

/** Arrow keys jump between columns (left/right) and nudge within a column (up/down). */
const columnKeyboardCoordinates: KeyboardCoordinateGetter = (event, { currentCoordinates, context }) => {
  const width = context.collisionRect?.width ?? 0;
  if (event.code === "ArrowDown" || event.code === "ArrowUp") {
    event.preventDefault();
    return { x: currentCoordinates.x, y: currentCoordinates.y + (event.code === "ArrowDown" ? 48 : -48) };
  }
  if (event.code !== "ArrowRight" && event.code !== "ArrowLeft") return undefined;
  event.preventDefault();
  const columns = [...context.droppableRects.entries()]
    .filter(([id]) => STATUS_SET.has(String(id)))
    .sort((a, b) => a[1].left - b[1].left);
  if (columns.length === 0) return undefined;
  const centerX = currentCoordinates.x + width / 2;
  let index = columns.findIndex(([, rect]) => centerX >= rect.left && centerX <= rect.left + rect.width);
  if (index < 0) index = context.over ? columns.findIndex(([id]) => id === context.over?.id) : 0;
  const next = columns[Math.min(columns.length - 1, Math.max(0, index + (event.code === "ArrowRight" ? 1 : -1)))];
  if (!next) return undefined;
  const rect = next[1];
  return { x: rect.left + (rect.width - width) / 2, y: currentCoordinates.y };
};

type DropTone = "neutral" | "valid" | "invalid";

const BoardCard = React.memo(function BoardCard({ task, dragging }: { task: Task; dragging: boolean }) {
  const m = useMessages(taskMessages);
  const actions = useTaskActions();
  const { attributes, listeners, setNodeRef, setActivatorNodeRef, isDragging } = useDraggable({
    id: task.id,
    data: { task }
  });

  /** Ignore events from the card's own controls and from portalled menus. */
  const fromCardSurface = (e: React.SyntheticEvent) =>
    e.currentTarget.contains(e.target as Node) && !(e.target as Element).closest?.("[data-card-control]");

  return (
    <TaskContextMenu task={task}>
      <li
        ref={setNodeRef}
        className={cn(
          "group relative rounded-md border bg-surface-raised p-2.5 transition-colors",
          "border-border hover:border-border-strong hover:bg-surface-hover",
          isDragging && "opacity-40"
        )}
        data-task-id={task.id}
      >
        <div
          ref={setActivatorNodeRef}
          {...attributes}
          aria-label={`${task.issue.identifier} · ${task.issue.title}`}
          title={m.card.openHint}
          className="cursor-grab rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-accent active:cursor-grabbing"
          onPointerDown={(e) => {
            if (fromCardSurface(e)) listeners?.onPointerDown?.(e);
          }}
          onKeyDown={(e) => {
            if (!fromCardSurface(e)) return;
            if (e.key === "Enter" && !isDragging && !dragging) {
              e.preventDefault();
              actions.openRun(task);
              return;
            }
            listeners?.onKeyDown?.(e);
          }}
          onClick={(e) => {
            if (fromCardSurface(e)) actions.inspect(task);
          }}
          onDoubleClick={(e) => {
            if (fromCardSurface(e)) actions.openRun(task);
          }}
        >
          <TaskCardBody task={task} />
        </div>
        <TaskActionMenu task={task} className="!absolute end-1.5 top-1.5 opacity-70 group-hover:opacity-100 focus-visible:opacity-100" />
      </li>
    </TaskContextMenu>
  );
});

const BoardColumn = React.memo(function BoardColumn({
  status,
  tasks,
  tone,
  dragging
}: {
  status: TaskStatus;
  tasks: Task[];
  tone: DropTone;
  dragging: boolean;
}) {
  const m = useMessages(taskMessages);
  const t = useT();
  const [limit, setLimit] = useState(PAGE_SIZE);
  const { setNodeRef, isOver } = useDroppable({ id: status });
  const visible = tasks.length > limit ? tasks.slice(0, limit) : tasks;
  const label = t.status[status];
  return (
    <section
      ref={setNodeRef}
      aria-label={label}
      data-status={status}
      data-drop={tone}
      className={cn(
        "flex shrink-0 flex-col rounded-md border bg-surface-sunken transition-[opacity,border-color,background-color]",
        // Empty columns stay slim (they still take drops) so more of the populated ones fit on screen.
        tasks.length === 0 ? "w-44" : "w-72",
        tone === "valid" && "border-accent bg-accent-subtle",
        tone === "valid" && isOver && "ring-2 ring-accent",
        tone === "invalid" && "border-border opacity-50",
        tone === "invalid" && isOver && "border-destructive",
        tone === "neutral" && "border-border"
      )}
    >
      <header className="flex h-9 shrink-0 items-center gap-2 border-b border-border px-2.5">
        <h2 className="min-w-0 flex-1 truncate text-xs font-semibold uppercase tracking-wide text-muted-foreground">{label}</h2>
        {tone === "invalid" && isOver ? <span className="text-xs text-destructive">{m.card.cannotDrop}</span> : null}
        <span
          className="inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-muted px-1 text-[10px] font-semibold tabular-nums text-muted-foreground"
          aria-label={formatMessage(m.column.count, { count: tasks.length, status: label })}
        >
          {tasks.length}
        </span>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto p-1.5">
        {tasks.length === 0 ? (
          <p className="flex h-16 items-center justify-center rounded-md border border-dashed border-border text-xs text-subtle-foreground">
            {dragging && tone === "valid" ? m.card.dropHere : m.card.empty}
          </p>
        ) : (
          <ul className="flex flex-col gap-1.5" aria-label={label}>
            {visible.map((task) => (
              <BoardCard key={task.id} task={task} dragging={dragging} />
            ))}
          </ul>
        )}
        {tasks.length > limit ? (
          <Button size="sm" variant="subtle" className="mt-1.5 w-full" onClick={() => setLimit((l) => l + PAGE_SIZE)}>
            {formatMessage(m.card.showMore, { count: Math.min(PAGE_SIZE, tasks.length - limit) })}
          </Button>
        ) : null}
      </div>
    </section>
  );
});

/** Kanban board over the task statuses with state-machine-validated drag and drop. */
export const TaskBoard: React.FC<{ tasks: ReadonlyArray<Task> }> = ({ tasks }) => {
  const m = useMessages(taskMessages);
  const t = useT();
  const actions = useTaskActions();
  const [activeId, setActiveId] = useState<UniqueIdentifier | null>(null);
  const grouped = useMemo(() => groupByStatus(tasks), [tasks]);
  const byId = useMemo(() => new Map(tasks.map((task) => [task.id, task])), [tasks]);
  const activeTask = activeId != null ? byId.get(String(activeId)) ?? null : null;

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: columnKeyboardCoordinates,
      keyboardCodes: { start: ["Space"], cancel: ["Escape"], end: ["Space", "Enter"] }
    })
  );

  const toneFor = useCallback(
    (status: TaskStatus): DropTone => {
      if (!activeTask) return "neutral";
      const reason = explainManualTransition(activeTask.status, status, actions.activeIds.has(activeTask.id));
      if (reason === null) return "valid";
      return reason === "same_status" ? "neutral" : "invalid";
    },
    [activeTask, actions.activeIds]
  );

  const announcements = useMemo<Announcements>(() => {
    const ident = (id: UniqueIdentifier) => byId.get(String(id))?.issue.identifier ?? String(id);
    const statusLabel = (id: UniqueIdentifier) => (STATUS_SET.has(String(id)) ? t.status[id as TaskStatus] : String(id));
    return {
      onDragStart: ({ active }) => formatMessage(m.dnd.pickedUp, { identifier: ident(active.id) }),
      onDragOver: ({ active, over }) => {
        if (!over) return undefined;
        const task = byId.get(String(active.id));
        const invalid =
          task && STATUS_SET.has(String(over.id))
            ? explainManualTransition(task.status, over.id as TaskStatus, actions.activeIds.has(task.id)) !== null
            : false;
        return formatMessage(invalid ? m.dnd.overInvalid : m.dnd.over, { identifier: ident(active.id), status: statusLabel(over.id) });
      },
      onDragEnd: ({ active, over }) =>
        over ? formatMessage(m.dnd.dropped, { identifier: ident(active.id), status: statusLabel(over.id) }) : formatMessage(m.dnd.cancelled, { identifier: ident(active.id) }),
      onDragCancel: ({ active }) => formatMessage(m.dnd.cancelled, { identifier: ident(active.id) })
    };
  }, [actions.activeIds, byId, m, t]);

  const onDragStart = (event: DragStartEvent) => setActiveId(event.active.id);
  const onDragEnd = (event: DragEndEvent) => {
    setActiveId(null);
    const task = byId.get(String(event.active.id));
    const overId = event.over?.id != null ? String(event.over.id) : null;
    if (!task) return;
    actions.drop(task, overId && STATUS_SET.has(overId) ? (overId as TaskStatus) : null);
  };

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={collisionDetection}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onDragCancel={() => setActiveId(null)}
      accessibility={{ announcements, screenReaderInstructions: { draggable: m.dnd.instructions } }}
    >
      <div className="flex h-full min-h-0 gap-2 overflow-x-auto p-3" role="group" aria-label={m.page.boardLabel}>
        {ALL_TASK_STATUSES.filter((status) => activeTask !== null || !OCCASIONAL_STATUSES.has(status) || grouped[status].length > 0).map((status) => (
          <BoardColumn
            key={status}
            status={status}
            tasks={grouped[status]}
            tone={toneFor(status)}
            dragging={activeTask !== null}
          />
        ))}
      </div>
      <DragOverlay dropAnimation={null}>
        {activeTask ? (
          <div className="w-72 cursor-grabbing rounded-md border border-accent bg-surface-raised p-2.5 shadow-lg">
            <TaskCardBody task={activeTask} />
          </div>
        ) : null}
      </DragOverlay>
    </DndContext>
  );
};
