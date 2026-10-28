import React, { useMemo } from "react";
import { ALL_PROVIDERS, formatMessage, type ProviderKind } from "@symphony/shared-types";
import { Combobox, Field, FieldGrid, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, StatusDot, cn } from "@symphony/ui";
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
  /** `stacked` always uses one column (popovers); the default follows the container width. */
  layout?: "responsive" | "stacked";
  className?: string;
}

/**
 * Provider select (all providers with detected install state from the runtime) + model combobox
 * offering the provider's reported models while still allowing a free-text model id. The two
 * fields share a row only when their container is wide enough; they never overlap.
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
  layout = "responsive",
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

  const fields = (
    <>
      <Field label={providerLabel ?? m.form.provider} hint={installLabel(provider) ?? undefined}>
        <Select value={provider} disabled={disabled} onValueChange={(next) => onProviderChange(next as ProviderKind)}>
          <SelectTrigger aria-label={providerLabel ?? m.form.provider}>
            <SelectValue>{t.provider[provider]}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            {ALL_PROVIDERS.map((kind) => {
              const status = statusByProvider.get(kind);
              const label = installLabel(kind);
              return (
                <SelectItem key={kind} value={kind} textValue={t.provider[kind]}>
                  <span className="flex w-full items-center gap-2">
                    {status ? <StatusDot tone={status.installed ? "success" : "neutral"} label={label ?? ""} /> : null}
                    <span className="flex-1">{t.provider[kind]}</span>
                    {label ? <span className="text-xs text-muted-foreground">{label}</span> : null}
                  </span>
                </SelectItem>
              );
            })}
          </SelectContent>
        </Select>
      </Field>
      <Field label={modelLabel ?? m.form.model}>
        <Combobox
          value={model}
          disabled={disabled}
          options={models}
          onValueChange={onModelChange}
          placeholder={defaultModel ? formatMessage(m.form.modelDefault, { model: defaultModel }) : m.form.modelPlaceholder}
        />
      </Field>
    </>
  );

  if (layout === "stacked") return <div className={cn("grid grid-cols-1 gap-3", className)}>{fields}</div>;
  return <FieldGrid className={className}>{fields}</FieldGrid>;
};
