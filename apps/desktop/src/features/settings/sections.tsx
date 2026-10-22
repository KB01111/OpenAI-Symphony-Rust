import React, { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import {
  Button,
  Combobox,
  Dropdown,
  Field,
  Input,
  MessageBar,
  MessageBarBody,
  MessageBarTitle,
  Option,
  Radio,
  RadioGroup,
  SpinButton,
  Spinner,
  Switch,
  Textarea
} from "@fluentui/react-components";
import { DatabaseBackup, Download, FileJson, KeyRound, LogOut, ShieldOff } from "lucide-react";
import { SymphonyApiError, errorMessage, isTauri, nativeHost } from "@symphony/api-client";
import { ALL_PROVIDERS, formatMessage, type AppSettings, type Locale, type ProviderKind } from "@symphony/shared-types";
import {
  ConfirmDialog,
  ErrorState,
  KeyValueList,
  LoadingRows,
  SdkCopyButton,
  Skeleton,
  TimeAgo,
  useNotify,
  type Density,
  type ThemeMode
} from "@symphony/ui";
import { useMutation } from "@tanstack/react-query";
import { paths } from "../../app/routes.js";
import { useLocale, useMessages, useT } from "../../lib/i18n.js";
import { useCredentials, useProviders, useRuntimeState } from "../../lib/queries.js";
import { useRuntime } from "../../runtime/RuntimeProvider.js";
import { CredentialRow } from "../providers/CredentialControls.js";
import { PROVIDER_CREDENTIAL_KEYS } from "../providers/ProviderCard.js";
import {
  useBackups,
  useCreateBackup,
  useExportDiagnostics,
  useNativeHostConfig,
  useRemoteAccess,
  useRevokeRemoteToken,
  useRotateRemoteToken,
  useRuntimeInfo
} from "./hooks.js";
import { settingsMessages } from "./messages.js";
import { formatBytes, parseOrigins, type SettingsErrors } from "./validation.js";

export type SectionId = keyof (typeof settingsMessages)["en"]["sections"];

export const SECTION_IDS: ReadonlyArray<SectionId> = [
  "appearance",
  "desktop",
  "scheduling",
  "defaults",
  "remote",
  "integrations",
  "backups",
  "diagnostics",
  "application"
];

export const sectionDomId = (id: SectionId) => `settings-${id}`;

export const SettingsSection: React.FC<{ id: SectionId; intro?: string; children: React.ReactNode }> = ({ id, intro, children }) => {
  const m = useMessages(settingsMessages);
  const titleId = `${sectionDomId(id)}-title`;
  return (
    <section id={sectionDomId(id)} aria-labelledby={titleId} className="scroll-mt-4 rounded-md border border-border bg-surface">
      <header className="border-b border-border px-4 py-2.5">
        <h2 id={titleId} className="text-sm font-semibold text-foreground">
          {m.sections[id]}
        </h2>
        {intro ? <p className="mt-0.5 text-xs text-muted-foreground">{intro}</p> : null}
      </header>
      <div className="space-y-4 p-4">{children}</div>
    </section>
  );
};

/** Fluent SpinButton bound to a number; typed text that is not a number is surfaced as NaN for validation. */
const NumberField: React.FC<{
  label: string;
  hint?: string;
  value: number;
  min?: number;
  max?: number;
  step?: number;
  error?: string;
  onChange: (value: number) => void;
}> = ({ label, hint, value, min, max, step = 1, error, onChange }) => (
  <Field label={label} hint={hint} validationMessage={error} validationState={error ? "error" : "none"}>
    <SpinButton
      value={Number.isFinite(value) ? value : null}
      min={min}
      max={max}
      step={step}
      onChange={(_, data) => {
        if (data.value !== undefined && data.value !== null) onChange(data.value);
        else if (data.displayValue !== undefined) {
          const text = data.displayValue.trim();
          onChange(text === "" ? Number.NaN : Number(text));
        }
      }}
    />
  </Field>
);

export interface FormSectionProps {
  draft: AppSettings;
  errors: SettingsErrors;
  update: (fn: (draft: AppSettings) => AppSettings) => void;
}

// ---------------------------------------------------------------------------------------------
// Appearance (theme/locale persist immediately; density is local)
// ---------------------------------------------------------------------------------------------

export const AppearanceSection: React.FC<{
  themeMode: ThemeMode;
  locale: Locale;
  density: Density;
  onTheme: (mode: ThemeMode) => void;
  onLocale: (locale: Locale) => void;
  onDensity: (density: Density) => void;
}> = ({ themeMode, locale, density, onTheme, onLocale, onDensity }) => {
  const m = useMessages(settingsMessages);
  return (
    <SettingsSection id="appearance" intro={m.appearance.intro}>
      <Field label={m.appearance.theme}>
        <RadioGroup layout="horizontal" value={themeMode} onChange={(_, d) => onTheme(d.value as ThemeMode)}>
          <Radio value="system" label={m.appearance.themeSystem} />
          <Radio value="dark" label={m.appearance.themeDark} />
          <Radio value="light" label={m.appearance.themeLight} />
        </RadioGroup>
      </Field>
      <Field label={m.appearance.language}>
        <RadioGroup layout="horizontal" value={locale} onChange={(_, d) => onLocale(d.value as Locale)}>
          <Radio value="en" label={m.appearance.english} />
          <Radio value="sv" label={m.appearance.swedish} />
        </RadioGroup>
      </Field>
      <Field label={m.appearance.density} hint={m.appearance.densityHint}>
        <RadioGroup layout="horizontal" value={density} onChange={(_, d) => onDensity(d.value as Density)}>
          <Radio value="compact" label={m.appearance.compact} />
          <Radio value="comfortable" label={m.appearance.comfortable} />
        </RadioGroup>
      </Field>
    </SettingsSection>
  );
};

// ---------------------------------------------------------------------------------------------
// Desktop behaviour
// ---------------------------------------------------------------------------------------------

export const DesktopSection: React.FC<FormSectionProps> = ({ draft, update }) => {
  const m = useMessages(settingsMessages);
  const desktop = isTauri();
  return (
    <SettingsSection id="desktop" intro={m.desktop.intro}>
      {!desktop ? (
        <MessageBar intent="info" layout="multiline">
          <MessageBarBody>{m.desktop.notDesktop}</MessageBarBody>
        </MessageBar>
      ) : null}
      <Field hint={m.desktop.trayHint}>
        <Switch
          label={m.desktop.tray}
          checked={draft.minimize_to_tray_on_close}
          onChange={(_, d) => update((s) => ({ ...s, minimize_to_tray_on_close: d.checked }))}
        />
      </Field>
      <Field hint={m.desktop.autostartHint}>
        <Switch
          label={m.desktop.autostart}
          checked={draft.start_background_runtime_on_login}
          onChange={(_, d) => update((s) => ({ ...s, start_background_runtime_on_login: d.checked }))}
        />
      </Field>
    </SettingsSection>
  );
};

// ---------------------------------------------------------------------------------------------
// Scheduling & resources (+ live hardware context)
// ---------------------------------------------------------------------------------------------

const HardwareContext: React.FC = () => {
  const m = useMessages(settingsMessages);
  const t = useT();
  const state = useRuntimeState();
  const hw = state.data?.hardware;
  if (state.isPending) return <Skeleton className="h-28 w-full" />;
  if (!hw) return <p className="text-xs text-muted-foreground">{m.scheduling.liveUnavailable}</p>;
  const power = hw.on_battery
    ? hw.battery_percent != null
      ? formatMessage(m.scheduling.battery, { percent: hw.battery_percent })
      : m.scheduling.batteryUnknown
    : m.scheduling.ac;
  return (
    <div className="space-y-2 rounded-md border border-border bg-surface-sunken p-3" aria-label={m.scheduling.live} role="group">
      <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-subtle-foreground">
        {m.scheduling.live}
        <span className="font-normal normal-case tracking-normal">
          <TimeAgo iso={hw.captured_at} />
        </span>
      </div>
      <KeyValueList
        items={[
          { label: m.scheduling.power, value: power },
          { label: m.scheduling.powerMode, value: hw.power_mode || t.common.unknown },
          { label: m.scheduling.cpuLoad, value: `${hw.cpu_usage_percent.toFixed(0)}%` },
          { label: m.scheduling.memoryLoad, value: `${hw.memory_usage_percent.toFixed(0)}%` },
          { label: m.scheduling.recommended, value: hw.recommended_max_concurrency },
          { label: m.scheduling.effective, value: hw.effective_max_concurrency },
          { label: m.scheduling.throttled, value: hw.thermal_throttling ? t.common.yes : t.common.no }
        ]}
      />
      <p className={hw.dispatch_delayed_reason ? "text-xs text-warning" : "text-xs text-muted-foreground"} role="status">
        {hw.dispatch_delayed_reason
          ? formatMessage(m.scheduling.delayed, { reason: hw.dispatch_delayed_reason })
          : m.scheduling.notDelayed}
      </p>
    </div>
  );
};

export const SchedulingSection: React.FC<FormSectionProps> = ({ draft, errors, update }) => {
  const m = useMessages(settingsMessages);
  const policy = (patch: Partial<AppSettings["resource_policy"]>) =>
    update((s) => ({ ...s, resource_policy: { ...s.resource_policy, ...patch } }));
  return (
    <SettingsSection id="scheduling" intro={m.scheduling.intro}>
      <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_280px]">
        <div className="grid content-start gap-3 sm:grid-cols-2">
          <NumberField
            label={m.scheduling.polling}
            hint={m.scheduling.pollingHint}
            value={draft.polling_interval_ms}
            min={2000}
            step={1000}
            error={errors.polling_interval_ms}
            onChange={(v) => update((s) => ({ ...s, polling_interval_ms: v }))}
          />
          <span className="hidden sm:block" aria-hidden />
          <NumberField
            label={m.scheduling.maxAc}
            hint={m.scheduling.maxAcHint}
            value={draft.resource_policy.max_concurrent_on_ac}
            min={1}
            max={32}
            error={errors.max_concurrent_on_ac}
            onChange={(v) => policy({ max_concurrent_on_ac: v })}
          />
          <NumberField
            label={m.scheduling.maxBattery}
            hint={m.scheduling.maxBatteryHint}
            value={draft.resource_policy.max_concurrent_on_battery}
            min={0}
            max={32}
            error={errors.max_concurrent_on_battery}
            onChange={(v) => policy({ max_concurrent_on_battery: v })}
          />
          <NumberField
            label={m.scheduling.memory}
            hint={m.scheduling.thresholdHint}
            value={draft.resource_policy.high_memory_threshold_percent}
            min={10}
            max={100}
            error={errors.high_memory_threshold_percent}
            onChange={(v) => policy({ high_memory_threshold_percent: v })}
          />
          <NumberField
            label={m.scheduling.cpu}
            hint={m.scheduling.thresholdHint}
            value={draft.resource_policy.high_cpu_threshold_percent}
            min={10}
            max={100}
            error={errors.high_cpu_threshold_percent}
            onChange={(v) => policy({ high_cpu_threshold_percent: v })}
          />
          <div className="sm:col-span-2">
            <Switch
              label={m.scheduling.thermal}
              checked={draft.resource_policy.reduce_on_thermal_throttle}
              onChange={(_, d) => policy({ reduce_on_thermal_throttle: d.checked })}
            />
          </div>
        </div>
        <HardwareContext />
      </div>
    </SettingsSection>
  );
};

