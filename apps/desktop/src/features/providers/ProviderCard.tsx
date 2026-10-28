import React, { useEffect, useMemo, useState } from "react";
import { Check, Star, X } from "lucide-react";
import { formatMessage, type AppSettings, type ProviderKind, type ProviderStatus } from "@symphony/shared-types";
import { Badge, Button, Combobox, Field, cn, Hint, KeyValueList, Panel, Spinner, TimeAgo, useNotify } from "@symphony/ui";
import { useMessages, useT } from "../../lib/i18n.js";
import { useSaveSettings } from "../../lib/queries.js";
import { CredentialRow } from "./CredentialControls.js";
import { providerMessages } from "./messages.js";

/** Vault key each provider CLI reads, when it supports API-key authentication. */
export const PROVIDER_CREDENTIAL_KEYS: Partial<Record<ProviderKind, string>> = {
  codex: "OPENAI_API_KEY",
  claude_code: "ANTHROPIC_API_KEY"
};

/** Install commands shown when a CLI is not on PATH (commands, not prose; not translated). */
const INSTALL_HINTS: Partial<Record<ProviderKind, string>> = {
  codex: "npm install -g @openai/codex",
  claude_code: "npm install -g @anthropic-ai/claude-code",
  opencode: "npm install -g opencode-ai"
};

type CapabilityKey = keyof (typeof providerMessages)["en"]["capability"];

const CAPABILITY_FIELDS: ReadonlyArray<{ key: CapabilityKey; pick: (p: ProviderStatus) => boolean }> = [
  { key: "session_resume", pick: (p) => p.capabilities.supports_session_resume },
  { key: "mid_turn_pause", pick: (p) => p.capabilities.supports_mid_turn_pause },
  { key: "cancel", pick: (p) => p.capabilities.supports_cancel },
  { key: "streaming_events", pick: (p) => p.capabilities.supports_streaming_events },
  { key: "approvals", pick: (p) => p.capabilities.supports_approvals },
  { key: "dynamic_tools", pick: (p) => p.capabilities.supports_dynamic_tools },
  { key: "token_usage", pick: (p) => p.capabilities.supports_token_usage }
];

const AUTH_COLORS = {
  authenticated: "success",
  unauthenticated: "warning",
  unknown: "neutral",
  not_required: "neutral"
} as const;

const Section: React.FC<{ title: string; children: React.ReactNode; className?: string }> = ({ title, children, className }) => (
  <section className={cn("space-y-1.5", className)}>
    <h3 className="text-xs font-semibold uppercase tracking-wide text-subtle-foreground">{title}</h3>
    {children}
  </section>
);

export interface ProviderCardProps {
  provider: ProviderStatus;
  settings: AppSettings | undefined;
  /** Vault state; `null` while unavailable. */
  credentials: { configured: ReadonlyArray<string>; allowed: ReadonlyArray<string> } | null;
}

