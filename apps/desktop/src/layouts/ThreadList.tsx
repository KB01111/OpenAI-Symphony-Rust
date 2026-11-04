import React, { useMemo, useState } from "react";
import { NavLink } from "react-router-dom";
import { ChevronDown, ChevronRight, FolderGit2, Search } from "lucide-react";
import { formatMessage, type Project, type Task, type TaskStatus } from "@symphony/shared-types";
import { Button, Input, StatusDot, TimeAgo, cn } from "@symphony/ui";
import { paths } from "../app/routes.js";
import { useMessages, useT } from "../lib/i18n.js";
import { useApprovals, useProjects, useTasks } from "../lib/queries.js";
import { useUiStore } from "../store/uiStore.js";
import { workbenchMessages } from "./messages.js";

type Tone = "success" | "warning" | "danger" | "neutral" | "accent";

export function statusTone(status: TaskStatus, needsApproval = false): { tone: Tone; pulse: boolean } {
  if (needsApproval) return { tone: "warning", pulse: true };
  switch (status) {
    case "running":
      return { tone: "accent", pulse: true };
    case "human_review":
      return { tone: "warning", pulse: false };
    case "blocked":
      return { tone: "warning", pulse: false };
    case "failed":
      return { tone: "danger", pulse: false };
    case "completed":
      return { tone: "success", pulse: false };
    default:
      return { tone: "neutral", pulse: false };
  }
}

const VISIBLE_PER_PROJECT = 6;

const ThreadItem: React.FC<{ task: Task; needsApproval: boolean }> = ({ task, needsApproval }) => {
  const t = useT();
  const m = useMessages(workbenchMessages);
  const { tone, pulse } = statusTone(task.status, needsApproval);
  const statusLabel = needsApproval ? m.sidebar.needsApproval : t.status[task.status];
  return (
    <li>
      <NavLink
        to={paths.run(task.id)}
        title={`${task.issue.identifier} · ${task.issue.title}`}
        className={({ isActive }) =>
          cn(
            "group flex h-8 min-w-0 items-center gap-2 rounded-lg px-2 text-sm text-muted-foreground outline-none transition-colors hover:bg-surface-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60",
            isActive && "bg-surface-selected text-foreground"
          )
        }
      >
        <StatusDot tone={tone} pulse={pulse} label={statusLabel} />
        <span className="min-w-0 flex-1 truncate">{task.issue.title}</span>
        <TimeAgo iso={task.updated_at} className="shrink-0 text-[11px] tabular-nums text-subtle-foreground" />
      </NavLink>
    </li>
  );
};

const ProjectGroup: React.FC<{ projectId: string; project: Project | null; tasks: Task[]; approvals: Set<string>; forceOpen: boolean }> = ({
  projectId,
  project,
  tasks,
  approvals,
  forceOpen
}) => {
  const m = useMessages(workbenchMessages);
  const id = projectId;
  const name = project?.name ?? m.sidebar.noProject;
  const collapsedIds = useUiStore((s) => s.collapsedProjects);
  const toggle = useUiStore((s) => s.toggleProjectCollapsed);
  const [expanded, setExpanded] = useState(false);
  const collapsed = !forceOpen && collapsedIds.includes(id);
  const shown = expanded || forceOpen ? tasks : tasks.slice(0, VISIBLE_PER_PROJECT);
  const hidden = tasks.length - shown.length;
  const attention = tasks.some((x) => approvals.has(x.id));
  return (
    <section aria-label={name} className="min-w-0">
      <button
        type="button"
        onClick={() => toggle(id)}
        aria-expanded={!collapsed}
        aria-label={formatMessage(collapsed ? m.sidebar.expandProject : m.sidebar.collapseProject, { name })}
        className="flex h-7 w-full min-w-0 items-center gap-1.5 rounded-md px-1.5 text-xs font-medium text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60"
      >
        {collapsed ? <ChevronRight className="size-3.5 shrink-0" aria-hidden /> : <ChevronDown className="size-3.5 shrink-0" aria-hidden />}
        <FolderGit2 className="size-3.5 shrink-0" aria-hidden />
        <span className="min-w-0 flex-1 truncate text-start">{name}</span>
        {collapsed && attention ? <span className="size-1.5 shrink-0 rounded-full bg-warning" aria-hidden /> : null}
        <span className="shrink-0 tabular-nums text-subtle-foreground">{tasks.length}</span>
      </button>
      {collapsed ? null : (
        <ul className="mt-0.5 flex flex-col gap-px" role="list">
          {shown.map((task) => (
            <ThreadItem key={task.id} task={task} needsApproval={approvals.has(task.id)} />
          ))}
          {hidden > 0 ? (
            <li>
              <button
                type="button"
                onClick={() => setExpanded(true)}
                className="h-7 w-full rounded-lg px-2 text-start text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60"
              >
                {formatMessage(m.sidebar.showMore, { count: hidden })}
              </button>
            </li>
          ) : expanded && tasks.length > VISIBLE_PER_PROJECT ? (
            <li>
              <button
                type="button"
                onClick={() => setExpanded(false)}
                className="h-7 w-full rounded-lg px-2 text-start text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60"
              >
                {m.sidebar.showLess}
              </button>
            </li>
          ) : null}
        </ul>
      )}
    </section>
  );
};

