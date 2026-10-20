import React, { useState } from "react";
import {
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  Input,
  MessageBar,
  MessageBarBody,
  MessageBarTitle,
  Spinner
} from "@fluentui/react-components";
import { errorMessage, isTauri, nativeHost } from "@symphony/api-client";
import { formatMessage, type RepositoryValidation } from "@symphony/shared-types";
import { cn, useNotify } from "@symphony/ui";
import { CheckCircle2, CircleAlert, FolderOpen, XCircle } from "lucide-react";
import { useMessages } from "../../lib/i18n.js";
import { useCreateProject, useSettings, useValidateRepository } from "../../lib/queries.js";
import { useUiStore } from "../../store/uiStore.js";
import { projectMessages } from "./messages.js";
import { ProjectFormFields } from "./ProjectFormFields.js";
import { emptyProjectForm, formToCreate, hasErrors, validateProjectForm, type ProjectFormState } from "./projectLogic.js";

type Step = 1 | 2 | 3;

const CheckRow: React.FC<{ label: string; ok: boolean; value?: React.ReactNode; okLabel: string; failLabel: string; warnOnly?: boolean }> = ({
  label,
  ok,
  value,
  okLabel,
  failLabel,
  warnOnly
}) => (
  <li className="flex items-center gap-2 py-1 text-sm">
    {ok ? (
      <CheckCircle2 className="size-4 shrink-0 text-success" aria-label={okLabel} />
    ) : warnOnly ? (
      <CircleAlert className="size-4 shrink-0 text-warning" aria-label={failLabel} />
    ) : (
      <XCircle className="size-4 shrink-0 text-destructive" aria-label={failLabel} />
    )}
    <span className="flex-1 text-foreground">{label}</span>
    {value !== undefined ? <span className="font-mono text-xs text-muted-foreground">{value}</span> : null}
  </li>
);

