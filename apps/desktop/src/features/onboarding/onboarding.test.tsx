import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import OnboardingDialog from "./OnboardingDialog.js";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { settings } from "../../test/fixtures.js";
import { useUiStore } from "../../store/uiStore.js";

const status = (provider: string, installed: boolean) => ({
  provider,
  display_name: provider,
  installed,
  binary_path: installed ? `/usr/bin/${provider}` : null,
  version: installed ? "1.2.3" : null,
  auth_state: installed ? "authenticated" : "unknown",
  auth_detail: null,
  default_model: null,
  capabilities: {},
  detected_at: "2026-01-01T00:00:00Z"
});

function runtime() {
  return createMockRuntime({
    "GET /settings": () => settings({ onboarding_completed: false }),
    "PUT /settings": ({ body }) => body,
    "GET /providers": () => [status("codex", true), status("claude_code", false), status("simulated", true)],
    "GET /projects": () => []
  });
}

describe("OnboardingDialog", () => {
  it("walks through detected agents and repository registration, then saves completion once", async () => {
    const rt = runtime();
    renderWithProviders(<OnboardingDialog />, { runtime: rt });
    expect(await screen.findByText("Orchestrate coding agents on your own machine")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(await screen.findByText("Detected coding agents")).toBeInTheDocument();
    expect(await screen.findByText(/Installed · 1.2.3 · Authenticated/)).toBeInTheDocument();
    expect(screen.getByText("Not installed")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(await screen.findByText("Register a repository")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Add repository…" }));
    // The Add repository flow opens on top; onboarding keeps its step.
    expect(useUiStore.getState().addProjectOpen).toBe(true);
    act(() => useUiStore.getState().setAddProjectOpen(false));

    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    const finish = await screen.findByRole("button", { name: "Start using Symphony" });
    fireEvent.click(finish);
    fireEvent.click(finish);
    await waitFor(() => expect(rt.calls.filter((c) => c.method === "PUT")).toHaveLength(1));
    expect(rt.calls.find((c) => c.method === "PUT")!.body).toMatchObject({ onboarding_completed: true });
  });

  it("can be skipped from the first step", async () => {
    const rt = runtime();
    renderWithProviders(<OnboardingDialog />, { runtime: rt });
    fireEvent.click(await screen.findByRole("button", { name: "Skip setup" }));
    await waitFor(() => expect(rt.calls.some((c) => c.method === "PUT")).toBe(true));
  });
});
