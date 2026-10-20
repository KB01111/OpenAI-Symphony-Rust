import React, { useEffect, useState } from "react";
import { Button, Dropdown, Input, Option } from "@fluentui/react-components";
import { ALL_PROVIDERS, ALL_TASK_STATUSES, formatMessage, type Project, type ProviderKind, type TaskStatus } from "@symphony/shared-types";
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

  const statusValue =
    filters.statuses.length === 0
      ? ""
      : filters.statuses.length === 1
        ? t.status[filters.statuses[0]!]
        : formatMessage(m.toolbar.statusCount, { count: filters.statuses.length });
  const providerValue =
    filters.providers.length === 0
      ? ""
      : filters.providers.length === 1
        ? (t.provider[filters.providers[0] as ProviderKind] ?? filters.providers[0])
        : formatMessage(m.toolbar.providerCount, { count: filters.providers.length });

  const dropdownClass = "!min-w-0 w-40";

  return (
    <div role="search" aria-label={m.toolbar.label} className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border bg-surface px-4 py-2">
      <Input
        size="small"
        className="w-64"
        aria-label={m.toolbar.search}
        placeholder={m.toolbar.searchPlaceholder}
        contentBefore={<Search className="size-3.5" aria-hidden />}
        value={search}
        onChange={(_, data) => setSearch(data.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape" && search) {
            e.preventDefault();
            setSearch("");
            setTaskFilters({ search: "" });
          }
        }}
      />
      <Dropdown
        size="small"
        className={dropdownClass}
        aria-label={m.toolbar.project}
        value={activeProject?.name ?? m.toolbar.allProjects}
        selectedOptions={[activeProject ? activeProject.id : ALL]}
        onOptionSelect={(_, data) => setActiveProject(data.optionValue && data.optionValue !== ALL ? data.optionValue : null)}
      >
        <Option value={ALL}>{m.toolbar.allProjects}</Option>
        {projects.map((p) => (
          <Option key={p.id} value={p.id} text={p.name}>
            {p.name}
          </Option>
        ))}
      </Dropdown>
      <Dropdown
        size="small"
        multiselect
        className={dropdownClass}
        aria-label={m.toolbar.status}
        placeholder={m.toolbar.anyStatus}
        value={statusValue}
        selectedOptions={filters.statuses}
        onOptionSelect={(_, data) => setTaskFilters({ statuses: data.selectedOptions as TaskStatus[] })}
      >
        {ALL_TASK_STATUSES.map((status) => (
          <Option key={status} value={status}>
            {t.status[status]}
          </Option>
        ))}
      </Dropdown>
      <Dropdown
        size="small"
        multiselect
        className={dropdownClass}
        aria-label={m.toolbar.provider}
        placeholder={m.toolbar.anyProvider}
        value={providerValue}
        selectedOptions={filters.providers}
        onOptionSelect={(_, data) => setTaskFilters({ providers: data.selectedOptions })}
      >
        {ALL_PROVIDERS.map((provider) => (
          <Option key={provider} value={provider}>
            {t.provider[provider]}
          </Option>
        ))}
      </Dropdown>
      <Dropdown
        size="small"
        className={dropdownClass}
        aria-label={m.toolbar.priority}
        value={filters.priority == null ? m.toolbar.anyPriority : priorityLabel(filters.priority)}
        selectedOptions={[filters.priority == null ? ALL : String(filters.priority)]}
        onOptionSelect={(_, data) => setTaskFilters({ priority: data.optionValue && data.optionValue !== ALL ? Number(data.optionValue) : null })}
      >
        <Option value={ALL}>{m.toolbar.anyPriority}</Option>
        {PRIORITY_VALUES.map((value) => (
          <Option key={value} value={String(value)} text={priorityLabel(value)}>
            {priorityLabel(value)}
          </Option>
        ))}
        <Option value={String(NO_PRIORITY)}>{m.priority.none}</Option>
      </Dropdown>
      <Button
        size="small"
        appearance="subtle"
        icon={<FilterX className="size-4" aria-hidden />}
        disabled={!hasActiveFilters(filters)}
        onClick={() => {
          setSearch("");
          resetTaskFilters();
        }}
      >
        {m.toolbar.clear}
      </Button>
      <span className="ms-auto text-xs text-muted-foreground tabular-nums" aria-live="polite">
        {formatMessage(m.toolbar.showing, { shown, total })}
      </span>
    </div>
  );
};