/** Tasks grouped by project: the sidebar's "threads", like the conversations list of a chat app. */
export const ThreadList: React.FC = () => {
  const m = useMessages(workbenchMessages);
  const t = useT();
  const tasks = useTasks(null);
  const projects = useProjects();
  const approvals = useApprovals(true);
  const [query, setQuery] = useState("");

  const approvalTaskIds = useMemo(() => new Set((approvals.data ?? []).map((a) => a.task_id)), [approvals.data]);

  const groups = useMemo(() => {
    const q = query.trim().toLowerCase();
    const byProject = new Map<string, Task[]>();
    for (const task of tasks.data ?? []) {
      if (q && !`${task.issue.identifier} ${task.issue.title}`.toLowerCase().includes(q)) continue;
      const list = byProject.get(task.project_id) ?? [];
      list.push(task);
      byProject.set(task.project_id, list);
    }
    const projectById = new Map((projects.data ?? []).map((p) => [p.id, p] as const));
    return [...byProject.entries()]
      .map(([projectId, list]) => ({
        projectId,
        project: projectById.get(projectId) ?? null,
        tasks: [...list].sort((a, b) => b.updated_at.localeCompare(a.updated_at))
      }))
      .sort((a, b) => (b.tasks[0]?.updated_at ?? "").localeCompare(a.tasks[0]?.updated_at ?? ""));
  }, [tasks.data, projects.data, query]);

  const empty = (tasks.data ?? []).length === 0;
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-1.5">
      <div className="px-2">
        <Input
          type="search"
          icon={<Search />}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          aria-label={m.sidebar.search}
          placeholder={m.sidebar.search}
          className="rounded-lg border-transparent bg-surface-sunken shadow-none"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2" role="region" aria-label={m.sidebar.threads} tabIndex={-1}>
        {tasks.isLoading ? null : tasks.isError && !tasks.data ? (
          <div role="alert" className="space-y-2 px-2 py-3 text-xs text-muted-foreground">
            <p>{m.sidebar.loadFailed}</p>
            <Button size="sm" onClick={() => void tasks.refetch()}>
              {t.common.retry}
            </Button>
          </div>
        ) : empty ? (
          <p className="px-2 py-3 text-xs text-muted-foreground">{m.sidebar.noThreads}</p>
        ) : groups.length === 0 ? (
          <p className="px-2 py-3 text-xs text-muted-foreground">{m.sidebar.noMatches}</p>
        ) : (
          <div className="flex flex-col gap-2">
            {approvals.isError && !approvals.data ? (
              <p role="status" className="px-2 text-xs text-warning">
                {m.sidebar.approvalsUnavailable}
              </p>
            ) : null}
            {groups.map((g) => (
              <ProjectGroup key={g.projectId} projectId={g.projectId} project={g.project} tasks={g.tasks} approvals={approvalTaskIds} forceOpen={query.trim().length > 0} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
};