// ---------------------------------------------------------------------------------------------
// Default provider / model
// ---------------------------------------------------------------------------------------------

export const DefaultsSection: React.FC<FormSectionProps> = ({ draft, update }) => {
  const m = useMessages(settingsMessages);
  const t = useT();
  const providers = useProviders();
  const status = providers.data?.find((p) => p.provider === draft.default_provider);
  const models = status?.capabilities.models ?? [];
  const label = (kind: ProviderKind) => {
    const s = providers.data?.find((p) => p.provider === kind);
    return s && !s.installed ? `${t.provider[kind]} (${m.defaults.notInstalled})` : t.provider[kind];
  };
  return (
    <SettingsSection id="defaults" intro={m.defaults.intro}>
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label={m.defaults.provider}>
          <Dropdown
            value={label(draft.default_provider)}
            selectedOptions={[draft.default_provider]}
            onOptionSelect={(_, d) => {
              const kind = d.optionValue as ProviderKind | undefined;
              if (!kind) return;
              update((s) => ({ ...s, default_provider: kind, default_model: s.provider_default_models[kind] ?? "" }));
            }}
          >
            {ALL_PROVIDERS.map((kind) => (
              <Option key={kind} value={kind} text={label(kind)}>
                {label(kind)}
              </Option>
            ))}
          </Dropdown>
        </Field>
        <Field label={m.defaults.model} hint={m.defaults.modelHint}>
          <Combobox
            freeform
            value={draft.default_model}
            selectedOptions={models.includes(draft.default_model) ? [draft.default_model] : []}
            placeholder={status?.default_model ?? undefined}
            onChange={(e) => {
              const value = e.target.value;
              update((s) => ({ ...s, default_model: value }));
            }}
            onOptionSelect={(_, d) => update((s) => ({ ...s, default_model: d.optionValue ?? d.optionText ?? "" }))}
          >
            {models.map((name) => (
              <Option key={name} value={name}>
                {name}
              </Option>
            ))}
          </Combobox>
        </Field>
      </div>
      <Link to={paths.providers()} className="text-sm text-accent hover:underline">
        {m.defaults.manage}
      </Link>
    </SettingsSection>
  );
};

