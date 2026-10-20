import { listenNativeEvents } from "../transport/tauri.js";
import { RecentIdSet, type EventStream, type EventStreamHandlers, type StreamStatus } from "./sse.js";

/**
 * Event stream backed by the desktop host's in-process `symphony://event` channel. Used when the
 * runtime is embedded in the Tauri app: no HTTP connection, no token, no reconnect loop.
 */
export class NativeEventStream implements EventStream {
  private unlisten: (() => void) | null = null;
  private readonly seen = new RecentIdSet();
  private _status: StreamStatus = "idle";
  private generation = 0;

  constructor(private readonly handlers: EventStreamHandlers) {}

  get status(): StreamStatus {
    return this._status;
  }

  start(): void {
    if (this.unlisten || this._status === "connecting") return;
    const generation = ++this.generation;
    this.setStatus("connecting");
    listenNativeEvents(
      (event) => {
        if (this.seen.add(event.id)) this.handlers.onEvent(event);
      },
      () => this.handlers.onResync?.("lagged")
    )
      .then((unlisten) => {
        if (generation !== this.generation) {
          unlisten();
          return;
        }
        this.unlisten = unlisten;
        this.setStatus("open");
      })
      .catch((error: unknown) => {
        this.setStatus("closed", { error: error instanceof Error ? error.message : String(error) });
      });
  }

  stop(): void {
    this.generation++;
    this.unlisten?.();
    this.unlisten = null;
    this.setStatus("closed");
  }

  reconnectNow(): void {
    // In-process channel cannot drop events silently, but after resume refetch to be safe.
    this.handlers.onResync?.("resumed");
  }

  private setStatus(status: StreamStatus, detail?: { error?: string }) {
    this._status = status;
    this.handlers.onStatus?.(status, detail);
  }
}
