import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { useLocation, useNavigate } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import { canRetryTask, getTranslations } from "@symphony/shared-types";
import NewTaskDialog from "../composer/NewTaskDialog.js";
import TasksPage from "./TasksPage.js";
import { deriveTitle } from "../composer/TaskComposer.js";
import { allowedMoveTargets, applyDrop, parseLabels, resolveDrop } from "./taskLogic.js";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { project, providerStatus, settings, snapshot, task } from "../../test/fixtures.js";
import { useUiStore } from "../../store/uiStore.js";

describe("board state machine", () => {
  it("never offers running as a manual move and blocks moves of an active session", () => {
    expect(allowedMoveTargets("ready", false)).not.toContain("running");
    expect(allowedMoveTargets("ready", false)).toContain("blocked");
    expect(allowedMoveTargets("running", true)).toEqual([]);
  });

  it("rejects disallowed drops locally with a localized reason and never calls the runtime", () => {
    const move = vi.fn();
    const warn = vi.fn();
    const t = getTranslations("en");
    const decision = applyDrop({ id: "t1", status: "ready" }, "running", new Set(), t, "Move not allowed", { move, warn });
    expect(decision).toEqual({ kind: "reject", reason: "use_dispatch" });
    expect(move).not.toHaveBeenCalled();
    expect(warn).toHaveBeenCalledWith("Move not allowed", expect.any(String));
  });

  it("moves valid drops exactly once and ignores drops on the same column", () => {
    const move = vi.fn();
    const t = getTranslations("en");
    applyDrop({ id: "t1", status: "ready" }, "blocked", new Set(), t, "x", { move, warn: vi.fn() });
    expect(move).toHaveBeenCalledTimes(1);
    expect(move).toHaveBeenCalledWith("t1", "blocked");
    expect(resolveDrop({ id: "t1", status: "ready" }, "ready", new Set())).toEqual({ kind: "noop" });
  });

  it("offers Retry on a ready task only once it has run", () => {
    expect(canRetryTask({ status: "ready", attempt_count: 0 })).toBe(false);
    expect(canRetryTask({ status: "ready", attempt_count: 2 })).toBe(true);
    expect(canRetryTask({ status: "failed", attempt_count: 0 })).toBe(true);
    expect(canRetryTask({ status: "running", attempt_count: 1 })).toBe(false);
  });

  it("parses comma separated labels", () => {
    expect(parseLabels(" bug, backend ,bug,, ")).toEqual(["bug", "backend"]);
  });
});

