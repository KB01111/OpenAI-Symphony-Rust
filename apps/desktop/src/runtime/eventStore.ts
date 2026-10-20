import { create } from "zustand";
import type { AgentEvent } from "@symphony/shared-types";

const MAX_EVENTS = 3000;

/**
 * Ring buffer of live runtime events received over the stream (oldest first). Events are
 * observability data, not authoritative task state; task/approval records are refetched from
 * the runtime when events indicate they changed.
 */
interface EventStoreState {
  events: AgentEvent[];
  /** Monotonic counter bumped per appended batch — cheap change signal for selectors. */
  version: number;
  append(batch: AgentEvent[]): void;
  clear(): void;
}

export const useEventStore = create<EventStoreState>((set) => ({
  events: [],
  version: 0,
  append: (batch) =>
    set((state) => {
      if (batch.length === 0) return state;
      const seen = new Set(state.events.slice(-500).map((e) => e.id));
      const fresh = batch.filter((e) => !seen.has(e.id));
      if (fresh.length === 0) return state;
      const next = state.events.concat(fresh);
      return {
        events: next.length > MAX_EVENTS ? next.slice(next.length - MAX_EVENTS) : next,
        version: state.version + 1
      };
    }),
  clear: () => set({ events: [], version: 0 })
}));

/** Merge persisted history (newest-first from the API) with live events, deduped, oldest first. */
export function mergeEventHistory(history: AgentEvent[] | undefined, live: AgentEvent[]): AgentEvent[] {
  const map = new Map<string, AgentEvent>();
  for (const e of history ?? []) map.set(e.id, e);
  for (const e of live) map.set(e.id, e);
  return Array.from(map.values()).sort((a, b) => (a.timestamp < b.timestamp ? -1 : a.timestamp > b.timestamp ? 1 : 0));
}
