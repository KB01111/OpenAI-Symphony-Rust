import { createJSONStorage, type StateStorage } from "zustand/middleware";

/**
 * localStorage wrapper that never throws (private mode, blocked storage, quota). Only UI
 * presentation preferences are persisted here; durable data lives in the Rust runtime.
 */
const guarded: StateStorage = {
  getItem: (name) => {
    try {
      return window.localStorage.getItem(name);
    } catch {
      return null;
    }
  },
  setItem: (name, value) => {
    try {
      window.localStorage.setItem(name, value);
    } catch {
      /* ignore */
    }
  },
  removeItem: (name) => {
    try {
      window.localStorage.removeItem(name);
    } catch {
      /* ignore */
    }
  }
};

export const safeJSONStorage = createJSONStorage(() => guarded);
/** Synchronous `getItem`/`setItem` view (for libraries that expect the DOM Storage shape). */
export const safeStateStorage = {
  getItem: (name: string): string | null => guarded.getItem(name) as string | null,
  setItem: (name: string, value: string): void => void guarded.setItem(name, value)
};
