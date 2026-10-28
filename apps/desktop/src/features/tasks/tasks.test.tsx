import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { getTranslations } from "@symphony/shared-types";
import NewTaskDialog from "../composer/NewTaskDialog.js";
import { deriveTitle } from "../composer/TaskComposer.js";
import { allowedMoveTargets, applyDrop, parseLabels, resolveDrop } from "./taskLogic.js";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { project, providerStatus, settings, task } from "../../test/fixtures.js";
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