export const ProviderCard: React.FC<ProviderCardProps> = React.memo(function ProviderCard({ provider, settings, credentials }) {
  const m = useMessages(providerMessages);
  const t = useT();
  const notify = useNotify();
  const saveModel = useSaveSettings();
  const saveGlobal = useSaveSettings();
  const kind = provider.provider;

  const savedModel = settings?.provider_default_models[kind] ?? "";
  const [model, setModel] = useState(savedModel);
  // Follow the persisted value when it changes elsewhere (other window, Settings page).
  useEffect(() => setModel(savedModel), [savedModel]);

  const models = provider.capabilities.models;
  const isGlobalDefault = settings?.default_provider === kind;
  const credentialKey = PROVIDER_CREDENTIAL_KEYS[kind];
  const credential =
    credentialKey && credentials?.allowed.includes(credentialKey)
      ? { key: credentialKey, configured: credentials.configured.includes(credentialKey) }
      : null;
  const trimmed = model.trim();
  const modelDirty = trimmed !== savedModel;
  const busy = saveModel.isPending || saveGlobal.isPending;

  const capabilities = useMemo(
    () => CAPABILITY_FIELDS.map((field) => ({ key: field.key, supported: field.pick(provider) })),
    [provider]
  );

  const saveDefaultModel = () => {
    if (!settings || busy) return;
    // Functional update: queued saves from several provider cards each apply only their own field
    // to the latest settings instead of overwriting each other with stale snapshots.
    saveModel.mutate((latest) => {
      const next: AppSettings["provider_default_models"] = { ...latest.provider_default_models };
      if (trimmed) next[kind] = trimmed;
      else delete next[kind];
      const update: AppSettings = { ...latest, provider_default_models: next };
      if (latest.default_provider === kind && trimmed) update.default_model = trimmed;
      return update;
    }, {
      onSuccess: () => notify("success", m.saved, formatMessage(m.savedDefault, { provider: provider.display_name }))
    });
  };

  const makeGlobal = () => {
    if (!settings || busy) return;
    // Only a model the user chose for this provider; otherwise "" so the CLI's own default keeps
    // applying (a detected default is not pinned, and never another provider's model).
    const defaultModel = trimmed || savedModel || "";
    saveGlobal.mutate(
      (latest) => ({ ...latest, default_provider: kind, default_model: defaultModel }),
      { onSuccess: () => notify("success", m.saved, formatMessage(m.savedGlobal, { provider: provider.display_name })) }
    );
  };

  const placeholder = provider.default_model
    ? formatMessage(m.defaultModelPlaceholder, { model: provider.default_model })
    : m.defaultModelPlaceholderNone;

  return (
    <Panel
      className="@container"
      aria-label={provider.display_name}
      title={
        <span className="flex min-w-0 items-center gap-2">
          <span className="truncate">{provider.display_name}</span>
          <Badge tone={provider.installed ? "success" : "danger"}>
            {provider.installed ? m.installed : m.notFound}
          </Badge>
          {isGlobalDefault ? (
            <Badge tone="info">
              {m.globalDefault}
            </Badge>
          ) : null}
        </span>
      }
      actions={
        <span className="text-xs text-subtle-foreground">
          {m.detectedAt} <TimeAgo iso={provider.detected_at} />
        </span>
      }
      bodyClassName="grid gap-4 p-3 @2xl:grid-cols-2"
    >
      <Section title={m.installation}>
        {provider.installed ? (
          <KeyValueList
            items={[
              { label: m.binaryPath, value: provider.binary_path ?? t.common.unknown, mono: true },
              { label: m.version, value: provider.version ?? t.common.unknown, mono: true }
            ]}
          />
        ) : (
          <div className="space-y-1.5 text-sm">
            <p className="text-destructive">{m.notFound}</p>
            {INSTALL_HINTS[kind] ? (
              <p className="text-xs text-muted-foreground">
                {m.installHint}:{" "}
                <code className="rounded bg-code px-1 py-0.5 font-mono text-foreground">{INSTALL_HINTS[kind]}</code>
              </p>
            ) : null}
          </div>
        )}
        {provider.capabilities.status_message ? (
          <p className="text-xs text-muted-foreground">{provider.capabilities.status_message}</p>
        ) : null}
      </Section>

      <Section title={m.authentication}>
        <div className="flex flex-wrap items-center gap-2">
          <Badge tone={AUTH_COLORS[provider.auth_state]}>
            {t.authState[provider.auth_state]}
          </Badge>
          {provider.auth_detail ? <span className="text-xs text-muted-foreground">{provider.auth_detail}</span> : null}
        </div>
        {provider.auth_state === "unknown" ? <p className="text-xs text-muted-foreground">{m.authUnknownHelp}</p> : null}
        {provider.auth_state === "unauthenticated" ? <p className="text-xs text-warning">{m.authUnauthenticatedHelp}</p> : null}
        {credential ? (
          <div className="pt-1">
            <CredentialRow keyName={credential.key} configured={credential.configured} />
          </div>
        ) : null}
      </Section>

      <Section title={m.capabilities}>
        <ul
          aria-label={formatMessage(m.capabilitiesLabel, { provider: provider.display_name })}
          className="grid grid-cols-1 gap-x-4 gap-y-1 @md:grid-cols-2"
        >
          {capabilities.map(({ key, supported }) => (
            <li key={key} className="flex items-center gap-1.5 text-sm">
              {supported ? (
                <Check className="size-4 shrink-0 text-success" aria-hidden />
              ) : (
                <X className="size-4 shrink-0 text-subtle-foreground" aria-hidden />
              )}
              <span className={supported ? "text-foreground" : "text-muted-foreground"}>{m.capability[key]}</span>
              <span className="sr-only">: {supported ? m.supported : m.unsupported}</span>
            </li>
          ))}
        </ul>
      </Section>

      <Section title={m.models}>
        {models.length > 0 ? (
          <ul className="flex flex-wrap gap-1.5" aria-label={m.models}>
            {models.map((name) => (
              <li key={name}>
                <Badge tone="outline" className="font-mono">
                  {name}
                </Badge>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-xs text-muted-foreground">{m.noModels}</p>
        )}
      </Section>

      <div className="space-y-2 border-t border-border pt-3 @2xl:col-span-2">
        <div className="flex flex-wrap items-end gap-2">
          <Field label={m.defaultModel} hint={m.defaultModelHint} className="min-w-64 flex-1">
            <Combobox
              value={model}
              options={models}
              placeholder={placeholder}
              disabled={!settings}
              onValueChange={setModel}
              onKeyDown={(e) => {
                if (e.key === "Enter" && modelDirty) saveDefaultModel();
              }}
            />
          </Field>
          <Button
            variant="default"
            disabled={!settings || !modelDirty || busy}
            onClick={saveDefaultModel}
          >
            {saveModel.isPending ? <Spinner /> : undefined}
            {m.saveDefault}
          </Button>
          {!isGlobalDefault ? (
            <Hint label={provider.installed ? m.makeGlobalDefaultHint : m.unavailable}>
              <Button
               
                disabled={!settings || busy || !provider.installed}
                onClick={makeGlobal}
              >
                {saveGlobal.isPending ? <Spinner /> : <Star className="size-4" />}
                {m.makeGlobalDefault}
              </Button>
            </Hint>
          ) : null}
        </div>
      </div>
    </Panel>
  );
});
