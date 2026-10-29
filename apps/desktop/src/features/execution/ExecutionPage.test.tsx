import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import type { AgentEvent, Task, TaskDiff, ToolCallRecord } from "@symphony/shared-types";
import { Link } from "react-router-dom";
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
  // Wide enough that the run-details panel docks beside the thread (see `DOCK_MIN_WIDTH`).
  for (const p of sizeProps) Object.defineProperty(HTMLElement.prototype, p, { configurable: true, get: () => (p === "clientWidth" ? 1400 : 800) });
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
  delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("ExecutionPage", () => {
  it("continues a reviewed task with the follow-up box", async () => {
    const reviewed = task({ status: "human_review", current_thread_id: "thread-1" });
    const rt = runtime(reviewed, {
      "POST /tasks/:id/continue": () => ({ ...reviewed, status: "running", attempt_count: reviewed.attempt_count + 1 })
    });
    renderPage(rt);
    const box = await screen.findByRole("textbox", { name: "Instructions for the agent" });
    const send = screen.getByRole("button", { name: "Send" });
    expect(send).toBeDisabled();
    fireEvent.change(box, { target: { value: "  Also handle empty input.  " } });
    fireEvent.click(send);
    await waitFor(() => expect(rt.calls.some((c) => c.method === "POST" && c.path === "/tasks/t1/continue")).toBe(true));
    expect(rt.calls.find((c) => c.path === "/tasks/t1/continue")!.body).toEqual({ instructions: "Also handle empty input." });
    await waitFor(() => expect(box).toHaveValue(""));
  });

  it("starts an empty follow-up box for another run, so typed text cannot reach that run", async () => {
    const first = task({ id: "t1", status: "human_review" });
    const second = task({ id: "t2", status: "failed", issue: { ...first.issue, identifier: "SYM-2", title: "Second run title" } });
    const rt = runtime(first, { "GET /tasks/:id": ({ path }) => (path.endsWith("/t2") ? second : first) });
    renderWithProviders(
      <>
        <ExecutionPage />
        <Link to="/runs/t1">first run</Link>
        <Link to="/runs/t2">next run</Link>
      </>,
      { runtime: rt, route: "/runs/t1", path: "/runs/:taskId" }
    );
    fireEvent.change(await screen.findByRole("textbox", { name: "Instructions for the agent" }), { target: { value: "for SYM-1 only" } });
    fireEvent.click(screen.getByText("next run"));
    expect((await screen.findAllByText(/Second run title/)).length).toBeGreaterThan(0);
    await waitFor(() => expect(screen.getByRole("textbox", { name: "Instructions for the agent" })).toHaveValue(""));
  });

  it("counts instructions in characters as the runtime does, so emoji are not counted twice", async () => {
    renderPage(runtime(task({ status: "human_review" })));
    const box = await screen.findByRole("textbox", { name: "Instructions for the agent" });
    // 10 001 emoji are 20 002 UTF-16 units but 10 001 characters: within the 20 000 limit.
    fireEvent.change(box, { target: { value: "😀".repeat(10_001) } });
    expect(screen.getByRole("button", { name: "Send" })).toBeEnabled();
    fireEvent.change(box, { target: { value: "x".repeat(20_001) } });
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    // The runtime trims first: surrounding whitespace does not count against the limit.
    fireEvent.change(box, { target: { value: `${"x".repeat(20_000)}   
  ` } });
    expect(screen.getByRole("button", { name: "Send" })).toBeEnabled();
  });

  it("shows and hides the run details with Ctrl+J", async () => {
    renderPage(runtime(task({ status: "human_review" })));
    expect(await screen.findByRole("tab", { name: "Context" })).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "j", ctrlKey: true });
    await waitFor(() => expect(screen.queryByRole("tab", { name: "Context" })).not.toBeInTheDocument());
    fireEvent.keyDown(window, { key: "j", ctrlKey: true });
    expect(await screen.findByRole("tab", { name: "Context" })).toBeInTheDocument();
  });

  it("offers no follow-up while a task runs and says why", async () => {
    renderPage(runtime(task({ status: "running", current_phase: "streaming_turn" })));
    await screen.findByRole("toolbar", { name: "Execution controls" });
    expect(screen.getByRole("textbox", { name: "Instructions for the agent" })).toBeDisabled();
    expect(screen.getAllByText(/You can send a follow-up when this turn finishes/).length).toBeGreaterThan(0);
  });

  it("loads older events on request, continuing from the oldest event shown", async () => {
    const at = (i: number) => new Date(Date.UTC(2026, 9, 20, 10, 0, 0) + i * 1000).toISOString();
    const newest = Array.from({ length: 500 }, (_, k) => ev(`n${499 - k}`, 0, { timestamp: at(1000 + 499 - k) }));
    const olderPage = [ev("o1", 0, { timestamp: at(1) }), ev("o0", 0, { timestamp: at(0) })];
    const cursors: (string | null)[] = [];
    const rt = runtime(task(), {
      "GET /tasks/:id/events": ({ query }) => {
        cursors.push(query.get("before"));
        return query.get("before") ? olderPage : newest;
      }
    });
    renderPage(rt);
    const load = await screen.findByRole("button", { name: "Load older events" });
    expect(screen.getByText("500 events")).toBeInTheDocument();
    fireEvent.click(load);
    await waitFor(() => expect(screen.getByText("502 events")).toBeInTheDocument());
    // The second request continues from the oldest event of the first page.
    expect(cursors).toEqual([null, "n0"]);
    // A short page means the beginning was reached: the control goes away.
    expect(screen.queryByRole("button", { name: "Load older events" })).not.toBeInTheDocument();
  });

  it("does not offer older events when the first page was not full", async () => {
    const rt = runtime(task(), { "GET /tasks/:id/events": () => [ev("e1", 1)] });
    renderPage(rt);
    await screen.findByText("1 event");
    expect(screen.queryByRole("button", { name: "Load older events" })).not.toBeInTheDocument();
  });

  it("renders history + live events in order without duplicates", async () => {
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
    expect(screen.getByText(/Agent is working/)).toBeInTheDocument();
  });

  it("leaves contentless protocol events out of the concise timeline but keeps them in technical", async () => {
    const rt = runtime(task(), {
      "GET /tasks/:id/events": () => [ev("e1", 1), ev("n1", 2, { event_type: "item/started", message: "Event: item/started" })]
    });
    renderPage(rt);
    expect(await screen.findByText("message e1")).toBeInTheDocument();
    expect(screen.getByText("1 event")).toBeInTheDocument();
    expect(screen.queryByText("Event: item/started")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("radio", { name: "Technical" }));
    expect(await screen.findByText("Event: item/started")).toBeInTheDocument();
    expect(screen.getByText("2 events")).toBeInTheDocument();
    useUiStore.getState().setLogMode("concise");
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

  it("lists the task's tool calls newest first and pages to older ones", async () => {
    const call = (n: number, overrides: Partial<ToolCallRecord> = {}): ToolCallRecord => ({
      id: `t1:call-${n}`,
      task_id: "t1",
      session_id: "s1",
      provider: "codex",
      call_id: `call-${n}`,
      tool_name: `tool_${n}`,
      kind: "command",
      arguments: { command: `echo ${n}` },
      output: `out ${n}`,
      output_truncated: false,
      status: "succeeded",
      started_at: "2026-10-20T10:00:00Z",
      finished_at: "2026-10-20T10:00:01Z",
      duration_ms: 1000,
      ...overrides
    });
    const firstPage = Array.from({ length: 100 }, (_, i) => call(200 - i));
    const rt = runtime(task({ status: "human_review" }), {
      "GET /tasks/:id/tool-calls": ({ query }) =>
        query.get("before") === "t1:call-101"
          ? [call(7, { tool_name: "github_api", kind: "dynamic", status: "failed", output: "refused", output_truncated: true })]
          : firstPage
    });
    renderPage(rt);
    fireEvent.click(await screen.findByRole("tab", { name: "Tool calls" }));
    const list = await screen.findByRole("list", { name: "Tool calls of this task" });
    expect(within(list).getAllByRole("listitem")).toHaveLength(100);
    expect(within(list).getAllByRole("listitem")[0]).toHaveTextContent("tool_200");

    fireEvent.click(screen.getByRole("button", { name: "Load older tool calls" }));
    const older = await screen.findByText("github_api");
    const item = older.closest("li")!;
    // A failed call opens with its output.
    expect(within(item).getByText("Failed")).toBeInTheDocument();
    expect(within(item).getByText("Symphony tool")).toBeInTheDocument();
    expect(within(item).getByText("refused")).toBeInTheDocument();
    expect(within(item).getByText("The output was longer than 64 KiB and was cut.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Load older tool calls" })).not.toBeInTheDocument();
    expect(rt.calls.filter((c) => c.path === "/tasks/t1/tool-calls")).toHaveLength(2);
  });

  it("keeps the loaded tool calls when an older page fails", async () => {
    const firstPage = Array.from({ length: 100 }, (_, i) => ({
      id: `t1:c${i}`,
      task_id: "t1",
      session_id: null,
      provider: "codex" as const,
      call_id: `c${i}`,
      tool_name: `tool_${i}`,
      kind: "command" as const,
      arguments: null,
      output: null,
      output_truncated: false,
      status: "succeeded" as const,
      started_at: "2026-10-20T10:00:00Z",
      finished_at: null,
      duration_ms: null
    }));
    const rt = runtime(task({ status: "human_review" }), {
      "GET /tasks/:id/tool-calls": ({ query }) =>
        query.get("before") ? { status: 500, body: { error: { code: "storage_error", message: "disk busy" } } } : firstPage
    });
    renderPage(rt);
    fireEvent.click(await screen.findByRole("tab", { name: "Tool calls" }));
    await screen.findByText("tool_0");
    fireEvent.click(screen.getByRole("button", { name: "Load older tool calls" }));
    expect(await screen.findByText("Could not load older tool calls")).toBeInTheDocument();
    expect(screen.getByText("tool_0")).toBeInTheDocument();
    expect(within(screen.getByRole("list", { name: "Tool calls of this task" })).getAllByRole("listitem")).toHaveLength(100);
  });

  it("says so when refreshing the tool calls fails and keeps what was loaded", async () => {
    let fail = false;
    const call = {
      id: "t1:c1", task_id: "t1", session_id: null, provider: "codex" as const, call_id: "c1", tool_name: "tool_one",
      kind: "command" as const, arguments: null, output: null, output_truncated: false, status: "succeeded" as const,
      started_at: "2026-10-20T10:00:00Z", finished_at: null, duration_ms: null
    };
    const rt = runtime(task({ status: "human_review" }), {
      "GET /tasks/:id/tool-calls": () => (fail ? { status: 500, body: { error: { code: "storage_error", message: "disk busy" } } } : [call])
    });
    const { queryClient } = renderPage(rt);
    fireEvent.click(await screen.findByRole("tab", { name: "Tool calls" }));
    await screen.findByText("tool_one");
    fail = true;
    await act(async () => {
      await queryClient.refetchQueries({ queryKey: ["task-tool-calls", "t1"] });
    });
    expect(await screen.findByText("Could not refresh the tool calls; they may be out of date")).toBeInTheDocument();
    expect(screen.getByText("tool_one")).toBeInTheDocument();
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
    fireEvent.click(await screen.findByRole("tab", { name: "Context" }));
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

  it("does not repeat the status as a phase badge once the run has ended", async () => {
    renderPage(runtime(task({ status: "cancelled", current_phase: "canceled_by_reconciliation" })));
    await screen.findByRole("button", { name: "Retry" });
    expect(screen.getAllByText("Cancelled")).toHaveLength(1);
  });

  it("offers only Start (no Retry) for a task that has not run yet", async () => {
    renderPage(runtime(task({ status: "ready" })));
    expect(await screen.findByRole("button", { name: "Start" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Retry" })).not.toBeInTheDocument();
  });
});
