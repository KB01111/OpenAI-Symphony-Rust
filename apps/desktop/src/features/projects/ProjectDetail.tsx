import React, { useState } from "react";
import { useNavigate } from "react-router-dom";
import { errorMessage } from "@symphony/api-client";
import { formatMessage, type Project } from "@symphony/shared-types";
import {
  Alert,
  Badge,
  Button,
  ConfirmDialog,
  KeyValueList,
  ProviderBadge,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  Spinner,
  TimeAgo,
  useNotify
} from "@symphony/ui";
import { CheckCircle2, KanbanSquare, Plus, Trash2 } from "lucide-react";
import { paths } from "../../app/routes.js";
import { useMessages, useT } from "../../lib/i18n.js";
import { useDeleteProject, useUpdateProject } from "../../lib/queries.js";
import { useUiStore } from "../../store/uiStore.js";
import { projectMessages } from "./messages.js";
import { ProjectFormFields, trackerLabel } from "./ProjectFormFields.js";
import {
  formToUpdate,
  hasErrors,
  isDirty,
  projectToForm,
  validateProjectForm,
  type ProjectFormState,
  type ProjectTaskCounts
} from "./projectLogic.js";

const EMPTY_COUNTS: ProjectTaskCounts = { total: 0, running: 0, review: 0, failed: 0, byStatus: {} };

/** Edit form for a project's settings (PATCH /projects/{id} with only the changed fields). */
const ProjectEditForm: React.FC<{ project: Project }> = ({ project }) => {
  const m = useMessages(projectMessages);
  const notify = useNotify();
  const update = useUpdateProject();
  const [form, setForm] = useState<ProjectFormState>(() => projectToForm(project));
  const [showErrors, setShowErrors] = useState(false);
  const errors = validateProjectForm(form);
  const dirty = isDirty(project, form);

  // Another client (or a sync) updated this project: rebase a clean form onto the new values so a
  // later save cannot PATCH stale fields over them. Unsaved edits are kept. Cleanliness is judged
  // against the snapshot the form was based on: compared with the updated project, the remote
  // change itself would look like a local edit.
  const [base, setBase] = useState(project);
  if (project.updated_at !== base.updated_at) {
    setBase(project);
    if (!isDirty(base, form)) setForm(projectToForm(project));
  }

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    setShowErrors(true);
    if (update.isPending || !dirty || hasErrors(errors)) return;
    update.mutate(
      { id: project.id, update: formToUpdate(project, form) },
      {
        onSuccess: (saved) => {
          setForm(projectToForm(saved));
          setShowErrors(false);
          notify("success", formatMessage(m.form.saved, { name: saved.name }));
        }
      }
    );
  };

  const reset = () => {
    setForm(projectToForm(project));
    setShowErrors(false);
    update.reset();
  };

  return (
    <form onSubmit={submit} aria-label={m.detail.settings} className="flex min-w-0 flex-col gap-4" noValidate>
      {update.isError ? (
        <Alert intent="error" title={m.form.saveError}>
          {errorMessage(update.error)}
        </Alert>
      ) : null}
      <ProjectFormFields
        form={form}
        onChange={(patch) => setForm((prev) => ({ ...prev, ...patch }))}
        errors={errors}
        showErrors={showErrors}
        disabled={update.isPending}
        extraTrackerKinds={[project.tracker_kind]}
        sectioned
      />
      <div className="sticky bottom-0 -mx-5 flex items-center gap-2 border-t border-border bg-surface px-5 py-3">
        {dirty ? <span className="flex-1 text-xs text-warning">{m.form.unsaved}</span> : <span className="flex-1" />}
        <Button type="button" size="sm" disabled={!dirty || update.isPending} onClick={reset}>
          {m.form.reset}
        </Button>
        <Button type="submit" size="sm" variant="default" disabled={!dirty || update.isPending}>
          {update.isPending ? <Spinner className="size-3.5 text-current" /> : null}
          {update.isPending ? m.form.saving : m.form.save}
        </Button>
      </div>
    </form>
  );
};

/**
 * Detail/edit content for the selected project, rendered inside a side sheet so the project table
 * keeps its full width. The sheet supplies the dialog, title and close button.
 */
