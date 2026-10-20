import { describe, expect, it, vi } from "vitest";
import {
  SymphonyApiError,
  SymphonyClient,
  configuredApiConfig,
  errorMessage,
  normalizeBaseUrl,
  resolveApiConfig
} from "../../src/index.js";

function jsonResponse(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

describe("SymphonyClient", () => {
  it("keeps the deadline armed while the response body is read", async () => {
    const fetchImpl = vi.fn(async (request: Request) => {
      const signal = request.signal;
      // Headers arrive immediately, but the body stalls until the request is aborted.
      return {
        ok: true,
        status: 200,
        headers: new Headers({ "Content-Type": "application/json" }),
        text: () =>
          new Promise<string>((_, reject) => {
            if (signal.aborted) reject(new Error("aborted"));
            signal.addEventListener("abort", () => reject(new Error("aborted")));
          })
      } as unknown as Response;
    });
    const client = new SymphonyClient({ baseUrl: "http://h:1", token: null, timeoutMs: 30, fetchImpl: fetchImpl as unknown as typeof fetch });
    await expect(client.getState()).rejects.toMatchObject({ kind: "network" });

    const caller = new AbortController();
    const pending = client.getState(caller.signal);
    caller.abort();
    await expect(pending).rejects.toMatchObject({ code: "aborted" });
  });

  it("targets the real Axum routes with the right verbs", async () => {
    const seen: string[] = [];
    const fetchImpl = vi.fn(async (request: Request) => {
      seen.push(`${request.method} ${request.url.replace("http://h:1", "")}`);
      return jsonResponse({});
    });
    const client = new SymphonyClient({ baseUrl: "http://h:1/", token: "t", fetchImpl: fetchImpl as unknown as typeof fetch });
    await client.getTask("a b");
    await client.moveTaskStatus("t1", "ready");
    await client.dispatchTask("t1");
    await client.decideApproval("ap1", "accept");
    await client.approvePullRequest("pr1");
    await client.saveWorkflow("---\n---\nx");
    await client.previewWorkflow({ raw_markdown: "x" });
    await client.listApprovals(true);
    await client.listTasks({ projectId: "p1" });
    await client.listRecentEvents(50);
    expect(seen).toEqual([
      "GET /api/v1/tasks/a%20b",
      "PATCH /api/v1/tasks/t1/status",
      "POST /api/v1/tasks/t1/dispatch",
      "POST /api/v1/approvals/ap1/decide",
      "POST /api/v1/prs/pr1/approve-merge",
      "PUT /api/v1/workflow",
      "POST /api/v1/workflow/preview",
      "GET /api/v1/approvals?only_pending=true",
      "GET /api/v1/tasks?project_id=p1",
      "GET /api/v1/events/recent?limit=50"
    ]);
    expect(fetchImpl.mock.calls[0]![0].headers.get("Authorization")).toBe("Bearer t");
  });

  it("sends the bearer token only to protected routes and sends JSON bodies", async () => {
    const requests: Request[] = [];
    const fetchImpl = vi.fn(async (request: Request) => {
      requests.push(request);
      return jsonResponse({});
    });
    const client = new SymphonyClient({ baseUrl: "http://h:1", token: "secret-token", fetchImpl: fetchImpl as unknown as typeof fetch });
    await client.health();
    await client.runtime();
    await client.createTask({ project_id: "p1", title: "T" });
    expect(requests[0]!.headers.get("Authorization")).toBeNull();
    expect(requests[1]!.headers.get("Authorization")).toBeNull();
    expect(requests[2]!.headers.get("Authorization")).toBe("Bearer secret-token");
    expect(requests[2]!.headers.get("Content-Type")).toBe("application/json");
    expect(await requests[2]!.json()).toEqual({ project_id: "p1", title: "T" });

    // A new connection replaces the token for subsequent requests.
    client.setConnection("http://other:2/api/v1/", "next-token");
    await client.getState();
    expect(requests[3]!.url).toBe("http://other:2/api/v1/state");
    expect(requests[3]!.headers.get("Authorization")).toBe("Bearer next-token");
  });

  it("returns undefined for 204 responses", async () => {
    const client = new SymphonyClient({
      baseUrl: "http://h:1",
      token: "t",
      fetchImpl: (async () => new Response(null, { status: 204 })) as unknown as typeof fetch
    });
    await expect(client.deleteProject("p1")).resolves.toBeUndefined();
  });

  it("maps runtime error bodies to typed errors", async () => {
    const client = new SymphonyClient({
      baseUrl: "http://h:1",
      fetchImpl: (async () =>
        jsonResponse(
          { error: { code: "invalid_state_transition", message: "invalid_state_transition: cannot transition from completed to backlog" } },
          409
        )) as unknown as typeof fetch
    });
    const err = await client.moveTaskStatus("t", "backlog").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(SymphonyApiError);
    expect((err as SymphonyApiError).kind).toBe("conflict");
    expect(errorMessage(err)).toBe("cannot transition from completed to backlog");
  });

  it("reports network failures distinctly", async () => {
    const client = new SymphonyClient({
      baseUrl: "http://h:1",
      fetchImpl: (async () => {
        throw new TypeError("fetch failed");
      }) as unknown as typeof fetch
    });
    const err = (await client.getState().catch((e: unknown) => e)) as SymphonyApiError;
    expect(err.kind).toBe("network");
  });
});

describe("API configuration", () => {
  it("normalizes base URLs", () => {
    expect(normalizeBaseUrl(" http://h:1/ ")).toBe("http://h:1");
    expect(normalizeBaseUrl("http://h:1/api/v1/")).toBe("http://h:1");
  });

  it("resolves remote and explicit local targets without discovery", async () => {
    const remote = await resolveApiConfig({ kind: "remote", baseUrl: "https://lumen:47821/", token: "abc" });
    expect(remote).toMatchObject({ baseUrl: "https://lumen:47821", token: "abc", endpoint: { source: "configured", mode: "remote" } });
    const local = await resolveApiConfig({ kind: "local", baseUrl: "http://127.0.0.1:4780" }, { timeoutMs: 5 });
    expect(local).toMatchObject({ token: null, timeoutMs: 5, endpoint: { source: "configured", mode: "attached" } });
  });

  it("discovers a local runtime through the dev-server route", async () => {
    const fetchImpl = vi.fn(async () => jsonResponse({ base_url: "http://127.0.0.1:4780/", token: "dev-token" }));
    const config = await resolveApiConfig({ kind: "local" }, { fetchImpl: fetchImpl as unknown as typeof fetch });
    expect(config).toMatchObject({ baseUrl: "http://127.0.0.1:4780", token: "dev-token", endpoint: { source: "dev-server", mode: "dev" } });
  });

  it("builds clients from a configuration", () => {
    const client = SymphonyClient.fromConfig(configuredApiConfig("http://h:1/", "t", "remote"));
    expect(client.connection).toEqual({ baseUrl: "http://h:1", source: "configured", mode: "remote" });
  });
});
