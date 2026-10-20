/**
 * Authenticated Server-Sent Events over `fetch`.
 *
 * Browser `EventSource` cannot send an `Authorization` header, and putting bearer tokens in query
 * strings leaks them into logs and history. This implementation streams `text/event-stream` with
 * `fetch`, parses it per the WHATWG SSE format, dispatches named events, and handles
 * reconnection with exponential backoff, `Last-Event-ID` resumption, duplicate suppression and
 * a heartbeat watchdog (to recover from dead connections after sleep/resume).
 */
import type { AgentEvent, StreamControlEvent } from "@symphony/shared-types";

export interface SseMessage {
  id: string | null;
  event: string;
  data: string;
  retry: number | null;
}

/** Incremental `text/event-stream` parser. Feed decoded text; receive complete messages. */
export class SseParser {
  private buffer = "";
  private data: string[] = [];
  private event = "";
  private id: string | null = null;
  private retry: number | null = null;
  /** Last id seen, persisted across messages per the SSE spec. */
  lastEventId: string | null = null;

  feed(chunk: string): SseMessage[] {
    this.buffer += chunk;
    const out: SseMessage[] = [];
    let newline: number;
    while ((newline = this.nextLineBreak()) >= 0) {
      let line = this.buffer.slice(0, newline);
      const skip = this.buffer[newline] === "\r" && this.buffer[newline + 1] === "\n" ? 2 : 1;
      this.buffer = this.buffer.slice(newline + skip);
      if (line.endsWith("\r")) line = line.slice(0, -1);
      if (line === "") {
        if (this.data.length > 0) {
          out.push({
            id: this.id ?? this.lastEventId,
            event: this.event || "message",
            data: this.data.join("\n"),
            retry: this.retry
          });
        }
        if (this.id !== null) this.lastEventId = this.id;
        this.data = [];
        this.event = "";
        this.id = null;
        this.retry = null;
        continue;
      }
      if (line.startsWith(":")) continue; // comment / keep-alive
      const colon = line.indexOf(":");
      const field = colon === -1 ? line : line.slice(0, colon);
      let value = colon === -1 ? "" : line.slice(colon + 1);
      if (value.startsWith(" ")) value = value.slice(1);
      switch (field) {
        case "data":
          this.data.push(value);
          break;
        case "event":
          this.event = value;
          break;
        case "id":
          if (!value.includes("\0")) this.id = value;
          break;
        case "retry":
          if (/^\d+$/.test(value)) this.retry = Number(value);
          break;
      }
    }
    return out;
  }

  private nextLineBreak(): number {
    const n = this.buffer.indexOf("\n");
    const r = this.buffer.indexOf("\r");
    if (r === -1) return n;
    if (n === -1) return r === this.buffer.length - 1 ? -1 : r; // wait for possible \r\n
    return Math.min(n, r);
  }
}

/** Bounded set of recently seen ids used to drop duplicates after replay/reconnect. */
export class RecentIdSet {
  private readonly ids = new Set<string>();
  private readonly order: string[] = [];
  constructor(private readonly capacity = 2000) {}

  /** Returns `true` when the id is new (and records it). */
  add(id: string): boolean {
    if (this.ids.has(id)) return false;
    this.ids.add(id);
    this.order.push(id);
    if (this.order.length > this.capacity) {
      const evicted = this.order.shift();
      if (evicted !== undefined) this.ids.delete(evicted);
    }
    return true;
  }
}

/** Exponential backoff with full jitter. */
export function backoffDelay(attempt: number, baseMs: number, maxMs: number, random = Math.random): number {
  const ceiling = Math.min(maxMs, baseMs * 2 ** Math.max(0, attempt));
  return Math.round(ceiling / 2 + random() * (ceiling / 2));
}

export type StreamStatus =
  | "idle"
  | "connecting"
  | "open"
  | "reconnecting"
  | "unauthorized"
  | "closed";

export interface EventStreamHandlers {
  onEvent: (event: AgentEvent) => void;
  /** The client may have missed events and should refetch server state. */
  onResync?: (reason: string) => void;
  onStatus?: (status: StreamStatus, detail?: { attempt?: number; retryInMs?: number; error?: string }) => void;
}

export interface EventStream {
  start(): void;
  stop(): void;
  /** Force an immediate reconnect (e.g. after resume from sleep or regained connectivity). */
  reconnectNow(): void;
  readonly status: StreamStatus;
}

export interface SseStreamOptions extends EventStreamHandlers {
  url: string;
  token?: string | null;
  fetchImpl?: typeof fetch;
  initialBackoffMs?: number;
  maxBackoffMs?: number;
  /** Abort and reconnect when no bytes arrive for this long (server keep-alive is 15 s). */
  heartbeatTimeoutMs?: number;
}

