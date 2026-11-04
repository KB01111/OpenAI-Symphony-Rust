import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { RunningSessionSummary } from "@symphony/shared-types";
import AgentRunsView from "./AgentRunsView.js";
import { buildAgentRows, groupAgentRows, rowActions } from "./grouping.js";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { project, providerStatus, snapshot, task } from "../../test/fixtures.js";

const session = (overrides: Partial<RunningSessionSummary> = {}): RunningSessionSummary => ({
  issue_id: "i1",
  issue_identifier: "SYM-7",
  task_id: "t7",
  project_id: "p1",
  provider: "simulated",
  model: null,
  state: "In Progress",
  phase: "streaming_turn",
  session_id: "s1",
  workspace_path: "/ws/SYM-7",
  turn_count: 1,
  last_event: "item/commandExecution",
  last_message: "Ran cargo test",
  started_at: new Date().toISOString(),
  last_event_at: new Date().toISOString(),
  tokens: { input_tokens: 10, output_tokens: 5, total_tokens: 15 },
  ...overrides
});

describe("agent grouping", () => {
  it("puts each task in exactly one segment with live sessions first", () => {
    const now = Date.now();
    const rows = buildAgentRows({
      snapshot: snapshot({ running: [session()] }),
      tasks: [
        task({ id: "t7", status: "running", identifier: "SYM-7" }),
        task({ id: "t8", status: "queued", identifier: "SYM-8" }),
        task({ id: "t9", status: "failed", identifier: "SYM-9", updated_at: new Date(now).toISOString() })
      ],
      approvals: [],
      projects: [project({ id: "p1" })],
      now
    });
    const groups = groupAgentRows(rows);
    expect(groups.running.map((r) => r.taskId)).toEqual(["t7"]);
    expect(groups.queued.map((r) => r.taskId)).toEqual(["t8"]);
    expect(groups.failed.map((r) => r.taskId)).toEqual(["t9"]);
    expect(new Set(rows.map((r) => r.taskId)).size).toBe(rows.length);
  });
});

describe("row actions", () => {
  it("offers Start but not Retry on a ready task that never ran, and both once it has", () => {
    const rowFor = (attempt_count: number) =>
      buildAgentRows({
        snapshot: snapshot(),
        tasks: [task({ id: "r1", status: "ready", project_id: "p1", attempt_count })],
        approvals: [],
        projects: [project({ id: "p1", auto_dispatch: true })],
        now: Date.now()
      })[0]!;
    expect(rowActions(rowFor(0), undefined)).toMatchObject({ dispatch: true, retry: false });
    expect(rowActions(rowFor(2), undefined)).toMatchObject({ dispatch: true, retry: true });
  });
});

describe("AgentRunsView", () => {
  it("opens a run from its title, but not from clicks inside the row's toolbar", async () => {
    const runtime = createMockRuntime({
      "GET /state": () => snapshot(),
      "GET /tasks": () => [task({ id: "t8", status: "queued", identifier: "SYM-8", title: "Queued work" })],
      "GET /approvals": () => [],
      "GET /projects": () => [project({ id: "p1" })],
      "GET /providers": () => [providerStatus({ provider: "simulated", installed: true })]
    });
    // Only /runs is routed, so opening a run (/runs/t8) unmounts the page.
    renderWithProviders(<AgentRunsView />, { runtime, route: "/runs", path: "/runs" });
    const toolbar = await screen.findByRole("toolbar", { name: /SYM-8/ });
    fireEvent.click(toolbar);
    expect(screen.getByRole("toolbar", { name: /SYM-8/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Queued work" }));
    await waitFor(() => expect(screen.queryByRole("toolbar", { name: /SYM-8/ })).not.toBeInTheDocument());
  });

  it("cancels a live run only after confirmation and exactly once", async () => {
    const runtime = createMockRuntime({
      "GET /state": () => snapshot({ running: [session()], counts: { running: 1, retrying: 0, queued: 0, blocked: 0, human_review: 0, completed: 0, failed: 0 } }),
      "GET /tasks": () => [task({ id: "t7", status: "running", identifier: "SYM-7", title: "Run tests" })],
      "GET /approvals": () => [],
      "GET /projects": () => [project({ id: "p1" })],
      "GET /providers": () => [providerStatus({ provider: "simulated", installed: true })],
      "POST /tasks/:id/cancel": () => task({ id: "t7", status: "cancelled" })
    });
    renderWithProviders(<AgentRunsView />, { runtime, route: "/runs", path: "/runs" });
    expect((await screen.findAllByText(/SYM-7/)).length).toBeGreaterThan(0);
    fireEvent.click(screen.getAllByRole("button", { name: /Cancel run/ })[0]!);
    const dialog = await screen.findByRole("alertdialog");
    expect(runtime.calls.some((c) => c.path === "/tasks/t7/cancel")).toBe(false);
    const confirm = await within(dialog).findByRole("button", { name: "Cancel run" });
    // Both clicks land in one frame, before the pending state can disable the button.
    act(() => {
      fireEvent.click(confirm);
      fireEvent.click(confirm);
    });
    await waitFor(() => expect(runtime.calls.filter((c) => c.method === "POST" && c.path === "/tasks/t7/cancel")).toHaveLength(1));
  });
});
