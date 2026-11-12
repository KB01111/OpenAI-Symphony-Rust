import React from "react";
import { Check, ChevronRight, CircleCheckBig, Hourglass, Play, ShieldQuestion, type LucideIcon, GitPullRequest } from "lucide-react";
import { formatMessage, type Task, type TaskStatus } from "@symphony/shared-types";
import { cn } from "@symphony/ui";
import { useMessages } from "../lib/i18n.js";
import { workbenchMessages } from "../layouts/messages.js";

/**
 * Symphony's workflow as the user meets it: a task waits, an agent works on it (and may stop to ask
 * for approval), the result is reviewed, and it is done.
 */
export type FlowStage = "queued" | "working" | "needsYou" | "review" | "done";

const STEPS = ["queued", "working", "review", "done"] as const;
type Step = (typeof STEPS)[number];

type StepTone = "accent" | "warning" | "danger" | "success" | "neutral";

interface Position {
  step: Step;
  /** Label of the current step when it is not the plain stage name (needs approval, failed…). */
  label: "needsYou" | "failed" | "cancelled" | null;
  tone: StepTone;
  live: boolean;
}

/** Where a task stands in the workflow. */
export function flowPosition(status: TaskStatus, needsApproval: boolean): Position {
  switch (status) {
    case "backlog":
    case "ready":
    case "queued":
      return { step: "queued", label: null, tone: "neutral", live: false };
    case "running":
      return needsApproval
        ? { step: "working", label: "needsYou", tone: "warning", live: true }
        : { step: "working", label: null, tone: "accent", live: true };
    case "blocked":
      return { step: "working", label: "needsYou", tone: "warning", live: false };
    case "failed":
      return { step: "working", label: "failed", tone: "danger", live: false };
    case "cancelled":
      return { step: "working", label: "cancelled", tone: "neutral", live: false };
    case "human_review":
      return { step: "review", label: null, tone: "accent", live: false };
    case "completed":
      return { step: "done", label: null, tone: "success", live: false };
  }
}

const CURRENT_TONE: Record<StepTone, string> = {
  accent: "border-accent bg-accent text-accent-foreground",
  warning: "border-warning bg-warning text-background",
  danger: "border-destructive bg-destructive text-background",
  success: "border-success bg-success text-background",
  neutral: "border-border-strong bg-muted text-foreground"
};

const CURRENT_TEXT: Record<StepTone, string> = {
  accent: "text-accent",
  warning: "text-warning",
  danger: "text-destructive",
  success: "text-success",
  neutral: "text-foreground"
};

/** A task's progress through the workflow: Queued, Working, Review, Done. */
export const StageStepper: React.FC<{ task: Pick<Task, "status">; needsApproval?: boolean; className?: string }> = ({
  task,
  needsApproval = false,
  className
}) => {
  const f = useMessages(workbenchMessages).flow;
  const position = flowPosition(task.status, needsApproval);
  const currentIndex = STEPS.indexOf(position.step);
  const currentLabel = position.label ? f[position.label] : f[position.step];
  return (
    <ol aria-label={formatMessage(f.progress, { label: currentLabel })} className={cn("flex min-w-0 items-center gap-1.5", className)}>
      {STEPS.map((step, index) => {
        const done = index < currentIndex || (position.step === "done" && index === currentIndex);
        const current = index === currentIndex && !done;
        const label = current ? currentLabel : f[step];
        return (
          <li key={step} aria-current={index === currentIndex ? "step" : undefined} className="flex min-w-0 items-center gap-1.5">
            {index > 0 ? <span aria-hidden className={cn("h-px w-4 shrink-0 @lg:w-6", index <= currentIndex ? "bg-border-strong" : "bg-border")} /> : null}
            <span
              aria-hidden
              className={cn(
                "relative grid size-4 shrink-0 place-items-center rounded-full border text-[9px] font-semibold",
                done ? "border-success bg-success text-background" : current ? CURRENT_TONE[position.tone] : "border-border-strong bg-transparent text-subtle-foreground"
              )}
            >
              {current && position.live ? <span className="absolute inset-0 animate-ping rounded-full bg-current opacity-30" /> : null}
              {done ? <Check className="size-2.5" strokeWidth={3} /> : null}
            </span>
            <span
              className={cn(
                "truncate text-xs",
                current ? cn("font-medium", CURRENT_TEXT[position.tone]) : done ? "text-muted-foreground" : "text-subtle-foreground",
                !current && "hidden @md:inline"
              )}
            >
              {label}
            </span>
          </li>
        );
      })}
    </ol>
  );
};

