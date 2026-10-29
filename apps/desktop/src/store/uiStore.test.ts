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
