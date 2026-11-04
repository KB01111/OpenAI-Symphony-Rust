import { useMemo } from "react";
import { Link } from "react-router-dom";
import { formatMessage } from "@symphony/shared-types";
import { StatusDot, TimeAgo } from "@symphony/ui";
import { paths } from "../../app/routes.js";
import { useMessages, useT } from "../../lib/i18n.js";
import { useApprovals, useProjects, useTasks } from "../../lib/queries.js";
import { workbenchMessages } from "../../layouts/messages.js";
import { statusTone } from "../../layouts/ThreadList.js";
import { TaskComposer } from "./TaskComposer.js";

/** Home: the prompt composer, centred, with the most recent threads underneath. */
export default function HomePage() {
  const m = useMessages(workbenchMessages);
  const t = useT();
  const tasks = useTasks(null);
  const projects = useProjects();
  const approvals = useApprovals(true);

  const recent = useMemo(
    () => [...(tasks.data ?? [])].sort((a, b) => b.updated_at.localeCompare(a.updated_at)).slice(0, 6),
    [tasks.data]
  );
  const approvalIds = useMemo(() => new Set((approvals.data ?? []).map((a) => a.task_id)), [approvals.data]);
  const projectName = useMemo(() => new Map((projects.data ?? []).map((p) => [p.id, p.name] as const)), [projects.data]);
  const all = tasks.data ?? [];
  const running = all.filter((x) => x.status === "running").length;
  const review = all.filter((x) => x.status === "human_review").length;
  const waiting = approvals.data?.length ?? 0;

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex min-h-full w-full max-w-3xl flex-col justify-center gap-8 px-6 py-10">
        <header className="space-y-1.5 text-center">
          <h1 className="font-display text-3xl font-semibold tracking-tight text-foreground">{m.home.title}</h1>
          <p className="text-sm text-muted-foreground">{m.home.subtitle}</p>
          {running + waiting + review > 0 ? (
            <p className="flex flex-wrap items-center justify-center gap-x-4 gap-y-1 pt-1 text-xs text-muted-foreground">
              {running > 0 ? <span>{formatMessage(m.home.statusRunning, { count: running })}</span> : null}
              {waiting > 0 ? <span className="text-warning">{formatMessage(m.home.statusApproval, { count: waiting })}</span> : null}
              {review > 0 ? <span>{formatMessage(m.home.statusReview, { count: review })}</span> : null}
            </p>
          ) : null}
        </header>

        <TaskComposer variant="hero" />

        <section aria-label={m.home.recent} className="space-y-2">
          <h2 className="px-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{m.home.recent}</h2>
          {recent.length === 0 ? (
            <p className="px-1 text-sm text-muted-foreground">{m.home.noRecent}</p>
          ) : (
            <ul className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-surface" role="list">
              {recent.map((task) => {
                const needs = approvalIds.has(task.id);
                const { tone, pulse } = statusTone(task.status, needs);
                return (
                  <li key={task.id}>
                    <Link
                      to={paths.run(task.id)}
                      className="flex min-w-0 items-center gap-3 px-3 py-2.5 text-sm outline-none transition-colors hover:bg-surface-hover focus-visible:bg-surface-hover"
                    >
                      <StatusDot tone={tone} pulse={pulse} label={needs ? m.sidebar.needsApproval : t.status[task.status]} />
                      <span className="min-w-0 flex-1 truncate text-foreground">{task.issue.title}</span>
                      <span className="hidden shrink-0 font-mono text-xs text-muted-foreground sm:inline">{task.issue.identifier}</span>
                      <span className="hidden max-w-40 shrink-0 truncate text-xs text-muted-foreground md:inline">{projectName.get(task.project_id)}</span>
                      <TimeAgo iso={task.updated_at} className="shrink-0 text-xs tabular-nums text-subtle-foreground" />
                    </Link>
                  </li>
                );
              })}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}
