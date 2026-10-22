import { act, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { settings } from "../test/fixtures.js";
import { createMockRuntime, renderWithProviders } from "../test/renderWithProviders.js";
import { useUiStore } from "../store/uiStore.js";
import { PreferencesBridge } from "./PreferencesBridge.js";

afterEach(() => {
  delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("PreferencesBridge: native language", () => {
  it("follows the language adopted from the runtime and later changes made in the UI", async () => {
    const invoke = vi.fn(async () => undefined);
    (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = { invoke };
    act(() => useUiStore.setState({ locale: "sv" }));
    renderWithProviders(<PreferencesBridge />, {
      runtime: createMockRuntime({ "GET /settings": () => settings({ locale: "en" }), "PUT /settings": () => settings({ locale: "sv" }) })
    });
    // Saved settings win at startup: the UI adopts "en" and the host is told.
    await waitFor(() => expect(invoke).toHaveBeenLastCalledWith("set_native_locale", { locale: "en" }));
    expect(useUiStore.getState().locale).toBe("en");

    act(() => useUiStore.getState().setLocale("sv"));
    await waitFor(() => expect(invoke).toHaveBeenLastCalledWith("set_native_locale", { locale: "sv" }));
  });

  it("does nothing outside the desktop host", async () => {
    renderWithProviders(<PreferencesBridge />, { runtime: createMockRuntime({ "GET /settings": () => settings() }) });
    act(() => useUiStore.getState().setLocale("sv"));
    expect((globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__).toBeUndefined();
  });
});
