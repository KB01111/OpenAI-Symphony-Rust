import { useCallback, useMemo, useRef, useState } from "react";
import { RotateCcw, Save } from "lucide-react";
import { errorMessage } from "@symphony/api-client";
import { formatMessage, type AppSettings, type Locale } from "@symphony/shared-types";
import { Alert, Badge, Button, cn, ErrorState, LoadingRows, PageHeader, Spinner, useNotify, type ThemeMode } from "@symphony/ui";
import { useMessages } from "../../lib/i18n.js";
import { useSaveSettings, useSettings } from "../../lib/queries.js";
import { useUiStore } from "../../store/uiStore.js";
import { settingsMessages } from "./messages.js";
import {
  AppearanceSection,
  ApplicationSection,
  DefaultsSection,
  DesktopSection,
  BackupsSection,
  DiagnosticsSection,
  IntegrationsSection,
  RemoteSection,
  SECTION_IDS,
  SchedulingSection,
  type SectionId
} from "./sections.js";
import { portOf, serverErrorFields, validateSettings, type SettingsErrors, type SettingsField } from "./validation.js";
import { useRuntime } from "../../runtime/RuntimeProvider.js";

/** The section each validated field lives in, so the nav can flag a section that has an error. */
const SECTION_FIELDS: Partial<Record<SectionId, ReadonlyArray<SettingsField>>> = {
  scheduling: [
    "polling_interval_ms",
    "max_concurrent_on_ac",
    "max_concurrent_on_battery",
    "high_memory_threshold_percent",
    "high_cpu_threshold_percent",
    "event_retention_days",
    "pr_refresh_interval_minutes"
  ],
  remote: ["remote_bind_address", "remote_port", "remote_allowed_origins"],
  backups: ["backup_interval_hours", "backup_keep"]
};

/** Fields edited through the draft form (theme/locale persist immediately; others are not edited here). */
type EditableSettings = Pick<
  AppSettings,
  | "minimize_to_tray_on_close"
  | "start_background_runtime_on_login"
  | "default_provider"
  | "default_model"
  | "polling_interval_ms"
  | "event_retention_days"
  | "backup_interval_hours"
  | "backup_keep"
  | "pr_refresh_interval_minutes"
  | "resource_policy"
  | "remote_api_enabled"
  | "remote_bind_address"
  | "remote_port"
> & { remote_allowed_origins: string[] };

function editable(s: AppSettings): EditableSettings {
  return {
    minimize_to_tray_on_close: s.minimize_to_tray_on_close,
    start_background_runtime_on_login: s.start_background_runtime_on_login,
    default_provider: s.default_provider,
    default_model: s.default_model,
    polling_interval_ms: s.polling_interval_ms,
    event_retention_days: s.event_retention_days ?? 30,
    backup_interval_hours: s.backup_interval_hours ?? 24,
    backup_keep: s.backup_keep ?? 7,
    pr_refresh_interval_minutes: s.pr_refresh_interval_minutes ?? 15,
    resource_policy: { ...s.resource_policy },
    remote_api_enabled: s.remote_api_enabled,
    remote_bind_address: s.remote_bind_address,
    remote_port: s.remote_port,
    remote_allowed_origins: [...(s.remote_allowed_origins ?? [])]
  };
}

function sameEditable(a: AppSettings, b: AppSettings): boolean {
  const x = editable(a);
  const y = editable(b);
  return (Object.keys(x) as (keyof EditableSettings)[]).every((key) =>
    key === "resource_policy"
      ? (Object.keys(x.resource_policy) as (keyof AppSettings["resource_policy"])[]).every((k) =>
          Object.is(x.resource_policy[k], y.resource_policy[k])
        )
      : key === "remote_allowed_origins"
        ? x.remote_allowed_origins.length === y.remote_allowed_origins.length &&
          x.remote_allowed_origins.every((origin, i) => origin === y.remote_allowed_origins[i])
        : Object.is(x[key], y[key])
  );
}

