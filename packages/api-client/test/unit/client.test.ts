import { describe, expect, it, vi } from "vitest";
import { SymphonyApiError, SymphonyClient, errorMessage } from "../../src/index.js";

function jsonResponse(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

describe("SymphonyClient", () => {
  it("keeps the deadline armed while the response body is read", async () => {
    const fetchImpl = vi.fn(async (_url: RequestInfo | URL, init?: RequestInit) => {
      const signal = init!.signal!;
      // Headers arrive immediately, but the body stalls until the request is aborted.
      return {
        ok: true,
        status: 200,
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
    const fetchImpl = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
      seen.push(`${init?.method} ${String(url).replace("http://h:1", "")}`);
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
    const headers = fetchImpl.mock.calls[0]![1]!.headers as Record<string, string>;
    expect(headers.Authorization).toBe("Bearer t");
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
