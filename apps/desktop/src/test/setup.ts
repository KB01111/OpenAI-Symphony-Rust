import "@testing-library/jest-dom/vitest";
import { configure } from "@testing-library/react";

// The default 1 s for `findBy…`/`waitFor` is too short when every workspace suite runs in parallel
// (`pnpm -r test`): pages that wait for several mocked queries then time out although they render.
configure({ asyncUtilTimeout: 5000 });

Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string) => ({
    matches: query.includes("dark"),
    media: query,
    onchange: null,
    addListener: () => undefined,
    removeListener: () => undefined,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    dispatchEvent: () => false
  })
});

if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

Element.prototype.scrollIntoView ??= () => undefined;
Element.prototype.scrollTo ??= () => undefined;

// jsdom has no layout engine: give elements a viewport-sized box so virtualized lists
// (@tanstack/react-virtual) render their first rows instead of nothing.
const FAKE_WIDTH = 1200;
const FAKE_HEIGHT = 800;
Object.defineProperty(HTMLElement.prototype, "offsetWidth", { configurable: true, get: () => FAKE_WIDTH });
Object.defineProperty(HTMLElement.prototype, "offsetHeight", { configurable: true, get: () => FAKE_HEIGHT });
Object.defineProperty(Element.prototype, "clientWidth", { configurable: true, get: () => FAKE_WIDTH });
Object.defineProperty(Element.prototype, "clientHeight", { configurable: true, get: () => FAKE_HEIGHT });
Element.prototype.getBoundingClientRect = function getBoundingClientRect() {
  return { x: 0, y: 0, top: 0, left: 0, right: FAKE_WIDTH, bottom: FAKE_HEIGHT, width: FAKE_WIDTH, height: FAKE_HEIGHT, toJSON: () => ({}) } as DOMRect;
};
