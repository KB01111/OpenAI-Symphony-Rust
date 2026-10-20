import "@testing-library/jest-dom/vitest";

type Listener = (e: MediaQueryListEvent) => void;
const listeners = new Map<string, Set<Listener>>();
export const mediaState: Record<string, boolean> = {
  "(prefers-color-scheme: dark)": true,
  "(prefers-reduced-motion: reduce)": false
};

Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string) => ({
    get matches() {
      return mediaState[query] ?? false;
    },
    media: query,
    onchange: null,
    addEventListener: (_: string, l: Listener) => {
      if (!listeners.has(query)) listeners.set(query, new Set());
      listeners.get(query)!.add(l);
    },
    removeEventListener: (_: string, l: Listener) => listeners.get(query)?.delete(l),
    addListener: () => undefined,
    removeListener: () => undefined,
    dispatchEvent: () => false
  })
});

export function setMedia(query: string, matches: boolean) {
  mediaState[query] = matches;
  listeners.get(query)?.forEach((l) => l({ matches, media: query } as MediaQueryListEvent));
}

class RO {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as unknown as { ResizeObserver: typeof RO }).ResizeObserver ??= RO;
Element.prototype.scrollIntoView ??= () => undefined;
