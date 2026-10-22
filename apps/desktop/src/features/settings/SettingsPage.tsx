import { useCallback, useMemo, useState } from "react";
import { Button, MessageBar, MessageBarBody, MessageBarTitle, Spinner } from "@fluentui/react-components";
import { RotateCcw, Save } from "lucide-react";
import { errorMessage } from "@symphony/api-client";
import { formatMessage, type AppSettings, type Locale } from "@symphony/shared-types";
import { ErrorState, LoadingRows, PageHeader, SdkBadge, cn, useDesign, useNotify, type ThemeMode } from "@symphony/ui";
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
  sectionDomId,
  type SectionId
} from "./sections.js";
import { portOf, serverErrorFields, validateSettings, type SettingsErrors, type SettingsField } from "./validation.js";
import { useRuntime } from "../../runtime/RuntimeProvider.js";

/** Fields edited through the draft form (theme/locale persist immediately; others are not edited here). */
type EditableSettings = Pick<
  AppSettings,
  | "minimize_to_tray_on_close"
  | "start_background_runtime_on_login"
  | "default_provider"
  | "default_model"
  | "polling_interval_ms"
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
  const { reducedMotion } = useDesign();
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

  const goTo = (id: SectionId) => {
    setActive(id);
    document.getElementById(sectionDomId(id))?.scrollIntoView({ block: "start", behavior: reducedMotion ? "auto" : "smooth" });
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader
        title={m.title}
        description={m.description}
        meta={
          dirty ? (
            <SdkBadge size="sm" pill variant="soft" color="warning">
              {m.unsaved}
            </SdkBadge>
          ) : null
        }
        actions={
          <>
            {dirty && errorCount > 0 ? (
              <span className="text-xs text-destructive" role="status">
                {formatMessage(m.invalidCount, { count: errorCount })}
              </span>
            ) : null}
            <Button icon={<RotateCcw className="size-4" />} disabled={!dirty || save.isPending} onClick={onReset}>
              {m.reset}
            </Button>
            <Button
              appearance="primary"
              icon={save.isPending ? <Spinner size="tiny" /> : <Save className="size-4" />}
              disabled={!dirty || errorCount > 0 || save.isPending}
              onClick={onSave}
            >
              {m.save}
            </Button>
          </>
        }
      />
      <div className="flex min-h-0 flex-1">
        <nav aria-label={m.sectionsNav} className="hidden w-52 shrink-0 overflow-auto border-e border-border bg-surface p-2 md:block">
          <ul className="space-y-0.5">
            {SECTION_IDS.map((id) => (
              <li key={id}>
                <button
                  type="button"
                  onClick={() => goTo(id)}
                  aria-current={active === id ? "location" : undefined}
                  className={cn(
                    "flex h-row w-full items-center rounded-md px-2 text-start text-sm transition-colors",
                    active === id
                      ? "bg-surface-selected font-medium text-foreground"
                      : "text-muted-foreground hover:bg-surface-hover hover:text-foreground"
                  )}
                >
                  {m.sections[id]}
                </button>
              </li>
            ))}
          </ul>
        </nav>
        <div className="min-h-0 flex-1 overflow-auto">
          <div className="mx-auto flex max-w-4xl flex-col gap-4 p-4">
            {settings.isPending ? (
              <LoadingRows rows={10} label={m.title} />
            ) : settings.isError || !current ? (
              <ErrorState title={m.loadError} message={errorMessage(settings.error)} onRetry={() => void settings.refetch()} />
            ) : (
              <>
                {serverMessage ? (
                  <MessageBar intent="error" layout="multiline">
                    <MessageBarBody>
                      <MessageBarTitle>{m.saveFailed}</MessageBarTitle>
                      {serverMessage}
                    </MessageBarBody>
                  </MessageBar>
                ) : null}
                <AppearanceSection
                  themeMode={themeMode}
                  locale={locale}
                  density={density}
                  onTheme={onTheme}
                  onLocale={onLocale}
                  onDensity={setDensity}
                />
                <DesktopSection draft={current} errors={errors} update={update} />
                <SchedulingSection draft={current} errors={errors} update={update} />
                <DefaultsSection draft={current} errors={errors} update={update} />
                <RemoteSection draft={current} errors={errors} update={update} />
                <IntegrationsSection />
                <BackupsSection />
                <DiagnosticsSection />
                <ApplicationSection />
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