export const ProjectDetail: React.FC<{ project: Project; counts: ProjectTaskCounts | undefined }> = ({ project, counts = EMPTY_COUNTS }) => {
  const m = useMessages(projectMessages);
  const t = useT();
  const notify = useNotify();
  const navigate = useNavigate();
  const activeProjectId = useUiStore((s) => s.activeProjectId);
  const setActiveProject = useUiStore((s) => s.setActiveProject);
  const setNewTaskOpen = useUiStore((s) => s.setNewTaskOpen);
  const deleteProject = useDeleteProject();
  const [confirmOpen, setConfirmOpen] = useState(false);
  const isActive = activeProjectId === project.id;

  const confirmDelete = () => {
    if (deleteProject.isPending) return;
    deleteProject.mutate(project.id, {
      onSuccess: () => {
        setConfirmOpen(false);
        notify("success", formatMessage(m.delete.deleted, { name: project.name }));
        if (useUiStore.getState().activeProjectId === project.id) setActiveProject(null);
        navigate(paths.projects());
      }
    });
  };

  const breakdown = [
    { label: formatMessage(m.counts.running, { count: counts.running }), tone: "info" as const, show: counts.running > 0 },
    { label: formatMessage(m.counts.review, { count: counts.review }), tone: "warning" as const, show: counts.review > 0 },
    { label: formatMessage(m.counts.failed, { count: counts.failed }), tone: "danger" as const, show: counts.failed > 0 }
  ];

  return (
    <section aria-label={m.detail.label} className="flex h-full min-h-0 flex-col">
      <SheetHeader className="shrink-0 gap-1 border-b border-border pr-12">
        <SheetTitle className="flex min-w-0 items-center gap-2">
          <span className="truncate font-display">{project.name}</span>
          {isActive ? (
            <Badge tone="accent">
              <CheckCircle2 aria-hidden />
              {m.detail.active}
            </Badge>
          ) : null}
        </SheetTitle>
        <SheetDescription className="truncate font-mono text-xs" title={project.repo_path}>
          {project.repo_path}
        </SheetDescription>
      </SheetHeader>

      <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-border px-4 py-2">
        {!isActive ? (
          <Button size="sm" onClick={() => setActiveProject(project.id)}>
            {m.detail.setActive}
          </Button>
        ) : null}
        <Button
          size="sm"
          onClick={() => {
            setActiveProject(project.id);
            setNewTaskOpen(true);
          }}
        >
          <Plus />
          {m.detail.newTask}
        </Button>
        <Button
          size="sm"
          onClick={() => {
            setActiveProject(project.id);
            navigate(paths.tasks());
          }}
        >
          <KanbanSquare />
          {m.detail.viewTasks}
        </Button>
        <span className="flex-1" />
        <Button
          size="sm"
          variant="destructive-ghost"
          onClick={() => {
            deleteProject.reset();
            setConfirmOpen(true);
          }}
        >
          <Trash2 />
          {m.detail.delete}
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-5 pt-4">
        <div className="flex min-w-0 flex-col gap-5">
          <div>
            <h3 className="mb-1.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{m.detail.tasks}</h3>
            {counts.total === 0 ? (
              <p className="text-sm text-muted-foreground">{m.detail.noTasks}</p>
            ) : (
              <div className="flex flex-wrap items-center gap-1.5" aria-label={m.counts.breakdown}>
                <span className="text-sm text-foreground">{formatMessage(m.counts.total, { count: counts.total })}</span>
                {breakdown
                  .filter((b) => b.show)
                  .map((b) => (
                    <Badge key={b.label} tone={b.tone}>
                      {b.label}
                    </Badge>
                  ))}
              </div>
            )}
          </div>
          <KeyValueList
            items={[
              { label: m.columns.provider, value: <ProviderBadge provider={project.preferred_provider} model={project.preferred_model} /> },
              { label: m.columns.tracker, value: trackerLabel(m, project.tracker_kind) },
              { label: m.detail.workspaceRoot, value: project.workspace_root, mono: true },
              { label: m.detail.workflowPath, value: project.workflow_path ?? t.common.none, mono: true },
              { label: m.detail.created, value: <TimeAgo iso={project.created_at} /> },
              { label: m.detail.updated, value: <TimeAgo iso={project.updated_at} /> }
            ]}
          />
          <div>
            <h3 className="mb-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{m.detail.settings}</h3>
            <ProjectEditForm key={project.id} project={project} />
          </div>
        </div>
      </div>

      <ConfirmDialog
        open={confirmOpen}
        onOpenChange={(open) => {
          setConfirmOpen(open);
          if (!open) deleteProject.reset();
        }}
        title={formatMessage(m.delete.title, { name: project.name })}
        description={m.delete.body}
        confirmLabel={m.delete.confirm}
        destructive
        pending={deleteProject.isPending}
        onConfirm={confirmDelete}
      >
        {counts.running > 0 ? <Alert intent="warning">{formatMessage(m.delete.runningWarning, { count: counts.running })}</Alert> : null}
        {deleteProject.isError ? (
          <Alert intent="error" title={m.delete.error}>
            {errorMessage(deleteProject.error)}
          </Alert>
        ) : null}
      </ConfirmDialog>
    </section>
  );
};
