import React, { useMemo, useState } from "react";
import { Button, Tab, TabList } from "@fluentui/react-components";
import { Plus } from "lucide-react";
import { canDispatch, formatMessage, type Task } from "@symphony/shared-types";
import { AiEmptyView, ErrorState, LoadingRows, ProviderBadge, TaskStatusBadge, TimeAgo, useNotify } from "@symphony/ui";
import { TASK_FILTERS, countByFilter, visibleTasks, type TaskFilter } from "../liveData.js";
import { useStrings } from "../i18n.js";
import { NewTaskForm } from "./NewTaskForm.js";
import { useDispatchTask, useTasks } from "./queries.js";

export const TasksScreen: React.FC = () => {
  const { m, t } = useStrings();
  const tasks = useTasks();
  const dispatch = useDispatchTask();
  const notify = useNotify();
  const [creating, setCreating] = useState(false);
  const [filter, setFilter] = useState<TaskFilter>("active");
  const list = useMemo(() => visibleTasks(tasks.data ?? [], filter), [tasks.data, filter]);
  const counts = useMemo(() => countByFilter(tasks.data ?? []), [tasks.data]);
  const label: Record<TaskFilter, string> = { all: m.filterAll, active: m.filterActive, attention: m.filterAttention, done: m.filterDone };

  const start = (task: Task) => {
    if (dispatch.isPending) return;
    dispatch.mutate(task.id, {
      onSuccess: () => notify("success", formatMessage(m.taskStarted, { identifier: task.issue.identifier })),
      onError: (error) => notify("error", m.taskStartFailed, error.message)
    });
  };

  const header = creating ? (
    <NewTaskForm onDone={() => setCreating(false)} />
  ) : (
    <div className="flex justify-end">
      <Button appearance="primary" size="small" icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
        {t.mobile.newTask}
      </Button>
    </div>
  );

  if (tasks.isLoading) return <LoadingRows rows={6} label={t.common.loading} />;
  if (tasks.isError) return <ErrorState title={m.loadTasksFailed} message={tasks.error.message} onRetry={() => void tasks.refetch()} retryLabel={t.common.retry} />;
  if ((tasks.data ?? []).length === 0) {
    return (
      <div className="space-y-3">
        {header}
        <AiEmptyView title={m.noTasks} description={m.noTasksBody} />
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {header}
      <TabList size="small" selectedValue={filter} onTabSelect={(_, d) => setFilter(d.value as TaskFilter)} aria-label={m.filterTasks} className="overflow-x-auto">
        {TASK_FILTERS.map((f) => (
          <Tab key={f} value={f}>
            {label[f]} ({counts[f]})
          </Tab>
        ))}
      </TabList>
      {list.length === 0 ? (
        <p className="p-3 text-sm text-muted-foreground">{m.noTasksInFilter}</p>
      ) : (
        <ul className="space-y-2">
          {list.map((task) => (
            <li key={task.id} className="rounded-md border border-border bg-surface p-3">
              <div className="flex items-center justify-between gap-2">
                <span className="font-mono text-xs text-muted-foreground">{task.issue.identifier}</span>
                <TaskStatusBadge status={task.status} />
              </div>
              <p className="mt-1 text-sm font-medium text-foreground">{task.issue.title}</p>
              <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                <ProviderBadge provider={task.provider} model={task.model} />
                {task.attempt_count > 0 ? <span>{formatMessage(m.attempt, { count: task.attempt_count })}</span> : null}
                <TimeAgo iso={task.updated_at} />
                {canDispatch(task.status) ? (
                  <Button
                    size="small"
                    appearance="secondary"
                    className="ms-auto"
                    disabled={dispatch.isPending}
                    onClick={() => start(task)}
                  >
                    {m.startTask}
                  </Button>
                ) : null}
              </div>
              {task.last_error ? (
                <p className="mt-2 rounded bg-destructive-subtle p-2 text-xs text-destructive">
                  {m.lastError}: {task.last_error}
                </p>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};
