import React, { useEffect, useState } from "react";
import { ALL_PROVIDERS, ALL_TASK_STATUSES, formatMessage, type Project, type ProviderKind, type TaskStatus } from "@symphony/shared-types";
import { Button, Input, MultiSelect, Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@symphony/ui";
import { FilterX, Search } from "lucide-react";
import { useMessages, useT } from "../../lib/i18n.js";
import { useUiStore } from "../../store/uiStore.js";
import { taskMessages } from "./messages.js";
import { NO_PRIORITY, PRIORITY_VALUES, hasActiveFilters } from "./taskLogic.js";

const ALL = "__all__";

/** Search + filters for the task views. Filters persist in the UI store. */
export const TaskToolbar: React.FC<{ projects: ReadonlyArray<Project>; shown: number; total: number }> = ({ projects, shown, total }) => {
  const m = useMessages(taskMessages);
  const t = useT();
  const filters = useUiStore((s) => s.taskFilters);
  const setTaskFilters = useUiStore((s) => s.setTaskFilters);
  const resetTaskFilters = useUiStore((s) => s.resetTaskFilters);
  const activeProjectId = useUiStore((s) => s.activeProjectId);
  const setActiveProject = useUiStore((s) => s.setActiveProject);

  // Local echo keeps typing responsive; the store (and filtering) updates after a short pause.
  const [search, setSearch] = useState(filters.search);
  useEffect(() => setSearch(filters.search), [filters.search]);
  useEffect(() => {
    if (search === filters.search) return;
    const handle = setTimeout(() => setTaskFilters({ search }), 150);
    return () => clearTimeout(handle);
  }, [search, filters.search, setTaskFilters]);

  const activeProject = projects.find((p) => p.id === activeProjectId);
  const priorityLabel = (value: number) =>
    value === NO_PRIORITY
      ? m.priority.none
      : `${formatMessage(m.priority.label, { value })} · ${m.priority[`p${value}` as "p1" | "p2" | "p3" | "p4"]}`;

  const statusSummary = filters.statuses.length > 1 ? formatMessage(m.toolbar.statusCount, { count: filters.statuses.length }) : undefined;
  const providerSummary = filters.providers.length > 1 ? formatMessage(m.toolbar.providerCount, { count: filters.providers.length }) : undefined;
  const control = "w-40";

  return (
    <div role="search" aria-label={m.toolbar.label} className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border bg-surface px-4 py-2">
      <div className="w-64">
        <Input
          type="search"
          icon={<Search />}
          aria-label={m.toolbar.search}
          placeholder={m.toolbar.searchPlaceholder}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape" && search) {
              e.preventDefault();
              setSearch("");
              setTaskFilters({ search: "" });
            }
          }}
        />
      </div>
      <Select value={activeProject ? activeProject.id : ALL} onValueChange={(v) => setActiveProject(v === ALL ? null : v)}>
        <SelectTrigger className={control} aria-label={m.toolbar.project}>
          <SelectValue>{activeProject?.name ?? m.toolbar.allProjects}</SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={ALL}>{m.toolbar.allProjects}</SelectItem>
          {projects.map((p) => (
            <SelectItem key={p.id} value={p.id}>
              {p.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <MultiSelect
        className={control}
        aria-label={m.toolbar.status}
        placeholder={m.toolbar.anyStatus}
        summary={statusSummary}
        options={ALL_TASK_STATUSES.map((status) => ({ value: status, label: t.status[status] }))}
        selected={filters.statuses}
        onChange={(statuses) => setTaskFilters({ statuses: statuses as TaskStatus[] })}
      />
      <MultiSelect
        className={control}
        aria-label={m.toolbar.provider}
        placeholder={m.toolbar.anyProvider}
        summary={providerSummary}
        options={ALL_PROVIDERS.map((provider) => ({ value: provider, label: t.provider[provider as ProviderKind] }))}
        selected={filters.providers}
        onChange={(providers) => setTaskFilters({ providers })}
      />
      <Select
        value={filters.priority == null ? ALL : String(filters.priority)}
        onValueChange={(v) => setTaskFilters({ priority: v === ALL ? null : Number(v) })}
      >
        <SelectTrigger className={control} aria-label={m.toolbar.priority}>
          <SelectValue>{filters.priority == null ? m.toolbar.anyPriority : priorityLabel(filters.priority)}</SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={ALL}>{m.toolbar.anyPriority}</SelectItem>
          {PRIORITY_VALUES.map((value) => (
            <SelectItem key={value} value={String(value)}>
              {priorityLabel(value)}
            </SelectItem>
          ))}
          <SelectItem value={String(NO_PRIORITY)}>{m.priority.none}</SelectItem>
        </SelectContent>
      </Select>
      <Button
        size="sm"
        variant="subtle"
        disabled={!hasActiveFilters(filters)}
        onClick={() => {
          setSearch("");
          resetTaskFilters();
        }}
      >
        <FilterX />
        {m.toolbar.clear}
      </Button>
      <span className="ms-auto text-xs text-muted-foreground tabular-nums" aria-live="polite">
        {formatMessage(m.toolbar.showing, { shown, total })}
      </span>
    </div>
  );
};
