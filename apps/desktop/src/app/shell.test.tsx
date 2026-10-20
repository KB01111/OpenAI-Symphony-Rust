import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AppRoutes } from "./AppRouter.js";
import { createMockRuntime, renderWithProviders } from "../test/renderWithProviders.js";
import { approval, settings, snapshot } from "../test/fixtures.js";
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
    for (const name of ["Overview", "Projects", "Tasks", "Agent runs", "Worktrees", "Pull requests", "Workflow Studio", "Providers", "Settings"]) {
      expect(screen.getByRole("link", { name: new RegExp(name) })).toBeInTheDocument();
    }
    await waitFor(() => expect(screen.getByText("1 running")).toBeInTheDocument());
    expect(screen.getByText("2 queued")).toBeInTheDocument();
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
});
