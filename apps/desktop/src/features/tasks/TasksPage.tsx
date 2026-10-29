import React, { Suspense, lazy, useEffect, useMemo } from "react";
import { useSearchParams } from "react-router-dom";
import { errorMessage } from "@symphony/api-client";
import { formatMessage } from "@symphony/shared-types";
import { AiEmptyView, Button, ErrorState, LoadingRows, PageHeader, ToggleGroup, ToggleGroupItem } from "@symphony/ui";
import { Bot, FilterX, FolderPlus, KanbanSquare, ListTodo, Plus, Table2 } from "lucide-react";
import { useMessages } from "../../lib/i18n.js";
import { useProjects, useTasks } from "../../lib/queries.js";
import { useUiStore, type TaskView } from "../../store/uiStore.js";
import { taskMessages } from "./messages.js";
import { TaskActionsProvider } from "./taskActions.js";
import { TaskBoard } from "./TaskBoard.js";
import { TaskTable } from "./TaskTable.js";
import { TaskToolbar } from "./TaskToolbar.js";
import { filterTasks, hasActiveFilters } from "./taskLogic.js";

// The Runs view (live sessions, retries, recent finishes) is its own chunk, loaded when first shown.
const AgentRunsView = lazy(() => import("../agents/AgentRunsView.js"));

