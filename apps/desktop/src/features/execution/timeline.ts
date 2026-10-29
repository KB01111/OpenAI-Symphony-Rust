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

/**
 * Protocol bookkeeping with nothing to read: the Codex driver labels notifications it has no text for
 * (`item/started`, `thread/status/changed`, ...) as exactly "Event: <method>", and uses the method as the
 * event type. Matching on that keeps other drivers' real text (e.g. an OpenCode "Event: ready") visible.
 * The concise timeline leaves these out; the technical one keeps every event. Errors always stay.
 */
export function isProtocolNoise(event: AgentEvent): boolean {
  return event.message === `Event: ${event.event_type}` && eventKind(event) === "other";
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


/** The event the operator last saw when they stopped following the tail. */
export interface PauseAnchor {
  id: string;
  timestamp: string;
}

/**
 * Events (of the visible kinds) after the event the operator last saw when they stopped following
 * the tail. `pausedAfter` is that event, `""` when the timeline was empty, `null` while following.
 * Older events prepended meanwhile are not counted. If the anchor itself has left the timeline
 * (bounded live retention), events newer than its timestamp are.
 */
export function unseenSince(
  events: AgentEvent[],
  pausedAfter: PauseAnchor | "" | null,
  hidden: ReadonlySet<AgentEventKind>
): number {
  if (pausedAfter === null) return 0;
  if (pausedAfter === "") return filterByKinds(events, hidden).length;
  const at = events.findIndex((e) => e.id === pausedAfter.id);
  const after = at >= 0 ? events.slice(at + 1) : events.filter((e) => e.timestamp > pausedAfter.timestamp);
  return filterByKinds(after, hidden).length;
}
