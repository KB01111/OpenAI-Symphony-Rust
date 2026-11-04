import { fireEvent, screen, waitFor } from "@testing-library/react";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import type { AgentEvent } from "@symphony/shared-types";
import ActivityPage from "./ActivityPage.js";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { approval, project, providerStatus, snapshot, task } from "../../test/fixtures.js";

function agentMessage(id: string, taskId: string, text: string): AgentEvent {
  return {
    id,
    task_id: taskId,
    issue_identifier: "SYM-1",
    session_id: "s1",
    provider: "simulated",
    event_type: "item/agentMessage",
    phase: "streaming_turn",
    message: text,
    payload: { item: { type: "agentMessage", text } },
    timestamp: "2026-10-20T10:00:01Z"
  };
}

// jsdom has no layout: give scroll containers a viewport so the virtualized feed renders rows.
const sizeProps = ["clientHeight", "clientWidth"] as const;
const originals = sizeProps.map((p) => Object.getOwnPropertyDescriptor(HTMLElement.prototype, p));
beforeAll(() => {
  for (const p of sizeProps) Object.defineProperty(HTMLElement.prototype, p, { configurable: true, get: () => 800 });
});
afterAll(() => {
  sizeProps.forEach((p, i) => {
    const d = originals[i];
    if (d) Object.defineProperty(HTMLElement.prototype, p, d);
    else delete (HTMLElement.prototype as unknown as Record<string, unknown>)[p];
  });
});

describe("ActivityPage", () => {
  it("shows the output of a task without opening its thread first", async () => {
    const running = task({ id: "t-run", status: "running", title: "Running work", identifier: "SYM-1" });
    const other = task({ id: "t-other", status: "completed", title: "Older work", identifier: "SYM-2", updated_at: "2026-10-19T10:00:00Z" });
    const rt = createMockRuntime({
      "GET /state": () => snapshot(),
      "GET /projects": () => [project()],
      "GET /providers": () => [providerStatus()],
      "GET /approvals": () => [approval()],
      "GET /tasks": () => [other, running],
      "GET /events/recent": () => [],
      "GET /tasks/:id/events": ({ path }) => (path.includes("t-run") ? [agentMessage("e1", "t-run", "Planning the change")] : []),
      "GET /tasks/:id/validations": () => []
    });
    renderWithProviders(<ActivityPage />, { runtime: rt, route: "/activity" });
    fireEvent.click(await screen.findByRole("tab", { name: "Agent output" }));
    // The running task is preselected and its events are shown; no thread route is involved.
    expect(await screen.findByRole("combobox", { name: "Task" })).toHaveTextContent("SYM-1 · Running work");
    await waitFor(() => expect(screen.getByText("Planning the change")).toBeInTheDocument());
    expect(screen.queryByText("Select a task to see its output.")).not.toBeInTheDocument();
  });
});