// ---------------------------------------------------------------------------------------------
// Remote access
// ---------------------------------------------------------------------------------------------

const RemoteStatusPanel: React.FC = () => {
  const m = useMessages(settingsMessages);
  const status = useRemoteAccess();
  if (status.isPending) return <Skeleton className="h-16 w-full" />;
  if (status.isError) {
    return <ErrorState title={m.remote.statusLoadError} message={errorMessage(status.error)} onRetry={() => void status.refetch()} />;
  }
  const s = status.data;
  const tone = s.state === "running" ? "text-success" : s.state === "failed" ? "text-destructive" : "text-muted-foreground";
  return (
    <div className="space-y-1 rounded-md border border-border bg-surface-sunken p-3" role="group" aria-label={m.remote.status}>
      <div className="text-xs font-semibold uppercase tracking-wide text-subtle-foreground">{m.remote.status}</div>
      <p className={`text-sm font-medium ${tone}`} role="status" data-testid="remote-state">
        {m.remote.states[s.state] ?? s.state}
        {s.listening_on ? <span className="ml-2 font-mono text-xs text-muted-foreground">{formatMessage(m.remote.listeningOn, { address: s.listening_on })}</span> : null}
      </p>
      {s.error ? <p className="text-xs text-destructive">{s.error}</p> : null}
      <p className="text-xs text-muted-foreground">{m.remote.applyHint}</p>
    </div>
  );
};

