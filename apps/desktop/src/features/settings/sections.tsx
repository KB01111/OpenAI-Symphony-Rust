import React, { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { DatabaseBackup, Download, FileJson, FolderOpen, Keyboard, KeyRound, LogOut, RefreshCw, ShieldOff } from "lucide-react";
import { SymphonyApiError, errorMessage, isTauri, nativeHost } from "@symphony/api-client";
import { ALL_CLOUD_AGENTS, ALL_PROVIDERS, formatMessage, type AppSettings, type CloudAgentKind, type Locale, type ProviderKind } from "@symphony/shared-types";
import {
  Alert,
  Badge,
  Button,
  Combobox,
  ConfirmDialog,
  CopyButton,
  ErrorState,
  Field,
  Input,
  KeyValueList,
  Label,
  LoadingRows,
  RadioGroup,
  RadioOption,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  Spinner,
  Switch,
  Textarea,
  TimeAgo,
  useNotify,
  type Density,
  type ThemeMode
} from "@symphony/ui";
import { useMutation } from "@tanstack/react-query";
import { paths } from "../../app/routes.js";
import { useLocale, useMessages, useT } from "../../lib/i18n.js";
import { useUiStore } from "../../store/uiStore.js";
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
import { UpdateActions } from "../updates/UpdateActions.js";
import { useAppUpdate } from "../updates/useAppUpdate.js";
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
    <section id={sectionDomId(id)} aria-labelledby={titleId} className="@container scroll-mt-4 rounded-xl border border-border bg-surface">
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

/** Number input bound to a number; typed text that is not a number is surfaced as NaN for validation. */
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
  <Field label={label} hint={hint} error={error}>
    <Input
      type="number"
      inputMode="numeric"
      value={Number.isFinite(value) ? String(value) : ""}
      min={min}
      max={max}
      step={step}
      onChange={(e) => {
        const text = e.target.value.trim();
        onChange(text === "" ? Number.NaN : Number(text));
      }}
    />
  </Field>
);

