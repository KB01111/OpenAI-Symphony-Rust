import { act, renderHook, waitFor } from "@testing-library/react";
import React from "react";
import { afterEach, describe, expect, it } from "vitest";
import { QueryClient } from "@tanstack/react-query";
import { AppProviders } from "../app/AppProviders.js";
import { settings } from "../test/fixtures.js";
import { createMockRuntime } from "../test/renderWithProviders.js";
import { observedAutostart, useSaveSettings } from "./queries.js";
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
      if (cmd === "set_autostart") {
        throw new Error("autostart_state=disabled; Windows rejected writing of the autostart entry: Access is denied.");
      }
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

describe("login autostart with an unknown outcome", () => {
  it("restores the last confirmed value and retries on the next save", async () => {
    let stored = settings({ start_background_runtime_on_login: false });
    const runtime = createMockRuntime({
      "GET /settings": () => stored,
      "PUT /settings": ({ body }) => (stored = body as typeof stored)
    });
    const calls: unknown[] = [];
    let fail = true;
    installHost(async (cmd, args) => {
      if (cmd === "set_autostart") {
        calls.push(args);
        if (fail) throw new Error("autostart_state=unknown; read-back failed");
      }
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
    // Not confirmed: the saved value goes back to the last confirmed one.
    expect(stored.start_background_runtime_on_login).toBe(false);

    // A later, unrelated save retries the native call even though the value did not change.
    fail = false;
    await act(async () => {
      await result.current.mutateAsync({ ...stored, polling_interval_ms: 5000 });
    });
    expect(calls).toHaveLength(2);
  });
});

describe("observedAutostart", () => {
  it("only trusts a state the host actually observed", () => {
    expect(observedAutostart(new Error("autostart_state=enabled; failed"))).toBe(true);
    expect(observedAutostart("autostart_state=disabled; failed")).toBe(false);
    expect(observedAutostart(new Error("autostart_state=unknown; read-back failed"))).toBeNull();
    expect(observedAutostart(new Error("unrelated"))).toBeNull();
  });
});
