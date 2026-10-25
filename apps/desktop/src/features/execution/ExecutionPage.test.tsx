import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import type { AgentEvent, Task, TaskDiff } from "@symphony/shared-types";
import ExecutionPage from "./ExecutionPage.js";
import { createMockRuntime, renderWithProviders, type MockHandler } from "../../test/renderWithProviders.js";
import { approval, project, providerStatus, snapshot, task } from "../../test/fixtures.js";
import { useEventStore } from "../../runtime/eventStore.js";
import { useUiStore } from "../../store/uiStore.js";

function ev(id: string, second: number, overrides: Partial<AgentEvent> = {}): AgentEvent {
  return {
    id,
    task_id: "t1",
    issue_identifier: "SYM-1",
    session_id: "s1",
    provider: "simulated",
    event_type: "task/updated",
    phase: null,
    message: `message ${id}`,
    payload: null,
    timestamp: `2026-10-20T10:00:${String(second).padStart(2, "0")}Z`,
    ...overrides
  };
}

const emptyDiff: TaskDiff = { task_id: "t1", workspace_path: null, branch_name: null, exists: false, changed_files: [], diff: "", truncated: false };

function runtime(current: Task, overrides: Record<string, MockHandler> = {}) {
  return createMockRuntime({
    "GET /tasks/:id": () => current,
    "GET /tasks/:id/events": () => [],
    "GET /tasks/:id/attempts": () => [],
    "GET /tasks/:id/validations": () => [],
    "GET /tasks/:id/diff": () => emptyDiff,
    "GET /approvals": () => [],
    "GET /projects": () => [project()],
    "GET /providers": () => [providerStatus()],
    "GET /state": () => snapshot(),
    "GET /workflow": () => ({ path: null, raw_markdown: "", valid: true, error: null, config: null, rendered_preview: null }),
    "GET /prs": () => [],
    "GET /settings": () => ({}),
    ...overrides
  });
}

function renderPage(rt: ReturnType<typeof runtime>) {
  return renderWithProviders(<ExecutionPage />, { runtime: rt, route: "/runs/t1", path: "/runs/:taskId" });
}

// jsdom has no layout: give scroll containers a viewport so the virtualized timeline renders rows.
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

