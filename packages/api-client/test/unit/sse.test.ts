import { describe, expect, it, vi } from "vitest";
import {
  AuthenticatedSseStream,
  Batcher,
  RecentIdSet,
  SseParser,
  backoffDelay,
  type StreamStatus
} from "../../src/index.js";

function sseResponse(chunks: string[], status = 200, keepOpen = false): Response {
  const encoder = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk));
      if (!keepOpen) controller.close();
    }
  });
  return new Response(body, { status, headers: { "Content-Type": "text/event-stream" } });
}

const agentEvent = (id: string, type = "item/agentMessage") =>
  JSON.stringify({
    id,
    task_id: "t1",
    issue_identifier: "SYM-1",
    session_id: null,
    provider: "simulated",
    event_type: type,
    phase: null,
    message: `msg ${id}`,
    payload: null,
    timestamp: "2026-01-01T00:00:00Z"
  });

describe("SseParser", () => {
  it("parses named events with ids across arbitrary chunk boundaries", () => {
    const parser = new SseParser();
    const raw = `event: ready\ndata: {}\n\n: keep-alive\n\nid: a1\nevent: task/created\ndata: {"x":1}\n\nid: a2\nevent: item/agentMessage\ndata: line1\ndata: line2\n\n`;
    const messages = [] as ReturnType<SseParser["feed"]>;
    for (let i = 0; i < raw.length; i += 3) messages.push(...parser.feed(raw.slice(i, i + 3)));
    expect(messages).toEqual([
      { id: null, event: "ready", data: "{}", retry: null },
      { id: "a1", event: "task/created", data: '{"x":1}', retry: null },
      { id: "a2", event: "item/agentMessage", data: "line1\nline2", retry: null }
    ]);
  });

  it("handles CRLF line endings and defaults the event name", () => {
    const parser = new SseParser();
    expect(parser.feed("data: hi\r\n\r\n")).toEqual([{ id: null, event: "message", data: "hi", retry: null }]);
  });
});

describe("RecentIdSet", () => {
  it("drops duplicates and evicts the oldest ids beyond capacity", () => {
    const set = new RecentIdSet(2);
    expect(set.add("a")).toBe(true);
    expect(set.add("a")).toBe(false);
    set.add("b");
    set.add("c");
    expect(set.add("a")).toBe(true);
  });
});

describe("backoffDelay", () => {
  it("grows exponentially and caps at the maximum", () => {
    expect(backoffDelay(0, 1000, 30000, () => 1)).toBe(1000);
    expect(backoffDelay(3, 1000, 30000, () => 1)).toBe(8000);
    expect(backoffDelay(10, 1000, 30000, () => 1)).toBe(30000);
    expect(backoffDelay(3, 1000, 30000, () => 0)).toBe(4000);
  });
});

describe("AuthenticatedSseStream", () => {
  it("sends the bearer token in a header (never the URL), dispatches events, dedupes, and resumes with Last-Event-ID", async () => {
    const calls: { url: string; headers: Record<string, string> }[] = [];
    let call = 0;
    const fetchImpl = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
      calls.push({ url: String(url), headers: init?.headers as Record<string, string> });
      call += 1;
      if (call === 1) {
        return sseResponse([
          `event: ready\ndata: {}\n\n`,
          `id: e1\nevent: item/agentMessage\ndata: ${agentEvent("e1")}\n\n`,
          `id: e2\nevent: item/agentMessage\ndata: ${agentEvent("e2")}\n\n`
        ]);
      }
      // Replay includes a duplicate of e2, then a new event.
      return sseResponse(
        [
          `event: ready\ndata: {}\n\n`,
          `id: e2\nevent: item/agentMessage\ndata: ${agentEvent("e2")}\n\n`,
          `id: e3\nevent: turn/completed\ndata: ${agentEvent("e3", "turn/completed")}\n\n`
        ],
        200,
        true
      );
    });

    const received: string[] = [];
    const statuses: StreamStatus[] = [];
    const stream = new AuthenticatedSseStream({
      url: "http://127.0.0.1:1/api/v1/events",
      token: "secret-token",
      fetchImpl: fetchImpl as unknown as typeof fetch,
      initialBackoffMs: 5,
      maxBackoffMs: 10,
      onEvent: (e) => received.push(e.id),
      onStatus: (s) => statuses.push(s)
    });
    stream.start();
    await vi.waitFor(() => expect(received).toEqual(["e1", "e2", "e3"]), { timeout: 2000 });
    stream.stop();

    expect(calls[0]!.url).not.toContain("secret-token");
    expect(calls[0]!.headers.Authorization).toBe("Bearer secret-token");
    expect(calls[0]!.headers["Last-Event-ID"]).toBeUndefined();
    expect(calls[1]!.headers["Last-Event-ID"]).toBe("e2");
    expect(statuses).toContain("reconnecting");
    expect(statuses.at(-1)).toBe("closed");
  });

  it("stops retrying when the runtime rejects the token", async () => {
    const fetchImpl = vi.fn(async () => new Response("{}", { status: 401 }));
    const statuses: StreamStatus[] = [];
    const stream = new AuthenticatedSseStream({
      url: "http://x/api/v1/events",
      token: "bad",
      fetchImpl: fetchImpl as unknown as typeof fetch,
      onEvent: () => undefined,
      onStatus: (s) => statuses.push(s)
    });
    stream.start();
    await vi.waitFor(() => expect(statuses).toContain("unauthorized"));
    await new Promise((r) => setTimeout(r, 50));
    expect(fetchImpl).toHaveBeenCalledTimes(1);
  });

  it("forwards server resync events", async () => {
    const fetchImpl = vi.fn(async () =>
      sseResponse([`event: resync\ndata: {"api_version":2,"server_time":"x","reason":"lagged"}\n\n`], 200, true)
    );
    const reasons: string[] = [];
    const stream = new AuthenticatedSseStream({
      url: "http://x/api/v1/events",
      fetchImpl: fetchImpl as unknown as typeof fetch,
      onEvent: () => undefined,
      onResync: (r) => reasons.push(r)
    });
    stream.start();
    await vi.waitFor(() => expect(reasons).toEqual(["lagged"]));
    stream.stop();
  });
});

describe("Batcher", () => {
  it("coalesces pushes into a single flush per interval", async () => {
    const flushes: number[][] = [];
    const batcher = new Batcher<number>((items) => flushes.push(items), 10);
    batcher.push(1);
    batcher.push(2);
    batcher.push(3);
    await new Promise((r) => setTimeout(r, 30));
    expect(flushes).toEqual([[1, 2, 3]]);
  });
});