/** Route `/settings`. */
export default function SettingsPage() {
  const m = useMessages(settingsMessages);
  const notify = useNotify();
  const settings = useSettings();
  const save = useSaveSettings();
  const themeMode = useUiStore((s) => s.themeMode);
  const locale = useUiStore((s) => s.locale);
  const density = useUiStore((s) => s.density);
  const setThemeMode = useUiStore((s) => s.setThemeMode);
  const setLocale = useUiStore((s) => s.setLocale);
  const setDensity = useUiStore((s) => s.setDensity);

  const base = settings.data;
  const [draft, setDraft] = useState<AppSettings | null>(null);
  const [active, setActive] = useState<SectionId>("appearance");
  const current = draft ?? base;

  const resetSaveError = save.reset;
  const update = useCallback(
    (fn: (d: AppSettings) => AppSettings) => {
      if (!base) return;
      resetSaveError();
      setDraft((d) => fn(d ?? base));
    },
    [base, resetSaveError]
  );

  const { client } = useRuntime();
  const localApiPort = portOf(client?.connection.baseUrl);
  const clientErrors = useMemo<SettingsErrors>(
    () => (current ? validateSettings(current, m.validation, localApiPort) : {}),
    [current, m.validation, localApiPort]
  );
  const serverMessage = save.isError ? errorMessage(save.error) : null;
  const errors = useMemo<SettingsErrors>(() => {
    if (!serverMessage) return clientErrors;
    const merged: SettingsErrors = { ...clientErrors };
    for (const field of serverErrorFields(serverMessage)) merged[field as SettingsField] ??= serverMessage;
    return merged;
  }, [clientErrors, serverMessage]);
  const errorCount = Object.keys(clientErrors).length;
  const dirty = Boolean(draft && base && !sameEditable(draft, base));

  const onSave = () => {
    if (!draft || !base || !dirty || errorCount > 0 || save.isPending) return;
    const edits = editable(draft);
    // Applied to the latest saved settings when the (serialized) write runs.
    save.mutate(
      (latest) => {
        const ui = useUiStore.getState();
        return { ...latest, ...edits, theme: ui.themeMode, locale: ui.locale };
      },
      {
        onSuccess: () => {
          setDraft(null);
          notify("success", m.saved);
        }
      }
    );
  };

  const onReset = () => {
    save.reset();
    setDraft(null);
  };

  /**
   * Theme and language apply at once without touching the unsaved draft; `PreferencesBridge`
   * persists them (one serialized write path for preferences).
   */
  const onTheme = (mode: ThemeMode) => setThemeMode(mode);
  const onLocale = (next: Locale) => setLocale(next);

  // One section at a time keeps the page short; edits live in the shared draft, so switching loses nothing.
  const content = useRef<HTMLDivElement>(null);
  const goTo = (id: SectionId) => {
    setActive(id);
    content.current?.scrollTo({ top: 0 });
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader
        title={m.title}
        description={m.description}
        meta={
          dirty ? (
            <Badge tone="warning">
              {m.unsaved}
            </Badge>
          ) : null
        }
        actions={
          <>
            {dirty && errorCount > 0 ? (
              <span className="text-xs text-destructive" role="status">
                {formatMessage(m.invalidCount, { count: errorCount })}
              </span>
            ) : null}
            {/* Also after a failed save: the error alert stays until Reset (or the next edit) clears it. */}
            {dirty || save.isError ? (
              <Button disabled={save.isPending} onClick={onReset}>
                <RotateCcw />
                {m.reset}
              </Button>
            ) : null}
            <Button
              variant="default"
              disabled={!dirty || errorCount > 0 || save.isPending}
              onClick={onSave}
            >
              {save.isPending ? <Spinner /> : <Save className="size-4" />}
              {m.save}
            </Button>
          </>
        }
      />
      {/* Container queries, not viewport breakpoints: the sidebar takes its share before this page is measured. */}
      <div className="@container flex min-h-0 flex-1 flex-col @3xl:flex-row">
        {/* A scrolling strip while the page is narrow, a side list once it has room. */}
        <nav
          aria-label={m.sectionsNav}
          className="shrink-0 overflow-x-auto border-b border-border bg-surface p-2 @3xl:w-52 @3xl:overflow-y-auto @3xl:overflow-x-hidden @3xl:border-b-0 @3xl:border-e"
        >
          <ul className="flex gap-1 @3xl:block @3xl:space-y-0.5">
            {SECTION_IDS.map((id) => (
              <li key={id}>
                <button
                  type="button"
                  onClick={() => goTo(id)}
                  aria-current={active === id ? "location" : undefined}
                  className={cn(
                    "flex h-row shrink-0 items-center whitespace-nowrap rounded-md px-2 text-start text-sm transition-colors @3xl:h-auto @3xl:min-h-row @3xl:w-full @3xl:whitespace-normal @3xl:py-1.5",
                    active === id
                      ? "bg-surface-selected font-medium text-foreground"
                      : "text-muted-foreground hover:bg-surface-hover hover:text-foreground"
                  )}
                >
                  {m.sections[id]}
                  {(SECTION_FIELDS[id] ?? []).some((field) => errors[field]) ? (
                    <span className="ms-1.5 size-2 shrink-0 rounded-full bg-destructive @3xl:ms-auto" role="img" aria-label={m.invalidSection} />
                  ) : null}
                </button>
              </li>
            ))}
          </ul>
        </nav>
        <div ref={content} className="min-h-0 flex-1 overflow-auto">
          <div className="mx-auto flex max-w-4xl flex-col gap-4 p-4">
            {settings.isPending ? (
              <LoadingRows rows={10} label={m.title} />
            ) : settings.isError || !current ? (
              <ErrorState title={m.loadError} message={errorMessage(settings.error)} onRetry={() => void settings.refetch()} />
            ) : (
              <>
                {serverMessage ? (
                  <Alert intent="error" title={m.saveFailed}>{serverMessage}</Alert>
                ) : null}
                {active === "appearance" ? (
                  <AppearanceSection
                    themeMode={themeMode}
                    locale={locale}
                    density={density}
                    onTheme={onTheme}
                    onLocale={onLocale}
                    onDensity={setDensity}
                  />
                ) : null}
                {active === "desktop" ? <DesktopSection draft={current} errors={errors} update={update} /> : null}
                {active === "scheduling" ? <SchedulingSection draft={current} errors={errors} update={update} /> : null}
                {active === "defaults" ? <DefaultsSection draft={current} errors={errors} update={update} /> : null}
                {/* Stays mounted (hidden) so a one-time access token that was just issued survives switching sections. */}
                <div hidden={active !== "remote"}>
                  <RemoteSection draft={current} errors={errors} update={update} />
                </div>
                {active === "integrations" ? <IntegrationsSection /> : null}
                {active === "backups" ? <BackupsSection draft={current} errors={errors} update={update} /> : null}
                {active === "diagnostics" ? <DiagnosticsSection /> : null}
                {active === "application" ? <ApplicationSection /> : null}
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
