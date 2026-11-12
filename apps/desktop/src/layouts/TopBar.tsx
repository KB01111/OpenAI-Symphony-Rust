import React from "react";
import { useLocation } from "react-router-dom";
import { Bell, Moon, Pause, Play, RefreshCw, Search, Sun, SunMoon } from "lucide-react";
import { formatMessage } from "@symphony/shared-types";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
  Hint,
  Popover,
  PopoverContent,
  PopoverTrigger,
  StatusDot,
  cn
} from "@symphony/ui";
import { shellMessages } from "../app/messages.js";
import { NAV_ITEMS } from "../app/routes.js";
import { ApprovalsTab } from "../features/activity/ActivityPanels.js";
import { useMessages } from "../lib/i18n.js";
import { useApprovals, useRefreshRuntime, useRuntimeState, useSchedulerToggle } from "../lib/queries.js";
import { useUiStore, type UiState } from "../store/uiStore.js";
import { useConnectionSummary } from "./useConnectionSummary.js";

/** Slim top bar: where you are, the command palette, runtime status and global actions. */
export const TopBar: React.FC<{ children?: React.ReactNode }> = ({ children }) => {
  const m = useMessages(shellMessages);
  const state = useRuntimeState();
  const approvals = useApprovals(true);
  const refresh = useRefreshRuntime();
  const scheduler = useSchedulerToggle();
  const location = useLocation();
  const openPalette = useUiStore((s) => s.openPalette);
  const themeMode = useUiStore((s) => s.themeMode);
  const setThemeMode = useUiStore((s) => s.setThemeMode);
  const connection = useConnectionSummary();
  const paused = state.data?.scheduler_paused ?? false;
  const pending = approvals.data?.length ?? 0;
  // A task thread (/runs/:id) belongs to Tasks.
  const current = location.pathname.startsWith("/runs") ? "/tasks" : location.pathname;
  const here = NAV_ITEMS.find((item) => current === item.path || current.startsWith(`${item.path}/`));
  const ThemeIcon = themeMode === "light" ? Sun : themeMode === "dark" ? Moon : SunMoon;

  return (
    <header className="flex h-11 shrink-0 items-center gap-3 ps-1 pe-1">
      <div className="flex min-w-0 flex-1 items-center gap-2.5">
        {children}
        {here ? <span className="hidden shrink-0 truncate text-sm font-medium text-muted-foreground lg:inline">{m.nav[here.key]}</span> : null}
        <button
          type="button"
          onClick={() => openPalette("all")}
          className="flex h-8 w-full min-w-0 max-w-md items-center gap-2 rounded-xl border border-border bg-surface/60 px-3 text-start text-sm text-muted-foreground outline-none transition-colors hover:border-border-strong hover:bg-surface hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60"
          aria-keyshortcuts="Control+K"
        >
          <Search className="size-3.5 shrink-0" aria-hidden />
          <span className="min-w-0 flex-1 truncate">{m.commandBar.search}</span>
          <kbd className="shrink-0 rounded-md bg-muted px-1.5 py-px font-mono text-[10px] text-muted-foreground">{m.shortcuts.palette}</kbd>
        </button>
      </div>

      <div className="flex shrink-0 items-center justify-end gap-0.5">
        {/* The footer always shows the connection; up here it only speaks up when something is wrong. */}
        {connection.tone !== "success" ? (
          <Hint label={connection.label}>
            <span className="inline-flex items-center gap-1.5 rounded-md px-2 text-xs text-muted-foreground">
              <StatusDot tone={connection.tone} pulse={connection.tone === "warning"} label={connection.label} />
              <span className="hidden xl:inline">{connection.short}</span>
            </span>
          </Hint>
        ) : null}
        <Hint label={paused ? m.commandBar.resume : m.commandBar.pause}>
          <Button
            variant="subtle"
            size="icon"
            aria-label={paused ? m.commandBar.resume : m.commandBar.pause}
            aria-pressed={paused}
            disabled={scheduler.isPending || !state.data}
            onClick={() => scheduler.mutate(!paused)}
          >
            {paused ? <Play /> : <Pause />}
          </Button>
        </Hint>
        <Hint label={`${m.commandBar.refresh} (${m.shortcuts.refresh})`}>
          <Button variant="subtle" size="icon" aria-label={m.commandBar.refresh} disabled={refresh.isPending} onClick={() => refresh.mutate()}>
            <RefreshCw className={cn(refresh.isPending && "animate-spin")} />
          </Button>
        </Hint>
        <Popover>
          <Hint label={pending > 0 ? formatMessage(m.commandBar.approvals, { count: pending }) : m.commandBar.noApprovals}>
            <PopoverTrigger asChild>
              <Button variant="subtle" size="icon" className="relative" aria-label={pending > 0 ? formatMessage(m.commandBar.approvals, { count: pending }) : m.commandBar.noApprovals}>
                <Bell />
                {pending > 0 ? (
                  <span aria-hidden className="absolute end-1 top-1 inline-flex h-3.5 min-w-3.5 items-center justify-center rounded-full bg-warning px-0.5 text-[9px] font-semibold text-background">
                    {pending}
                  </span>
                ) : null}
              </Button>
            </PopoverTrigger>
          </Hint>
          <PopoverContent align="end" className="max-h-[70vh] w-[28rem] overflow-auto rounded-xl p-0">
            <div className="h-[min(28rem,60vh)]">
              <ApprovalsTab />
            </div>
          </PopoverContent>
        </Popover>
        <DropdownMenu>
          <Hint label={m.commandBar.theme}>
            <DropdownMenuTrigger asChild>
              <Button variant="subtle" size="icon" aria-label={m.commandBar.theme}>
                <ThemeIcon />
              </Button>
            </DropdownMenuTrigger>
          </Hint>
          <DropdownMenuContent>
            <DropdownMenuRadioGroup value={themeMode} onValueChange={(v) => setThemeMode(v as UiState["themeMode"])}>
              <DropdownMenuRadioItem value="system">{m.commandBar.themeSystem}</DropdownMenuRadioItem>
              <DropdownMenuRadioItem value="dark">{m.commandBar.themeDark}</DropdownMenuRadioItem>
              <DropdownMenuRadioItem value="light">{m.commandBar.themeLight}</DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </header>
  );
};