const RemoteTokenControls: React.FC = () => {
  const m = useMessages(settingsMessages);
  const t = useT();
  const notify = useNotify();
  const status = useRemoteAccess();
  const rotate = useRotateRemoteToken();
  const revoke = useRevokeRemoteToken();
  const [issued, setIssued] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<"rotate" | "revoke" | null>(null);
  const configured = status.data?.token_configured ?? false;
  const fail = (error: unknown) => notify("error", m.remote.tokenFailed, errorMessage(error));
  const runRotate = () =>
    rotate.mutate(undefined, {
      onSuccess: (result) => setIssued(result.token),
      onError: fail,
      onSettled: () => setConfirm(null)
    });

  return (
    <div className="space-y-2">
      <div className="text-sm font-medium text-foreground">{m.remote.token}</div>
      <p className="text-xs text-muted-foreground">{configured ? m.remote.tokenConfigured : m.remote.tokenMissing}</p>
      <div className="flex flex-wrap gap-2">
        <Button
          appearance={configured ? "secondary" : "primary"}
          icon={rotate.isPending ? <Spinner size="tiny" /> : <KeyRound className="size-4" />}
          disabled={rotate.isPending || status.isPending || status.isError}
          onClick={() => (configured ? setConfirm("rotate") : runRotate())}
        >
          {configured ? m.remote.rotate : m.remote.generate}
        </Button>
        {configured ? (
          <Button icon={<ShieldOff className="size-4" />} disabled={revoke.isPending || status.isError} onClick={() => setConfirm("revoke")}>
            {m.remote.revoke}
          </Button>
        ) : null}
      </div>
      {issued ? (
        <MessageBar intent="success" layout="multiline">
          <MessageBarBody>
            <MessageBarTitle>{m.remote.issuedTitle}</MessageBarTitle>
            <span className="block">{m.remote.issuedBody}</span>
            <span className="mt-2 flex items-center gap-2">
              <Input readOnly value={issued} aria-label={m.remote.token} className="w-full font-mono" spellCheck={false} />
              <SdkCopyButton copyValue={issued} aria-label={m.remote.copy} size="sm" variant="ghost" color="secondary" />
            </span>
            <Button className="mt-2" size="small" onClick={() => setIssued(null)}>
              {m.remote.done}
            </Button>
          </MessageBarBody>
        </MessageBar>
      ) : null}
      <ConfirmDialog
        open={confirm === "rotate"}
        onOpenChange={(open) => setConfirm(open ? "rotate" : null)}
        title={m.remote.rotateTitle}
        description={m.remote.rotateDescription}
        confirmLabel={m.remote.rotate}
        pending={rotate.isPending}
        onConfirm={runRotate}
      />
      <ConfirmDialog
        open={confirm === "revoke"}
        onOpenChange={(open) => setConfirm(open ? "revoke" : null)}
        title={m.remote.revokeTitle}
        description={m.remote.revokeDescription}
        confirmLabel={m.remote.revoke}
        cancelLabel={t.common.cancel}
        destructive
        pending={revoke.isPending}
        onConfirm={() =>
          revoke.mutate(undefined, {
            onSuccess: () => setIssued(null),
            onError: fail,
            onSettled: () => setConfirm(null)
          })
        }
      />
    </div>
  );
};

