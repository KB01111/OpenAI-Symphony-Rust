import { useEffect } from "react";
import { useRefreshRuntime } from "../lib/queries.js";
import { useUiStore } from "../store/uiStore.js";

function isEditable(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  return el.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName) || Boolean(el.closest?.(".cm-editor"));
}

/** Shortcut help and the command palette are modal: opening one closes the other. */
function toggleShortcuts(store: ReturnType<typeof useUiStore.getState>): void {
  if (!store.shortcutsOpen) store.closePalette();
  store.setShortcutsOpen(!store.shortcutsOpen);
}

/**
 * Global keyboard shortcuts:
 * Ctrl+K command palette · Ctrl+N new task · Ctrl+P project switcher · Ctrl+R refresh data
 * (instead of reloading the webview) · Ctrl+B sidebar · Ctrl+J activity panel · Ctrl+I inspector ·
 * F1 or Ctrl+/ keyboard shortcut help. Plain typing in inputs/editors is never intercepted except
 * for Ctrl+K, F1 and Ctrl+/.
 */
export function useGlobalShortcuts(): void {
  // Same action as the refresh button: queue a runtime poll/reconcile tick, then refetch.
  const { mutate: refresh, isPending: refreshing } = useRefreshRuntime();
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const store = useUiStore.getState();
      if (e.key === "F1" && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey) {
        e.preventDefault();
        // Holding the key auto-repeats keydown: toggle once, not on every repeat.
        if (!e.repeat) toggleShortcuts(store);
        return;
      }
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
      const key = e.key.toLowerCase();
      if (key === "/") {
        e.preventDefault();
        if (!e.repeat) toggleShortcuts(store);
        return;
      }
      if (key === "k") {
        e.preventDefault();
        if (store.paletteOpen) store.closePalette();
        else {
          store.setShortcutsOpen(false);
          store.openPalette("all");
        }
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
