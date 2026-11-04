import { afterEach, describe, expect, it } from "vitest";
import { useUiStore } from "./uiStore.js";

const KEY = "symphony.ui.v2";

async function hydrateWith(taskView: unknown) {
  window.localStorage.setItem(KEY, JSON.stringify({ state: { taskView }, version: 1 }));
  await useUiStore.persist.rehydrate();
}

describe("uiStore persisted task view", () => {
  afterEach(() => {
    window.localStorage.removeItem(KEY);
    useUiStore.getState().setTaskView("board");
  });

  it("keeps a saved table view", async () => {
    await hydrateWith("table");
    expect(useUiStore.getState().taskView).toBe("table");
  });

  it("falls back to the board for a view that no longer exists, such as a saved Runs", async () => {
    await hydrateWith("runs");
    expect(useUiStore.getState().taskView).toBe("board");
    await hydrateWith(42);
    expect(useUiStore.getState().taskView).toBe("board");
  });
});

describe("uiStore persisted language", () => {
  afterEach(() => {
    window.localStorage.removeItem(KEY);
    useUiStore.getState().setLocale("en");
  });

  async function hydrateLocale(locale: unknown) {
    window.localStorage.setItem(KEY, JSON.stringify({ state: { locale }, version: 1 }));
    await useUiStore.persist.rehydrate();
  }

  it("keeps a saved language that is offered", async () => {
    await hydrateLocale("sv");
    expect(useUiStore.getState().locale).toBe("sv");
  });

  it("falls back to the default for a language that is not offered, instead of breaking every screen", async () => {
    await hydrateLocale("de");
    expect(useUiStore.getState().locale).toBe("en");
    await hydrateLocale(7);
    expect(useUiStore.getState().locale).toBe("en");
    await hydrateLocale(null);
    expect(useUiStore.getState().locale).toBe("en");
  });
});
