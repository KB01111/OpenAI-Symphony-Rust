import React from "react";
import {
  Field,
  FieldGrid,
  FormSection,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
  Textarea
} from "@symphony/ui";
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

function parseCount(raw: string): number {
  const parsed = Number.parseInt(raw, 10);
  return Number.isNaN(parsed) ? 0 : parsed;
}

/** Titled group when `sectioned`, otherwise the bare children. */
const Section: React.FC<{ title: string; show: boolean; children: React.ReactNode }> = ({ title, show, children }) =>
  show ? <FormSection title={title}>{children}</FormSection> : <>{children}</>;

/**
 * Editable project settings shared by the detail drawer and the add-repository wizard. Every row
 * lays itself out from the width of its own container, so it fits a narrow drawer as well as a
 * wide dialog without overlapping.
 */
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
  const err = (key: keyof ProjectFormErrors, message: string) => (showErrors && errors[key] ? { error: message } : {});

  return (
    <div className="flex min-w-0 flex-col gap-5">
      <Section title={m.detail.sectionGeneral} show={sectioned}>
        <FieldGrid>
          <Field label={m.form.name} required {...err("name", m.form.nameRequired)}>
            <Input disabled={disabled} value={form.name} onChange={(e) => onChange({ name: e.target.value })} />
          </Field>
          <Field label={m.form.defaultBranch} required hint={branchHint} {...err("defaultBranch", m.form.branchRequired)}>
            <Input className="font-mono" disabled={disabled} value={form.defaultBranch} onChange={(e) => onChange({ defaultBranch: e.target.value })} />
          </Field>
        </FieldGrid>
        {generalExtra}
      </Section>

      <Section title={m.detail.sectionTracker} show={sectioned}>
        <Field label={m.form.trackerKind}>
          <Select disabled={disabled} value={form.trackerKind} onValueChange={(kind) => onChange({ trackerKind: kind })}>
            <SelectTrigger>
              <SelectValue>{trackerLabel(m, form.trackerKind)}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {trackerKinds.map((kind) => (
                <SelectItem key={kind} value={kind}>
                  {trackerLabel(m, kind)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        {form.trackerKind === "github" ? (
          <Field label={m.form.githubRepo} {...err("githubRepo", m.form.githubRepoInvalid)}>
            <Input
              className="font-mono"
              disabled={disabled}
              value={form.githubRepo}
              placeholder={m.form.githubRepoPlaceholder}
              onChange={(e) => onChange({ githubRepo: e.target.value })}
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
              onChange={(e) => onChange({ linearSlug: e.target.value })}
            />
          </Field>
        ) : null}
        <Field label={m.form.requiredLabels} hint={m.form.requiredLabelsHint}>
          <Input disabled={disabled} value={form.requiredLabels} onChange={(e) => onChange({ requiredLabels: e.target.value })} />
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
        <FieldGrid>
          <Field label={m.form.maxAgents} hint={m.form.maxAgentsHint} {...err("maxAgents", m.form.maxAgentsInvalid)}>
            <Input
              type="number"
              inputMode="numeric"
              disabled={disabled}
              min={1}
              max={32}
              value={String(form.maxAgents)}
              onChange={(e) => onChange({ maxAgents: parseCount(e.target.value) })}
            />
          </Field>
          <div className="flex min-w-0 flex-col gap-1.5">
            <div className="flex h-8 items-center gap-2">
              <Switch id="project-auto-dispatch" disabled={disabled} checked={form.autoDispatch} onCheckedChange={(checked) => onChange({ autoDispatch: checked })} />
              <Label htmlFor="project-auto-dispatch" className="text-sm">
                {m.form.autoDispatch}
              </Label>
            </div>
            <p className="text-xs text-muted-foreground">{m.form.autoDispatchHint}</p>
          </div>
        </FieldGrid>
      </Section>

      <Section title={m.detail.sectionValidation} show={sectioned}>
        <Field label={m.form.validationCommands} hint={m.form.validationCommandsHint}>
          <Textarea
            className="font-mono"
            disabled={disabled}
            rows={3}
            value={form.validationCommands}
            onChange={(e) => onChange({ validationCommands: e.target.value })}
          />
        </Field>
      </Section>
    </div>
  );
};
