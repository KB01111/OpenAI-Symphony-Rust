import React, { useMemo, useRef, useState } from "react";
import {
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  useReactTable,
  type ColumnDef,
  type Row,
  type SortingState
} from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, ArrowUp, ArrowUpDown } from "lucide-react";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "../primitives/table.js";
import { cn } from "../lib/utils.js";

/** Clicks/keys on a button, link or input inside a cell belong to that control, not the row. */
function fromInteractiveChild(e: React.SyntheticEvent<HTMLElement>): boolean {
  const target = e.target as HTMLElement | null;
  if (!target || target === e.currentTarget) return false;
  const interactive = target.closest("button, a, input, select, textarea, [role='button'], [role='menuitem'], [role='checkbox']");
  return interactive !== null && interactive !== e.currentTarget && e.currentTarget.contains(interactive);
}

export type { ColumnDef, SortingState } from "@tanstack/react-table";

export interface DataTableProps<T> {
  data: ReadonlyArray<T>;
  columns: ColumnDef<T, unknown>[];
  getRowId: (row: T) => string;
  selectedId?: string | null;
  onRowActivate?: (row: T) => void;
  /** Wrap each row (e.g. in a shadcn ContextMenu trigger). */
  rowWrapper?: (row: T, element: React.ReactElement) => React.ReactElement;
  initialSorting?: SortingState;
  ariaLabel: string;
  className?: string;
  /** Rows above which rendering is virtualized. */
  virtualizeAbove?: number;
  rowHeight?: number;
}

/**
 * shadcn Table + TanStack Table with sortable headers, keyboard row activation (Enter/Space) and
 * windowed rendering for large datasets.
 */
export function DataTable<T>({
  data,
  columns,
  getRowId,
  selectedId,
  onRowActivate,
  rowWrapper,
  initialSorting = [],
  ariaLabel,
  className,
  virtualizeAbove = 150,
  rowHeight = 32
}: DataTableProps<T>) {
  const [sorting, setSorting] = useState<SortingState>(initialSorting);
  const table = useReactTable({
    data: data as T[],
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getRowId: (row) => getRowId(row),
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel()
  });
  const rows = table.getRowModel().rows;
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtual = rows.length > virtualizeAbove;
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    overscan: 12,
    enabled: virtual
  });
  const items = virtual ? virtualizer.getVirtualItems() : null;
  const padTop = items && items.length > 0 ? items[0]!.start : 0;
  const padBottom = items && items.length > 0 ? virtualizer.getTotalSize() - items[items.length - 1]!.end : 0;
  const visibleRows: Row<T>[] = useMemo(
    () => (items ? items.map((i) => rows[i.index]!).filter(Boolean) : rows),
    [items, rows]
  );

  const renderRow = (row: Row<T>) => {
    const element = (
      <TableRow
        key={row.id}
        data-state={row.id === selectedId ? "selected" : undefined}
        tabIndex={onRowActivate ? 0 : undefined}
        aria-selected={row.id === selectedId}
        className={cn(onRowActivate && "cursor-pointer", "h-row")}
        onClick={(e) => {
          if (!fromInteractiveChild(e)) onRowActivate?.(row.original);
        }}
        onKeyDown={(e) => {
          if (onRowActivate && (e.key === "Enter" || e.key === " ") && !fromInteractiveChild(e)) {
            e.preventDefault();
            onRowActivate(row.original);
          }
        }}
      >
        {row.getVisibleCells().map((cell) => (
          <TableCell key={cell.id} className="py-1">
            {flexRender(cell.column.columnDef.cell, cell.getContext())}
          </TableCell>
        ))}
      </TableRow>
    );
    return rowWrapper ? <React.Fragment key={row.id}>{rowWrapper(row.original, element)}</React.Fragment> : element;
  };

  return (
    <div ref={scrollRef} className={cn("relative min-h-0 overflow-auto", className)}>
      <Table aria-label={ariaLabel}>
        <TableHeader className="sticky top-0 z-10 bg-surface">
          {table.getHeaderGroups().map((group) => (
            <TableRow key={group.id}>
              {group.headers.map((header) => {
                const sorted = header.column.getIsSorted();
                const canSort = header.column.getCanSort();
                return (
                  <TableHead
                    key={header.id}
                    aria-sort={sorted === "asc" ? "ascending" : sorted === "desc" ? "descending" : undefined}
                    className="h-8 text-xs"
                  >
                    {header.isPlaceholder ? null : canSort ? (
                      <button
                        type="button"
                        className="inline-flex items-center gap-1 hover:text-foreground"
                        onClick={header.column.getToggleSortingHandler()}
                      >
                        {flexRender(header.column.columnDef.header, header.getContext())}
                        {sorted === "asc" ? (
                          <ArrowUp className="size-3" aria-hidden />
                        ) : sorted === "desc" ? (
                          <ArrowDown className="size-3" aria-hidden />
                        ) : (
                          <ArrowUpDown className="size-3 opacity-40" aria-hidden />
                        )}
                      </button>
                    ) : (
                      flexRender(header.column.columnDef.header, header.getContext())
                    )}
                  </TableHead>
                );
              })}
            </TableRow>
          ))}
        </TableHeader>
        <TableBody>
          {padTop > 0 ? (
            <tr aria-hidden>
              <td style={{ height: padTop }} />
            </tr>
          ) : null}
          {visibleRows.map(renderRow)}
          {padBottom > 0 ? (
            <tr aria-hidden>
              <td style={{ height: padBottom }} />
            </tr>
          ) : null}
        </TableBody>
      </Table>
    </div>
  );
}
