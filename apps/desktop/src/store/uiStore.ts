import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { Locale, TaskStatus } from "@symphony/shared-types";
import type { Density, ThemeMode } from "@symphony/ui";
import { safeJSONStorage } from "./safeStorage.js";

/** Tabs of the run details panel next to a thread, and of the Activity view. */
export type DrawerTab = "events" | "agent" | "terminal" | "validations" | "approvals" | "problems";
export type TaskView = "board" | "table" | "runs";
export type LogMode = "concise" | "technical";

/** Where an in-app update install stands. Shared by the banner, Settings and the confirm dialog. */
export type UpdateInstallState =
  | { phase: "idle" }
  /** Agents are running: waiting for the user to confirm that they are interrupted. */
  | { phase: "confirm"; running: number }
  | { phase: "downloading"; downloaded: number; total: number | null }
  | { phase: "installing" }
  | { phase: "error"; message: string };

export interface TaskFilters {
  search: string;
  statuses: TaskStatus[];
  providers: string[];
  priority: number | null;
}

/**
 * Presentation-only state. Server data (tasks, projects, approvals…) is never copied here; it
 * lives in the TanStack Query cache fed by the runtime API and event stream.
 */
export interface UiState {
  // persisted preferences
  sidebarCollapsed: boolean;
  activeProjectId: string | null;
  taskView: TaskView;
  density: Density;
  /** Applied immediately; mirrored to runtime settings when saved. */
  themeMode: ThemeMode;
  locale: Locale;
  /** Selected tab of the Activity view. */
  drawerTab: DrawerTab;
  /** Run details panel next to an open thread. */
  threadPanelOpen: boolean;
  /** The "Manage" group (secondary views) in the sidebar. */
  manageOpen: boolean;
  /** Project ids whose thread group is collapsed in the sidebar. */
  collapsedProjects: string[];
  logMode: LogMode;
  taskFilters: TaskFilters;
  /** Look for updates at startup and every few hours (desktop only). Off = only on request. */
  autoCheckUpdates: boolean;
  /** Version whose banner the user dismissed; a newer version shows the banner again. */
  dismissedUpdate: string | null;

  // session-only
  updateInstall: UpdateInstallState;
  paletteOpen: boolean;
  paletteMode: "all" | "projects";
  newTaskOpen: boolean;
  addProjectOpen: boolean;
  shortcutsOpen: boolean;

  setSidebarCollapsed(collapsed: boolean): void;
  setActiveProject(id: string | null): void;
  setTaskView(view: TaskView): void;
  setDensity(density: Density): void;
  setThemeMode(mode: ThemeMode): void;
  setLocale(locale: Locale): void;
  setDrawerTab(tab: DrawerTab): void;
  setThreadPanelOpen(open: boolean): void;
  setManageOpen(open: boolean): void;
  toggleProjectCollapsed(id: string): void;
  setLogMode(mode: LogMode): void;
  setTaskFilters(filters: Partial<TaskFilters>): void;
  resetTaskFilters(): void;
  setAutoCheckUpdates(enabled: boolean): void;
  dismissUpdate(version: string | null): void;
  setUpdateInstall(state: UpdateInstallState): void;
  openPalette(mode?: "all" | "projects"): void;
  closePalette(): void;
  setNewTaskOpen(open: boolean): void;
  setAddProjectOpen(open: boolean): void;
  setShortcutsOpen(open: boolean): void;
}

export const DEFAULT_TASK_FILTERS: TaskFilters = { search: "", statuses: [], providers: [], priority: null };

export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
      sidebarCollapsed: false,
      activeProjectId: null,
      taskView: "board",
      density: "compact",
      themeMode: "system",
      locale: "en",
      drawerTab: "events",
      threadPanelOpen: true,
      manageOpen: true,
      collapsedProjects: [],
      logMode: "concise",
      taskFilters: DEFAULT_TASK_FILTERS,
      autoCheckUpdates: true,
      dismissedUpdate: null,

      updateInstall: { phase: "idle" },
      paletteOpen: false,
      paletteMode: "all",
      newTaskOpen: false,
      addProjectOpen: false,
      shortcutsOpen: false,

      setSidebarCollapsed: (sidebarCollapsed) => set({ sidebarCollapsed }),
      setActiveProject: (activeProjectId) => set({ activeProjectId }),
      setTaskView: (taskView) => set({ taskView }),
      setDensity: (density) => set({ density }),
      setThemeMode: (themeMode) => set({ themeMode }),
      setLocale: (locale) => set({ locale }),
      setDrawerTab: (drawerTab) => set({ drawerTab }),
      setThreadPanelOpen: (threadPanelOpen) => set({ threadPanelOpen }),
      setManageOpen: (manageOpen) => set({ manageOpen }),
      toggleProjectCollapsed: (id) =>
        set((s) => ({
          collapsedProjects: s.collapsedProjects.includes(id) ? s.collapsedProjects.filter((x) => x !== id) : [...s.collapsedProjects, id]
        })),
      setLogMode: (logMode) => set({ logMode }),
      setTaskFilters: (filters) => set((s) => ({ taskFilters: { ...s.taskFilters, ...filters } })),
      resetTaskFilters: () => set({ taskFilters: DEFAULT_TASK_FILTERS }),
      setAutoCheckUpdates: (autoCheckUpdates) => set({ autoCheckUpdates }),
      dismissUpdate: (dismissedUpdate) => set({ dismissedUpdate }),
      setUpdateInstall: (updateInstall) => set({ updateInstall }),
      openPalette: (paletteMode = "all") => set({ paletteOpen: true, paletteMode }),
      closePalette: () => set({ paletteOpen: false }),
      setNewTaskOpen: (newTaskOpen) => set({ newTaskOpen }),
      setAddProjectOpen: (addProjectOpen) => set({ addProjectOpen }),
      setShortcutsOpen: (shortcutsOpen) => set({ shortcutsOpen })
    }),
    {
      name: "symphony.ui.v2",
      version: 1,
      storage: safeJSONStorage,
      partialize: (s) => ({
        sidebarCollapsed: s.sidebarCollapsed,
        activeProjectId: s.activeProjectId,
        taskView: s.taskView,
        density: s.density,
        themeMode: s.themeMode,
        locale: s.locale,
        drawerTab: s.drawerTab,
        threadPanelOpen: s.threadPanelOpen,
        manageOpen: s.manageOpen,
        collapsedProjects: s.collapsedProjects,
        logMode: s.logMode,
        taskFilters: s.taskFilters,
        autoCheckUpdates: s.autoCheckUpdates,
        dismissedUpdate: s.dismissedUpdate
      })
    }
  )
);