export class AuthenticatedSseStream implements EventStream {
  private controller: AbortController | null = null;
  private stopped = true;
  private attempt = 0;
  private lastEventId: string | null = null;
  private hasConnectedBefore = false;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private watchdog: ReturnType<typeof setTimeout> | null = null;
  private readonly seen = new RecentIdSet();
  private _status: StreamStatus = "idle";

  constructor(private readonly options: SseStreamOptions) {}

  get status(): StreamStatus {
    return this._status;
  }

  start(): void {
    if (!this.stopped) return;
    this.stopped = false;
    void this.connect();
  }

  stop(): void {
    this.stopped = true;
    this.clearTimers();
    this.controller?.abort();
    this.controller = null;
    this.setStatus("closed");
  }

  reconnectNow(): void {
    if (this.stopped) return;
    this.clearTimers();
    this.controller?.abort();
    this.attempt = 0;
    void this.connect();
  }

  private setStatus(status: StreamStatus, detail?: Parameters<NonNullable<EventStreamHandlers["onStatus"]>>[1]) {
    this._status = status;
    this.options.onStatus?.(status, detail);
  }

  private clearTimers() {
    if (this.retryTimer) clearTimeout(this.retryTimer);
    if (this.watchdog) clearTimeout(this.watchdog);
    this.retryTimer = null;
    this.watchdog = null;
  }

  private armWatchdog(controller: AbortController) {
    if (this.watchdog) clearTimeout(this.watchdog);
    const timeout = this.options.heartbeatTimeoutMs ?? 45_000;
    this.watchdog = setTimeout(() => controller.abort(new Error("heartbeat timeout")), timeout);
  }

  private async connect(): Promise<void> {
    if (this.stopped) return;
    const controller = new AbortController();
    this.controller = controller;
    this.setStatus(this.hasConnectedBefore ? "reconnecting" : "connecting", { attempt: this.attempt });

    const headers: Record<string, string> = { Accept: "text/event-stream" };
    if (this.options.token) headers.Authorization = `Bearer ${this.options.token}`;
    if (this.lastEventId) headers["Last-Event-ID"] = this.lastEventId;

    const fetchImpl = this.options.fetchImpl ?? fetch;
    try {
      const response = await fetchImpl(this.options.url, {
        headers,
        signal: controller.signal,
        cache: "no-store"
      });
      if (response.status === 401 || response.status === 403) {
        this.setStatus("unauthorized");
        this.stopped = true;
        return;
      }
      if (!response.ok || !response.body) {
        throw new Error(`Event stream HTTP ${response.status}`);
      }

      const reconnecting = this.hasConnectedBefore;
      this.hasConnectedBefore = true;
      this.attempt = 0;
      this.setStatus("open");
      // Without a resumable id the server cannot replay, so state may have drifted.
      if (reconnecting && !this.lastEventId) this.options.onResync?.("reconnected");

      const reader = response.body.pipeThrough(new TextDecoderStream()).getReader();
      const parser = new SseParser();
      this.armWatchdog(controller);
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        this.armWatchdog(controller);
        for (const message of parser.feed(value)) this.dispatch(message);
      }
      throw new Error("Event stream ended");
    } catch (error) {
      if (this.stopped) return;
      if (this.controller !== controller) return; // superseded by reconnectNow()
      this.scheduleReconnect(error);
    }
  }

  private dispatch(message: SseMessage) {
    if (message.event === "ready") return;
    if (message.event === "resync") {
      let reason = "server";
      try {
        reason = (JSON.parse(message.data) as StreamControlEvent).reason ?? reason;
      } catch {
        /* keep default */
      }
      this.options.onResync?.(reason);
      return;
    }
    if (message.id) {
      this.lastEventId = message.id;
      if (!this.seen.add(message.id)) return;
    }
    try {
      this.options.onEvent(JSON.parse(message.data) as AgentEvent);
    } catch {
      // Malformed payloads are ignored; the stream stays open.
    }
  }

  private scheduleReconnect(error: unknown) {
    if (this.watchdog) clearTimeout(this.watchdog);
    const delay = backoffDelay(
      this.attempt,
      this.options.initialBackoffMs ?? 1_000,
      this.options.maxBackoffMs ?? 30_000
    );
    this.attempt += 1;
    this.setStatus("reconnecting", {
      attempt: this.attempt,
      retryInMs: delay,
      error: error instanceof Error ? error.message : String(error)
    });
    this.retryTimer = setTimeout(() => void this.connect(), delay);
  }
}
