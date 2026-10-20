/**
 * Pure timeline logic for the execution workspace: merging persisted history with live stream
 * events, classifying events by kind (cached per event object), and filtering by kind.
 */
import type { AgentEvent } from "@symphony/shared-types";
import { EVENT_KINDS, formatAgentEvent, type AgentEventKind } from "@symphony/ui";
import { mergeEventHistory } from "../../runtime/eventStore.js";

const kindCache = new WeakMap<AgentEvent, AgentEventKind>();

/** Presentation kind of an event; cached so streaming never re-parses existing events. */
export function eventKind(event: AgentEvent): AgentEventKind {
  let kind = kindCache.get(event);
  if (!kind) {
    kind = formatAgentEvent(event).kind;
    kindCache.set(event, kind);
  }
  return kind;
}

/**
 * Timeline for one task: persisted history (any order) merged with the live ring buffer filtered
 * to the task, deduplicated by event id and sorted oldest first.
 */
export function buildTaskTimeline(
  taskId: string,
  history: ReadonlyArray<AgentEvent> | undefined,
  live: ReadonlyArray<AgentEvent>
): AgentEvent[] {
  const scopedHistory = (history ?? []).filter((e) => e.task_id === taskId);
  const scopedLive = live.filter((e) => e.task_id === taskId);
  return mergeEventHistory(scopedHistory, scopedLive);
}

export type KindCounts = Record<AgentEventKind, number>;

export function countByKind(events: ReadonlyArray<AgentEvent>): KindCounts {
  const counts = Object.fromEntries(EVENT_KINDS.map((k) => [k, 0])) as KindCounts;
  for (const e of events) counts[eventKind(e)] += 1;
  return counts;
}

/** Drop events whose kind is hidden. Returns the input array unchanged when nothing is hidden. */
export function filterByKinds(events: AgentEvent[], hidden: ReadonlySet<AgentEventKind>): AgentEvent[] {
  if (hidden.size === 0) return events;
  return events.filter((e) => !hidden.has(eventKind(e)));
}

/** Toggle a kind in an immutable hidden-kinds set. */
export function toggleKind(hidden: ReadonlySet<AgentEventKind>, kind: AgentEventKind): Set<AgentEventKind> {
  const next = new Set(hidden);
  if (next.has(kind)) next.delete(kind);
  else next.add(kind);
  return next;
}

/** Number of events that arrived after the operator stopped following the tail. */
export function newSince(total: number, pausedAt: number | null): number {
  return pausedAt === null ? 0 : Math.max(0, total - pausedAt);
}
