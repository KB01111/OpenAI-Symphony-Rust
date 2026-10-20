import { describe, expect, it } from "vitest";
import type { Task } from "@symphony/shared-types";
import { countByFilter, keysForEvents, mobileKeys, visibleTasks } from "../src/liveData.js";

const task = (id: string, status: Task["status"], updated: string) => ({ id, status, updated_at: updated }) as Task;

describe("keysForEvents", () => {
  it("only refreshes the task timeline for streaming item events", () => {
    expect(keysForEvents([{ event_type: "item/agentMessage/delta", task_id: "t1" }])).toEqual([mobileKeys.taskEvents("t1")]);
  });

  it("refreshes lists and state for lifecycle and approval events, deduplicated", () => {
    const keys = keysForEvents([
      { event_type: "task/status_changed", task_id: "t1" },
      { event_type: "approval/requested", task_id: "t1" },
      { event_type: "attempt/succeeded", task_id: "t1" }
    ]).map((k) => k.join("/"));
    expect(new Set(keys)).toEqual(new Set(["symphony/task-events/t1", "symphony/tasks", "symphony/state", "symphony/approvals"]));
    expect(keys.length).toBe(4);
  });
});

describe("task filters", () => {
  const tasks = [
    task("a", "running", "2026-01-01T00:00:00Z"),
    task("b", "failed", "2026-01-03T00:00:00Z"),
    task("c", "completed", "2026-01-02T00:00:00Z"),
    task("d", "queued", "2026-01-04T00:00:00Z")
  ];

  it("groups by filter and sorts newest first", () => {
    expect(visibleTasks(tasks, "active").map((t) => t.id)).toEqual(["d", "a"]);
    expect(visibleTasks(tasks, "attention").map((t) => t.id)).toEqual(["b"]);
    expect(visibleTasks(tasks, "all").map((t) => t.id)).toEqual(["d", "b", "c", "a"]);
  });

  it("counts every filter", () => {
    expect(countByFilter(tasks)).toEqual({ all: 4, active: 2, attention: 1, done: 1 });
  });
});