const STAGE_META: Record<FlowStage, { icon: LucideIcon; hint: "queuedHint" | "workingHint" | "needsYouHint" | "reviewHint" | "doneHint"; tone: string; chip: string }> = {
  queued: { icon: Hourglass, hint: "queuedHint", tone: "text-foreground", chip: "bg-muted text-muted-foreground" },
  working: { icon: Play, hint: "workingHint", tone: "text-accent", chip: "bg-accent-subtle text-accent" },
  needsYou: { icon: ShieldQuestion, hint: "needsYouHint", tone: "text-warning", chip: "bg-warning-subtle text-warning" },
  review: { icon: GitPullRequest, hint: "reviewHint", tone: "text-info", chip: "bg-info-subtle text-info" },
  done: { icon: CircleCheckBig, hint: "doneHint", tone: "text-success", chip: "bg-success-subtle text-success" }
};

export const FLOW_STAGES: ReadonlyArray<FlowStage> = ["queued", "working", "needsYou", "review", "done"];

/** The whole workflow with how much work sits in each stage; every stage opens where it is handled. */
export const WorkflowPipeline: React.FC<{
  counts: Record<FlowStage, number> | null;
  onOpen: (stage: FlowStage) => void;
  className?: string;
}> = ({ counts, onOpen, className }) => {
  const f = useMessages(workbenchMessages).flow;
  return (
    <div className={cn("@container", className)}>
      <ol aria-label={f.label} className="grid grid-cols-2 gap-2 @md:grid-cols-3 @2xl:flex @2xl:items-stretch @2xl:gap-0">
        {FLOW_STAGES.map((stage, index) => {
          const meta = STAGE_META[stage];
          const Icon = meta.icon;
          const count = counts?.[stage] ?? null;
          const active = (count ?? 0) > 0;
          return (
            <li key={stage} className="flex min-w-0 items-center @2xl:flex-1">
              {index > 0 ? <ChevronRight aria-hidden className="mx-0.5 hidden size-4 shrink-0 text-border-strong @2xl:block" /> : null}
              <button
                type="button"
                onClick={() => onOpen(stage)}
                aria-label={formatMessage(f.stage, { label: f[stage], count: count ?? "–" })}
                title={f[meta.hint]}
                className={cn(
                  "group flex w-full min-w-0 items-center gap-2.5 rounded-xl border bg-surface px-3 py-2.5 text-start shadow-card outline-none transition-[border-color,background-color,transform] duration-150 hover:-translate-y-px hover:border-border-strong focus-visible:ring-2 focus-visible:ring-ring/60",
                  stage === "needsYou" && active ? "border-warning/50" : "border-border"
                )}
              >
                <span aria-hidden className={cn("grid size-8 shrink-0 place-items-center rounded-lg", active ? meta.chip : "bg-muted text-subtle-foreground")}>
                  <Icon className="size-4" />
                </span>
                <span aria-hidden className="min-w-0 flex-1">
                  <span className={cn("block font-display text-lg font-semibold leading-tight tabular-nums", active ? meta.tone : "text-subtle-foreground")}>{count ?? "–"}</span>
                  <span className="block truncate text-xs text-muted-foreground">{f[stage]}</span>
                </span>
              </button>
            </li>
          );
        })}
      </ol>
    </div>
  );
};
