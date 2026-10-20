import { act, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { PreferencesBridge } from "./PreferencesBridge.js";
import { createMockRuntime, renderWithProviders } from "../test/renderWithProviders.js";
import { settings } from "../test/fixtures.js";
import { useUiStore } from "../store/uiStore.js";

describe("PreferencesBridge", () => {
  it("adopts runtime theme/locale on load and writes later UI changes back", async () => {
    let stored = settings({ theme: "light", locale: "sv" });
    const runtime = createMockRuntime({
      "GET /settings": () => stored,
      "PUT /settings": ({ body }) => (stored = body as typeof stored)
    });
    act(() => {
      useUiStore.getState().setThemeMode("dark");
      useUiStore.getState().setLocale("en");
    });
    renderWithProviders(<PreferencesBridge />, { runtime });

    await waitFor(() => expect(useUiStore.getState().themeMode).toBe("light"));
    expect(useUiStore.getState().locale).toBe("sv");
    // Adopting runtime values must not write them back (or overwrite them with stale local ones).
    expect(runtime.calls.filter((c) => c.method === "PUT")).toHaveLength(0);

    act(() => useUiStore.getState().setThemeMode("dark"));
    await waitFor(() => expect(runtime.calls.filter((c) => c.method === "PUT")).toHaveLength(1));
    expect(runtime.calls.find((c) => c.method === "PUT")!.body).toMatchObject({ theme: "dark", locale: "sv" });
  });
});
