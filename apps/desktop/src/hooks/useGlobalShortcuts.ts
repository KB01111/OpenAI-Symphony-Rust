import { useEffect } from "react";
import { useRefreshRuntime } from "../lib/queries.js";
import { useUiStore } from "../store/uiStore.js";

function isEditable(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  return el.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName) || Boolean(el.closest?.(".cm-editor"));
}

/**
 * Global keyboard shortcuts:
 * Ctrl+K command palette · Ctrl+N new task · Ctrl+P project switcher · Ctrl+R refresh data
 * (instead of reloading the webview) · Ctrl+B sidebar · Ctrl+J activity panel · Ctrl+I inspector.
 * Plain typing in inputs/editors is never intercepted except for Ctrl+K.
 */
export function useGlobalShortcuts(): void {
  // Same action as the refresh button: queue a runtime poll/reconcile tick, then refetch.
  const { mutate: refresh, isPending: refreshing } = useRefreshRuntime();
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
      const key = e.key.toLowerCase();
      const store = useUiStore.getState();
      if (key === "k") {
        e.preventDefault();
        if (store.paletteOpen) store.closePalette();
        else store.openPalette("all");
        return;
      }
      if (isEditable(e.target) && key !== "r") return;
      switch (key) {
        case "n":
          if (e.shiftKey) return;
          e.preventDefault();
          store.setNewTaskOpen(true);
          break;
        case "p":
          e.preventDefault();
          store.openPalette("projects");
          break;
        case "r":
          if (e.shiftKey) return;
          e.preventDefault();
          if (!refreshing) refresh();
          break;
        case "b":
          e.preventDefault();
          store.setSidebarCollapsed(!store.sidebarCollapsed);
          break;
        case "j":
          e.preventDefault();
          store.setDrawerOpen(!store.drawerOpen);
          break;
        case "i":
          e.preventDefault();
          store.setInspectorOpen(!store.inspectorOpen);
          break;
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [refresh, refreshing]);
}
