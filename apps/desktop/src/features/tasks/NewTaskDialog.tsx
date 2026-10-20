import React, { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import {
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Dropdown,
  Field,
  Input,
  MessageBar,
  MessageBarBody,
  MessageBarTitle,
  Option,
  Spinner,
  Switch,
  Textarea
} from "@fluentui/react-components";
import { errorMessage } from "@symphony/api-client";
import { formatMessage, type CreateTaskRequest, type ProviderKind } from "@symphony/shared-types";
import { LoadingRows, useNotify } from "@symphony/ui";
import { paths } from "../../app/routes.js";
import { useMessages, useT } from "../../lib/i18n.js";
import { useCreateTask, useProjects, useProviders } from "../../lib/queries.js";
import { useUiStore } from "../../store/uiStore.js";
import { ProviderModelFields } from "../projects/ProviderModelFields.js";
import { taskMessages } from "./messages.js";
import { PRIORITY_VALUES, parseLabels } from "./taskLogic.js";

const NONE = "__none__";

/** Create a manual task (optionally dispatching it immediately). Mounted by the shell. */
export default function NewTaskDialog() {
  const m = useMessages(taskMessages);
  const t = useT();
  const notify = useNotify();
  const navigate = useNavigate();
  const activeProjectId = useUiStore((s) => s.activeProjectId);
  const setNewTaskOpen = useUiStore((s) => s.setNewTaskOpen);
  const setAddProjectOpen = useUiStore((s) => s.setAddProjectOpen);
  const inspect = useUiStore((s) => s.inspect);
  const projects = useProjects();
  const createTask = useCreateTask();

  const [projectId, setProjectId] = useState<string | null>(null);
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [identifier, setIdentifier] = useState("");
  const [priority, setPriority] = useState<number | null>(null);
  const [providerOverride, setProviderOverride] = useState<ProviderKind | null>(null);
  const [model, setModel] = useState("");
  const [labels, setLabels] = useState("");
  const [autoStart, setAutoStart] = useState(false);
  const [submitted, setSubmitted] = useState(false);

  const projectList = useMemo(() => projects.data ?? [], [projects.data]);
  const project =
    projectList.find((p) => p.id === projectId) ??
    (projectId === null ? (projectList.find((p) => p.id === activeProjectId) ?? projectList[0]) : undefined);
  const provider: ProviderKind = providerOverride ?? project?.preferred_provider ?? "codex";
  const fallbackModel = project && provider === project.preferred_provider ? project.preferred_model : null;
  // The model shown as the placeholder is the one submitted when the field is left blank, so an
  // overridden provider never inherits the project's model for a different provider.
  const providerStatuses = useProviders();
  const placeholderModel =
    fallbackModel ?? providerStatuses.data?.find((p) => p.provider === provider)?.default_model ?? null;

  const titleError = submitted && !title.trim() ? m.dialog.titleRequired : undefined;
  const projectError = submitted && !project ? m.dialog.projectRequired : undefined;

  const close = () => {
    if (!createTask.isPending) setNewTaskOpen(false);
  };

  const submit = () => {
    setSubmitted(true);
    if (createTask.isPending || !project || !title.trim()) return;
    const input: CreateTaskRequest = {
      project_id: project.id,
      title: title.trim(),
      description: description.trim() || null,
      identifier: identifier.trim() || null,
      priority,
      provider,
      model: model.trim() || placeholderModel,
      labels: parseLabels(labels),
      auto_start: autoStart
    };
    createTask.mutate(input, {
      onSuccess: (task) => {
        setNewTaskOpen(false);
        notify("success", formatMessage(autoStart ? m.dialog.createdStarted : m.dialog.created, { identifier: task.issue.identifier }));
        inspect({ kind: "task", id: task.id });
        if (autoStart) navigate(paths.run(task.id));
      }
    });
  };

  const priorityText = (value: number | null) =>
    value === null ? m.priority.none : `${formatMessage(m.priority.label, { value })} · ${m.priority[`p${value}` as "p1" | "p2" | "p3" | "p4"]}`;

  let body: React.ReactNode;
  if (projects.isLoading) {
    body = <LoadingRows rows={4} />;
  } else if (projectList.length === 0) {
    body = (
      <MessageBar intent="info" layout="multiline">
        <MessageBarBody>{m.dialog.noProjects}</MessageBarBody>
      </MessageBar>
    );
  } else {
    body = (
      <div className="flex flex-col gap-3">
        {createTask.isError ? (
          <MessageBar intent="error" layout="multiline">
            <MessageBarBody>
              <MessageBarTitle>{m.dialog.error}</MessageBarTitle>
              {errorMessage(createTask.error)}
            </MessageBarBody>
          </MessageBar>
        ) : null}
        <Field label={m.dialog.project} required validationMessage={projectError} validationState={projectError ? "error" : "none"}>
          <Dropdown
            placeholder={m.dialog.projectPlaceholder}
            value={project?.name ?? ""}
            selectedOptions={project ? [project.id] : []}
            onOptionSelect={(_, data) => {
              if (!data.optionValue) return;
              setProjectId(data.optionValue);
              setProviderOverride(null);
              setModel("");
            }}
          >
            {projectList.map((p) => (
              <Option key={p.id} value={p.id} text={p.name}>
                <span className="flex w-full items-center gap-2">
                  <span className="flex-1 truncate">{p.name}</span>
                  <span className="truncate font-mono text-xs text-muted-foreground">{p.default_branch}</span>
                </span>
              </Option>
            ))}
          </Dropdown>
        </Field>
        <Field label={m.dialog.taskTitle} required validationMessage={titleError} validationState={titleError ? "error" : "none"}>
          <Input autoFocus value={title} placeholder={m.dialog.titlePlaceholder} onChange={(_, d) => setTitle(d.value)} />
        </Field>
        <Field label={m.dialog.description}>
          <Textarea
            value={description}
            placeholder={m.dialog.descriptionPlaceholder}
            onChange={(_, d) => setDescription(d.value)}
            rows={5}
            resize="vertical"
          />
        </Field>
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <Field label={m.dialog.identifier} hint={m.dialog.identifierHint}>
            <Input className="font-mono" value={identifier} placeholder={m.dialog.identifierPlaceholder} onChange={(_, d) => setIdentifier(d.value)} />
          </Field>
          <Field label={m.dialog.priority}>
            <Dropdown
              value={priorityText(priority)}
              selectedOptions={[priority === null ? NONE : String(priority)]}
              onOptionSelect={(_, data) => setPriority(data.optionValue && data.optionValue !== NONE ? Number(data.optionValue) : null)}
            >
              <Option value={NONE}>{m.priority.none}</Option>
              {PRIORITY_VALUES.map((value) => (
                <Option key={value} value={String(value)} text={priorityText(value)}>
                  {priorityText(value)}
                </Option>
              ))}
            </Dropdown>
          </Field>
        </div>
        <ProviderModelFields
          provider={provider}
          onProviderChange={(next) => {
            setProviderOverride(next);
            setModel("");
          }}
          model={model}
          onModelChange={setModel}
          fallbackModel={fallbackModel}
          providerLabel={m.dialog.provider}
          modelLabel={m.dialog.model}
        />
        <Field label={m.dialog.labels} hint={m.dialog.labelsHint}>
          <Input value={labels} placeholder={m.dialog.labelsPlaceholder} onChange={(_, d) => setLabels(d.value)} />
        </Field>
        <Field hint={m.dialog.autoStartHint}>
          <Switch checked={autoStart} onChange={(_, d) => setAutoStart(d.checked)} label={m.dialog.autoStart} />
        </Field>
      </div>
    );
  }

  const canSubmit = projectList.length > 0 && !createTask.isPending;

  return (
    <Dialog open onOpenChange={(_, data) => !data.open && close()}>
      <DialogSurface aria-describedby={undefined} className="!max-w-[640px]">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            submit();
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
              e.preventDefault();
              submit();
            }
          }}
          noValidate
        >
          <DialogBody>
            <DialogTitle>{m.dialog.title}</DialogTitle>
            <DialogContent>{body}</DialogContent>
            <DialogActions fluid className="items-center">
              {projectList.length > 0 ? (
                <span className="me-auto hidden text-xs text-muted-foreground sm:inline">{m.dialog.shortcut}</span>
              ) : null}
              <Button appearance="secondary" disabled={createTask.isPending} onClick={close}>
                {t.common.cancel}
              </Button>
              {projectList.length === 0 && !projects.isLoading ? (
                <Button
                  appearance="primary"
                  onClick={() => {
                    setNewTaskOpen(false);
                    setAddProjectOpen(true);
                  }}
                >
                  {m.dialog.addRepository}
                </Button>
              ) : (
                <Button
                  appearance="primary"
                  type="submit"
                  disabled={!canSubmit}
                  icon={createTask.isPending ? <Spinner size="tiny" /> : undefined}
                >
                  {createTask.isPending ? m.dialog.submitting : autoStart ? m.dialog.submitAndStart : m.dialog.submit}
                </Button>
              )}
            </DialogActions>
          </DialogBody>
        </form>
      </DialogSurface>
    </Dialog>
  );
}