afterEach(() => {
  useEventStore.getState().clear();
  useUiStore.getState().inspect(null);
  delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("ExecutionPage", () => {
  it("loads older events on request, continuing from the oldest event shown", async () => {
    const at = (i: number) => new Date(Date.UTC(2026, 9, 20, 10, 0, 0) + i * 1000).toISOString();
    const newest = Array.from({ length: 500 }, (_, k) => ev(`n${499 - k}`, 0, { timestamp: at(1000 + 499 - k) }));
    const olderPage = [ev("o1", 0, { timestamp: at(1) }), ev("o0", 0, { timestamp: at(0) })];
    const rt = runtime(task(), {
      "GET /tasks/:id/events": ({ query }) => (query.get("before") ? olderPage : newest)
    });
    renderPage(rt);
    const load = await screen.findByRole("button", { name: "Load older events" });
    expect(screen.getByText("500 events")).toBeInTheDocument();
    fireEvent.click(load);
    await waitFor(() => expect(screen.getByText("502 events")).toBeInTheDocument());
    const paged = rt.calls.filter((c) => c.method === "GET" && c.path === "/tasks/t1/events");
    expect(paged).toHaveLength(2);
    // A short page means the beginning was reached: the control goes away.
    expect(screen.queryByRole("button", { name: "Load older events" })).not.toBeInTheDocument();
  });

  it("does not offer older events when the first page was not full", async () => {
    const rt = runtime(task(), { "GET /tasks/:id/events": () => [ev("e1", 1)] });
    renderPage(rt);
    await screen.findByText("1 events");
    expect(screen.queryByRole("button", { name: "Load older events" })).not.toBeInTheDocument();
  });

  it("focuses the inspector and renders history + live events in order without duplicates", async () => {
    const rt = runtime(task({ status: "running", current_phase: "streaming_turn" }), {
      "GET /tasks/:id/events": () => [ev("e3", 3), ev("e1", 1)]
    });
    act(() => useEventStore.getState().append([ev("e2", 2), ev("e3", 3), ev("x", 4, { task_id: "other", message: "foreign" })]));
    renderPage(rt);
    await screen.findByText("message e1");
    await waitFor(() => expect(screen.getByText("message e3")).toBeInTheDocument());
    const texts = screen.getAllByText(/^message e\d$/).map((n) => n.textContent);
    expect(texts).toEqual(["message e1", "message e2", "message e3"]);
    expect(screen.queryByText("foreign")).not.toBeInTheDocument();
    expect(useUiStore.getState().inspector).toEqual({ kind: "task", id: "t1" });
    expect(screen.getByText(/Agent is working/)).toBeInTheDocument();
  });

  it("hides event kinds with the filter chips", async () => {
    const rt = runtime(task(), {
      "GET /tasks/:id/events": () => [
        ev("e1", 1),
        ev("c1", 2, {
          event_type: "item/commandExecution/completed",
          message: "ran a command",
          payload: { item: { type: "commandExecution", command: "cargo build", exitCode: 0 } }
        })
      ]
    });
    renderPage(rt);
    expect(await screen.findByText("ran a command")).toBeInTheDocument();
    const chip = screen.getByRole("button", { name: "Commands (1)" });
    expect(chip).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(chip);
    await waitFor(() => expect(screen.queryByText("ran a command")).not.toBeInTheDocument());
    expect(screen.getByText("message e1")).toBeInTheDocument();
    expect(chip).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(screen.getByRole("button", { name: "Show all" }));
    expect(await screen.findByText("ran a command")).toBeInTheDocument();
  });

  it("submits an approval decision once and disables the banner while pending", async () => {
    let release: (() => void) | null = null;
    const rt = runtime(task({ status: "running" }), {
      "GET /approvals": () => [approval()],
      "POST /approvals/:id/decide": () =>
        new Promise((resolve) => {
          release = () => resolve(approval({ status: "approved", decision: "accept" }));
        })
    });
    renderPage(rt);
    const accept = await screen.findByRole("button", { name: "Approve once" });
    fireEvent.click(accept);
    fireEvent.click(accept);
    await waitFor(() => expect(screen.getByRole("button", { name: "Decline" })).toBeDisabled());
    fireEvent.click(screen.getByRole("button", { name: "Decline" }));
    const decides = rt.calls.filter((c) => c.method === "POST" && c.path === "/approvals/a1/decide");
    expect(decides).toHaveLength(1);
    expect(decides[0]!.body).toEqual({ decision: "accept" });
    await act(async () => release?.());
  });

  it("requires confirmation before cancelling and posts to the cancel endpoint", async () => {
    const running = task({ status: "running", current_phase: "streaming_turn" });
    const rt = runtime(running, {
      "POST /tasks/:id/cancel": () => ({ ...running, status: "cancelled", current_phase: null })
    });
    renderPage(rt);
    fireEvent.click(await screen.findByRole("button", { name: "Cancel run" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(rt.calls.some((c) => c.path === "/tasks/t1/cancel")).toBe(false);
    fireEvent.click(await within(dialog).findByRole("button", { name: "Cancel run" }));
    await waitFor(() => expect(rt.calls.filter((c) => c.method === "POST" && c.path === "/tasks/t1/cancel")).toHaveLength(1));
  });

  it("never renders pause/resume and hides cancel when the provider cannot cancel", async () => {
    const rt = runtime(task({ status: "running" }), {
      "GET /providers": () => [
        providerStatus({ capabilities: { ...providerStatus().capabilities, supports_cancel: false, supports_mid_turn_pause: false } })
      ]
    });
    renderPage(rt);
    await screen.findByText(/SYM-1 · Add retry backoff/);
    await waitFor(() => expect(screen.getAllByText("Not supported").length).toBeGreaterThan(0));
    expect(screen.queryByRole("button", { name: /pause|resume/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Cancel run" })).not.toBeInTheDocument();
  });

  it("hides Open workspace outside the embedded Tauri host", async () => {
    const rt = runtime(task({ status: "human_review", workspace_path: "C:\\ws\\SYM-1" }));
    renderPage(rt);
    await screen.findByText(/SYM-1 · Add retry backoff/);
    expect(screen.queryByRole("button", { name: "Open workspace" })).not.toBeInTheDocument();
    // Even inside Tauri, a non-embedded (remote) runtime must not offer native folder access.
    (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = { invoke: async () => "" };
    renderPage(runtime(task({ status: "human_review", workspace_path: "C:\\ws\\SYM-1" })));
    await waitFor(() => expect(screen.getAllByText(/SYM-1 · Add retry backoff/).length).toBe(2));
    expect(screen.queryByRole("button", { name: "Open workspace" })).not.toBeInTheDocument();
  });

  it("shows a not-found state with a way back to agent runs", async () => {
    const rt = runtime(task(), {
      "GET /tasks/:id": () => ({ status: 404, body: { error: { code: "not_found", message: "task t1 not found" } } })
    });
    renderPage(rt);
    expect((await screen.findAllByText("Task not found")).length).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: "Back to agent runs" })).toBeInTheDocument();
  });

  it("gates Start and Retry on the task status", async () => {
    const rt = runtime(task({ status: "failed" }), {
      "POST /tasks/:id/retry": () => task({ status: "queued" })
    });
    renderPage(rt);
    expect(await screen.findByRole("button", { name: "Start" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(rt.calls.filter((c) => c.method === "POST" && c.path === "/tasks/t1/retry")).toHaveLength(1));
  });
});
