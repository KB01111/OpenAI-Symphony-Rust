import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { Locale, TaskStatus } from "@symphony/shared-types";
import type { Density, ThemeMode } from "@symphony/ui";
import { safeJSONStorage } from "./safeStorage.js";

export type InspectorTarget =
  | { kind: "task"; id: string }
  | { kind: "project"; id: string }
  | { kind: "worktree"; id: string }
  | { kind: "review"; id: string };

export type DrawerTab = "events" | "agent" | "terminal" | "validations" | "approvals" | "problems";
export type TaskView = "board" | "table";
export type LogMode = "concise" | "technical";

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
  inspectorOpen: boolean;
  drawerOpen: boolean;
  drawerTab: DrawerTab;
  logMode: LogMode;
  lastRoute: string;
  taskFilters: TaskFilters;

  // session-only
  inspector: InspectorTarget | null;
  paletteOpen: boolean;
  paletteMode: "all" | "projects";
  newTaskOpen: boolean;
  addProjectOpen: boolean;

  setSidebarCollapsed(collapsed: boolean): void;
  setActiveProject(id: string | null): void;
  setTaskView(view: TaskView): void;
  setDensity(density: Density): void;
  setThemeMode(mode: ThemeMode): void;
  setLocale(locale: Locale): void;
  setInspectorOpen(open: boolean): void;
  inspect(target: InspectorTarget | null): void;
  setDrawerOpen(open: boolean): void;
  setDrawerTab(tab: DrawerTab): void;
  setLogMode(mode: LogMode): void;
  setLastRoute(route: string): void;
  setTaskFilters(filters: Partial<TaskFilters>): void;
  resetTaskFilters(): void;
  openPalette(mode?: "all" | "projects"): void;
  closePalette(): void;
  setNewTaskOpen(open: boolean): void;
  setAddProjectOpen(open: boolean): void;
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
      inspectorOpen: true,
      drawerOpen: true,
      drawerTab: "events",
      logMode: "concise",
      lastRoute: "/overview",
      taskFilters: DEFAULT_TASK_FILTERS,

      inspector: null,
      paletteOpen: false,
      paletteMode: "all",
      newTaskOpen: false,
      addProjectOpen: false,

      setSidebarCollapsed: (sidebarCollapsed) => set({ sidebarCollapsed }),
      setActiveProject: (activeProjectId) => set({ activeProjectId }),
      setTaskView: (taskView) => set({ taskView }),
      setDensity: (density) => set({ density }),
      setThemeMode: (themeMode) => set({ themeMode }),
      setLocale: (locale) => set({ locale }),
      setInspectorOpen: (inspectorOpen) => set({ inspectorOpen }),
      inspect: (inspector) => set(inspector ? { inspector, inspectorOpen: true } : { inspector: null }),
      setDrawerOpen: (drawerOpen) => set({ drawerOpen }),
      setDrawerTab: (drawerTab) => set({ drawerTab, drawerOpen: true }),
      setLogMode: (logMode) => set({ logMode }),
      setLastRoute: (lastRoute) => set({ lastRoute }),
      setTaskFilters: (filters) => set((s) => ({ taskFilters: { ...s.taskFilters, ...filters } })),
      resetTaskFilters: () => set({ taskFilters: DEFAULT_TASK_FILTERS }),
      openPalette: (paletteMode = "all") => set({ paletteOpen: true, paletteMode }),
      closePalette: () => set({ paletteOpen: false }),
      setNewTaskOpen: (newTaskOpen) => set({ newTaskOpen }),
      setAddProjectOpen: (addProjectOpen) => set({ addProjectOpen })
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
        inspectorOpen: s.inspectorOpen,
        drawerOpen: s.drawerOpen,
        drawerTab: s.drawerTab,
        logMode: s.logMode,
        lastRoute: s.lastRoute,
        taskFilters: s.taskFilters
      })
    }
  )
);
