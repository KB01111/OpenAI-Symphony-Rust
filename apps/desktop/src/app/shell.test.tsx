import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AppRoutes } from "./AppRouter.js";
import { createMockRuntime, renderWithProviders } from "../test/renderWithProviders.js";
import { approval, project, settings, snapshot, task } from "../test/fixtures.js";
import { useUiStore } from "../store/uiStore.js";

function runtime() {
  return createMockRuntime({
    "GET /state": () => snapshot({ counts: { running: 1, retrying: 0, queued: 2, blocked: 0, human_review: 0, completed: 0, failed: 0 } }),
    "GET /settings": () => settings(),
    "GET /projects": () => [],
    "GET /tasks": () => [],
    "GET /approvals": () => [approval()],
    "GET /providers": () => []
  });
}

describe("Desktop shell", () => {
  it("renders navigation, live runtime status and pending approvals", async () => {
    renderWithProviders(<AppRoutes />, { runtime: runtime(), route: "/settings" });
    expect(await screen.findByRole("navigation", { name: "Primary navigation" })).toBeInTheDocument();
    for (const name of ["Overview", "Projects", "Tasks", "Agent runs", "Activity", "Worktrees", "Pull requests", "Workflow Studio", "Providers", "Settings"]) {
      expect(screen.getByRole("link", { name: new RegExp(name) })).toBeInTheDocument();
    }
    await waitFor(() => expect(screen.getByText("1 running")).toBeInTheDocument());
    expect(screen.getByText("2 queued")).toBeInTheDocument();
  });

  it("opens on the prompt composer", async () => {
    renderWithProviders(<AppRoutes />, { runtime: runtime(), route: "/" });
    expect(await screen.findByRole("heading", { level: 1, name: "What should Symphony work on?" })).toBeInTheDocument();
  });

  it("switches language and persists the choice in the UI store", async () => {
    useUiStore.getState().setLocale("sv");
    renderWithProviders(<AppRoutes />, { runtime: runtime(), route: "/overview" });
    expect(await screen.findByRole("navigation", { name: "Huvudnavigering" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Arbetsträd/ })).toBeInTheDocument();
    useUiStore.getState().setLocale("en");
  });

  it("collapses the sidebar and keeps labels available as tooltips", async () => {
    useUiStore.getState().setSidebarCollapsed(true);
    renderWithProviders(<AppRoutes />, { runtime: runtime(), route: "/overview" });
    const link = await screen.findByRole("link", { name: "Projects" });
    expect(link).not.toHaveTextContent("Projects");
    useUiStore.getState().setSidebarCollapsed(false);
  });

  it("lists tasks as threads grouped by project and marks the ones waiting for approval", async () => {
    const repo = project({ id: "p1", name: "Backend" });
    const failed = task({ id: "t-failed", identifier: "SYM-7", title: "Repair the build", status: "failed", last_error: "tests failed" });
    const waiting = task({ id: "t-wait", identifier: "SYM-3", title: "Needs a decision", status: "running" });
    const rt = createMockRuntime({
      "GET /state": () => snapshot(),
      "GET /settings": () => settings(),
      "GET /projects": () => [repo],
      "GET /tasks": () => [failed, waiting],
      "GET /approvals": () => [approval({ task_id: "t-wait", issue_identifier: "SYM-3" })],
      "GET /providers": () => []
    });
    renderWithProviders(<AppRoutes />, { runtime: rt, route: "/settings" });
    const threads = await screen.findByRole("region", { name: "Threads" });
    expect(await within(threads).findByRole("link", { name: /Repair the build/ })).toHaveAttribute("href", expect.stringContaining("/runs/t-failed"));
    // The marker sits on the task that waits for approval, not on the failed one.
    expect(within(within(threads).getByRole("link", { name: /Needs a decision/ })).getByRole("img", { name: "Needs approval" })).toBeInTheDocument();
    expect(within(within(threads).getByRole("link", { name: /Repair the build/ })).queryByRole("img", { name: "Needs approval" })).not.toBeInTheDocument();
    expect(within(threads).getByRole("button", { name: "Collapse Backend" })).toBeInTheDocument();
    fireEvent.click(within(threads).getByRole("button", { name: "Collapse Backend" }));
    expect(within(threads).queryByRole("link", { name: /Repair the build/ })).not.toBeInTheDocument();
    useUiStore.getState().toggleProjectCollapsed("p1");
  });

  it("filters the thread list with the sidebar search", async () => {
    const rt = createMockRuntime({
      "GET /state": () => snapshot(),
      "GET /settings": () => settings(),
      "GET /projects": () => [project({ id: "p1", name: "Backend" })],
      "GET /tasks": () => [task({ id: "a", title: "Alpha work" }), task({ id: "b", title: "Beta work" })],
      "GET /approvals": () => [],
      "GET /providers": () => []
    });
    renderWithProviders(<AppRoutes />, { runtime: rt, route: "/settings" });
    const threads = await screen.findByRole("region", { name: "Threads" });
    await within(threads).findByRole("link", { name: /Alpha work/ });
    fireEvent.change(screen.getByRole("searchbox", { name: "Search" }), { target: { value: "beta" } });
    expect(within(threads).queryByRole("link", { name: /Alpha work/ })).not.toBeInTheDocument();
    expect(within(threads).getByRole("link", { name: /Beta work/ })).toBeInTheDocument();
  });

  it("says so when the tasks could not be loaded instead of showing an empty list", async () => {
    const rt = createMockRuntime({
      "GET /state": () => snapshot(),
      "GET /settings": () => settings(),
      "GET /projects": () => [],
      "GET /tasks": () => ({ status: 500, body: { error: { code: "internal", message: "boom" } } }),
      "GET /approvals": () => [],
      "GET /providers": () => []
    });
    renderWithProviders(<AppRoutes />, { runtime: rt, route: "/settings" });
    const threads = await screen.findByRole("region", { name: "Threads" });
    expect(await within(threads).findByRole("alert")).toHaveTextContent("Could not load the tasks.");
    expect(within(threads).queryByText(/No tasks yet/)).not.toBeInTheDocument();
  });

  it("keeps tasks of projects that have not loaded in separate groups", async () => {
    const rt = createMockRuntime({
      "GET /state": () => snapshot(),
      "GET /settings": () => settings(),
      "GET /projects": () => [],
      "GET /tasks": () => [task({ id: "a", project_id: "p-a", title: "From A" }), task({ id: "b", project_id: "p-b", title: "From B" })],
      "GET /approvals": () => [],
      "GET /providers": () => []
    });
    renderWithProviders(<AppRoutes />, { runtime: rt, route: "/settings" });
    const threads = await screen.findByRole("region", { name: "Threads" });
    await within(threads).findByRole("link", { name: /From A/ });
    const groups = within(threads).getAllByRole("button", { name: /^Collapse/ });
    expect(groups).toHaveLength(2);
    const [first, second] = groups.map((button) => button.closest("section")!);
    fireEvent.click(groups[0]!);
    // Collapsing a group hides that group's task and leaves the other group's task visible.
    expect(within(first!).queryByRole("link")).not.toBeInTheDocument();
    expect(within(second!).getByRole("link")).toBeInTheDocument();
    useUiStore.setState({ collapsedProjects: [] });
  });
});