/** Task management: Kanban board, data table or agent runs over the runtime's tasks. */
export default function TasksPage() {
  const m = useMessages(taskMessages);
  const taskView = useUiStore((s) => s.taskView);
  const setTaskView = useUiStore((s) => s.setTaskView);
  // The address is the source of truth for Runs (`/tasks?view=runs`, also what the old /runs page opens),
  // so the view is shareable and follows Back/Forward. It is not a saved preference: a plain /tasks link
  // shows the board or table.
  const [search, setSearch] = useSearchParams();
  const runsOpen = search.get("view") === "runs";
  // A history entry each, so Back returns to the view you came from.
  const showRuns = (open: boolean) => setSearch(open ? { view: "runs" } : {});
  const activeProjectId = useUiStore((s) => s.activeProjectId);
  const filters = useUiStore((s) => s.taskFilters);
  const resetTaskFilters = useUiStore((s) => s.resetTaskFilters);
  const setNewTaskOpen = useUiStore((s) => s.setNewTaskOpen);
  const setAddProjectOpen = useUiStore((s) => s.setAddProjectOpen);

  const projects = useProjects();
  const projectList = useMemo(() => projects.data ?? [], [projects.data]);
  // A persisted selection whose project was removed falls back to all projects (matching the
  // toolbar), and is cleared once a settled project list confirms it no longer exists. While the
  // list is refetching (e.g. right after creating a project) the selection is kept.
  const selectionExists = activeProjectId !== null && projectList.some((p) => p.id === activeProjectId);
  const listSettled = projects.data !== undefined && !projects.isFetching;
  const selectionGone = listSettled && activeProjectId !== null && !selectionExists;
  const effectiveProjectId = selectionGone ? null : activeProjectId;
  const setActiveProject = useUiStore((s) => s.setActiveProject);
  useEffect(() => {
    if (selectionGone) setActiveProject(null);
  }, [selectionGone, setActiveProject]);
  const tasks = useTasks(effectiveProjectId);
  const allTasks = useMemo(() => tasks.data ?? [], [tasks.data]);
  const visible = useMemo(() => filterTasks(allTasks, filters), [allTasks, filters]);
  const activeProject = projectList.find((p) => p.id === activeProjectId);

  const viewToggle = (
    <ToggleGroup
      type="single"
      aria-label={m.page.viewLabel}
      value={runsOpen ? "runs" : taskView}
      onValueChange={(next) => {
        if (!next) return;
        showRuns(next === "runs");
        if (next !== "runs") setTaskView(next as TaskView);
      }}
    >
      <ToggleGroupItem value="board" className="gap-1.5">
        <KanbanSquare className="size-3.5" aria-hidden />
        {m.page.board}
      </ToggleGroupItem>
      <ToggleGroupItem value="table" className="gap-1.5">
        <Table2 className="size-3.5" aria-hidden />
        {m.page.table}
      </ToggleGroupItem>
      <ToggleGroupItem value="runs" className="gap-1.5">
        <Bot className="size-3.5" aria-hidden />
        {m.page.runs}
      </ToggleGroupItem>
    </ToggleGroup>
  );

  const header = (
    <PageHeader
      title={m.page.title}
      // Runs spans every project, so it never carries the selected project's name.
      description={activeProject && !runsOpen ? formatMessage(m.page.descriptionProject, { project: activeProject.name }) : m.page.description}
      actions={
        <>
          {viewToggle}
          <Button variant="default" size="sm" onClick={() => setNewTaskOpen(true)}>
            <Plus />
            {m.page.newTask}
          </Button>
        </>
      }
    />
  );

  let content: React.ReactNode;
  if (projects.isError && !projects.data) {
    // Runs groups ready tasks by their project's auto-dispatch setting, so it needs the projects too. A failed
    // refresh with projects already loaded keeps showing the cached ones.
    content = <ErrorState className="m-4" title={m.page.projectsError} message={errorMessage(projects.error)} onRetry={() => void projects.refetch()} />;
  } else if (runsOpen) {
    // Runs has its own tabs, loading and empty states, so the task filters and lists stay out of the way.
    content = (
      <Suspense fallback={<LoadingRows className="p-4" rows={8} label={m.page.loading} />}>
        <AgentRunsView onShowTasks={() => showRuns(false)} />
      </Suspense>
    );
  } else if (tasks.isError) {
    content = <ErrorState className="m-4" title={m.page.loadError} message={errorMessage(tasks.error)} onRetry={() => void tasks.refetch()} />;
  } else if (projects.isLoading || tasks.isLoading) {
    content = <LoadingRows className="p-4" rows={8} label={m.page.loading} />;
  } else if (projectList.length === 0) {
    content = (
      <AiEmptyView
        className="m-4"
        icon={<FolderPlus className="size-6 text-accent" aria-hidden />}
        title={m.empty.noProjectsTitle}
        description={m.empty.noProjectsBody}
        action={
          <Button variant="default" onClick={() => setAddProjectOpen(true)}>
            {m.empty.addRepository}
          </Button>
        }
      />
    );
  } else if (allTasks.length === 0) {
    content = (
      <AiEmptyView
        className="m-4"
        icon={<ListTodo className="size-6 text-accent" aria-hidden />}
        title={m.empty.noTasksTitle}
        description={m.empty.noTasksBody}
        action={
          <Button variant="default" onClick={() => setNewTaskOpen(true)}>
            <Plus />
            {m.page.newTask}
          </Button>
        }
      />
    );
  } else if (visible.length === 0 && hasActiveFilters(filters)) {
    content = (
      <AiEmptyView
        className="m-4"
        icon={<FilterX className="size-6 text-accent" aria-hidden />}
        title={m.empty.noMatchesTitle}
        description={m.empty.noMatchesBody}
        action={<Button onClick={resetTaskFilters}>{m.toolbar.clear}</Button>}
      />
    );
  } else if (taskView === "table") {
    content = <TaskTable tasks={visible} projects={projectList} />;
  } else {
    content = <TaskBoard tasks={visible} />;
  }

  return (
    <TaskActionsProvider>
      <div className="flex h-full min-h-0 flex-col">
        {header}
        {projectList.length > 0 && !runsOpen ? <TaskToolbar projects={projectList} shown={visible.length} total={allTasks.length} /> : null}
        <div className="min-h-0 flex-1 overflow-hidden">{content}</div>
      </div>
    </TaskActionsProvider>
  );
}
