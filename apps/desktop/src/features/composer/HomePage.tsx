import { useMemo } from "react";
import { Link } from "react-router-dom";
import { ArrowRight, Ban, GitPullRequest, ShieldQuestion, XCircle, type LucideIcon } from "lucide-react";
import { formatMessage, type Task } from "@symphony/shared-types";
import { StatusDot, TimeAgo, cn } from "@symphony/ui";
import { paths } from "../../app/routes.js";
import { STAGE_STATUSES, WorkflowPipeline, useOpenWorkflowStage, type FlowStage } from "../../components/WorkflowStages.js";
import { useMessages, useT } from "../../lib/i18n.js";
import { useApprovals, useProjects, useTasks } from "../../lib/queries.js";
import { workbenchMessages } from "../../layouts/messages.js";
import { statusTone } from "../../layouts/ThreadList.js";
import { TaskComposer } from "./TaskComposer.js";

type AttentionKind = "approval" | "blocked" | "review" | "failed";

const ATTENTION: Record<AttentionKind, { icon: LucideIcon; chip: string }> = {
  approval: { icon: ShieldQuestion, chip: "bg-warning-subtle text-warning" },
  blocked: { icon: Ban, chip: "bg-warning-subtle text-warning" },
  review: { icon: GitPullRequest, chip: "bg-info-subtle text-info" },
  failed: { icon: XCircle, chip: "bg-destructive-subtle text-destructive" }
};

const MAX_ATTENTION = 5;

/**
 * Home: say what to work on, see where everything stands in the workflow, and pick up whatever is
 * waiting for you.
 */
