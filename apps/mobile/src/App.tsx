import React, { useCallback, useMemo, useState } from "react";
import { QueryClient, QueryClientProvider, useIsFetching } from "@tanstack/react-query";
import { Button, Tooltip } from "@fluentui/react-components";
import { Activity, Bot, ListChecks, LogOut, RefreshCw, ShieldQuestion } from "lucide-react";
import { formatMessage, type Locale } from "@symphony/shared-types";
import { StatusDot, SymphonyDesignProvider } from "@symphony/ui";
import { FluentBridge } from "@symphony/ui/legacy";
import { isSymphonyApiError } from "@symphony/api-client";
import {
  browserStorage,
  forgetToken,
  loadConnection,
  loadLocale,
  saveConnection,
  saveLocale,
  type ConnectionSettings
} from "./connection.js";
import { LocaleProvider, useStrings } from "./i18n.js";
import { SessionProvider, useSession } from "./session.js";
import { ApprovalsScreen } from "./screens/ApprovalsScreen.js";
import { ConnectScreen } from "./screens/ConnectScreen.js";
import { OverviewScreen } from "./screens/OverviewScreen.js";
import { RunsScreen } from "./screens/RunsScreen.js";
import { TasksScreen } from "./screens/TasksScreen.js";
import { usePendingApprovals } from "./screens/queries.js";

type TabId = "overview" | "tasks" | "approvals" | "runs";

function createQueryClient(onUnauthorized: () => void) {
  const client = new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 2_000,
        retry: (count, error) => !(isSymphonyApiError(error) && (error.status === 401 || error.status === 403)) && count < 2
      }
    }
  });
  client.getQueryCache().subscribe((event) => {
    const error = event.query.state.error;
    if (event.type === "updated" && isSymphonyApiError(error) && error.status === 401) onUnauthorized();
  });
  return client;
}

const statusTone = { open: "success", connecting: "warning", reconnecting: "warning", idle: "neutral", closed: "neutral", unauthorized: "danger" } as const;

const CompanionShell: React.FC<{ locale: Locale; onToggleLocale: () => void }> = ({ onToggleLocale }) => {
  const { m, t } = useStrings();
  const session = useSession();
  const approvals = usePendingApprovals();
  const fetching = useIsFetching() > 0;
  const [tab, setTab] = useState<TabId>("overview");
  const host = useMemo(() => {
    try {
      return new URL(session.baseUrl).host;
    } catch {
      return session.baseUrl;
    }
  }, [session.baseUrl]);

  const tabs: { id: TabId; label: string; icon: React.ReactNode; badge?: number }[] = [
    { id: "overview", label: m.tabOverview, icon: <Activity className="size-5" /> },
    { id: "tasks", label: m.tabTasks, icon: <ListChecks className="size-5" /> },
    { id: "approvals", label: m.tabApprovals, icon: <ShieldQuestion className="size-5" />, badge: approvals.data?.length },
    { id: "runs", label: m.tabRuns, icon: <Bot className="size-5" /> }
  ];
  const title = tabs.find((x) => x.id === tab)?.label ?? "";

  return (
    <div className="flex h-full flex-col bg-background text-foreground">
      <header className="flex items-center gap-2 border-b border-border bg-surface px-4 py-2" style={{ paddingTop: "max(0.5rem, env(safe-area-inset-top))" }}>
        <div className="min-w-0 flex-1">
          <h1 className="truncate font-display text-base font-semibold">{title}</h1>
          <p className="flex items-center gap-1.5 truncate text-xs text-muted-foreground">
            <StatusDot
              tone={statusTone[session.streamStatus]}
              pulse={session.streamStatus === "open"}
              label={session.streamStatus === "open" ? t.connection.live : session.streamStatus === "unauthorized" ? t.connection.unauthorized : t.connection.reconnecting}
            />
            {formatMessage(m.connectedTo, { host })}
          </p>
        </div>
        <Tooltip content={m.switchLanguageLabel} relationship="label">
          <Button appearance="subtle" size="small" onClick={onToggleLocale}>
            {m.switchLanguage}
          </Button>
        </Tooltip>
        <Tooltip content={fetching ? m.refreshing : t.common.refresh} relationship="label">
          <Button appearance="subtle" size="small" icon={<RefreshCw className="size-4" />} disabled={fetching} onClick={() => void session.refresh()} />
        </Tooltip>
        <Tooltip content={t.mobile.disconnect} relationship="label">
          <Button appearance="subtle" size="small" icon={<LogOut className="size-4" />} onClick={session.disconnect} />
        </Tooltip>
      </header>
      <main className="min-h-0 flex-1 overflow-y-auto p-3" aria-live="off">
        {tab === "overview" ? <OverviewScreen onOpen={setTab} /> : null}
        {tab === "tasks" ? <TasksScreen /> : null}
        {tab === "approvals" ? <ApprovalsScreen /> : null}
        {tab === "runs" ? <RunsScreen /> : null}
      </main>
      <nav aria-label={m.sections} className="grid grid-cols-4 border-t border-border bg-surface" style={{ paddingBottom: "env(safe-area-inset-bottom)" }}>
        {tabs.map((x) => (
          <button
            key={x.id}
            type="button"
            aria-current={tab === x.id ? "page" : undefined}
            onClick={() => setTab(x.id)}
            className={`relative flex min-h-14 flex-col items-center justify-center gap-0.5 text-xs ${tab === x.id ? "text-accent" : "text-muted-foreground"}`}
          >
            <span aria-hidden className="size-5">
              {x.icon}
            </span>
            {x.label}
            {x.badge ? (
              <span className="absolute right-1/4 top-1.5 min-w-4 rounded-full bg-warning px-1 text-[10px] font-semibold text-background">{x.badge}</span>
            ) : null}
          </button>
        ))}
      </nav>
    </div>
  );
};

