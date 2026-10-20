import { act, renderHook, waitFor } from "@testing-library/react";
import React from "react";
import { afterEach, describe, expect, it } from "vitest";
import { QueryClient } from "@tanstack/react-query";
import { AppProviders } from "../app/AppProviders.js";
import { settings } from "../test/fixtures.js";
import { createMockRuntime } from "../test/renderWithProviders.js";
import { useSaveSettings } from "./queries.js";
import { qk } from "./queryKeys.js";

type Internals = { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };

function installHost(invoke: Internals["invoke"]) {
  (globalThis as { __TAURI_INTERNALS__?: Internals }).__TAURI_INTERNALS__ = { invoke };
}

afterEach(() => {
  delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("login autostart", () => {
  it("does not report autostart as enabled when Windows rejects the registry change", async () => {
    let stored = settings({ start_background_runtime_on_login: false });
    const runtime = createMockRuntime({
      "GET /settings": () => stored,
      "PUT /settings": ({ body }) => (stored = body as typeof stored)
    });
    installHost(async (cmd) => {
      if (cmd === "set_autostart") throw new Error("Windows rejected writing of the autostart entry: Access is denied.");
      return null;
    });
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
    queryClient.setQueryData(qk.settings, stored);
    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <AppProviders queryClient={queryClient} client={runtime.client}>
        {children}
      </AppProviders>
    );
    const { result } = renderHook(() => useSaveSettings(), { wrapper });

    await act(async () => {
      await result.current.mutateAsync({ ...stored, start_background_runtime_on_login: true }).catch(() => undefined);
    });

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(String(result.current.error)).toMatch(/Access is denied/);
    // The runtime and the cache are reverted to the state Windows actually has.
    expect(stored.start_background_runtime_on_login).toBe(false);
    expect(queryClient.getQueryData<typeof stored>(qk.settings)?.start_background_runtime_on_login).toBe(false);
    const puts = runtime.calls.filter((c) => c.method === "PUT");
    expect(puts.map((c) => (c.body as typeof stored).start_background_runtime_on_login)).toEqual([true, false]);
  });
});