describe("NewTaskDialog (composer)", () => {
  it("creates a task from one prompt with auto_start (never dispatch_immediately) exactly once", async () => {
    const p = project({ id: "p1", name: "Repo", preferred_provider: "simulated" });
    const runtime = createMockRuntime({
      "GET /projects": () => [p],
      "GET /providers": () => [providerStatus({ provider: "simulated", installed: true })],
      "GET /settings": () => settings(),
      "GET /tasks": () => [],
      "POST /tasks": ({ body }) => task({ id: "new", project_id: "p1", title: (body as { title: string }).title })
    });
    useUiStore.setState({ activeProjectId: "p1", newTaskOpen: true });
    renderWithProviders(<NewTaskDialog />, { runtime });
    const dialog = await screen.findByRole("dialog", { name: "New task" });
    const prompt = within(dialog).getByRole("textbox", { name: "Task prompt" });
    fireEvent.change(prompt, { target: { value: "Fix flaky test\n\nMake it deterministic." } });
    // "Run now" is the default mode.
    const submit = within(dialog).getByRole("button", { name: "Run" });
    fireEvent.click(submit);
    fireEvent.click(submit);
    await waitFor(() => expect(runtime.calls.filter((c) => c.method === "POST" && c.path === "/tasks")).toHaveLength(1));
    const body = runtime.calls.find((c) => c.method === "POST" && c.path === "/tasks")!.body as Record<string, unknown>;
    expect(body).toMatchObject({
      project_id: "p1",
      title: "Fix flaky test",
      description: "Fix flaky test\n\nMake it deterministic.",
      provider: "simulated",
      auto_start: true
    });
    expect(body).not.toHaveProperty("dispatch_immediately");
  });

  it("queues instead of starting when the mode is switched, and requires a prompt", async () => {
    const runtime = createMockRuntime({
      "GET /projects": () => [project({ id: "p1", name: "Repo", preferred_provider: "simulated" })],
      "GET /providers": () => [providerStatus({ provider: "simulated", installed: true })],
      "GET /settings": () => settings(),
      "GET /tasks": () => [],
      "POST /tasks": () => task({ id: "new", project_id: "p1" })
    });
    useUiStore.setState({ activeProjectId: "p1", newTaskOpen: true });
    renderWithProviders(<NewTaskDialog />, { runtime });
    const dialog = await screen.findByRole("dialog", { name: "New task" });
    fireEvent.click(within(dialog).getByRole("radio", { name: "Add to queue" }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Queue task" }));
    expect(await within(dialog).findByText("Describe the task first.")).toBeInTheDocument();
    expect(runtime.calls.some((c) => c.method === "POST" && c.path === "/tasks")).toBe(false);
    fireEvent.change(within(dialog).getByRole("textbox", { name: "Task prompt" }), { target: { value: "Write the docs" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Queue task" }));
    await waitFor(() => expect(runtime.calls.filter((c) => c.method === "POST" && c.path === "/tasks")).toHaveLength(1));
    const body = runtime.calls.find((c) => c.method === "POST" && c.path === "/tasks")!.body as Record<string, unknown>;
    expect(body).toMatchObject({ title: "Write the docs", description: null, auto_start: false });
  });
});

describe("deriveTitle", () => {
  it("strips Markdown markers but keeps numbers and letters that belong to the sentence", () => {
    expect(deriveTitle("# Fix the build\n\nMore detail")).toBe("Fix the build");
    expect(deriveTitle("- add retry")).toBe("add retry");
    expect(deriveTitle("1. First step")).toBe("First step");
    expect(deriveTitle("2026 roadmap")).toBe("2026 roadmap");
    expect(deriveTitle("3D rendering fix")).toBe("3D rendering fix");
    expect(deriveTitle("1Password integration")).toBe("1Password integration");
  });

  it("skips lines that are only a marker or a horizontal rule", () => {
    expect(deriveTitle("---\nFix the flaky test")).toBe("Fix the flaky test");
    expect(deriveTitle("***\n\n- \n-\n#\n2. Real instruction")).toBe("Real instruction");
    expect(deriveTitle("- - -\nDo the thing")).toBe("Do the thing");
    expect(deriveTitle("___\nDo the thing")).toBe("Do the thing");
  });

  it("keeps a line that only looks like a rule but mixes characters", () => {
    expect(deriveTitle("-*-\nDo the thing")).toBe("-*-");
    expect(deriveTitle("===\nDo the thing")).toBe("===");
  });

  it("shortens long prompts at a word boundary", () => {
    // 79 characters of text, then a word that crosses the 80-character cutoff.
    const head = `${"a".repeat(30)} ${"b".repeat(30)} ${"c".repeat(17)}`;
    expect(head).toHaveLength(79);
    expect(deriveTitle(`${head} partialword and more`)).toBe(`${head}…`);
    // A cutoff inside a word drops the partial word.
    const early = `${"a".repeat(30)} ${"b".repeat(30)} ${"c".repeat(10)} ${"d".repeat(40)}`;
    expect(deriveTitle(early)).toBe(`${"a".repeat(30)} ${"b".repeat(30)} ${"c".repeat(10)}…`);
    expect(deriveTitle("#\nFix the build")).toBe("Fix the build");
  });
});

describe("task board columns", () => {
  const boardRuntime = (tasks: ReturnType<typeof task>[]) =>
    createMockRuntime({
      "GET /settings": () => settings(),
      "GET /state": () => snapshot(),
      "GET /projects": () => [project({ id: "p1" })],
      "GET /tasks": () => tasks,
      "GET /approvals": () => [],
      "GET /providers": () => []
    });

  it("opens the Runs view from /tasks?view=runs and switches back with the toggle", async () => {
    useUiStore.getState().setTaskView("board");
    renderWithProviders(<TasksPage />, {
      runtime: boardRuntime([task({ id: "q", project_id: "p1", status: "queued", identifier: "SYM-9", title: "Queued work" })]),
      route: "/tasks?view=runs",
      path: "/tasks"
    });
    expect(await screen.findByRole("button", { name: "Queued work" })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Runs" })).toHaveAttribute("aria-checked", "true");
    // The task filters belong to Board and Table only.
    expect(screen.queryByRole("search", { name: /filters/i })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("radio", { name: "Board" }));
    expect(await screen.findByRole("region", { name: "Queued" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Queued work" })).not.toBeInTheDocument();
    useUiStore.getState().setTaskView("board");
  });

  it("never keeps Runs as the saved view: /tasks opens the board, and Go to Tasks leaves Runs", async () => {
    useUiStore.getState().setTaskView("board");
    // A plain /tasks link (Overview tiles, project links) shows the board even right after Runs was used.
    const first = renderWithProviders(<TasksPage />, { runtime: boardRuntime([]), route: "/tasks?view=runs", path: "/tasks" });
    expect(await screen.findByText("No agent runs yet")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Go to Tasks" }));
    expect(await screen.findByRole("radio", { name: "Board" })).toHaveAttribute("aria-checked", "true");
    expect(screen.queryByText("No agent runs yet")).not.toBeInTheDocument();
    first.unmount();
    renderWithProviders(<TasksPage />, { runtime: boardRuntime([]), route: "/tasks", path: "/tasks" });
    expect(await screen.findByRole("radio", { name: "Board" })).toHaveAttribute("aria-checked", "true");
  });

  it("makes Back return to the view you came from when switching to Runs", async () => {
    useUiStore.getState().setTaskView("board");
    const Back = () => {
      const navigate = useNavigate();
      return <button onClick={() => navigate(-1)}>history-back</button>;
    };
    renderWithProviders(
      <>
        <TasksPage />
        <Back />
      </>,
      { runtime: boardRuntime([]), route: "/tasks", path: "/tasks" }
    );
    fireEvent.click(await screen.findByRole("radio", { name: "Runs" }));
    expect(await screen.findByText("No agent runs yet")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "history-back" }));
    expect(await screen.findByRole("radio", { name: "Board" })).toHaveAttribute("aria-checked", "true");
    expect(screen.queryByText("No agent runs yet")).not.toBeInTheDocument();
  });

  it("adds a history entry only when entering or leaving Runs, not when switching Board and Table", async () => {
    useUiStore.getState().setTaskView("board");
    const Entry = () => <output data-testid="entry">{useLocation().key}</output>;
    renderWithProviders(
      <>
        <TasksPage />
        <Entry />
      </>,
      { runtime: boardRuntime([task({ id: "a", project_id: "p1", status: "ready" })]), route: "/tasks", path: "/tasks" }
    );
    const entry = () => screen.getByTestId("entry").textContent;
    fireEvent.click(await screen.findByRole("radio", { name: "Table" }));
    expect(useUiStore.getState().taskView).toBe("table");
    const afterTable = entry();
    fireEvent.click(screen.getByRole("radio", { name: "Board" }));
    expect(useUiStore.getState().taskView).toBe("board");
    // Board and Table are a preference: the history entry does not change.
    expect(entry()).toBe(afterTable);
    fireEvent.click(screen.getByRole("radio", { name: "Runs" }));
    await waitFor(() => expect(entry()).not.toBe(afterTable));
    useUiStore.getState().setTaskView("board");
  });

  it("keeps showing Runs from cached projects when a later project refresh fails", async () => {
    let calls = 0;
    const rt = createMockRuntime({
      "GET /settings": () => settings(),
      "GET /state": () => snapshot(),
      "GET /projects": () => (++calls === 1 ? [project({ id: "p1" })] : { status: 500, body: { error: { code: "internal", message: "projects unavailable" } } }),
      "GET /tasks": () => [],
      "GET /approvals": () => [],
      "GET /providers": () => []
    });
    const { queryClient } = renderWithProviders(<TasksPage />, { runtime: rt, route: "/tasks?view=runs", path: "/tasks" });
    expect(await screen.findByText("No agent runs yet")).toBeInTheDocument();
    await act(async () => {
      await queryClient.refetchQueries({ queryKey: ["projects"] });
    });
    expect(calls).toBeGreaterThan(1);
    expect(screen.getByText("No agent runs yet")).toBeInTheDocument();
    expect(screen.queryByText("Projects could not be loaded")).not.toBeInTheDocument();
  });

  it("shows the project-load failure in Runs too instead of a list built without projects", async () => {
    const rt = createMockRuntime({
      "GET /settings": () => settings(),
      "GET /state": () => snapshot(),
      "GET /projects": () => ({ status: 500, body: { error: { code: "internal", message: "projects unavailable" } } }),
      "GET /tasks": () => [],
      "GET /approvals": () => [],
      "GET /providers": () => []
    });
    renderWithProviders(<TasksPage />, { runtime: rt, route: "/tasks?view=runs", path: "/tasks" });
    expect(await screen.findByText("Projects could not be loaded")).toBeInTheDocument();
    expect(screen.queryByText("No agent runs yet")).not.toBeInTheDocument();
  });

  it("keeps the selected project's name off the Runs heading, since Runs spans every project", async () => {
    useUiStore.getState().setTaskView("board");
    useUiStore.getState().setActiveProject("p1");
    const rt = createMockRuntime({
      "GET /settings": () => settings(),
      "GET /state": () => snapshot(),
      "GET /projects": () => [project({ id: "p1", name: "Alpha" })],
      "GET /tasks": () => [task({ id: "a", project_id: "p1", status: "ready" })],
      "GET /approvals": () => [],
      "GET /providers": () => []
    });
    renderWithProviders(<TasksPage />, { runtime: rt, route: "/tasks?view=runs", path: "/tasks" });
    await screen.findByRole("radio", { name: "Runs" });
    expect(screen.getByText(/Plan, dispatch and track agent work/)).toBeInTheDocument();
    expect(screen.queryByText("Tasks in Alpha")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("radio", { name: "Board" }));
    expect(await screen.findByText("Tasks in Alpha")).toBeInTheDocument();
    useUiStore.getState().setActiveProject(null);
  });

  it("shows Backlog, Blocked and Cancelled only once they hold a task", async () => {
    useUiStore.getState().setTaskView("board");
    const { unmount } = renderWithProviders(<TasksPage />, { runtime: boardRuntime([task({ id: "a", project_id: "p1", status: "ready" })]), route: "/tasks" });
    expect(await screen.findByRole("region", { name: "Ready" })).toBeInTheDocument();
    for (const name of ["Backlog", "Blocked", "Cancelled"]) expect(screen.queryByRole("region", { name })).not.toBeInTheDocument();
    unmount();

    renderWithProviders(<TasksPage />, { runtime: boardRuntime([task({ id: "b", project_id: "p1", status: "blocked" })]), route: "/tasks" });
    expect(await screen.findByRole("region", { name: "Blocked" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Backlog" })).not.toBeInTheDocument();
  });
});
