/**
 * Test harness: renders a component tree with the real providers (Query, design system, runtime)
 * and a SymphonyClient whose `fetch` is answered by an in-memory route table, so feature tests
 * exercise the real client, hooks and cache invalidation without a network.
 */
import React from "react";
import { render, type RenderResult } from "@testing-library/react";
import { RouterProvider, createMemoryRouter } from "react-router-dom";
import { QueryClient } from "@tanstack/react-query";
import { SymphonyClient } from "@symphony/api-client";
import { AppProviders } from "../app/AppProviders.js";

export type MockHandler = (request: { method: string; path: string; query: URLSearchParams; body: unknown }) =>
  | unknown
  | { status: number; body: unknown };

export interface MockRuntime {
  client: SymphonyClient;
  calls: { method: string; path: string; body: unknown }[];
  /** Replace or add a route handler, keyed like `"GET /tasks"` or `"POST /tasks/:id/dispatch"`. */
  on(route: string, handler: MockHandler): void;
}

function matchRoute(pattern: string, path: string): boolean {
  const a = pattern.split("/");
  const b = path.split("/");
  return a.length === b.length && a.every((seg, i) => seg.startsWith(":") || seg === b[i]);
}

export function createMockRuntime(routes: Record<string, MockHandler> = {}): MockRuntime {
  const table = new Map(Object.entries(routes));
  const calls: MockRuntime["calls"] = [];
  const fetchImpl: typeof fetch = async (input, init) => {
    const url = new URL(String(input));
    const method = (init?.method ?? "GET").toUpperCase();
    const path = url.pathname.replace(/^\/api\/v1/, "");
    const body = init?.body ? JSON.parse(String(init.body)) : undefined;
    calls.push({ method, path, body });
    if (path === "/events") {
      return new Response(new ReadableStream({ start() {} }), { status: 200, headers: { "Content-Type": "text/event-stream" } });
    }
    for (const [key, handler] of table) {
      const [m, p] = key.split(" ");
      if (m === method && matchRoute(p!, path)) {
        const result = await handler({ method, path, query: url.searchParams, body });
        if (result && typeof result === "object" && "status" in (result as object) && "body" in (result as object)) {
          const r = result as { status: number; body: unknown };
          return new Response(JSON.stringify(r.body), { status: r.status, headers: { "Content-Type": "application/json" } });
        }
        return new Response(result === undefined ? null : JSON.stringify(result), {
          status: result === undefined ? 204 : 200,
          headers: { "Content-Type": "application/json" }
        });
      }
    }
    return new Response(JSON.stringify({ error: { code: "not_found", message: `No mock for ${method} ${path}` } }), { status: 404 });
  };
  return {
    client: new SymphonyClient({ baseUrl: "http://runtime.test", token: "test", fetchImpl }),
    calls,
    on: (route, handler) => void table.set(route, handler)
  };
}

export function renderWithProviders(
  ui: React.ReactElement,
  { runtime, route = "/", path = "*" }: { runtime: MockRuntime; route?: string; path?: string }
): RenderResult & { queryClient: QueryClient } {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  // A data router (like the app's) so views using `useBlocker` work under test.
  const router = createMemoryRouter([{ path, element: ui }], { initialEntries: [route] });
  const result = render(
    <AppProviders client={runtime.client} queryClient={queryClient}>
      <RouterProvider router={router} />
    </AppProviders>
  );
  return { ...result, queryClient };
}