export const RemoteSection: React.FC<FormSectionProps> = ({ draft, errors, update }) => {
  const m = useMessages(settingsMessages);
  const origins = draft.remote_allowed_origins ?? [];
  const [originsText, setOriginsText] = useState(() => origins.join("\n"));
  // Keep the text in sync when the draft changes from outside (Reset, a save, a refetch), while
  // preserving the user's own formatting (blank lines, commas) as long as it parses to the draft.
  const parsedText = parseOrigins(originsText);
  if (parsedText.length !== origins.length || parsedText.some((o, i) => o !== origins[i])) {
    setOriginsText(origins.join("\n"));
  }
  return (
    <SettingsSection id="remote" intro={m.remote.intro}>
      <MessageBar intent={draft.remote_api_enabled ? "warning" : "info"} layout="multiline">
        <MessageBarBody>
          <MessageBarTitle>{m.remote.warningTitle}</MessageBarTitle>
          {m.remote.warning}
        </MessageBarBody>
      </MessageBar>
      <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_280px]">
        <div className="space-y-3">
          <Switch
            label={m.remote.enabled}
            checked={draft.remote_api_enabled}
            onChange={(_, d) => update((s) => ({ ...s, remote_api_enabled: d.checked }))}
          />
          <div className="grid gap-3 sm:grid-cols-2">
            <Field
              label={m.remote.bind}
              hint={m.remote.bindHint}
              validationMessage={errors.remote_bind_address}
              validationState={errors.remote_bind_address ? "error" : "none"}
            >
              <Input
                value={draft.remote_bind_address}
                spellCheck={false}
                className="font-mono"
                onChange={(_, d) => update((s) => ({ ...s, remote_bind_address: d.value }))}
              />
            </Field>
            <NumberField
              label={m.remote.port}
              value={draft.remote_port}
              min={1}
              max={65535}
              error={errors.remote_port}
              onChange={(v) => update((s) => ({ ...s, remote_port: v }))}
            />
          </div>
          <Field
            label={m.remote.origins}
            hint={m.remote.originsHint}
            validationMessage={errors.remote_allowed_origins}
            validationState={errors.remote_allowed_origins ? "error" : "none"}
          >
            <Textarea
              value={originsText}
              spellCheck={false}
              className="font-mono"
              resize="vertical"
              onChange={(_, d) => {
                setOriginsText(d.value);
                update((s) => ({ ...s, remote_allowed_origins: parseOrigins(d.value) }));
              }}
            />
          </Field>
          <RemoteTokenControls />
        </div>
        <RemoteStatusPanel />
      </div>
    </SettingsSection>
  );
};

