import React, { useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { Badge, Button, Dropdown, Field, Input, Option } from "@fluentui/react-components";
import { errorMessage } from "@symphony/api-client";
import { formatMessage, type Project } from "@symphony/shared-types";
import { AiEmptyView, DataTable, ErrorState, LoadingRows, PageHeader, ProviderBadge, type ColumnDef } from "@symphony/ui";
import { CheckCircle2, FilterX, FolderPlus, Plus, Search } from "lucide-react";
import { paths } from "../../app/routes.js";
import { useMessages, useT } from "../../lib/i18n.js";
import { useProjects, useTasks } from "../../lib/queries.js";
import { useUiStore } from "../../store/uiStore.js";
import { projectMessages } from "./messages.js";
import { ProjectDetail } from "./ProjectDetail.js";
import { trackerLabel } from "./ProjectFormFields.js";
import { TRACKER_KINDS, countTasksByProject, filterProjects, type ProjectTaskCounts } from "./projectLogic.js";

const ANY = "__any__";

const TaskCountCell: React.FC<{ counts: ProjectTaskCounts | undefined }> = ({ counts }) => {
  const m = useMessages(projectMessages);
  if (!counts) return <span className="text-xs text-subtle-foreground">0</span>;
  return (
    <span className="inline-flex items-center gap-1" title={formatMessage(m.counts.total, { count: counts.total })}>
      <span className="text-xs tabular-nums text-foreground">{counts.total}</span>
      {counts.running > 0 ? (
        <Badge size="small" appearance="tint" color="informative">
          {formatMessage(m.counts.running, { count: counts.running })}
        </Badge>
      ) : null}
      {counts.review > 0 ? (
        <Badge size="small" appearance="tint" color="warning">
          {formatMessage(m.counts.review, { count: counts.review })}
        </Badge>
      ) : null}
      {counts.failed > 0 ? (
        <Badge size="small" appearance="tint" color="danger">
          {formatMessage(m.counts.failed, { count: counts.failed })}
        </Badge>
      ) : null}
    </span>
  );
};

/** Registered repositories: searchable table plus a detail/edit panel for the selected project. */
export default function ProjectsPage() {
  const m = useMessages(projectMessages);
  const t = useT();
  const navigate = useNavigate();
  const { projectId } = useParams<{ projectId?: string }>();
  const activeProjectId = useUiStore((s) => s.activeProjectId);
  const setActiveProject = useUiStore((s) => s.setActiveProject);
  const inspect = useUiStore((s) => s.inspect);
  const setAddProjectOpen = useUiStore((s) => s.setAddProjectOpen);

  const projects = useProjects();
  const tasks = useTasks();
  const [search, setSearch] = useState("");
  const [tracker, setTracker] = useState<string | null>(null);

  const projectList = useMemo(() => projects.data ?? [], [projects.data]);
  const counts = useMemo(() => countTasksByProject(tasks.data ?? []), [tasks.data]);
  const visible = useMemo(() => filterProjects(projectList, search, tracker), [projectList, search, tracker]);
  const trackerKinds = useMemo(
    () => [...new Set<string>([...TRACKER_KINDS, ...projectList.map((p) => p.tracker_kind)])],
    [projectList]
  );
  const selected = projectId ? projectList.find((p) => p.id === projectId) : undefined;

  // Deep links (/projects/:id) make the project the active one and focus it in the inspector.
  const selectedId = selected?.id;
  useEffect(() => {
    if (!selectedId) return;
    setActiveProject(selectedId);
    inspect({ kind: "project", id: selectedId });
  }, [selectedId, setActiveProject, inspect]);

  const columns = useMemo<ColumnDef<Project, unknown>[]>(
    () => [
      {
        id: "name",
        header: m.columns.name,
        accessorFn: (p) => p.name.toLowerCase(),
        cell: ({ row }) => (
          <span className="inline-flex min-w-0 items-center gap-1.5">
            <span className="truncate text-sm font-medium text-foreground">{row.original.name}</span>
            {row.original.id === activeProjectId ? (
              <CheckCircle2 className="size-3.5 shrink-0 text-accent" aria-label={m.detail.active} />
            ) : null}
          </span>
        )
      },
      {
        id: "repo",
        header: m.columns.repo,
        accessorFn: (p) => p.repo_path,
        cell: ({ row }) => (
          <span className="block max-w-[22rem] truncate font-mono text-xs text-muted-foreground" title={row.original.repo_path}>
            {row.original.repo_path}
          </span>
        )
      },
      {
        id: "branch",
        header: m.columns.branch,
        accessorFn: (p) => p.default_branch,
        cell: ({ row }) => <span className="font-mono text-xs text-foreground">{row.original.default_branch}</span>
      },
      {
        id: "tracker",
        header: m.columns.tracker,
        accessorFn: (p) => p.tracker_kind,
        cell: ({ row }) => (
          <span className="text-xs text-foreground">
            {trackerLabel(m, row.original.tracker_kind)}
            {row.original.github_repo ? <span className="ml-1 font-mono text-muted-foreground">{row.original.github_repo}</span> : null}
            {row.original.linear_project_slug ? (
              <span className="ml-1 font-mono text-muted-foreground">{row.original.linear_project_slug}</span>
            ) : null}
          </span>
        )
      },
      {
        id: "provider",
        header: m.columns.provider,
        accessorFn: (p) => `${p.preferred_provider}:${p.preferred_model ?? ""}`,
        cell: ({ row }) => <ProviderBadge provider={row.original.preferred_provider} model={row.original.preferred_model} />
      },
      {
        id: "auto",
        header: m.columns.autoDispatch,
        accessorFn: (p) => (p.auto_dispatch ? 1 : 0),
        cell: ({ row }) => (
          <span className={row.original.auto_dispatch ? "text-xs text-success" : "text-xs text-muted-foreground"}>
            {row.original.auto_dispatch ? t.common.yes : t.common.no}
          </span>
        )
      },
      {
        id: "agents",
        header: m.columns.maxAgents,
        accessorFn: (p) => p.max_concurrent_agents,
        cell: ({ row }) => <span className="text-xs tabular-nums text-foreground">{row.original.max_concurrent_agents}</span>
      },
      {
        id: "tasks",
        header: m.columns.tasks,
        accessorFn: (p) => counts.get(p.id)?.total ?? 0,
        cell: ({ row }) => <TaskCountCell counts={counts.get(row.original.id)} />
      }
    ],
    [m, t, activeProjectId, counts]
  );

  const select = (p: Project) => {
    setActiveProject(p.id);
    inspect({ kind: "project", id: p.id });
    navigate(paths.project(p.id));
  };

  const clearFilters = () => {
    setSearch("");
    setTracker(null);
  };

  const header = (
    <PageHeader
      title={m.page.title}
      description={m.page.description}
      actions={
        <Button appearance="primary" size="small" icon={<Plus className="size-4" aria-hidden />} onClick={() => setAddProjectOpen(true)}>
          {m.page.addRepository}
        </Button>
      }
    />
  );

  let content: React.ReactNode;
  if (projects.isError) {
    content = <ErrorState className="m-4" title={m.page.loadError} message={errorMessage(projects.error)} onRetry={() => void projects.refetch()} />;
  } else if (projects.isLoading) {
    content = <LoadingRows className="p-4" rows={6} label={m.page.loading} />;
  } else if (projectList.length === 0) {
    content = (
      <AiEmptyView
        className="m-4"
        icon={<FolderPlus className="size-6 text-accent" aria-hidden />}
        title={m.page.emptyTitle}
        description={m.page.emptyBody}
        action={
          <Button appearance="primary" onClick={() => setAddProjectOpen(true)}>
            {m.page.addRepository}
          </Button>
        }
      />
    );
  } else {
    const list =
      visible.length === 0 ? (
        <AiEmptyView
          className="m-4"
          icon={<FilterX className="size-6 text-accent" aria-hidden />}
          title={m.page.noMatchesTitle}
          description={m.page.noMatchesBody}
          action={<Button onClick={clearFilters}>{m.page.clearFilters}</Button>}
        />
      ) : (
        <DataTable
          className="h-full"
          data={visible}
          columns={columns}
          getRowId={(p) => p.id}
          selectedId={projectId ?? null}
          onRowActivate={select}
          ariaLabel={m.page.listLabel}
          initialSorting={[{ id: "name", desc: false }]}
        />
      );

    const detail = projectId ? (
      selected ? (
        <ProjectDetail project={selected} counts={counts.get(selected.id)} />
      ) : (
        <div className="flex h-full flex-col gap-3 border-l border-border bg-surface p-4">
          <p className="text-sm text-muted-foreground">{m.page.notFound}</p>
          <div>
            <Button size="small" onClick={() => navigate(paths.projects())}>
              {m.page.backToList}
            </Button>
          </div>
        </div>
      )
    ) : null;

    content = (
      <div className="flex h-full min-h-0 flex-col">
        <div className="flex shrink-0 flex-wrap items-end gap-2 border-b border-border bg-surface px-4 py-2" role="search">
          <Field label={m.page.search} size="small" className="w-72 max-w-full">
            <Input
              size="small"
              type="search"
              value={search}
              placeholder={m.page.searchPlaceholder}
              contentBefore={<Search className="size-3.5" aria-hidden />}
              onChange={(_, d) => setSearch(d.value)}
            />
          </Field>
          <Field label={m.page.tracker} size="small" className="w-48">
            <Dropdown
              size="small"
              value={tracker ? trackerLabel(m, tracker) : m.page.anyTracker}
              selectedOptions={[tracker ?? ANY]}
              onOptionSelect={(_, d) => setTracker(!d.optionValue || d.optionValue === ANY ? null : d.optionValue)}
            >
              <Option value={ANY}>{m.page.anyTracker}</Option>
              {trackerKinds.map((kind) => (
                <Option key={kind} value={kind}>
                  {trackerLabel(m, kind)}
                </Option>
              ))}
            </Dropdown>
          </Field>
          {search || tracker ? (
            <Button size="small" appearance="subtle" icon={<FilterX className="size-4" aria-hidden />} onClick={clearFilters}>
              {m.page.clearFilters}
            </Button>
          ) : null}
          <span className="ml-auto pb-1 text-xs text-muted-foreground" aria-live="polite">
            {formatMessage(m.page.showing, { shown: visible.length, total: projectList.length })}
          </span>
        </div>
        <div className="flex min-h-0 flex-1">
          <div className="min-h-0 min-w-0 flex-1">{list}</div>
          {detail ? <div className="h-full w-[min(460px,50%)] shrink-0">{detail}</div> : null}
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      {header}
      <div className="min-h-0 flex-1 overflow-hidden">{content}</div>
    </div>
  );
}
