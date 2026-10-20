import React, { useMemo } from "react";
import type { Project, Task } from "@symphony/shared-types";
import { DataTable, ProviderBadge, TaskStatusBadge, TimeAgo, formatTokens, type ColumnDef } from "@symphony/ui";
import { useLocale, useMessages } from "../../lib/i18n.js";
import { useUiStore } from "../../store/uiStore.js";
import { taskMessages } from "./messages.js";
import { TaskActionMenu, TaskContextMenu, useTaskActions } from "./taskActions.js";
import { PriorityChip } from "./TaskCard.js";
import { normalizePriority, sortForBoard, statusOrder } from "./taskLogic.js";

/** Sortable, virtualized task table (TanStack via `DataTable`). */
export const TaskTable: React.FC<{ tasks: ReadonlyArray<Task>; projects: ReadonlyArray<Project> }> = ({ tasks, projects }) => {
  const m = useMessages(taskMessages);
  const locale = useLocale();
  const actions = useTaskActions();
  const inspector = useUiStore((s) => s.inspector);
  const selectedId = inspector?.kind === "task" ? inspector.id : null;
  const projectNames = useMemo(() => new Map(projects.map((p) => [p.id, p.name])), [projects]);
  const rows = useMemo(() => sortForBoard(tasks), [tasks]);

  const columns = useMemo<ColumnDef<Task, unknown>[]>(
    () => [
      {
        id: "identifier",
        header: m.table.identifier,
        accessorFn: (t) => t.issue.identifier,
        cell: ({ row }) => <span className="font-mono text-xs text-muted-foreground">{row.original.issue.identifier}</span>
      },
      {
        id: "title",
        header: m.table.title,
        accessorFn: (t) => t.issue.title,
        cell: ({ row }) => <span className="line-clamp-1 max-w-[28rem] text-sm text-foreground">{row.original.issue.title}</span>
      },
      {
        id: "status",
        header: m.table.status,
        accessorFn: (t) => statusOrder(t.status),
        cell: ({ row }) => <TaskStatusBadge status={row.original.status} />
      },
      {
        id: "priority",
        header: m.table.priority,
        accessorFn: (t) => normalizePriority(t.issue.priority) ?? 99,
        cell: ({ row }) => <PriorityChip priority={row.original.issue.priority} />
      },
      {
        id: "provider",
        header: m.table.provider,
        accessorFn: (t) => `${t.provider}:${t.model ?? ""}`,
        cell: ({ row }) => <ProviderBadge provider={row.original.provider} model={row.original.model} />
      },
      {
        id: "project",
        header: m.table.project,
        accessorFn: (t) => projectNames.get(t.project_id) ?? "",
        cell: ({ getValue }) => <span className="truncate text-xs text-muted-foreground">{String(getValue() || "—")}</span>
      },
      {
        id: "attempts",
        header: m.table.attempts,
        accessorFn: (t) => t.attempt_count,
        cell: ({ row }) => <span className="tabular-nums">{row.original.attempt_count}</span>
      },
      {
        id: "tokens",
        header: m.table.tokens,
        accessorFn: (t) => t.total_tokens,
        cell: ({ row }) => <span className="tabular-nums">{formatTokens(row.original.total_tokens, locale)}</span>
      },
      {
        id: "updated",
        header: m.table.updated,
        accessorFn: (t) => t.updated_at,
        cell: ({ row }) => <TimeAgo iso={row.original.updated_at} className="text-xs text-muted-foreground" />
      },
      {
        id: "actions",
        header: () => <span className="sr-only">{m.table.actions}</span>,
        enableSorting: false,
        cell: ({ row }) => <TaskActionMenu task={row.original} />
      }
    ],
    [locale, m, projectNames]
  );

  return (
    <DataTable<Task>
      data={rows}
      columns={columns}
      getRowId={(t) => t.id}
      selectedId={selectedId}
      onRowActivate={actions.inspect}
      rowWrapper={(task, element) => <TaskContextMenu task={task}>{element}</TaskContextMenu>}
      ariaLabel={m.page.tableLabel}
      className="h-full"
    />
  );
};
