import React, { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Batcher, SymphonyClient, configuredApiConfig, type EventStream, type StreamStatus } from "@symphony/api-client";
import type { AgentEvent } from "@symphony/shared-types";
import { keysForEvents, mobileKeys } from "./liveData.js";

export interface MobileSession {
  client: SymphonyClient;
  baseUrl: string;
  streamStatus: StreamStatus;
  /** Fallback polling interval while the event stream is not live. */
  pollInterval: number;
  /** Refetch everything now and reconnect the event stream if it is down. */
  refresh(): Promise<void>;
  disconnect(): void;
}

const SessionContext = createContext<MobileSession | null>(null);

export function useSession(): MobileSession {
  const value = useContext(SessionContext);
  if (!value) throw new Error("useSession must be used inside <SessionProvider>");
  return value;
}

const LIVE_POLL_MS = 15_000;
const FALLBACK_POLL_MS = 5_000;
const HIDDEN_POLL_MS = 60_000;

export interface SessionProviderProps {
  baseUrl: string;
  token: string;
  onUnauthorized: () => void;
  onDisconnect: () => void;
  children: React.ReactNode;
  /** Injected client (tests). */
  client?: SymphonyClient;
}

/**
 * Owns one authenticated runtime connection: the typed client, the fetch-based SSE event stream
 * (bearer token in the `Authorization` header, never in the URL), batched cache invalidation,
 * fallback polling while the stream is down and resume handling when the phone wakes up.
 */
export const SessionProvider: React.FC<SessionProviderProps> = ({
  baseUrl,
  token,
  onUnauthorized,
  onDisconnect,
  children,
  client: injected
}) => {
  const queryClient = useQueryClient();
  const client = useMemo(
    () => injected ?? SymphonyClient.fromConfig(configuredApiConfig(baseUrl, token || null, "remote")),
    [injected, baseUrl, token]
  );
  const [streamStatus, setStreamStatus] = useState<StreamStatus>("idle");
  const [hidden, setHidden] = useState(() => typeof document !== "undefined" && document.hidden);
  const streamRef = useRef<EventStream | null>(null);
  const unauthorizedRef = useRef(onUnauthorized);
  unauthorizedRef.current = onUnauthorized;

  useEffect(() => {
    const batcher = new Batcher<AgentEvent>((batch) => {
      for (const key of keysForEvents(batch)) void queryClient.invalidateQueries({ queryKey: key });
    }, 250);
    const stream = client.createEventStream({
      onEvent: (event) => batcher.push(event),
      onResync: () => void queryClient.invalidateQueries({ queryKey: mobileKeys.all }),
      onStatus: (status) => {
        setStreamStatus(status);
        if (status === "unauthorized") unauthorizedRef.current();
      }
    });
    streamRef.current = stream;
    stream.start();
    const onVisibility = () => batcher.setSlowMode(document.hidden);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      stream.stop();
      batcher.dispose();
      streamRef.current = null;
    };
  }, [client, queryClient]);

  useEffect(() => {
    let hiddenAt = 0;
    const onVisibility = () => {
      setHidden(document.hidden);
      if (document.hidden) {
        hiddenAt = Date.now();
      } else if (hiddenAt && Date.now() - hiddenAt > 30_000) {
        // Phones suspend background tabs: reconnect and refresh once after a long pause.
        streamRef.current?.reconnectNow();
        void queryClient.invalidateQueries({ queryKey: mobileKeys.all });
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

  const refresh = useCallback(async () => {
    if (streamRef.current && streamRef.current.status !== "open") streamRef.current.reconnectNow();
    await queryClient.invalidateQueries({ queryKey: mobileKeys.all });
  }, [queryClient]);

  const pollInterval = hidden ? HIDDEN_POLL_MS : streamStatus === "open" ? LIVE_POLL_MS : FALLBACK_POLL_MS;

  const value = useMemo<MobileSession>(
    () => ({
      client,
      baseUrl: client.connection.baseUrl,
      streamStatus,
      pollInterval,
      refresh,
      disconnect: onDisconnect
    }),
    [client, streamStatus, pollInterval, refresh, onDisconnect]
  );

  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>;
};