/** Switch with its label (and optional hint) on one row. */
const SwitchRow: React.FC<{
  label: string;
  hint?: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  disabled?: boolean;
}> = ({ label, hint, checked, onCheckedChange, disabled }) => {
  const id = React.useId();
  return (
    <div className="flex items-start gap-2.5">
      <Switch id={id} checked={checked} onCheckedChange={onCheckedChange} disabled={disabled} className="mt-0.5" />
      <div className="min-w-0">
        <Label htmlFor={id} className="text-sm font-normal">
          {label}
        </Label>
        {hint ? <p className="text-xs text-muted-foreground">{hint}</p> : null}
      </div>
    </div>
  );
};

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
      <Field group label={m.appearance.theme}>
        <RadioGroup className="flex-row flex-wrap gap-x-5 gap-y-2" value={themeMode} onValueChange={(v) => onTheme(v as ThemeMode)}>
          <RadioOption value="system" label={m.appearance.themeSystem} />
          <RadioOption value="dark" label={m.appearance.themeDark} />
          <RadioOption value="light" label={m.appearance.themeLight} />
        </RadioGroup>
      </Field>
      <Field group label={m.appearance.language}>
        <RadioGroup className="flex-row flex-wrap gap-x-5 gap-y-2" value={locale} onValueChange={(v) => onLocale(v as Locale)}>
          <RadioOption value="en" label={m.appearance.english} />
          <RadioOption value="sv" label={m.appearance.swedish} />
        </RadioGroup>
      </Field>
      <Field group label={m.appearance.density} hint={m.appearance.densityHint}>
        <RadioGroup className="flex-row flex-wrap gap-x-5 gap-y-2" value={density} onValueChange={(v) => onDensity(v as Density)}>
          <RadioOption value="compact" label={m.appearance.compact} />
          <RadioOption value="comfortable" label={m.appearance.comfortable} />
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
        <Alert intent="info">{m.desktop.notDesktop}</Alert>
      ) : null}
      <SwitchRow label={m.desktop.tray} hint={m.desktop.trayHint} checked={draft.minimize_to_tray_on_close} onCheckedChange={(checked) => update((s) => ({ ...s, minimize_to_tray_on_close: checked }))} />
      <SwitchRow label={m.desktop.autostart} hint={m.desktop.autostartHint} checked={draft.start_background_runtime_on_login} onCheckedChange={(checked) => update((s) => ({ ...s, start_background_runtime_on_login: checked }))} />
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
      <div className="grid gap-4 @3xl:grid-cols-[minmax(0,1fr)_280px]">
        <div className="grid content-start gap-3 @md:grid-cols-2">
          <NumberField
            label={m.scheduling.polling}
            hint={m.scheduling.pollingHint}
            value={draft.polling_interval_ms}
            min={2000}
            step={1000}
            error={errors.polling_interval_ms}
            onChange={(v) => update((s) => ({ ...s, polling_interval_ms: v }))}
          />
          <span className="hidden @md:block" aria-hidden />
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
          <NumberField
            label={m.scheduling.retention}
            hint={m.scheduling.retentionHint}
            value={draft.event_retention_days ?? 30}
            min={0}
            max={3650}
            error={errors.event_retention_days}
            onChange={(v) => update((s) => ({ ...s, event_retention_days: v }))}
          />
          <NumberField
            label={m.scheduling.prRefresh}
            hint={m.scheduling.prRefreshHint}
            value={draft.pr_refresh_interval_minutes ?? 15}
            min={0}
            max={1440}
            error={errors.pr_refresh_interval_minutes}
            onChange={(v) => update((s) => ({ ...s, pr_refresh_interval_minutes: v }))}
          />
          <div className="@md:col-span-2">
            <SwitchRow label={m.scheduling.thermal} checked={draft.resource_policy.reduce_on_thermal_throttle} onCheckedChange={(checked) => policy({ reduce_on_thermal_throttle: checked })} />
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
      <div className="grid gap-3 @md:grid-cols-2">
        <Field label={m.defaults.provider}>
          <Select
            value={draft.default_provider}
            onValueChange={(value) => {
              const kind = value as ProviderKind;
              update((s) => ({ ...s, default_provider: kind, default_model: s.provider_default_models[kind] ?? "" }));
            }}
          >
            <SelectTrigger>
              <SelectValue>{label(draft.default_provider)}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {ALL_PROVIDERS.map((kind) => (
                <SelectItem key={kind} value={kind}>
                  {label(kind)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        <Field label={m.defaults.model} hint={m.defaults.modelHint}>
          <Combobox
            value={draft.default_model}
            options={models}
            placeholder={status?.default_model ?? undefined}
            onValueChange={(value) => update((s) => ({ ...s, default_model: value }))}
          />
        </Field>
        <Field label={m.defaults.cloudAgent} hint={m.defaults.cloudAgentHint}>
          <Select
            value={draft.default_cloud_agent ?? "mini_swe_agent"}
            onValueChange={(value) => update((s) => ({ ...s, default_cloud_agent: value as CloudAgentKind }))}
          >
            <SelectTrigger>
              <SelectValue>{t.cloudAgent[draft.default_cloud_agent ?? "mini_swe_agent"]}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {ALL_CLOUD_AGENTS.map((agent) => (
                <SelectItem key={agent} value={agent}>
                  {t.cloudAgent[agent]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
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
          variant={configured ? "secondary" : "default"}
          disabled={rotate.isPending || status.isPending || status.isError}
          onClick={() => (configured ? setConfirm("rotate") : runRotate())}
        >
          {rotate.isPending ? <Spinner /> : <KeyRound className="size-4" />}
          {configured ? m.remote.rotate : m.remote.generate}
        </Button>
        {configured ? (
          <Button disabled={revoke.isPending || status.isError} onClick={() => setConfirm("revoke")}>
            <ShieldOff />
            {m.remote.revoke}
          </Button>
        ) : null}
      </div>
      {issued ? (
        <Alert intent="success" title={m.remote.issuedTitle}><span className="block">{m.remote.issuedBody}</span>
            <span className="mt-2 flex items-center gap-2">
              <Input readOnly value={issued} aria-label={m.remote.token} className="w-full font-mono" spellCheck={false} />
              <CopyButton value={issued} label={m.remote.copy} />
            </span>
            <Button className="mt-2" size="sm" onClick={() => setIssued(null)}>
              {m.remote.done}
            </Button></Alert>
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
      <Alert intent={draft.remote_api_enabled ? "warning" : "info"} title={m.remote.warningTitle}>
        {m.remote.warning}
      </Alert>
      <div className="grid gap-4 @3xl:grid-cols-[minmax(0,1fr)_280px]">
        <div className="space-y-3">
          <SwitchRow label={m.remote.enabled} checked={draft.remote_api_enabled} onCheckedChange={(checked) => update((s) => ({ ...s, remote_api_enabled: checked }))} />
          <div className="grid gap-3 @md:grid-cols-2">
            <Field
              label={m.remote.bind}
              hint={m.remote.bindHint}
              error={errors.remote_bind_address}
            >
              <Input
                value={draft.remote_bind_address}
                spellCheck={false}
                className="font-mono"
                onChange={(e) => update((s) => ({ ...s, remote_bind_address: e.target.value }))}
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
            error={errors.remote_allowed_origins}
          >
            <Textarea
              value={originsText}
              spellCheck={false}
              className="font-mono"
              onChange={(e) => {
                setOriginsText(e.target.value);
                update((s) => ({ ...s, remote_allowed_origins: parseOrigins(e.target.value) }));
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

export const BackupsSection: React.FC<FormSectionProps> = ({ draft, errors, update }) => {
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
             
              disabled={create.isPending}
              onClick={onCreate}
            >
              {create.isPending ? <Spinner /> : <DatabaseBackup className="size-4" />}
              {create.isPending ? m.backups.creating : m.backups.create}
            </Button>
            {backups.data ? (
              <span className="text-xs text-muted-foreground">{formatMessage(m.backups.directory, { path: backups.data.directory })}</span>
            ) : null}
          </div>
          <div className="grid gap-3 @md:grid-cols-2">
            <NumberField
              label={m.backups.interval}
              hint={m.backups.intervalHint}
              value={draft.backup_interval_hours ?? 24}
              min={0}
              max={720}
              error={errors.backup_interval_hours}
              onChange={(v) => update((s) => ({ ...s, backup_interval_hours: v }))}
            />
            <NumberField
              label={m.backups.keep}
              hint={m.backups.keepHint}
              value={draft.backup_keep ?? 7}
              min={1}
              max={100}
              error={errors.backup_keep}
              onChange={(v) => update((s) => ({ ...s, backup_keep: v }))}
            />
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
                  {b.scheduled ? (
                    <Badge tone="neutral">
                      {m.backups.scheduled}
                    </Badge>
                  ) : null}
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
         
          disabled={exporter.isPending}
          onClick={() => exporter.mutate()}
        >
          {exporter.isPending ? <Spinner /> : <FileJson className="size-4" />}
          {exporter.isPending ? m.diagnostics.exporting : m.diagnostics.export}
        </Button>
        {bundle ? (
          <>
            <span className="text-sm text-muted-foreground" role="status">
              {formatMessage(m.diagnostics.ready, { size: formatBytes(bundle.bytes, locale) })}
            </span>
            {bundle.url ? (
              <Button asChild variant="default">
                <a href={bundle.url} download="symphony-diagnostics.json">
                  <Download />
                  {m.diagnostics.download}
                </a>
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

/** Desktop only: reveal the data and log folders in Explorer (the host takes no path). */
const FolderActions: React.FC = () => {
  const m = useMessages(settingsMessages);
  const notify = useNotify();
  const open = useMutation({
    mutationFn: (which: "data" | "logs") => (which === "data" ? nativeHost.openDataDirectory() : nativeHost.openLogDirectory()),
    onError: (error) => notify("error", m.application.openFailed, errorMessage(error))
  });
  return (
    <div className="space-y-2 border-t border-border pt-3">
      <h3 className="text-sm font-medium text-foreground">{m.application.folders}</h3>
      <p className="text-xs text-muted-foreground">{m.application.foldersHint}</p>
      <div className="flex flex-wrap gap-2">
        <Button disabled={open.isPending} onClick={() => open.mutate("data")}>
          <FolderOpen />
          {m.application.openData}
        </Button>
        <Button disabled={open.isPending} onClick={() => open.mutate("logs")}>
          <FolderOpen />
          {m.application.openLogs}
        </Button>
      </div>
    </div>
  );
};

/** Desktop only: look for a newer GitHub release and install it from inside the app. */
const UpdateCheck: React.FC = () => {
  const m = useMessages(settingsMessages);
  const query = useAppUpdate();
  const autoCheck = useUiStore((s) => s.autoCheckUpdates);
  const setAutoCheck = useUiStore((s) => s.setAutoCheckUpdates);
  // A failed automatic check stays silent; an error is shown only for a check the user asked for.
  const [asked, setAsked] = useState(false);
  const result = query.data;
  const checking = query.isFetching;
  return (
    <div className="space-y-2 border-t border-border pt-3">
      <h3 className="text-sm font-medium text-foreground">{m.application.updates}</h3>
      <p className="text-xs text-muted-foreground">{m.application.updatesHint}</p>
      <SwitchRow label={m.application.autoCheckUpdates} checked={autoCheck} onCheckedChange={setAutoCheck} />
      <p className="text-xs text-muted-foreground">{m.application.autoCheckUpdatesHint}</p>
      <div className="flex flex-wrap gap-2">
        <Button
         
          disabled={checking}
          onClick={() => {
            setAsked(true);
            void query.refetch();
          }}
        >
          {checking ? <Spinner /> : <RefreshCw className="size-4" />}
          {checking ? m.application.checking : m.application.checkUpdates}
        </Button>
      </div>
      <div aria-live="polite">
        {asked && query.isError ? (
          <Alert intent="error" title={m.application.checkFailed}>{errorMessage(query.error)}</Alert>
        ) : result ? (
          <Alert intent={result.update_available ? "info" : "success"}>
            {result.latest === null
              ? m.application.noReleases
              : result.update_available
                ? formatMessage(result.prerelease ? m.application.updateAvailablePre : m.application.updateAvailable, {
                    version: result.latest,
                    current: result.current
                  })
                : formatMessage(m.application.upToDate, { version: result.current })}
          </Alert>
        ) : null}
      </div>
      {result?.update_available ? (
        <div className="space-y-2">
          {result.notes ? (
            <details className="rounded-md border border-border p-2 text-sm">
              <summary className="cursor-pointer text-xs font-medium">{m.application.releaseNotes}</summary>
              <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap font-sans text-xs">{result.notes}</pre>
            </details>
          ) : null}
          <UpdateActions update={result} />
        </div>
      ) : null}
    </div>
  );
};

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
  const setShortcutsOpen = useUiStore((s) => s.setShortcutsOpen);

  return (
    <SettingsSection id="application" intro={m.application.intro}>
      {discoveryError ? (
        <Alert intent="warning" title={m.application.discoveryErrorTitle}>{formatMessage(m.application.discoveryError, { error: discoveryError })}</Alert>
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
      <div className="border-t border-border pt-3">
        <h3 className="mb-2 text-sm font-medium text-foreground">{m.application.shortcuts}</h3>
        <Button onClick={() => setShortcutsOpen(true)}>
          <Keyboard />
          {m.application.showShortcuts}
        </Button>
      </div>
      {desktop ? <FolderActions /> : null}
      {desktop ? <UpdateCheck /> : null}
      {desktop ? (
        <div className="border-t border-border pt-3">
          <Button onClick={() => setConfirmExit(true)} disabled={exit.isPending}>
            <LogOut />
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