// ---------------------------------------------------------------------------------------------
// Integrations credentials
// ---------------------------------------------------------------------------------------------

const PROVIDER_KEYS = new Set(Object.values(PROVIDER_CREDENTIAL_KEYS));

export const IntegrationsSection: React.FC = () => {
  const m = useMessages(settingsMessages);
  const credentials = useCredentials();
  const keys = useMemo(() => (credentials.data?.allowed ?? []).filter((k) => !PROVIDER_KEYS.has(k)), [credentials.data]);
  return (
    <SettingsSection id="integrations" intro={m.integrations.intro}>
      {credentials.isPending ? (
        <LoadingRows rows={3} />
      ) : credentials.isError ? (
        <ErrorState
          title={m.integrations.loadError}
          message={errorMessage(credentials.error)}
          onRetry={() => void credentials.refetch()}
        />
      ) : (
        <ul className="divide-y divide-border">
          {keys.map((key) => (
            <li key={key} className="py-2 first:pt-0 last:pb-0">
              <CredentialRow
                keyName={key}
                configured={credentials.data.configured.includes(key)}
                description={m.integrations.descriptions[key]}
              />
            </li>
          ))}
        </ul>
      )}
      <p className="text-xs text-muted-foreground">
        {m.integrations.providerKeys}{" "}
        <Link to={paths.providers()} className="text-accent hover:underline">
          {m.integrations.openProviders}
        </Link>
      </p>
    </SettingsSection>
  );
};

// ---------------------------------------------------------------------------------------------
// Backups
// ---------------------------------------------------------------------------------------------

export const BackupsSection: React.FC = () => {
  const m = useMessages(settingsMessages);
  const locale = useLocale();
  const notify = useNotify();
  const backups = useBackups();
  const create = useCreateBackup();
  const onCreate = () =>
    create.mutate(undefined, {
      onSuccess: (backup) => notify("success", m.backups.created, backup.path),
      onError: (error) => notify("error", m.backups.createFailed, errorMessage(error))
    });
  const unavailable = backups.error instanceof SymphonyApiError && backups.error.code === "backups_unavailable";
  return (
    <SettingsSection id="backups" intro={m.backups.intro}>
      {unavailable ? (
        <p className="text-sm text-muted-foreground">{m.backups.unavailable}</p>
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              icon={create.isPending ? <Spinner size="tiny" /> : <DatabaseBackup className="size-4" />}
              disabled={create.isPending}
              onClick={onCreate}
            >
              {create.isPending ? m.backups.creating : m.backups.create}
            </Button>
            {backups.data ? (
              <span className="text-xs text-muted-foreground">{formatMessage(m.backups.directory, { path: backups.data.directory })}</span>
            ) : null}
          </div>
          {backups.isLoading ? <LoadingRows rows={2} /> : null}
          {backups.isError ? <ErrorState title={m.backups.loadFailed} message={errorMessage(backups.error)} /> : null}
          {backups.data && backups.data.backups.length === 0 ? (
            <p className="text-sm text-muted-foreground">{m.backups.empty}</p>
          ) : null}
          {backups.data && backups.data.backups.length > 0 ? (
            <ul className="divide-y divide-border rounded-md border border-border" aria-label={m.backups.listLabel}>
              {backups.data.backups.map((b) => (
                <li key={b.id} className="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2 text-sm">
                  <TimeAgo iso={b.created_at} />
                  <span className="text-muted-foreground tabular-nums">{formatBytes(b.size_bytes, locale)}</span>
                  <span className="text-muted-foreground">
                    {formatMessage(m.backups.version, { version: b.app_version, schema: b.schema_version })}
                  </span>
                  <code className="ms-auto max-w-full truncate font-mono text-xs text-subtle-foreground" title={b.path}>
                    {b.id}
                  </code>
                </li>
              ))}
            </ul>
          ) : null}
        </>
      )}
      <p className="text-xs text-muted-foreground">{m.backups.scope}</p>
      <p className="text-xs text-muted-foreground">{m.backups.restore}</p>
    </SettingsSection>
  );
};

