import { describe, expect, it } from "vitest";
import type { AgentEvent } from "@symphony/shared-types";
import { buildTaskTimeline, countByKind, eventKind, filterByKinds, isProtocolNoise, toggleKind, unseenSince } from "./timeline.js";

function ev(id: string, second: number, overrides: Partial<AgentEvent> = {}): AgentEvent {
  return {
    id,
    task_id: "t1",
    issue_identifier: "SYM-1",
    session_id: "s1",
    provider: "simulated",
    event_type: "task/updated",
    phase: null,
    message: `event ${id}`,
    payload: null,
    timestamp: `2026-10-20T10:00:${String(second).padStart(2, "0")}Z`,
    ...overrides
  };
}

const command = (id: string, second: number) =>
  ev(id, second, { event_type: "item/commandExecution/completed", payload: { item: { type: "commandExecution", command: "ls", exitCode: 0 } } });

describe("isProtocolNoise", () => {
  it("flags the bare 'Event: <method>' label but never real content or errors", () => {
    expect(isProtocolNoise(ev("a", 1, { event_type: "item/started", message: "Event: item/started" }))).toBe(true);
    expect(isProtocolNoise(ev("b", 1, { event_type: "item/started", message: "Event: item/started with details" }))).toBe(false);
    expect(isProtocolNoise(ev("c", 1, { event_type: "item/started", message: "Ran the tests" }))).toBe(false);
    expect(isProtocolNoise(ev("d", 1, { event_type: "error", message: "Event: error" }))).toBe(false);
    // Another driver's real text that merely looks like the label stays visible.
    expect(isProtocolNoise(ev("e", 1, { event_type: "opencode/event", message: "Event: ready" }))).toBe(false);
  });
});

describe("execution timeline", () => {
  it("merges newest-first history with live events, oldest first, without duplicates", () => {
    const history = [ev("e3", 3), ev("e1", 1)];
    const live = [ev("e2", 2), ev("e3", 3), ev("other", 4, { task_id: "t2" })];
    const merged = buildTaskTimeline("t1", history, live);
    expect(merged.map((e) => e.id)).toEqual(["e1", "e2", "e3"]);
  });

  it("ignores history entries for other tasks and handles missing history", () => {
    expect(buildTaskTimeline("t1", undefined, [ev("a", 1)]).map((e) => e.id)).toEqual(["a"]);
    expect(buildTaskTimeline("t1", [ev("x", 1, { task_id: "t9" })], []).length).toBe(0);
  });

  it("classifies, counts and filters events by kind", () => {
    const events = [ev("a", 1), command("b", 2), command("c", 3)];
    expect(eventKind(events[1]!)).toBe("command");
    const counts = countByKind(events);
    expect(counts.command).toBe(2);
    expect(counts.lifecycle).toBe(1);
    expect(counts.message).toBe(0);
    const hidden = toggleKind(new Set(), "command");
    expect(filterByKinds(events, hidden).map((e) => e.id)).toEqual(["a"]);
    expect(filterByKinds(events, toggleKind(hidden, "command"))).toBe(events);
  });

  it("counts only events after the one last seen, not older pages loaded later", () => {
    const ev = (id: string) => ({ id, task_id: "t1", issue_identifier: null, session_id: null, provider: null, event_type: "task/updated", phase: null, message: id, payload: null, timestamp: "2026-10-20T10:00:00Z" });
    const seen = [ev("b"), ev("c")];
    expect(unseenSince(seen, null, new Set())).toBe(0);
    const c = { id: "c", timestamp: "2026-10-20T10:00:00Z" };
    expect(unseenSince([...seen, ev("d")], c, new Set())).toBe(1);
    // Two older events prepended while paused: still one new event.
    expect(unseenSince([ev("x"), ev("a"), ...seen, ev("d")], c, new Set())).toBe(1);
    expect(unseenSince([ev("a")], "", new Set())).toBe(1);
  });

  it("keeps counting new events when the anchor itself has left the timeline", () => {
    const at = (id: string, second: number) => ({ id, task_id: "t1", issue_identifier: null, session_id: null, provider: null, event_type: "task/updated", phase: null, message: id, payload: null, timestamp: `2026-10-20T10:00:0${second}Z` });
    // "c" (second 2) was evicted by bounded retention; "d" and "e" arrived after it.
    const events = [at("a", 0), at("d", 3), at("e", 4)];
    expect(unseenSince(events, { id: "c", timestamp: at("c", 2).timestamp }, new Set())).toBe(2);
  });
});
