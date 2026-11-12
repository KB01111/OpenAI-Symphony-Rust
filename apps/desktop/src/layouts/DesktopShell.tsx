import React, { Suspense, lazy } from "react";
import { Outlet } from "react-router-dom";
import { useGlobalShortcuts } from "../hooks/useGlobalShortcuts.js";
import { useSettings } from "../lib/queries.js";
import { CommandPalette } from "../features/command-palette/CommandPalette.js";
import { UpdateBanner } from "../features/updates/UpdateBanner.js";
import { ShortcutsDialog } from "../features/shortcuts/ShortcutsDialog.js";
import { useUiStore } from "../store/uiStore.js";
import { Sidebar } from "./Sidebar.js";
import { StatusBar } from "./StatusBar.js";
import { TopBar } from "./TopBar.js";

const NewTaskDialog = lazy(() => import("../features/composer/NewTaskDialog.js"));
const AddRepositoryDialog = lazy(() => import("../features/projects/AddRepositoryDialog.js"));
const OnboardingDialog = lazy(() => import("../features/onboarding/OnboardingDialog.js"));

/**
 * Desktop shell: a thread-first sidebar and a slim top bar on the window canvas, the routed
 * workspace as one rounded sheet, and a status line underneath.
 * There are no nested panel groups or fixed-width side panes at this level; views own their layout.
 */
export const DesktopShell: React.FC = () => {
  useGlobalShortcuts();
  const newTaskOpen = useUiStore((s) => s.newTaskOpen);
  const addProjectOpen = useUiStore((s) => s.addProjectOpen);
  const settings = useSettings();

  return (
    <div className="grid h-full grid-cols-[auto_minmax(0,1fr)] grid-rows-[minmax(0,1fr)_auto] overflow-hidden bg-canvas text-foreground">
      <Sidebar />
      <div className="flex min-h-0 min-w-0 flex-col pe-2">
        <TopBar />
        {/* The workspace is one sheet of paper on the window canvas; every view lives inside it. */}
        <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-2xl border border-border bg-background shadow-card">
          <UpdateBanner />
          <main id="main" className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden outline-none" tabIndex={-1}>
            <Outlet />
          </main>
        </div>
      </div>
      <div className="col-span-2 min-w-0">
        <StatusBar />
      </div>

      <CommandPalette />
      <ShortcutsDialog />
      <Suspense fallback={null}>
        {newTaskOpen ? <NewTaskDialog /> : null}
        {addProjectOpen ? <AddRepositoryDialog /> : null}
        {settings.data && !settings.data.onboarding_completed ? <OnboardingDialog /> : null}
      </Suspense>
    </div>
  );
};
