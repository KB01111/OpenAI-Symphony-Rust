import React from "react";
import { Dropdown, Field, Input, Option, SpinButton, Switch, Textarea } from "@fluentui/react-components";
import { useMessages } from "../../lib/i18n.js";
import { projectMessages, type ProjectMessages } from "./messages.js";
import { ProviderModelFields } from "./ProviderModelFields.js";
import { TRACKER_KINDS, type ProjectFormErrors, type ProjectFormState } from "./projectLogic.js";

export function trackerLabel(m: ProjectMessages, kind: string): string {
  return (TRACKER_KINDS as ReadonlyArray<string>).includes(kind) ? m.tracker[kind as keyof ProjectMessages["tracker"]] : kind;
}

export interface ProjectFormFieldsProps {
  form: ProjectFormState;
  onChange: (patch: Partial<ProjectFormState>) => void;
  /** Errors are only shown once the user attempted to submit. */
  errors: ProjectFormErrors;
  showErrors: boolean;
  disabled?: boolean;
  /** Extra tracker kinds to offer (e.g. a custom kind already stored on the project). */
  extraTrackerKinds?: ReadonlyArray<string>;
  /** Optional content rendered after the general section (e.g. the workspace root input). */
  generalExtra?: React.ReactNode;
  branchHint?: string;
  sectioned?: boolean;
}

function parseSpin(value: number | null | undefined, displayValue: string | undefined): number {
  if (typeof value === "number") return value;
  const parsed = Number.parseInt(displayValue ?? "", 10);
  return Number.isNaN(parsed) ? 0 : parsed;
}

const Section: React.FC<{ title: string; show: boolean; children: React.ReactNode }> = ({ title, show, children }) =>
  show ? (
    <fieldset className="flex flex-col gap-3 border-t border-border pt-3 first:border-t-0 first:pt-0">
      <legend className="float-left mb-1 w-full text-xs font-semibold uppercase tracking-wide text-muted-foreground">{title}</legend>
      {children}
    </fieldset>
  ) : (
    <>{children}</>
  );

/** Editable project settings shared by the detail panel and the add-repository wizard. */
export const ProjectFormFields: React.FC<ProjectFormFieldsProps> = ({
  form,
  onChange,
  errors,
  showErrors,
  disabled,
  extraTrackerKinds = [],
  generalExtra,
  branchHint,
  sectioned = false
}) => {
  const m = useMessages(projectMessages);
  const trackerKinds = [...TRACKER_KINDS, ...extraTrackerKinds.filter((k) => !(TRACKER_KINDS as ReadonlyArray<string>).includes(k))];
  const err = (key: keyof ProjectFormErrors, message: string) =>
    showErrors && errors[key] ? { validationMessage: message, validationState: "error" as const } : {};

  return (
    <div className="flex flex-col gap-4">
      <Section title={m.detail.sectionGeneral} show={sectioned}>
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <Field label={m.form.name} required {...err("name", m.form.nameRequired)}>
            <Input disabled={disabled} value={form.name} onChange={(_, d) => onChange({ name: d.value })} />
          </Field>
          <Field label={m.form.defaultBranch} required hint={branchHint} {...err("defaultBranch", m.form.branchRequired)}>
            <Input className="font-mono" disabled={disabled} value={form.defaultBranch} onChange={(_, d) => onChange({ defaultBranch: d.value })} />
          </Field>
        </div>
        {generalExtra}
      </Section>

      <Section title={m.detail.sectionTracker} show={sectioned}>
        <Field label={m.form.trackerKind}>
          <Dropdown
            disabled={disabled}
            value={trackerLabel(m, form.trackerKind)}
            selectedOptions={[form.trackerKind]}
            onOptionSelect={(_, d) => d.optionValue && onChange({ trackerKind: d.optionValue })}
          >
            {trackerKinds.map((kind) => (
              <Option key={kind} value={kind}>
                {trackerLabel(m, kind)}
              </Option>
            ))}
          </Dropdown>
        </Field>
        {form.trackerKind === "github" ? (
          <Field label={m.form.githubRepo} {...err("githubRepo", m.form.githubRepoInvalid)}>
            <Input
              className="font-mono"
              disabled={disabled}
              value={form.githubRepo}
              placeholder={m.form.githubRepoPlaceholder}
              onChange={(_, d) => onChange({ githubRepo: d.value })}
            />
          </Field>
        ) : null}
        {form.trackerKind === "linear" ? (
          <Field label={m.form.linearSlug}>
            <Input
              className="font-mono"
              disabled={disabled}
              value={form.linearSlug}
              placeholder={m.form.linearSlugPlaceholder}
              onChange={(_, d) => onChange({ linearSlug: d.value })}
            />
          </Field>
        ) : null}
        <Field label={m.form.requiredLabels} hint={m.form.requiredLabelsHint}>
          <Input disabled={disabled} value={form.requiredLabels} onChange={(_, d) => onChange({ requiredLabels: d.value })} />
        </Field>
      </Section>

      <Section title={m.detail.sectionAgents} show={sectioned}>
        <ProviderModelFields
          provider={form.provider}
          onProviderChange={(provider) => onChange({ provider, model: "" })}
          model={form.model}
          onModelChange={(model) => onChange({ model })}
          disabled={disabled}
        />
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <Field label={m.form.maxAgents} hint={m.form.maxAgentsHint} {...err("maxAgents", m.form.maxAgentsInvalid)}>
            <SpinButton
              disabled={disabled}
              min={1}
              max={32}
              value={form.maxAgents}
              onChange={(_, d) => onChange({ maxAgents: parseSpin(d.value, d.displayValue) })}
            />
          </Field>
          <Field hint={m.form.autoDispatchHint}>
            <Switch
              disabled={disabled}
              checked={form.autoDispatch}
              onChange={(_, d) => onChange({ autoDispatch: d.checked })}
              label={m.form.autoDispatch}
            />
          </Field>
        </div>
      </Section>

      <Section title={m.detail.sectionValidation} show={sectioned}>
        <Field label={m.form.validationCommands} hint={m.form.validationCommandsHint}>
          <Textarea
            className="font-mono"
            disabled={disabled}
            rows={3}
            resize="vertical"
            value={form.validationCommands}
            onChange={(_, d) => onChange({ validationCommands: d.value })}
          />
        </Field>
      </Section>
    </div>
  );
};