// ---------------------------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------------------------

export const DiagnosticsSection: React.FC = () => {
  const m = useMessages(settingsMessages);
  const locale = useLocale();
  const exporter = useExportDiagnostics();
  const bundle = exporter.bundle;
  return (
    <SettingsSection id="diagnostics" intro={m.diagnostics.intro}>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          icon={exporter.isPending ? <Spinner size="tiny" /> : <FileJson className="size-4" />}
          disabled={exporter.isPending}
          onClick={() => exporter.mutate()}
        >
          {exporter.isPending ? m.diagnostics.exporting : m.diagnostics.export}
        </Button>
        {bundle ? (
          <>
            <span className="text-sm text-muted-foreground" role="status">
              {formatMessage(m.diagnostics.ready, { size: formatBytes(bundle.bytes, locale) })}
            </span>
            {bundle.url ? (
              <Button as="a" href={bundle.url} download="symphony-diagnostics.json" appearance="primary" icon={<Download className="size-4" />}>
                {m.diagnostics.download}
              </Button>
            ) : null}
          </>
        ) : null}
      </div>
      {exporter.isError ? <ErrorState title={m.diagnostics.failed} message={errorMessage(exporter.error)} /> : null}
    </SettingsSection>
  );
};

// ---------------------------------------------------------------------------------------------
// Application
// ---------------------------------------------------------------------------------------------

export const ApplicationSection: React.FC = () => {
  const m = useMessages(settingsMessages);
  const t = useT();
  const notify = useNotify();
  const info = useRuntimeInfo();
  const runtime = useRuntime();
  const [confirmExit, setConfirmExit] = useState(false);
  const exit = useMutation({
    mutationFn: () => nativeHost.exitApplication(),
    onError: (error) => notify("error", t.errors.generic, errorMessage(error))
  });
  const desktop = isTauri();
  const mode = runtime.mode ? (m.application.modes[runtime.mode] ?? runtime.mode) : t.common.unknown;
  const discoveryError = useNativeHostConfig().data?.discovery_error;

  return (
    <SettingsSection id="application" intro={m.application.intro}>
      {discoveryError ? (
        <MessageBar intent="warning" layout="multiline">
          <MessageBarBody>
            <MessageBarTitle>{m.application.discoveryErrorTitle}</MessageBarTitle>
            {formatMessage(m.application.discoveryError, { error: discoveryError })}
          </MessageBarBody>
        </MessageBar>
      ) : null}
      {info.isPending ? (
        <LoadingRows rows={4} />
      ) : info.isError ? (
        <ErrorState title={m.application.loadError} message={errorMessage(info.error)} onRetry={() => void info.refetch()} />
      ) : (
        <KeyValueList
          items={[
            { label: m.application.service, value: info.data.service, mono: true },
            { label: m.application.version, value: info.data.version, mono: true },
            { label: m.application.apiVersion, value: String(info.data.api_version), mono: true },
            { label: m.application.mode, value: mode },
            { label: m.application.baseUrl, value: runtime.baseUrl ?? t.common.unknown, mono: true },
            { label: m.application.startedAt, value: <TimeAgo iso={info.data.started_at} /> },
            { label: m.application.auth, value: info.data.auth_required ? m.application.authRequired : m.application.authNone }
          ]}
        />
      )}
      {desktop ? (
        <div className="border-t border-border pt-3">
          <Button icon={<LogOut className="size-4" />} onClick={() => setConfirmExit(true)} disabled={exit.isPending}>
            {m.application.exit}
          </Button>
          <ConfirmDialog
            open={confirmExit}
            onOpenChange={setConfirmExit}
            title={m.application.exitTitle}
            description={m.application.exitDescription}
            confirmLabel={m.application.exitConfirm}
            destructive
            pending={exit.isPending}
            onConfirm={() => exit.mutate(undefined, { onSettled: () => setConfirmExit(false) })}
          />
        </div>
      ) : null}
    </SettingsSection>
  );
};