/** Symphony Remote: phone-first companion for a runtime with remote access enabled. */
export default function MobileApp() {
  const storage = useMemo(() => browserStorage(), []);
  const [locale, setLocale] = useState<Locale>(() => loadLocale(storage, typeof navigator === "undefined" ? null : navigator.language));
  const [connection, setConnection] = useState<ConnectionSettings | null>(() => loadConnection(storage));
  const [connected, setConnected] = useState(() => Boolean(connection?.token));
  const [rejected, setRejected] = useState(false);

  const onUnauthorized = useCallback(() => {
    forgetToken(storage);
    setConnection((c) => (c ? { ...c, token: "" } : c));
    setConnected(false);
    setRejected(true);
  }, [storage]);
  const queryClient = useMemo(() => createQueryClient(onUnauthorized), [onUnauthorized]);

  const toggleLocale = () => {
    const next: Locale = locale === "en" ? "sv" : "en";
    saveLocale(storage, next);
    setLocale(next);
  };

  return (
    <SymphonyDesignProvider themeMode="system" density="comfortable" locale={locale}>
      <FluentBridge>
      <LocaleProvider locale={locale}>
        <QueryClientProvider client={queryClient}>
          {connected && connection ? (
            <SessionProvider
              key={`${connection.baseUrl}|${connection.token}`}
              baseUrl={connection.baseUrl}
              token={connection.token}
              onUnauthorized={onUnauthorized}
              onDisconnect={() => {
                forgetToken(storage);
                queryClient.clear();
                setConnection((c) => (c ? { ...c, token: "", remember: false } : c));
                setConnected(false);
                setRejected(false);
              }}
            >
              <CompanionShell locale={locale} onToggleLocale={toggleLocale} />
            </SessionProvider>
          ) : (
            <ConnectScreen
              initial={connection}
              rejected={rejected}
              onConnected={(settings) => {
                saveConnection(storage, settings);
                queryClient.clear();
                setConnection(settings);
                setRejected(false);
                setConnected(true);
              }}
            />
          )}
        </QueryClientProvider>
      </LocaleProvider>
      </FluentBridge>
    </SymphonyDesignProvider>
  );
}
