import React from "react";
import {
  CounterBadge,
  Menu,
  MenuItemRadio,
  MenuList,
  MenuPopover,
  MenuTrigger,
  Toolbar,
  ToolbarButton,
  ToolbarDivider,
  Tooltip
} from "@fluentui/react-components";
import { Bell, Moon, PanelBottom, PanelRight, Pause, Play, Plus, RefreshCw, Search, Sun, SunMoon } from "lucide-react";
import { formatMessage } from "@symphony/shared-types";
import { StatusDot } from "@symphony/ui";
import { shellMessages } from "../app/messages.js";
import { useMessages } from "../lib/i18n.js";
import { useApprovals, useProjects, useRefreshRuntime, useRuntimeState, useSchedulerToggle } from "../lib/queries.js";
import { useUiStore } from "../store/uiStore.js";
import { useConnectionSummary } from "./useConnectionSummary.js";

/**
 * In-app command area below the native Windows title bar (native decorations are kept so
 * dragging, snapping, maximize and system controls behave as expected).
 */
export const CommandBar: React.FC = () => {
  const m = useMessages(shellMessages);
  const state = useRuntimeState();
  const approvals = useApprovals(true);
  const projects = useProjects();
  const refresh = useRefreshRuntime();
  const scheduler = useSchedulerToggle();
  const openPalette = useUiStore((s) => s.openPalette);
  const setNewTaskOpen = useUiStore((s) => s.setNewTaskOpen);
  const inspectorOpen = useUiStore((s) => s.inspectorOpen);
  const setInspectorOpen = useUiStore((s) => s.setInspectorOpen);
  const drawerOpen = useUiStore((s) => s.drawerOpen);
  const setDrawerOpen = useUiStore((s) => s.setDrawerOpen);
  const themeMode = useUiStore((s) => s.themeMode);
  const setThemeMode = useUiStore((s) => s.setThemeMode);
  const activeProjectId = useUiStore((s) => s.activeProjectId);
  const connection = useConnectionSummary();
  const paused = state.data?.scheduler_paused ?? false;
  const pending = approvals.data?.length ?? 0;
  const project = projects.data?.find((p) => p.id === activeProjectId);

  return (
    <header className="flex h-11 shrink-0 items-center gap-2 border-b border-border bg-surface px-2">
      <div className="flex min-w-0 items-center gap-2 ps-1">
        <span aria-hidden className="grid size-6 place-items-center rounded-md bg-accent font-display text-xs font-bold text-accent-foreground">
          S
        </span>
        <span className="font-display text-sm font-semibold text-foreground">Symphony</span>
        <span className="text-subtle-foreground" aria-hidden>
          /
        </span>
        <span className="max-w-48 truncate text-sm text-muted-foreground" aria-label={m.project.current}>
          {project?.name ?? m.project.all}
        </span>
      </div>

      <button
        type="button"
        onClick={() => openPalette("all")}
        className="mx-auto flex h-7 w-full max-w-md items-center gap-2 rounded-md border border-border bg-surface-sunken px-2.5 text-start text-sm text-muted-foreground transition-colors hover:border-border-strong hover:text-foreground"
        aria-keyshortcuts="Control+K"
      >
        <Search className="size-3.5" aria-hidden />
        <span className="flex-1 truncate">{m.commandBar.search}</span>
        <kbd className="rounded border border-border px-1 font-mono text-[10px]">{m.shortcuts.palette}</kbd>
      </button>

      <Toolbar aria-label="Symphony" size="small" className="!gap-0.5 !p-0">
        <Tooltip content={connection.label} relationship="description">
          <span className="inline-flex items-center gap-1.5 px-2 text-xs text-muted-foreground">
            <StatusDot tone={connection.tone} pulse={connection.tone === "warning"} label={connection.label} />
            <span className="hidden xl:inline">{connection.short}</span>
          </span>
        </Tooltip>
        <ToolbarDivider />
        <Tooltip content={`${m.commandBar.newTask} (${m.shortcuts.newTask})`} relationship="label">
          <ToolbarButton appearance="primary" icon={<Plus className="size-4" />} onClick={() => setNewTaskOpen(true)}>
            <span className="hidden lg:inline">{m.commandBar.newTask}</span>
          </ToolbarButton>
        </Tooltip>
        <Tooltip content={paused ? m.commandBar.resume : m.commandBar.pause} relationship="label">
          <ToolbarButton
            icon={paused ? <Play className="size-4" /> : <Pause className="size-4" />}
            aria-pressed={paused}
            disabled={scheduler.isPending || !state.data}
            onClick={() => scheduler.mutate(!paused)}
          />
        </Tooltip>
        <Tooltip content={`${m.commandBar.refresh} (${m.shortcuts.refresh})`} relationship="label">
          <ToolbarButton
            icon={<RefreshCw className={refresh.isPending ? "size-4 animate-spin" : "size-4"} />}
            disabled={refresh.isPending}
            onClick={() => refresh.mutate()}
          />
        </Tooltip>
        <Tooltip content={pending > 0 ? formatMessage(m.commandBar.approvals, { count: pending }) : m.commandBar.noApprovals} relationship="label">
          <span className="relative inline-flex">
            <ToolbarButton icon={<Bell className="size-4" />} onClick={() => useUiStore.getState().setDrawerTab("approvals")} />
            {pending > 0 ? <CounterBadge count={pending} size="extra-small" color="important" className="!absolute end-0.5 top-0.5 pointer-events-none" /> : null}
          </span>
        </Tooltip>
        <Menu checkedValues={{ theme: [themeMode] }} onCheckedValueChange={(_, d) => setThemeMode(d.checkedItems[0] as typeof themeMode)}>
          <MenuTrigger disableButtonEnhancement>
            <Tooltip content={m.commandBar.theme} relationship="label">
              <ToolbarButton icon={themeMode === "light" ? <Sun className="size-4" /> : themeMode === "dark" ? <Moon className="size-4" /> : <SunMoon className="size-4" />} />
            </Tooltip>
          </MenuTrigger>
          <MenuPopover>
            <MenuList>
              <MenuItemRadio name="theme" value="system">{m.commandBar.themeSystem}</MenuItemRadio>
              <MenuItemRadio name="theme" value="dark">{m.commandBar.themeDark}</MenuItemRadio>
              <MenuItemRadio name="theme" value="light">{m.commandBar.themeLight}</MenuItemRadio>
            </MenuList>
          </MenuPopover>
        </Menu>
        <ToolbarDivider />
        <Tooltip content={m.commandBar.toggleDrawer} relationship="label">
          <ToolbarButton icon={<PanelBottom className="size-4" />} aria-pressed={drawerOpen} onClick={() => setDrawerOpen(!drawerOpen)} />
        </Tooltip>
        <Tooltip content={m.commandBar.toggleInspector} relationship="label">
          <ToolbarButton icon={<PanelRight className="size-4" />} aria-pressed={inspectorOpen} onClick={() => setInspectorOpen(!inspectorOpen)} />
        </Tooltip>
      </Toolbar>
    </header>
  );
};
