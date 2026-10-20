import { useRuntime } from "../runtime/RuntimeProvider.js";
import { useMessages, useT } from "../lib/i18n.js";
import { shellMessages } from "../app/messages.js";
import { useRuntimeState } from "../lib/queries.js";

export interface ConnectionSummary {
  tone: "success" | "warning" | "danger" | "neutral";
  label: string;
  short: string;
}

/** Actual runtime connectivity — never inferred from local defaults. */
export function useConnectionSummary(): ConnectionSummary {
  const { streamStatus, mode } = useRuntime();
  const state = useRuntimeState();
  const t = useT();
  const m = useMessages(shellMessages);
  const modeLabel =
    mode === "embedded" ? m.runtime.modeEmbedded : mode === "attached" ? m.runtime.modeAttached : mode === "dev" ? m.runtime.modeDev : m.runtime.modeRemote;

  if (state.isError && state.error?.kind === "unauthorized") {
    return { tone: "danger", label: t.connection.unauthorized, short: m.status.unauthorized };
  }
  if (state.isError && !state.data) {
    return { tone: "danger", label: t.connection.disconnected, short: m.status.offline };
  }
  if (streamStatus === "open") {
    const label = state.data?.scheduler_paused ? t.connection.paused : t.connection.connected;
    return { tone: state.data?.scheduler_paused ? "warning" : "success", label: `${label} · ${modeLabel}`, short: m.status.live };
  }
  if (streamStatus === "unauthorized") return { tone: "danger", label: t.connection.unauthorized, short: m.status.unauthorized };
  if (streamStatus === "reconnecting" || streamStatus === "connecting") {
    return { tone: "warning", label: `${t.connection.reconnecting} · ${t.connection.polling}`, short: m.status.reconnecting };
  }
  return { tone: "warning", label: `${t.connection.stale} · ${t.connection.polling}`, short: m.status.polling };
}
