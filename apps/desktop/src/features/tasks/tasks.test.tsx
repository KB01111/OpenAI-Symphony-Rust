import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { getTranslations } from "@symphony/shared-types";
import NewTaskDialog from "./NewTaskDialog.js";
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

describe("NewTaskDialog", () => {
  it("creates a task with auto_start (never dispatch_immediately) exactly once", async () => {
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
    fireEvent.change(within(dialog).getByLabelText(/^Title/), { target: { value: "Fix flaky test" } });
    fireEvent.change(within(dialog).getByLabelText(/^Description/), { target: { value: "Make it deterministic." } });
    fireEvent.click(within(dialog).getByRole("switch", { name: /Start immediately/ }));
    const submit = within(dialog).getByRole("button", { name: /Create and start/ });
    fireEvent.click(submit);
    fireEvent.click(submit);
    await waitFor(() => expect(runtime.calls.filter((c) => c.method === "POST" && c.path === "/tasks")).toHaveLength(1));
    const body = runtime.calls.find((c) => c.method === "POST" && c.path === "/tasks")!.body as Record<string, unknown>;
    expect(body).toMatchObject({ project_id: "p1", title: "Fix flaky test", description: "Make it deterministic.", auto_start: true });
    expect(body).not.toHaveProperty("dispatch_immediately");
  });
});
