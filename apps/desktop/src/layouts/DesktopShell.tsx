import React, { Suspense, lazy, useEffect, useRef } from "react";
import { Outlet, useLocation } from "react-router-dom";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
  useDefaultLayout,
  type PanelImperativeHandle
} from "@symphony/ui";
import { safeStateStorage } from "../store/safeStorage.js";
import { useUiStore } from "../store/uiStore.js";
import { useGlobalShortcuts } from "../hooks/useGlobalShortcuts.js";
import { useSettings } from "../lib/queries.js";
import { CommandPalette } from "../features/command-palette/CommandPalette.js";
import { UpdateBanner } from "../features/updates/UpdateBanner.js";
import { ShortcutsDialog } from "../features/shortcuts/ShortcutsDialog.js";
import { ActivityDrawer } from "./ActivityDrawer.js";
import { CommandBar } from "./CommandBar.js";
import { ContextInspector } from "./ContextInspector.js";
import { Sidebar } from "./Sidebar.js";
import { StatusBar } from "./StatusBar.js";

const NewTaskDialog = lazy(() => import("../features/tasks/NewTaskDialog.js"));
const AddRepositoryDialog = lazy(() => import("../features/projects/AddRepositoryDialog.js"));
const OnboardingDialog = lazy(() => import("../features/onboarding/OnboardingDialog.js"));

const DRAWER_COLLAPSED_PX = 33;

/** Keeps a collapsible panel in sync with a persisted boolean in the UI store. */
function useCollapsibleSync(ref: React.RefObject<PanelImperativeHandle | null>, open: boolean) {
  useEffect(() => {
    const panel = ref.current;
    if (!panel) return;
    if (open && panel.isCollapsed()) panel.expand();
    if (!open && !panel.isCollapsed()) panel.collapse();
  }, [ref, open]);
}

/**
 * Desktop shell: command bar, collapsible sidebar, resizable workspace with an optional right
 * inspector and a dockable bottom activity drawer, plus a status bar. Panel sizes persist.
 */
export const DesktopShell: React.FC = () => {
  useGlobalShortcuts();
  const location = useLocation();
  const setLastRoute = useUiStore((s) => s.setLastRoute);
  const inspectorOpen = useUiStore((s) => s.inspectorOpen);
  const setInspectorOpen = useUiStore((s) => s.setInspectorOpen);
  const drawerOpen = useUiStore((s) => s.drawerOpen);
  const setDrawerOpen = useUiStore((s) => s.setDrawerOpen);
  const newTaskOpen = useUiStore((s) => s.newTaskOpen);
  const addProjectOpen = useUiStore((s) => s.addProjectOpen);
  const settings = useSettings();
  const inspectorRef = useRef<PanelImperativeHandle | null>(null);
  const drawerRef = useRef<PanelImperativeHandle | null>(null);

  const horizontal = useDefaultLayout({ id: "symphony-shell-h", storage: safeStateStorage, onlySaveAfterUserInteractions: true });
  const vertical = useDefaultLayout({ id: "symphony-shell-v", storage: safeStateStorage, onlySaveAfterUserInteractions: true });

  useCollapsibleSync(inspectorRef, inspectorOpen);
  useCollapsibleSync(drawerRef, drawerOpen);

  useEffect(() => {
    setLastRoute(location.pathname);
  }, [location.pathname, setLastRoute]);

  return (
    <div className="grid h-full grid-cols-[minmax(0,1fr)] grid-rows-[auto_minmax(0,1fr)_auto] overflow-hidden bg-background text-foreground">
      <div className="min-w-0">
        <CommandBar />
        <UpdateBanner />
      </div>
      <div className="flex min-h-0 min-w-0">
        <Sidebar />
        <ResizablePanelGroup orientation="horizontal" id="symphony-shell-h" {...horizontal} className="min-w-0 flex-1">
          <ResizablePanel id="workspace" minSize="40">
            <ResizablePanelGroup orientation="vertical" id="symphony-shell-v" {...vertical}>
              <ResizablePanel id="content" minSize="30">
                <main id="main" className="flex h-full min-h-0 flex-col overflow-hidden" tabIndex={-1}>
                  <Outlet />
                </main>
              </ResizablePanel>
              <ResizableHandle />
              <ResizablePanel
                id="drawer"
                panelRef={drawerRef}
                defaultSize="28"
                minSize={140}
                maxSize="70"
                collapsible
                collapsedSize={DRAWER_COLLAPSED_PX}
                onResize={() => {
                  const collapsed = drawerRef.current?.isCollapsed() ?? false;
                  if (collapsed === drawerOpen) setDrawerOpen(!collapsed);
                }}
              >
                <ActivityDrawer collapsed={!drawerOpen} onToggle={() => setDrawerOpen(!drawerOpen)} />
              </ResizablePanel>
            </ResizablePanelGroup>
          </ResizablePanel>
          <ResizableHandle />
          <ResizablePanel
            id="inspector"
            panelRef={inspectorRef}
            defaultSize={320}
            minSize={260}
            maxSize="45"
            collapsible
            collapsedSize={0}
            onResize={() => {
              const collapsed = inspectorRef.current?.isCollapsed() ?? false;
              if (collapsed === inspectorOpen) setInspectorOpen(!collapsed);
            }}
          >
            <ContextInspector />
          </ResizablePanel>
        </ResizablePanelGroup>
      </div>
      <StatusBar />

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
