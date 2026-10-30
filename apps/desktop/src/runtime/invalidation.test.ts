import { describe, expect, it } from "vitest";
import type { AgentEvent } from "@symphony/shared-types";
import { qk } from "../lib/queryKeys.js";
import { carriesToolCalls, keysToInvalidate } from "./invalidation.js";

function event(event_type: string): AgentEvent {
  return {
    id: event_type,
    task_id: "t1",
    issue_identifier: "SYM-1",
    session_id: null,
    provider: "codex",
    event_type,
    phase: null,
    message: event_type,
    payload: null,
    timestamp: "2026-10-20T10:00:00Z"
  };
}

const has = (keys: unknown[], key: unknown) => keys.some((k) => JSON.stringify(k) === JSON.stringify(key));

describe("keysToInvalidate", () => {
  it("refetches a task's tool calls only for events that can carry one", () => {
    for (const type of ["item/completed", "tool/result", "claude/user", "opencode/event", "item/commandExecution"]) {
      expect(carriesToolCalls(type)).toBe(true);
      expect(has(keysToInvalidate([event(type)]), qk.taskToolCalls("t1"))).toBe(true);
    }
    // Streaming text never refetches the history.
    for (const type of ["item/agentMessage/delta", "turn/completed", "task/updated"]) {
      expect(has(keysToInvalidate([event(type)]), qk.taskToolCalls("t1"))).toBe(false);
    }
  });
});

describe("CodeSandbox cloud agent events", () => {
  it("refetch the tool calls of the task, and its changes when a push reached the worktree", () => {
    expect(carriesToolCalls("codesandbox/tool")).toBe(true);
    expect(has(keysToInvalidate([event("codesandbox/tool")]), qk.taskToolCalls("t1"))).toBe(true);
    for (const type of ["codesandbox/worktree_updated", "codesandbox/pushed"]) {
      expect(has(keysToInvalidate([event(type)]), qk.taskDiff("t1"))).toBe(true);
    }
    // Plain output and usage reports change nothing but the timeline.
    for (const type of ["codesandbox/output", "codesandbox/usage"]) {
      const keys = keysToInvalidate([event(type)]);
      expect(has(keys, qk.taskDiff("t1"))).toBe(false);
      expect(has(keys, qk.taskToolCalls("t1"))).toBe(false);
      expect(has(keys, qk.taskEvents("t1"))).toBe(true);
    }
  });
});

describe("pull request events", () => {
  it("refresh the open review detail named in the payload", () => {
    const changed = { ...event("pr/status_changed"), payload: { review_id: "r1", ci_status: "failed" } };
    const keys = keysToInvalidate([changed]);
    expect(has(keys, qk.pr("r1"))).toBe(true);
    expect(has(keys, qk.prsAll)).toBe(true);
    expect(has(keysToInvalidate([event("pr/status_changed")]), qk.pr("r1"))).toBe(false);
  });
});
