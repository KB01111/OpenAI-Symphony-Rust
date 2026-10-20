import React, { useMemo } from "react";
import { Button, Toolbar, ToolbarRadioButton, ToolbarRadioGroup } from "@fluentui/react-components";
import { errorMessage } from "@symphony/api-client";
import { formatMessage } from "@symphony/shared-types";
import { AiEmptyView, ErrorState, LoadingRows, PageHeader } from "@symphony/ui";
import { FilterX, FolderPlus, KanbanSquare, ListTodo, Plus, Table2 } from "lucide-react";
import { useMessages } from "../../lib/i18n.js";
import { useProjects, useTasks } from "../../lib/queries.js";
import { useUiStore, type TaskView } from "../../store/uiStore.js";
import { taskMessages } from "./messages.js";
import { TaskActionsProvider } from "./taskActions.js";
import { TaskBoard } from "./TaskBoard.js";
import { TaskTable } from "./TaskTable.js";
import { TaskToolbar } from "./TaskToolbar.js";
import { filterTasks, hasActiveFilters } from "./taskLogic.js";

/** Task management: Kanban board or data table over the runtime's tasks, with shared filters. */
export default function TasksPage() {
  const m = useMessages(taskMessages);
  const taskView = useUiStore((s) => s.taskView);
  const setTaskView = useUiStore((s) => s.setTaskView);
  const activeProjectId = useUiStore((s) => s.activeProjectId);
  const filters = useUiStore((s) => s.taskFilters);
  const resetTaskFilters = useUiStore((s) => s.resetTaskFilters);
  const setNewTaskOpen = useUiStore((s) => s.setNewTaskOpen);
  const setAddProjectOpen = useUiStore((s) => s.setAddProjectOpen);

  const projects = useProjects();
  const tasks = useTasks(activeProjectId);
  const projectList = useMemo(() => projects.data ?? [], [projects.data]);
  const allTasks = useMemo(() => tasks.data ?? [], [tasks.data]);
  const visible = useMemo(() => filterTasks(allTasks, filters), [allTasks, filters]);
  const activeProject = projectList.find((p) => p.id === activeProjectId);

  const viewToggle = (
    <Toolbar
      size="small"
      aria-label={m.page.viewLabel}
      checkedValues={{ view: [taskView] }}
      onCheckedValueChange={(_, data) => {
        const next = data.checkedItems[0] as TaskView | undefined;
        if (data.name === "view" && next) setTaskView(next);
      }}
      className="!p-0"
    >
      <ToolbarRadioGroup>
        <ToolbarRadioButton name="view" value="board" icon={<KanbanSquare className="size-4" aria-hidden />}>
          {m.page.board}
        </ToolbarRadioButton>
        <ToolbarRadioButton name="view" value="table" icon={<Table2 className="size-4" aria-hidden />}>
          {m.page.table}
        </ToolbarRadioButton>
      </ToolbarRadioGroup>
    </Toolbar>
  );

  const header = (
    <PageHeader
      title={m.page.title}
      description={activeProject ? formatMessage(m.page.descriptionProject, { project: activeProject.name }) : m.page.description}
      actions={
        <>
          {viewToggle}
          <Button appearance="primary" size="small" icon={<Plus className="size-4" aria-hidden />} onClick={() => setNewTaskOpen(true)}>
            {m.page.newTask}
          </Button>
        </>
      }
    />
  );

  let content: React.ReactNode;
  if (projects.isError) {
    content = <ErrorState className="m-4" title={m.page.projectsError} message={errorMessage(projects.error)} onRetry={() => void projects.refetch()} />;
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
          <Button appearance="primary" onClick={() => setAddProjectOpen(true)}>
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
          <Button appearance="primary" icon={<Plus className="size-4" aria-hidden />} onClick={() => setNewTaskOpen(true)}>
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
        {projectList.length > 0 ? <TaskToolbar projects={projectList} shown={visible.length} total={allTasks.length} /> : null}
        <div className="min-h-0 flex-1 overflow-hidden">{content}</div>
      </div>
    </TaskActionsProvider>
  );
}
