import React, { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { ArrowUp, Flag, FolderGit2, GitBranch, Plus, SlidersHorizontal, Tag } from "lucide-react";
import { errorMessage } from "@symphony/api-client";
import { formatMessage, type CreateTaskRequest, type Project, type ProviderKind } from "@symphony/shared-types";
import {
  Alert,
  Button,
  Field,
  Hint,
  Input,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Spinner,
  Textarea,
  ToggleGroup,
  ToggleGroupItem,
  cn,
  useNotify
} from "@symphony/ui";
import { paths } from "../../app/routes.js";
import { useMessages, useT } from "../../lib/i18n.js";
import { useCreateTask, useProjects, useProviders } from "../../lib/queries.js";
import { useUiStore } from "../../store/uiStore.js";
import { workbenchMessages } from "../../layouts/messages.js";
import { ProviderModelFields } from "../projects/ProviderModelFields.js";
import { PRIORITY_VALUES, parseLabels } from "../tasks/taskLogic.js";

const MAX_TITLE = 80;

/** The first meaningful line of the prompt, without markdown markers, as the task title. */
export function deriveTitle(prompt: string): string {
  const line = prompt
    .split(/\r?\n/)
    .map((l) => l.replace(/^[\s>#*\-\d.)]+/, "").trim())
    .find((l) => l.length > 0);
  const base = (line ?? prompt.trim()).replace(/\s+/g, " ");
  if (base.length <= MAX_TITLE) return base;
  const cut = base.slice(0, MAX_TITLE);
  const space = cut.lastIndexOf(" ");
  return `${(space > MAX_TITLE * 0.6 ? cut.slice(0, space) : cut).trimEnd()}…`;
}

const Chip = React.forwardRef<HTMLButtonElement, React.ComponentProps<"button"> & { icon: React.ReactNode; active?: boolean }>(
  ({ icon, active, children, className, ...props }, ref) => (
    <button
      ref={ref}
      type="button"
      className={cn(
        "inline-flex h-7 max-w-full min-w-0 items-center gap-1.5 rounded-full border px-2.5 text-xs font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring/60 [&_svg]:size-3.5 [&_svg]:shrink-0",
        active ? "border-transparent bg-accent-subtle text-accent" : "border-border bg-transparent text-muted-foreground hover:bg-surface-hover hover:text-foreground",
        className
      )}
      {...props}
    >
      {icon}
      <span className="min-w-0 truncate">{children}</span>
    </button>
  )
);
Chip.displayName = "Chip";

export interface TaskComposerProps {
  /** `hero` is the large home composer; `dialog` is the compact modal variant. */
  variant?: "hero" | "dialog";
  onCreated?: () => void;
  autoFocus?: boolean;
  className?: string;
}

/**
 * Codex-style prompt composer tailored to Symphony: one prompt box plus chips for repository
 * (and its base branch), provider/model, priority, labels and whether to run now or queue.
 */
export const TaskComposer: React.FC<TaskComposerProps> = ({ variant = "hero", onCreated, autoFocus = true, className }) => {
  const w = useMessages(workbenchMessages);
  const m = w.composer;
  const t = useT();
  const notify = useNotify();
  const navigate = useNavigate();
  const activeProjectId = useUiStore((s) => s.activeProjectId);
  const setActiveProject = useUiStore((s) => s.setActiveProject);
  const setAddProjectOpen = useUiStore((s) => s.setAddProjectOpen);
  const projects = useProjects();
  const providerStatuses = useProviders();
  const createTask = useCreateTask();
  const textarea = useRef<HTMLTextAreaElement>(null);

  const [prompt, setPrompt] = useState("");
  const [projectId, setProjectId] = useState<string | null>(null);
  const [providerOverride, setProviderOverride] = useState<ProviderKind | null>(null);
  const [model, setModel] = useState("");
  const [priority, setPriority] = useState<number | null>(null);
  const [labels, setLabels] = useState("");
  const [identifier, setIdentifier] = useState("");
  const [runNow, setRunNow] = useState(true);
  const [submitted, setSubmitted] = useState(false);

  const projectList = useMemo(() => projects.data ?? [], [projects.data]);
  const project: Project | undefined =
    projectList.find((p) => p.id === projectId) ??
    (projectId === null ? (projectList.find((p) => p.id === activeProjectId) ?? projectList[0]) : undefined);
  const provider: ProviderKind = providerOverride ?? project?.preferred_provider ?? "codex";
  const fallbackModel = project && provider === project.preferred_provider ? project.preferred_model : null;
  // The model shown as the placeholder is the one submitted when the field is left blank, so an
  // overridden provider never inherits the project's model for a different provider.
  const placeholderModel = fallbackModel ?? providerStatuses.data?.find((p) => p.provider === provider)?.default_model ?? null;
  const effectiveModel = model.trim() || placeholderModel;

  useEffect(() => {
    if (autoFocus) textarea.current?.focus();
  }, [autoFocus]);

  const promptError = submitted && !prompt.trim() ? m.promptRequired : undefined;
  const projectError = submitted && !project ? m.projectRequired : undefined;
  const hero = variant === "hero";

  const submit = () => {
    setSubmitted(true);
    if (createTask.isPending || !project || !prompt.trim()) return;
    const text = prompt.trim();
    const title = deriveTitle(text);
    const input: CreateTaskRequest = {
      project_id: project.id,
      title,
      description: text === title ? null : text,
      identifier: identifier.trim() || null,
      priority,
      provider,
      model: effectiveModel,
      labels: parseLabels(labels),
      auto_start: runNow
    };
    createTask.mutate(input, {
      onSuccess: (task) => {
        setActiveProject(project.id);
        setPrompt("");
        setSubmitted(false);
        notify("success", formatMessage(runNow ? m.createdStarted : m.created, { identifier: task.issue.identifier }));
        onCreated?.();
        navigate(paths.run(task.id));
      }
    });
  };

  const priorityText = (value: number | null) =>
    value === null ? m.priorityNone : `${formatMessage(m.priorityValue, { value })} · ${m.priorityLabels[`p${value}` as "p1" | "p2" | "p3" | "p4"]}`;
  const labelCount = parseLabels(labels).length;

  if (!projects.isLoading && projectList.length === 0) {
    return (
      <Alert intent="info" className={className}>
        <div className="flex flex-wrap items-center gap-3">
          <span className="flex-1">{m.noProjects}</span>
          <Button
            variant="default"
            size="sm"
            onClick={() => {
              useUiStore.getState().setNewTaskOpen(false);
              setAddProjectOpen(true);
            }}
          >
            <Plus />
            {w.sidebar.addRepository}
          </Button>
        </div>
      </Alert>
    );
  }

  return (
    <form
      aria-label={m.dialogTitle}
      className={cn("flex flex-col gap-2", className)}
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
      noValidate
    >
      {createTask.isError ? (
        <Alert intent="error" title={m.error}>
          {errorMessage(createTask.error)}
        </Alert>
      ) : null}
      <div
        className={cn(
          "rounded-2xl border border-border-strong bg-surface shadow-sm transition-shadow focus-within:border-ring focus-within:ring-2 focus-within:ring-ring/30",
          promptError && "border-destructive"
        )}
      >
        <Textarea
          ref={textarea}
          value={prompt}
          aria-label={m.promptLabel}
          aria-invalid={promptError ? true : undefined}
          placeholder={m.placeholder}
          rows={hero ? 3 : 4}
          onChange={(e) => setPrompt(e.target.value)}
          onKeyDown={(e) => {
            if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
              e.preventDefault();
              submit();
            }
          }}
          className={cn(
            "resize-none rounded-2xl border-0 bg-transparent px-4 pb-1 pt-3.5 shadow-none ring-0 [field-sizing:content] focus-visible:ring-0",
            hero ? "min-h-24 max-h-72 text-base" : "min-h-24 max-h-56 text-sm"
          )}
        />
        <div className="flex flex-wrap items-center gap-1.5 px-3 pb-3 pt-1">
          <Popover>
            <PopoverTrigger asChild>
              <Chip icon={<FolderGit2 />} aria-label={`${m.project}: ${project?.name ?? m.projectPlaceholder}`} active={Boolean(projectError)}>
                {project?.name ?? m.projectPlaceholder}
              </Chip>
            </PopoverTrigger>
            <PopoverContent className="w-64">
              <ul role="listbox" aria-label={m.project}>
                {projectList.map((p) => (
                  <li key={p.id} role="option" aria-selected={p.id === project?.id}>
                    <button
                      type="button"
                      onClick={() => {
                        setProjectId(p.id);
                        setProviderOverride(null);
                        setModel("");
                      }}
                      className={cn("flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-start text-sm outline-none hover:bg-surface-hover focus-visible:bg-surface-hover", p.id === project?.id && "bg-surface-selected")}
                    >
                      <span className="min-w-0 flex-1 truncate">{p.name}</span>
                      <span className="shrink-0 font-mono text-xs text-muted-foreground">{p.default_branch}</span>
                    </button>
                  </li>
                ))}
              </ul>
              <button
                type="button"
                onClick={() => {
                  useUiStore.getState().setNewTaskOpen(false);
                  setAddProjectOpen(true);
                }}
                className="mt-1 flex w-full items-center gap-2 rounded-md border-t border-border px-2 py-1.5 text-start text-sm text-muted-foreground outline-none hover:bg-surface-hover hover:text-foreground"
              >
                <Plus className="size-4" />
                {w.sidebar.addRepository}
              </button>
            </PopoverContent>
          </Popover>
          {project ? (
            <Hint label={formatMessage(m.branch, { branch: project.default_branch })}>
              <span className="inline-flex h-7 min-w-0 items-center gap-1.5 rounded-full px-2 font-mono text-xs text-muted-foreground">
                <GitBranch className="size-3.5 shrink-0" aria-hidden />
                <span className="min-w-0 max-w-32 truncate">{project.default_branch}</span>
              </span>
            </Hint>
          ) : null}
          <Popover>
            <PopoverTrigger asChild>
              <Chip icon={<SlidersHorizontal />} aria-label={`${m.provider} / ${m.model}`}>
                {t.provider[provider]}
                {effectiveModel ? ` · ${effectiveModel}` : ""}
              </Chip>
            </PopoverTrigger>
            <PopoverContent className="w-80 p-3">
              <ProviderModelFields
                layout="stacked"
                provider={provider}
                onProviderChange={(next) => {
                  setProviderOverride(next);
                  setModel("");
                }}
                model={model}
                onModelChange={setModel}
                fallbackModel={fallbackModel}
                providerLabel={m.provider}
                modelLabel={m.model}
              />
            </PopoverContent>
          </Popover>
          <Popover>
            <PopoverTrigger asChild>
              <Chip icon={<Flag />} active={priority !== null} aria-label={`${m.priority}: ${priorityText(priority)}`}>
                {priority === null ? m.priority : formatMessage(m.priorityValue, { value: priority })}
              </Chip>
            </PopoverTrigger>
            <PopoverContent className="w-52">
              <ul role="listbox" aria-label={m.priority}>
                {[null, ...PRIORITY_VALUES].map((value) => (
                  <li key={String(value)} role="option" aria-selected={priority === value}>
                    <button
                      type="button"
                      onClick={() => setPriority(value)}
                      className={cn("flex w-full rounded-md px-2 py-1.5 text-start text-sm outline-none hover:bg-surface-hover focus-visible:bg-surface-hover", priority === value && "bg-surface-selected")}
                    >
                      {priorityText(value)}
                    </button>
                  </li>
                ))}
              </ul>
            </PopoverContent>
          </Popover>
          <Popover>
            <PopoverTrigger asChild>
              <Chip icon={<Tag />} active={labelCount > 0} aria-label={m.labels}>
                {labelCount > 0 ? `${m.labels} · ${labelCount}` : m.labels}
              </Chip>
            </PopoverTrigger>
            <PopoverContent className="w-72 space-y-3 p-3">
              <Field label={m.labels} hint={m.labelsHint}>
                <Input value={labels} placeholder={m.labelsPlaceholder} onChange={(e) => setLabels(e.target.value)} />
              </Field>
              <Field label={m.identifier} hint={m.identifierHint}>
                <Input className="font-mono" value={identifier} placeholder={m.identifierPlaceholder} onChange={(e) => setIdentifier(e.target.value)} />
              </Field>
            </PopoverContent>
          </Popover>

          <span className="flex-1" />

          <ToggleGroup type="single" value={runNow ? "run" : "queue"} onValueChange={(v) => v && setRunNow(v === "run")} aria-label={m.modeLabel}>
            <Hint label={m.runNowHint}>
              <ToggleGroupItem value="run">{m.runNow}</ToggleGroupItem>
            </Hint>
            <Hint label={m.queueHint}>
              <ToggleGroupItem value="queue">{m.queue}</ToggleGroupItem>
            </Hint>
          </ToggleGroup>
          <Hint label={`${runNow ? m.submitRun : m.submitQueue} (${m.shortcut})`}>
            <Button
              type="submit"
              variant="default"
              size="icon"
              className="rounded-full"
              disabled={createTask.isPending}
              aria-label={createTask.isPending ? m.submitting : runNow ? m.submitRun : m.submitQueue}
            >
              {createTask.isPending ? <Spinner className="text-current" /> : <ArrowUp />}
            </Button>
          </Hint>
        </div>
      </div>
      {promptError || projectError ? (
        <p role="alert" className="px-2 text-xs text-destructive">
          {promptError ?? projectError}
        </p>
      ) : (
        <p className="px-2 text-xs text-subtle-foreground">{m.shortcut}</p>
      )}
    </form>
  );
};
