import { create } from "zustand";
import type { Locale } from "@symphony/shared-types";

export type DesktopSection =
  | "dashboard"
  | "projects"
  | "taskBoard"
  | "agentControl"
  | "executionView"
  | "prReview"
  | "workflowEditor"
  | "settings";

export interface UiState {
  activeSection: DesktopSection;
  selectedProjectId: string | null;
  selectedTaskId: string | null;
  selectedIssueIdentifier: string | null;
  logMode: "concise" | "detailed";
  themeMode: "system" | "dark" | "light";
  locale: Locale;
  commandPaletteOpen: boolean;
  onboardingOpen: boolean;
  setActiveSection: (section: DesktopSection) => void;
  selectProject: (projectId: string | null) => void;
  selectTask: (taskId: string | null, issueIdentifier?: string | null) => void;
  setLogMode: (mode: "concise" | "detailed") => void;
  setThemeMode: (theme: "system" | "dark" | "light") => void;
  setLocale: (locale: Locale) => void;
  setCommandPaletteOpen: (open: boolean) => void;
  setOnboardingOpen: (open: boolean) => void;
}

export const useUiStore = create<UiState>((set) => ({
  activeSection: "dashboard",
  selectedProjectId: null,
  selectedTaskId: null,
  selectedIssueIdentifier: null,
  logMode: "concise",
  themeMode: "dark",
  locale: "en",
  commandPaletteOpen: false,
  onboardingOpen: false,
  setActiveSection: (activeSection) => set({ activeSection }),
  selectProject: (selectedProjectId) => set({ selectedProjectId }),
  selectTask: (selectedTaskId, selectedIssueIdentifier = null) =>
    set({ selectedTaskId, selectedIssueIdentifier }),
  setLogMode: (logMode) => set({ logMode }),
  setThemeMode: (themeMode) => set({ themeMode }),
  setLocale: (locale) => set({ locale }),
  setCommandPaletteOpen: (commandPaletteOpen) => set({ commandPaletteOpen }),
  setOnboardingOpen: (onboardingOpen) => set({ onboardingOpen })
}));
