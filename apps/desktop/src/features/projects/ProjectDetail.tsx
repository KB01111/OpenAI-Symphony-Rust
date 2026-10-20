import React, { useState } from "react";
import { useNavigate } from "react-router-dom";
import {
  Badge,
  Button,
  MessageBar,
  MessageBarBody,
  MessageBarTitle,
  Spinner,
  Tooltip
} from "@fluentui/react-components";
import { errorMessage } from "@symphony/api-client";
import { formatMessage, type Project } from "@symphony/shared-types";
import { ConfirmDialog, KeyValueList, ProviderBadge, TimeAgo, useNotify } from "@symphony/ui";
import { CheckCircle2, KanbanSquare, Plus, Trash2, X } from "lucide-react";
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
    <form onSubmit={submit} aria-label={m.detail.settings} className="flex flex-col gap-4" noValidate>
      {update.isError ? (
        <MessageBar intent="error" layout="multiline">
          <MessageBarBody>
            <MessageBarTitle>{m.form.saveError}</MessageBarTitle>
            {errorMessage(update.error)}
          </MessageBarBody>
        </MessageBar>
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
      <div className="sticky bottom-0 -mx-4 flex items-center gap-2 border-t border-border bg-surface px-4 py-2">
        {dirty ? <span className="flex-1 text-xs text-warning">{m.form.unsaved}</span> : <span className="flex-1" />}
        <Button type="button" size="small" disabled={!dirty || update.isPending} onClick={reset}>
          {m.form.reset}
        </Button>
        <Button
          type="submit"
          size="small"
          appearance="primary"
          disabled={!dirty || update.isPending}
          icon={update.isPending ? <Spinner size="tiny" /> : undefined}
        >
          {update.isPending ? m.form.saving : m.form.save}
        </Button>
      </div>
    </form>
  );
};

/** Right-hand detail/edit panel for the selected project. */
export const ProjectDetail: React.FC<{ project: Project; counts: ProjectTaskCounts | undefined }> = ({ project, counts = EMPTY_COUNTS }) => {
  const m = useMessages(projectMessages);
  const t = useT();
  const notify = useNotify();
  const navigate = useNavigate();
  const activeProjectId = useUiStore((s) => s.activeProjectId);
  const setActiveProject = useUiStore((s) => s.setActiveProject);
  const setNewTaskOpen = useUiStore((s) => s.setNewTaskOpen);
  const inspect = useUiStore((s) => s.inspect);
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
        const inspector = useUiStore.getState().inspector;
        if (inspector?.kind === "project" && inspector.id === project.id) inspect(null);
        navigate(paths.projects());
      }
    });
  };

  const breakdown = [
    { label: formatMessage(m.counts.running, { count: counts.running }), tone: "informative" as const, show: counts.running > 0 },
    { label: formatMessage(m.counts.review, { count: counts.review }), tone: "warning" as const, show: counts.review > 0 },
    { label: formatMessage(m.counts.failed, { count: counts.failed }), tone: "danger" as const, show: counts.failed > 0 }
  ];

  return (
    <section aria-label={m.detail.label} className="flex h-full min-h-0 flex-col border-l border-border bg-surface">
      <header className="flex shrink-0 items-center gap-2 border-b border-border px-4 py-2">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h2 className="truncate font-display text-sm font-semibold text-foreground">{project.name}</h2>
            {isActive ? (
              <Badge size="small" appearance="tint" color="brand" icon={<CheckCircle2 className="size-3" aria-hidden />}>
                {m.detail.active}
              </Badge>
            ) : null}
          </div>
          <p className="truncate font-mono text-xs text-muted-foreground" title={project.repo_path}>
            {project.repo_path}
          </p>
        </div>
        <Tooltip content={m.detail.close} relationship="label">
          <Button size="small" appearance="subtle" icon={<X className="size-4" aria-hidden />} onClick={() => navigate(paths.projects())} />
        </Tooltip>
      </header>

      <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-border px-4 py-2">
        {!isActive ? (
          <Button size="small" onClick={() => setActiveProject(project.id)}>
            {m.detail.setActive}
          </Button>
        ) : null}
        <Button
          size="small"
          icon={<Plus className="size-4" aria-hidden />}
          onClick={() => {
            setActiveProject(project.id);
            setNewTaskOpen(true);
          }}
        >
          {m.detail.newTask}
        </Button>
        <Button
          size="small"
          icon={<KanbanSquare className="size-4" aria-hidden />}
          onClick={() => {
            setActiveProject(project.id);
            navigate(paths.tasks());
          }}
        >
          {m.detail.viewTasks}
        </Button>
        <span className="flex-1" />
        <Button
          size="small"
          appearance="subtle"
          icon={<Trash2 className="size-4" aria-hidden />}
          className="!text-destructive"
          onClick={() => {
            deleteProject.reset();
            setConfirmOpen(true);
          }}
        >
          {m.detail.delete}
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-auto px-4 pt-3">
        <div className="flex flex-col gap-4">
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
                    <Badge key={b.label} size="small" appearance="tint" color={b.tone}>
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
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{m.detail.settings}</h3>
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
        {counts.running > 0 ? (
          <MessageBar intent="warning" layout="multiline">
            <MessageBarBody>{formatMessage(m.delete.runningWarning, { count: counts.running })}</MessageBarBody>
          </MessageBar>
        ) : null}
        {deleteProject.isError ? (
          <MessageBar intent="error" layout="multiline" role="alert">
            <MessageBarBody>
              <MessageBarTitle>{m.delete.error}</MessageBarTitle>
              {errorMessage(deleteProject.error)}
            </MessageBarBody>
          </MessageBar>
        ) : null}
      </ConfirmDialog>
    </section>
  );
};
