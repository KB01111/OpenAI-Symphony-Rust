import React, { useMemo } from "react";
import { Combobox, Dropdown, Field, Option } from "@fluentui/react-components";
import { ALL_PROVIDERS, formatMessage, type ProviderKind } from "@symphony/shared-types";
import { StatusDot } from "@symphony/ui";
import { useMessages, useT } from "../../lib/i18n.js";
import { useProviders } from "../../lib/queries.js";
import { projectMessages } from "./messages.js";

export interface ProviderModelFieldsProps {
  provider: ProviderKind;
  onProviderChange: (provider: ProviderKind) => void;
  model: string;
  onModelChange: (model: string) => void;
  /** Model used when the field is left empty (shown as placeholder). */
  fallbackModel?: string | null;
  providerLabel?: string;
  modelLabel?: string;
  disabled?: boolean;
  className?: string;
}

/**
 * Provider dropdown (all providers with detected install state from the runtime) + model combobox
 * offering the provider's reported models while still allowing a free-text model id.
 */
export const ProviderModelFields: React.FC<ProviderModelFieldsProps> = ({
  provider,
  onProviderChange,
  model,
  onModelChange,
  fallbackModel,
  providerLabel,
  modelLabel,
  disabled,
  className
}) => {
  const m = useMessages(projectMessages);
  const t = useT();
  const providers = useProviders();
  const statusByProvider = useMemo(() => new Map((providers.data ?? []).map((p) => [p.provider, p] as const)), [providers.data]);
  const current = statusByProvider.get(provider);
  const models = current?.capabilities.models ?? [];
  const defaultModel = fallbackModel ?? current?.default_model ?? null;

  const installLabel = (kind: ProviderKind): string | null => {
    const status = statusByProvider.get(kind);
    if (!status) return null;
    return status.installed ? m.form.installed : m.form.notInstalled;
  };

  return (
    <div className={className ?? "grid grid-cols-1 gap-3 sm:grid-cols-2"}>
      <Field label={providerLabel ?? m.form.provider} hint={installLabel(provider) ?? undefined}>
        <Dropdown
          disabled={disabled}
          value={t.provider[provider]}
          selectedOptions={[provider]}
          onOptionSelect={(_, data) => data.optionValue && onProviderChange(data.optionValue as ProviderKind)}
        >
          {ALL_PROVIDERS.map((kind) => {
            const status = statusByProvider.get(kind);
            const label = installLabel(kind);
            return (
              <Option key={kind} value={kind} text={t.provider[kind]}>
                <span className="flex w-full items-center gap-2">
                  {status ? <StatusDot tone={status.installed ? "success" : "neutral"} label={label ?? ""} /> : null}
                  <span className="flex-1">{t.provider[kind]}</span>
                  {label ? <span className="text-xs text-muted-foreground">{label}</span> : null}
                </span>
              </Option>
            );
          })}
        </Dropdown>
      </Field>
      <Field label={modelLabel ?? m.form.model}>
        <Combobox
          freeform
          disabled={disabled}
          value={model}
          selectedOptions={model && models.includes(model) ? [model] : []}
          placeholder={defaultModel ? formatMessage(m.form.modelDefault, { model: defaultModel }) : m.form.modelPlaceholder}
          onChange={(e) => onModelChange(e.target.value)}
          onOptionSelect={(_, data) => onModelChange(data.optionText ?? data.optionValue ?? "")}
          className="font-mono"
        >
          {models.map((id) => (
            <Option key={id} value={id}>
              {id}
            </Option>
          ))}
        </Combobox>
      </Field>
    </div>
  );
};
