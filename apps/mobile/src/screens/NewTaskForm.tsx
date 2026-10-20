import React, { useState } from "react";
import { Button, Dropdown, Field, Input, Option, Spinner, Switch, Textarea } from "@fluentui/react-components";
import { formatMessage } from "@symphony/shared-types";
import { useNotify } from "@symphony/ui";
import { useStrings } from "../i18n.js";
import { useCreateTask, useProjects } from "./queries.js";

/** Create a task (optionally dispatching it at once) from the phone. */
export const NewTaskForm: React.FC<{ onDone: () => void }> = ({ onDone }) => {
  const { m, t } = useStrings();
  const notify = useNotify();
  const projects = useProjects();
  const create = useCreateTask();
  const list = projects.data ?? [];
  const [projectId, setProjectId] = useState<string | null>(null);
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [dispatchNow, setDispatchNow] = useState(false);
  const [submitted, setSubmitted] = useState(false);
  const project = list.find((p) => p.id === projectId) ?? list[0];

  if (projects.isLoading) return <Spinner size="small" label={t.common.loading} />;
  if (list.length === 0) return <p className="text-sm text-muted-foreground">{m.noProjects}</p>;

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    setSubmitted(true);
    if (!project || !title.trim() || create.isPending) return;
    create.mutate(
      { project_id: project.id, title: title.trim(), description: description.trim() || null, auto_start: dispatchNow },
      {
        onSuccess: (task) => {
          notify("success", formatMessage(m.taskCreated, { identifier: task.issue.identifier }));
          onDone();
        },
        onError: (error) => notify("error", m.taskCreateFailed, error.message)
      }
    );
  };

  return (
    <form className="space-y-3 rounded-md border border-border bg-surface p-3" onSubmit={submit} noValidate>
      <Field label={t.mobile.project}>
        <Dropdown
          value={project?.name ?? ""}
          selectedOptions={project ? [project.id] : []}
          onOptionSelect={(_, d) => setProjectId(d.optionValue ?? null)}
        >
          {list.map((p) => (
            <Option key={p.id} value={p.id}>
              {p.name}
            </Option>
          ))}
        </Dropdown>
      </Field>
      <Field
        label={t.mobile.taskTitle}
        required
        validationState={submitted && !title.trim() ? "error" : "none"}
        validationMessage={submitted && !title.trim() ? m.titleRequired : undefined}
      >
        <Input value={title} onChange={(_, d) => setTitle(d.value)} />
      </Field>
      <Field label={t.mobile.taskDescription}>
        <Textarea value={description} onChange={(_, d) => setDescription(d.value)} resize="vertical" />
      </Field>
      <Switch checked={dispatchNow} onChange={(_, d) => setDispatchNow(d.checked)} label={t.mobile.dispatchNow} />
      <div className="flex justify-end gap-2">
        <Button type="button" appearance="subtle" onClick={onDone} disabled={create.isPending}>
          {t.common.cancel}
        </Button>
        <Button type="submit" appearance="primary" disabled={create.isPending} icon={create.isPending ? <Spinner size="tiny" /> : undefined}>
          {t.mobile.createTask}
        </Button>
      </div>
    </form>
  );
};
