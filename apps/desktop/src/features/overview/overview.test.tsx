import { fireEvent, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import OverviewPage from "./OverviewPage.js";
import { computeOverviewMetrics, recentTimeline } from "./overviewLogic.js";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { approval, project, settings, snapshot, task } from "../../test/fixtures.js";
import { useUiStore } from "../../store/uiStore.js";
import type { AgentEvent } from "@symphony/shared-types";

const ev = (id: string, ts: string): AgentEvent => ({
  id,
  task_id: "t1",
  issue_identifier: "SYM-1",
  session_id: null,
  provider: "simulated",
  event_type: "task/created",
  phase: null,
  message: `event ${id}`,
  payload: null,
  timestamp: ts
});

describe("overview logic", () => {
  it("derives metrics from runtime data only", () => {
    const state = snapshot({
      counts: { running: 2, retrying: 1, queued: 3, blocked: 0, human_review: 1, completed: 4, failed: 1 },
      codex_totals: { input_tokens: 10, output_tokens: 5, total_tokens: 15, seconds_running: 30, estimated_cost_usd: 0.5 }
    });
    const tasks = [task({ id: "a", project_id: "p1", status: "running" }), task({ id: "b", project_id: "p2", status: "completed" })];
    const m = computeOverviewMetrics(state, tasks, true);
    expect(m).toMatchObject({ activeAgents: 2, queued: 3, failed: 1, completed: 4, activeRepositories: 1, totalTokens: 15, health: "healthy" });
    expect(computeOverviewMetrics(state, tasks, false).health).toBe("degraded");
    expect(computeOverviewMetrics({ ...state, scheduler_paused: true }, tasks, true).health).toBe("paused");
  });

  it("merges history and live events newest first without duplicates", () => {
    const out = recentTimeline([ev("1", "2026-01-01T00:00:01Z"), ev("2", "2026-01-01T00:00:02Z")], [ev("2", "2026-01-01T00:00:02Z"), ev("3", "2026-01-01T00:00:03Z")]);
    expect(out.map((e) => e.id)).toEqual(["3", "2", "1"]);
  });
});

describe("OverviewPage", () => {
  it("shows metrics, live sessions, approvals and recent activity from the runtime", async () => {
    const runtime = createMockRuntime({
      "GET /state": () =>
        snapshot({
          counts: { running: 1, retrying: 0, queued: 2, blocked: 0, human_review: 0, completed: 5, failed: 0 },
          running: [
            {
              issue_id: "i1",
              issue_identifier: "SYM-7",
              task_id: "t7",
              project_id: "p1",
              provider: "codex",
              model: "gpt-5-codex",
              state: "In Progress",
              phase: "streaming_turn",
              session_id: "s",
              workspace_path: "/ws",
              turn_count: 1,
              last_event: "item/commandExecution",
              last_message: "Ran cargo test",
              started_at: new Date().toISOString(),
              last_event_at: new Date().toISOString(),
              tokens: { input_tokens: 1, output_tokens: 1, total_tokens: 1234 }
            }
          ]
        }),
      "GET /settings": () => settings(),
      "GET /projects": () => [project()],
      "GET /tasks": () => [task({ status: "running" })],
      "GET /approvals": () => [approval({ issue_identifier: "SYM-7" })],
      "GET /events/recent": () => [ev("e1", "2026-01-01T00:00:01Z")]
    });
    renderWithProviders(<OverviewPage />, { runtime, route: "/overview" });
    await waitFor(() => expect(screen.getByText("SYM-7")).toBeInTheDocument());
    expect(screen.getByText("Ran cargo test")).toBeInTheDocument();
    expect(screen.getByText("1,234")).toBeInTheDocument();
    expect(screen.getByText("Approval required · SYM-7")).toBeInTheDocument();
    expect(await screen.findByText("event e1")).toBeInTheDocument();
    expect(screen.getByText("Active agents").closest("button")).toHaveTextContent("1");
  });

  it("opens the approvals list in Activity from the Approvals tile", async () => {
    const runtime = createMockRuntime({
      "GET /state": () => snapshot(),
      "GET /settings": () => settings(),
      "GET /projects": () => [project()],
      "GET /tasks": () => [task()],
      "GET /approvals": () => [approval({ issue_identifier: "SYM-7" })],
      "GET /events/recent": () => []
    });
    useUiStore.getState().setDrawerTab("events");
    // Only /overview is routed, so leaving it for /activity unmounts the page.
    renderWithProviders(<OverviewPage />, { runtime, route: "/overview", path: "/overview" });
    // The label also titles the approvals panel; the tile is the one inside a button.
    await screen.findByText("Active agents");
    const tile = screen.getAllByText("Pending approvals").map((el) => el.closest("button")).find(Boolean);
    expect(tile).toBeTruthy();
    fireEvent.click(tile!);
    expect(useUiStore.getState().drawerTab).toBe("approvals");
    await waitFor(() => expect(screen.queryByText("Active agents")).not.toBeInTheDocument());
  });

  it("invites registering a repository when none exist", async () => {
    const runtime = createMockRuntime({
      "GET /state": () => snapshot(),
      "GET /settings": () => settings(),
      "GET /projects": () => [],
      "GET /tasks": () => [],
      "GET /approvals": () => [],
      "GET /events/recent": () => []
    });
    renderWithProviders(<OverviewPage />, { runtime, route: "/overview" });
    expect(await screen.findByText("Register your first repository")).toBeInTheDocument();
    expect(screen.getByText("No agents are running")).toBeInTheDocument();
  });
});
