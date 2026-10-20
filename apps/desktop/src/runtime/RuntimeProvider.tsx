import React, { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  Batcher,
  SymphonyClient,
  backoffDelay,
  errorMessage,
  type EventStream,
  type StreamStatus
} from "@symphony/api-client";
import type { AgentEvent } from "@symphony/shared-types";
import { useEventStore } from "./eventStore.js";
import { invalidateForEvents } from "./invalidation.js";

export type ConnectionPhase = "discovering" | "ready" | "error";

export interface RuntimeContextValue {
  phase: ConnectionPhase;
  client: SymphonyClient | null;
  error: string | null;
  streamStatus: StreamStatus;
  /** `embedded` | `attached` | `dev` | `remote` */
  mode: string | null;
  baseUrl: string | null;
  /** Interval for fallback polling: `false` while the event stream is live. */
  pollInterval: number | false;
  documentHidden: boolean;
  retry(): void;
}

const RuntimeContext = createContext<RuntimeContextValue | null>(null);

export function useRuntime(): RuntimeContextValue {
  const ctx = useContext(RuntimeContext);
  if (!ctx) throw new Error("useRuntime must be used inside <RuntimeProvider>");
  return ctx;
}

/** The connected client. Only call below `<RuntimeGate>`, which guarantees readiness. */
export function useClient(): SymphonyClient {
  const { client } = useRuntime();
  if (!client) throw new Error("Runtime client is not ready");
  return client;
}

const FALLBACK_POLL_MS = 5_000;
const HIDDEN_POLL_MS = 30_000;

export interface RuntimeProviderProps {
  children: React.ReactNode;
  /** Inject a client (tests). Otherwise the runtime is discovered via Tauri IPC / dev server. */
  client?: SymphonyClient;
  discover?: () => Promise<SymphonyClient>;
}

/**
 * Owns the runtime connection lifecycle: endpoint discovery (with retry while the runtime starts),
 * the authenticated event stream, batched cache invalidation, fallback polling when the stream is
 * down, and throttling while the window is hidden or the machine resumes from sleep.
 */
export const RuntimeProvider: React.FC<RuntimeProviderProps> = ({ children, client: injected, discover }) => {
  const queryClient = useQueryClient();
  const append = useEventStore((s) => s.append);
  const [client, setClient] = useState<SymphonyClient | null>(injected ?? null);
  const [phase, setPhase] = useState<ConnectionPhase>(injected ? "ready" : "discovering");
  const [error, setError] = useState<string | null>(null);
  const [streamStatus, setStreamStatus] = useState<StreamStatus>("idle");
  const [documentHidden, setDocumentHidden] = useState(() => typeof document !== "undefined" && document.hidden);
  const [attempt, setAttempt] = useState(0);
  const streamRef = useRef<EventStream | null>(null);

  // 1. Discovery with backoff.
  useEffect(() => {
    if (injected) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const run = async (n: number) => {
      setPhase("discovering");
      try {
        const found = await (discover ?? (() => SymphonyClient.discover()))();
        // `/state` is protected: this proves the discovered token, not just that a runtime
        // answers (a stale runtime.json from a previous launch would pass a public probe).
        await found.getState();
        if (cancelled) return;
        setClient(found);
        setError(null);
        setPhase("ready");
      } catch (e) {
        if (cancelled) return;
        setError(errorMessage(e));
        if (n >= 4) setPhase("error");
        timer = setTimeout(() => void run(n + 1), backoffDelay(n, 500, 10_000));
      }
    };
    void run(0);
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [injected, discover, attempt]);

  // 2. Event stream + batched invalidation.
  useEffect(() => {
    if (!client) return;
    const batcher = new Batcher<AgentEvent>((batch) => {
      append(batch);
      invalidateForEvents(queryClient, batch);
    }, 100);
    const stream = client.createEventStream({
      onEvent: (event) => batcher.push(event),
      onResync: () => void queryClient.invalidateQueries(),
      onStatus: (status) => {
        setStreamStatus(status);
        // The runtime restarted with a new per-launch token: discover it again instead of
        // staying on a client every request of which is rejected.
        if (status === "unauthorized" && !injected) {
          setClient(null);
          setAttempt((a) => a + 1);
        }
      }
    });
    streamRef.current = stream;
    stream.start();
    return () => {
      stream.stop();
      batcher.dispose();
      streamRef.current = null;
    };
  }, [client, queryClient, append, injected]);

  // 3. Visibility / connectivity / resume handling.
  useEffect(() => {
    let hiddenAt = 0;
    const onVisibility = () => {
      const hidden = document.hidden;
      setDocumentHidden(hidden);
      if (hidden) {
        hiddenAt = Date.now();
      } else if (hiddenAt && Date.now() - hiddenAt > 60_000) {
        // Long background period (or sleep): reconnect and refresh everything once.
        streamRef.current?.reconnectNow();
        void queryClient.invalidateQueries();
      }
    };
    const onOnline = () => streamRef.current?.reconnectNow();
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("online", onOnline);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("online", onOnline);
    };
  }, [queryClient]);

  const retry = useCallback(() => {
    if (client) streamRef.current?.reconnectNow();
    else setAttempt((n) => n + 1);
  }, [client]);

  const pollInterval: number | false =
    streamStatus === "open" ? false : documentHidden ? HIDDEN_POLL_MS : FALLBACK_POLL_MS;

  const value = useMemo<RuntimeContextValue>(
    () => ({
      phase,
      client,
      error,
      streamStatus,
      mode: client?.connection.mode ?? null,
      baseUrl: client?.connection.baseUrl ?? null,
      pollInterval,
      documentHidden,
      retry
    }),
    [phase, client, error, streamStatus, pollInterval, documentHidden, retry]
  );

  return <RuntimeContext.Provider value={value}>{children}</RuntimeContext.Provider>;
};