/** Guided "add repository" wizard: choose folder → validate on the runtime → configure → create. */
export default function AddRepositoryDialog() {
  const m = useMessages(projectMessages);
  const notify = useNotify();
  const setAddProjectOpen = useUiStore((s) => s.setAddProjectOpen);
  const setActiveProject = useUiStore((s) => s.setActiveProject);
  const inspect = useUiStore((s) => s.inspect);
  const settings = useSettings();
  const validate = useValidateRepository();
  const create = useCreateProject();
  const native = isTauri();

  const [step, setStep] = useState<Step>(1);
  const [path, setPath] = useState("");
  const [pathTouched, setPathTouched] = useState(false);
  const [browseError, setBrowseError] = useState<string | null>(null);
  const [browsing, setBrowsing] = useState(false);
  const [validation, setValidation] = useState<RepositoryValidation | null>(null);
  const [workspaceRoot, setWorkspaceRoot] = useState("");
  const [form, setForm] = useState<ProjectFormState | null>(null);
  const [showErrors, setShowErrors] = useState(false);

  const pending = validate.isPending || create.isPending;
  const close = () => {
    if (!create.isPending) setAddProjectOpen(false);
  };

  const runValidation = (target: string) => {
    setPathTouched(true);
    const trimmed = target.trim();
    if (!trimmed || validate.isPending) return;
    validate.mutate(trimmed, {
      onSuccess: (result) => {
        if (result.repo_path !== validation?.repo_path) setForm(null);
        setValidation(result);
        setStep(2);
      }
    });
  };

  const browse = async () => {
    setBrowseError(null);
    setBrowsing(true);
    try {
      const picked = await nativeHost.pickRepositoryFolder();
      if (picked) {
        setPath(picked);
        validate.reset();
        runValidation(picked);
      }
    } catch (error) {
      setBrowseError(errorMessage(error));
    } finally {
      setBrowsing(false);
    }
  };

  const toConfigure = () => {
    if (!validation?.is_git_repository) return;
    if (!form) {
      const provider = settings.data?.default_provider ?? "codex";
      const fallbackName = validation.repo_path.split(/[\\/]/).filter(Boolean).pop() ?? "";
      setForm(emptyProjectForm(provider, validation.suggested_name ?? fallbackName, validation.current_branch ?? ""));
    }
    setStep(3);
  };

  const errors = form ? validateProjectForm(form) : {};
  const submit = () => {
    setShowErrors(true);
    if (!form || !validation || create.isPending || hasErrors(errors)) return;
    create.mutate(formToCreate(validation.repo_path, workspaceRoot, form), {
      onSuccess: (project) => {
        notify("success", formatMessage(m.add.created, { name: project.name }));
        setActiveProject(project.id);
        inspect({ kind: "project", id: project.id });
        setAddProjectOpen(false);
      }
    });
  };

  const steps: { n: Step; label: string }[] = [
    { n: 1, label: m.add.stepFolder },
    { n: 2, label: m.add.stepValidate },
    { n: 3, label: m.add.stepConfigure }
  ];

  const pathError = pathTouched && !path.trim() ? m.add.pathRequired : undefined;

  let body: React.ReactNode;
  if (step === 1) {
    body = (
      <div className="flex flex-col gap-3">
        <p className="text-sm text-muted-foreground">{m.add.folderIntro}</p>
        <Field
          label={m.add.path}
          required
          hint={native ? undefined : m.add.pathHint}
          validationMessage={pathError}
          validationState={pathError ? "error" : "none"}
        >
          <div className="flex gap-2">
            <Input
              autoFocus
              className="flex-1 font-mono"
              value={path}
              placeholder={m.add.pathPlaceholder}
              disabled={validate.isPending}
              onChange={(_, d) => {
                setPath(d.value);
                if (validate.isError) validate.reset();
              }}
            />
            {native ? (
              <Button
                icon={browsing ? <Spinner size="tiny" /> : <FolderOpen className="size-4" aria-hidden />}
                disabled={browsing || validate.isPending}
                onClick={() => void browse()}
              >
                {m.add.browse}
              </Button>
            ) : null}
          </div>
        </Field>
        {browseError ? (
          <MessageBar intent="error" layout="multiline">
            <MessageBarBody>
              <MessageBarTitle>{m.add.browseError}</MessageBarTitle>
              {browseError}
            </MessageBarBody>
          </MessageBar>
        ) : null}
        {validate.isError ? (
          <MessageBar intent="error" layout="multiline">
            <MessageBarBody>
              <MessageBarTitle>{m.add.validateError}</MessageBarTitle>
              {errorMessage(validate.error)}
            </MessageBarBody>
          </MessageBar>
        ) : null}
      </div>
    );
  } else if (step === 2 && validation) {
    body = (
      <div className="flex flex-col gap-3">
        <p className="break-all font-mono text-xs text-muted-foreground">{validation.repo_path}</p>
        <section aria-label={m.add.checks} className="rounded-md border border-border bg-surface-sunken px-3 py-1.5">
          <ul>
            <CheckRow label={m.add.checkExists} ok={validation.exists} okLabel={m.add.passed} failLabel={m.add.failed} />
            <CheckRow label={m.add.checkDirectory} ok={validation.is_directory} okLabel={m.add.passed} failLabel={m.add.failed} />
            <CheckRow label={m.add.checkGit} ok={validation.is_git_repository} okLabel={m.add.passed} failLabel={m.add.failed} />
            {validation.is_git_repository ? (
              <CheckRow
                label={m.add.checkBranch}
                ok={Boolean(validation.current_branch)}
                warnOnly
                value={validation.current_branch ?? m.add.detachedBranch}
                okLabel={m.add.passed}
                failLabel={m.add.failed}
              />
            ) : null}
            {validation.is_git_repository ? (
              <CheckRow
                label={m.add.checkWorkflow}
                ok={validation.has_workflow_file}
                warnOnly
                okLabel={m.add.passed}
                failLabel={m.add.failed}
              />
            ) : null}
          </ul>
        </section>
        {validation.errors.length > 0 ? (
          <MessageBar intent="error" layout="multiline">
            <MessageBarBody>
              <MessageBarTitle>{validation.is_git_repository ? m.add.validateError : m.add.notGit}</MessageBarTitle>
              <ul className="list-disc pl-4">
                {validation.errors.map((e) => (
                  <li key={e}>{e}</li>
                ))}
              </ul>
            </MessageBarBody>
          </MessageBar>
        ) : !validation.is_git_repository ? (
          <MessageBar intent="error" layout="multiline">
            <MessageBarBody>{m.add.notGit}</MessageBarBody>
          </MessageBar>
        ) : null}
        {validation.is_git_repository && !validation.has_workflow_file ? (
          <MessageBar intent="warning" layout="multiline">
            <MessageBarBody>{m.add.workflowMissing}</MessageBarBody>
          </MessageBar>
        ) : null}
      </div>
    );
  } else if (step === 3 && form && validation) {
    body = (
      <div className="flex flex-col gap-3">
        <p className="break-all text-xs text-muted-foreground">{formatMessage(m.add.suggestedFrom, { path: validation.repo_path })}</p>
        {create.isError ? (
          <MessageBar intent="error" layout="multiline" role="alert">
            <MessageBarBody>
              <MessageBarTitle>{m.add.createError}</MessageBarTitle>
              {errorMessage(create.error)}
            </MessageBarBody>
          </MessageBar>
        ) : null}
        <ProjectFormFields
          form={form}
          onChange={(patch) => setForm((prev) => (prev ? { ...prev, ...patch } : prev))}
          errors={errors}
          showErrors={showErrors}
          disabled={create.isPending}
          branchHint={validation.current_branch ? m.add.branchHint : undefined}
          generalExtra={
            <Field label={m.form.workspaceRoot} hint={m.form.workspaceRootHint}>
              <Input
                className="font-mono"
                disabled={create.isPending}
                value={workspaceRoot}
                placeholder={m.form.workspaceRootPlaceholder}
                onChange={(_, d) => setWorkspaceRoot(d.value)}
              />
            </Field>
          }
        />
      </div>
    );
  }

  let actions: React.ReactNode;
  if (step === 1) {
    actions = (
      <>
        <Button onClick={close}>{m.add.cancel}</Button>
        <Button
          type="submit"
          appearance="primary"
          disabled={validate.isPending}
          icon={validate.isPending ? <Spinner size="tiny" /> : undefined}
        >
          {validate.isPending ? m.add.validating : m.add.validate}
        </Button>
      </>
    );
  } else if (step === 2) {
    actions = (
      <>
        <Button
          onClick={() => {
            validate.reset();
            setStep(1);
          }}
        >
          {validation?.is_git_repository ? m.add.back : m.add.changePath}
        </Button>
        <Button type="submit" appearance="primary" disabled={!validation?.is_git_repository}>
          {m.add.continue}
        </Button>
      </>
    );
  } else {
    actions = (
      <>
        <Button disabled={create.isPending} onClick={() => setStep(2)}>
          {m.add.back}
        </Button>
        <Button
          type="submit"
          appearance="primary"
          disabled={create.isPending}
          icon={create.isPending ? <Spinner size="tiny" /> : undefined}
        >
          {create.isPending ? m.add.creating : m.add.create}
        </Button>
      </>
    );
  }

  return (
    <Dialog open onOpenChange={(_, data) => !data.open && close()}>
      <DialogSurface aria-describedby={undefined} className="!max-w-[640px]">
        <form
          noValidate
          onSubmit={(e) => {
            e.preventDefault();
            if (step === 1) runValidation(path);
            else if (step === 2) toConfigure();
            else submit();
          }}
        >
          <DialogBody>
            <DialogTitle>{m.add.title}</DialogTitle>
            <DialogContent>
              <div className="flex flex-col gap-4">
                <nav aria-label={m.add.progress}>
                  <ol className="flex items-center gap-2 text-xs">
                    {steps.map((s, i) => (
                      <li key={s.n} className="flex items-center gap-2" aria-current={s.n === step ? "step" : undefined}>
                        {i > 0 ? <span className="h-px w-6 bg-border" aria-hidden /> : null}
                        <span
                          className={cn(
                            "inline-flex size-5 items-center justify-center rounded-full border text-[11px] font-semibold",
                            s.n === step && "border-accent bg-accent text-accent-foreground",
                            s.n < step && "border-accent text-accent",
                            s.n > step && "border-border text-muted-foreground"
                          )}
                          aria-hidden
                        >
                          {s.n}
                        </span>
                        <span className={s.n === step ? "font-medium text-foreground" : "text-muted-foreground"}>{s.label}</span>
                      </li>
                    ))}
                  </ol>
                  <span className="sr-only">{formatMessage(m.add.steps, { step })}</span>
                </nav>
                {body}
              </div>
            </DialogContent>
            <DialogActions>{actions}</DialogActions>
          </DialogBody>
        </form>
        {pending ? <span className="sr-only" aria-live="polite">{validate.isPending ? m.add.validating : m.add.creating}</span> : null}
      </DialogSurface>
    </Dialog>
  );
}