export default function HomePage() {
  const m = useMessages(workbenchMessages);
  const t = useT();
  const openStage = useOpenWorkflowStage();
  const tasks = useTasks(null);
  const projects = useProjects();
  const approvals = useApprovals(true);

  const all = useMemo(() => tasks.data ?? [], [tasks.data]);
  const byRecency = useMemo(() => [...all].sort((a, b) => b.updated_at.localeCompare(a.updated_at)), [all]);
  const approvalIds = useMemo(() => new Set((approvals.data ?? []).map((a) => a.task_id)), [approvals.data]);
  const projectName = useMemo(() => new Map((projects.data ?? []).map((p) => [p.id, p.name] as const)), [projects.data]);
  const running = all.filter((x) => x.status === "running").length;
  const review = all.filter((x) => x.status === "human_review").length;
  // Unknown (not zero) until the approvals are loaded, so a failed request never reads as "nothing waiting".
  const waiting: number | null = approvals.data ? approvals.data.length : null;
  const approvalsFailed = approvals.isError && !approvals.data;

  const counts: Record<FlowStage, number | null> | null = tasks.data
    ? {
        queued: all.filter((x) => STAGE_STATUSES.queued.includes(x.status)).length,
        working: running,
        needsYou: waiting,
        review,
        done: all.filter((x) => STAGE_STATUSES.done.includes(x.status)).length
      }
    : null;

  const attention = useMemo(() => {
    const items: { task: Task; kind: AttentionKind }[] = [];
    for (const task of byRecency) {
      if (approvalIds.has(task.id)) items.push({ task, kind: "approval" });
      else if (task.status === "blocked") items.push({ task, kind: "blocked" });
      else if (task.status === "human_review") items.push({ task, kind: "review" });
      else if (task.status === "failed") items.push({ task, kind: "failed" });
    }
    const order: Record<AttentionKind, number> = { approval: 0, blocked: 1, review: 2, failed: 3 };
    return items.sort((a, b) => order[a.kind] - order[b.kind]);
  }, [byRecency, approvalIds]);

  const recent = byRecency.slice(0, 6);

  const attentionText: Record<AttentionKind, string> = {
    approval: m.home.attentionApproval,
    blocked: m.home.attentionBlocked,
    review: m.home.attentionReview,
    failed: m.home.attentionFailed
  };

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex min-h-full w-full max-w-3xl flex-col gap-9 px-6 pb-12 pt-[min(14vh,7rem)]">
        <header className="space-y-2 text-center">
          <h1 className="font-display text-[2rem] font-semibold leading-tight tracking-tight text-foreground">{m.home.title}</h1>
          <p className="text-[15px] text-muted-foreground">{m.home.subtitle}</p>
          {running + (waiting ?? 0) + review > 0 ? (
            <p className="flex flex-wrap items-center justify-center gap-x-4 gap-y-1 pt-1 text-xs text-muted-foreground">
              {running > 0 ? <span>{formatMessage(m.home.statusRunning, { count: running })}</span> : null}
              {waiting ? <span className="text-warning">{formatMessage(m.home.statusApproval, { count: waiting })}</span> : null}
              {review > 0 ? <span>{formatMessage(m.home.statusReview, { count: review })}</span> : null}
            </p>
          ) : null}
        </header>

        <TaskComposer variant="hero" />

        <section aria-label={m.flow.label} className="space-y-2.5">
          <h2 className="px-1 text-[11px] font-semibold uppercase tracking-wider text-subtle-foreground">{m.flow.label}</h2>
          <WorkflowPipeline counts={counts} onOpen={openStage} />
        </section>

        {attention.length > 0 || approvalsFailed ? (
          <section aria-label={m.home.attention} className="space-y-2.5">
            <h2 className="px-1 text-[11px] font-semibold uppercase tracking-wider text-subtle-foreground">{m.home.attention}</h2>
            {approvalsFailed ? (
              <p role="status" className="px-1 text-xs text-warning">
                {m.sidebar.approvalsUnavailable}
              </p>
            ) : null}
            <ul className="divide-y divide-border overflow-hidden rounded-2xl border border-border bg-surface shadow-card" role="list">
              {attention.slice(0, MAX_ATTENTION).map(({ task, kind }) => {
                const Icon = ATTENTION[kind].icon;
                return (
                  <li key={task.id}>
                    <Link
                      to={paths.run(task.id)}
                      className="group flex min-w-0 items-center gap-3 px-3.5 py-3 outline-none transition-colors hover:bg-surface-hover focus-visible:bg-surface-hover"
                    >
                      <span aria-hidden className={cn("grid size-8 shrink-0 place-items-center rounded-lg", ATTENTION[kind].chip)}>
                        <Icon className="size-4" />
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-sm font-medium text-foreground">{task.issue.title}</span>
                        <span className="block truncate text-xs text-muted-foreground">
                          {attentionText[kind]} · <span className="font-mono">{task.issue.identifier}</span>
                        </span>
                      </span>
                      <ArrowRight aria-hidden className="size-4 shrink-0 text-subtle-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-foreground" />
                    </Link>
                  </li>
                );
              })}
            </ul>
          </section>
        ) : null}

        <section aria-label={m.home.recent} className="space-y-2.5">
          <div className="flex items-center gap-2 px-1">
            <h2 className="flex-1 text-[11px] font-semibold uppercase tracking-wider text-subtle-foreground">{m.home.recent}</h2>
            {all.length > recent.length ? (
              <Link to={paths.tasks()} className="rounded text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60">
                {m.home.viewAll}
              </Link>
            ) : null}
          </div>
          {recent.length === 0 ? (
            <p className="rounded-2xl border border-dashed border-border px-4 py-6 text-center text-sm text-muted-foreground">{m.home.noRecent}</p>
          ) : (
            <ul className="divide-y divide-border overflow-hidden rounded-2xl border border-border bg-surface shadow-card" role="list">
              {recent.map((task) => {
                const needs = approvalIds.has(task.id);
                const { tone, pulse } = statusTone(task.status, needs);
                return (
                  <li key={task.id}>
                    <Link
                      to={paths.run(task.id)}
                      className="flex min-w-0 items-center gap-3 px-4 py-3 text-sm outline-none transition-colors hover:bg-surface-hover focus-visible:bg-surface-hover"
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
