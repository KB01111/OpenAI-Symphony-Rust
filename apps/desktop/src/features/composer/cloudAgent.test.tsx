import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import NewTaskDialog from "./NewTaskDialog.js";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { project, providerStatus, settings, task } from "../../test/fixtures.js";
import { useUiStore } from "../../store/uiStore.js";

type Agent = "mini_swe_agent" | "pi" | "pi_durable";

/**
 * `defaultAgent` is the application setting (`null`: never chosen); `workflowAgent` is
 * `codesandbox.default_agent` of the active WORKFLOW.md, when the test wants the workflow loaded.
 */
function cloudRuntime(preferred: "codesandbox" | "simulated", defaultAgent: Agent | null = "mini_swe_agent", workflowAgent?: Agent) {
  const base = providerStatus();
  return createMockRuntime({
    ...(workflowAgent
      ? { "GET /workflow": () => ({ valid: true, raw_markdown: "", config: { codesandbox: { default_agent: workflowAgent } } }) }
      : {}),
    "GET /projects": () => [project({ id: "p1", name: "Repo", preferred_provider: preferred })],
    "GET /providers": () => [
      providerStatus({
        provider: "codesandbox",
        display_name: "CodeSandbox Cloud Agent",
        installed: true,
        capabilities: { ...base.capabilities, provider: "codesandbox", models: ["anthropic/claude-sonnet-5-5"] }
      }),
      providerStatus({ provider: "simulated", installed: true })
    ],
    "GET /settings": () => settings({ default_cloud_agent: defaultAgent }),
    "GET /tasks": () => [],
    "POST /tasks": ({ body }) => task({ id: "new", project_id: "p1", title: (body as { title: string }).title })
  });
}

/** Radix Select under jsdom: open it from the keyboard, then click the option. */
async function choose(trigger: HTMLElement, option: string | RegExp) {
  fireEvent.keyDown(trigger, { key: "Enter" });
  fireEvent.click(await screen.findByRole("option", { name: option }));
}

async function openProviderChip(dialog: HTMLElement) {
  fireEvent.click(within(dialog).getByRole("button", { name: /Provider \/ Model/ }));
  return screen.findByRole("combobox", { name: "Cloud agent" }).catch(() => null);
}

async function submit(dialog: HTMLElement, runtime: ReturnType<typeof cloudRuntime>) {
  fireEvent.change(within(dialog).getByRole("textbox", { name: "Task prompt" }), { target: { value: "Fix the flaky test" } });
  fireEvent.click(within(dialog).getByRole("button", { name: "Run" }));
  await waitFor(() => expect(runtime.calls.filter((c) => c.method === "POST" && c.path === "/tasks")).toHaveLength(1));
  return runtime.calls.find((c) => c.method === "POST" && c.path === "/tasks")!.body as Record<string, unknown>;
}

describe("composer: cloud agent for CodeSandbox tasks", () => {
  beforeEach(() => {
    useUiStore.setState({ activeProjectId: "p1", newTaskOpen: true });
  });

  it("offers the three agents with the configured default marked, and sends the chosen one", async () => {
    const runtime = cloudRuntime("codesandbox");
    renderWithProviders(<NewTaskDialog />, { runtime });
    const dialog = await screen.findByRole("dialog", { name: "New task" });

    const select = await openProviderChip(dialog);
    expect(select).not.toBeNull();
    // The default is shown selected and labelled as such.
    expect(select).toHaveTextContent("mini-swe-agent (default)");

    fireEvent.keyDown(select!, { key: "Enter" });
    expect(await screen.findByRole("option", { name: "mini-swe-agent (default)" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Pi coding agent" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Pi Durable" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("option", { name: "Pi Durable" }));

    // Close the popover and run.
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    const body = await submit(dialog, runtime);
    expect(body).toMatchObject({ provider: "codesandbox", cloud_agent: "pi_durable", auto_start: true });
  });

  it("sends no explicit agent when the default is kept, so the configured default applies", async () => {
    const runtime = cloudRuntime("codesandbox", "pi");
    renderWithProviders(<NewTaskDialog />, { runtime });
    const dialog = await screen.findByRole("dialog", { name: "New task" });

    const select = await openProviderChip(dialog);
    expect(select).toHaveTextContent("Pi coding agent (default)");
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });

    const body = await submit(dialog, runtime);
    expect(body).toMatchObject({ provider: "codesandbox", cloud_agent: null });
  });

  it("marks the workflow's default agent when the setting was never chosen, and the setting once it was", async () => {
    // New install: no application setting, so WORKFLOW.md's codesandbox.default_agent is the default.
    const fromWorkflow = cloudRuntime("codesandbox", null, "pi_durable");
    const first = renderWithProviders(<NewTaskDialog />, { runtime: fromWorkflow });
    const dialog = await screen.findByRole("dialog", { name: "New task" });
    // Open the picker once; the workflow loads after it, so wait for the label to settle.
    const select = (await openProviderChip(dialog))!;
    await waitFor(() => expect(select).toHaveTextContent("Pi Durable (default)"));
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    const body = await submit(dialog, fromWorkflow);
    // Nothing explicit is sent: the runtime applies the same default.
    expect(body).toMatchObject({ provider: "codesandbox", cloud_agent: null });
    first.unmount();

    // A setting that was chosen wins over the workflow.
    useUiStore.setState({ activeProjectId: "p1", newTaskOpen: true });
    renderWithProviders(<NewTaskDialog />, { runtime: cloudRuntime("codesandbox", "pi", "pi_durable") });
    const second = await screen.findByRole("dialog", { name: "New task" });
    const chosen = (await openProviderChip(second))!;
    await waitFor(() => expect(chosen).toHaveTextContent("Pi coding agent (default)"));
  });

  it("choosing the default agent again clears the explicit choice", async () => {
    const runtime = cloudRuntime("codesandbox");
    renderWithProviders(<NewTaskDialog />, { runtime });
    const dialog = await screen.findByRole("dialog", { name: "New task" });

    const select = (await openProviderChip(dialog))!;
    await choose(select, "Pi coding agent");
    await choose(select, "mini-swe-agent (default)");
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });

    const body = await submit(dialog, runtime);
    expect(body).toMatchObject({ cloud_agent: null });
  });

  it("does not show the picker for other providers and never sends a cloud agent for them", async () => {
    const runtime = cloudRuntime("simulated");
    renderWithProviders(<NewTaskDialog />, { runtime });
    const dialog = await screen.findByRole("dialog", { name: "New task" });

    expect(await openProviderChip(dialog)).toBeNull();
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    const body = await submit(dialog, runtime);
    expect(body).toMatchObject({ provider: "simulated", cloud_agent: null });
  });
});
