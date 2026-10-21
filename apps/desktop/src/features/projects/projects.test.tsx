import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import AddRepositoryDialog from "./AddRepositoryDialog.js";
import ProjectsPage from "./ProjectsPage.js";
import { formToUpdate, projectToForm, validateProjectForm } from "./projectLogic.js";
import { createMockRuntime, renderWithProviders } from "../../test/renderWithProviders.js";
import { project, providerStatus, settings } from "../../test/fixtures.js";
import { useUiStore } from "../../store/uiStore.js";

const validation = (ok: boolean) => ({
  repo_path: ok ? "/src/app" : "/nope",
  exists: ok,
  is_directory: ok,
  is_git_repository: ok,
  current_branch: ok ? "main" : null,
  has_workflow_file: false,
  suggested_name: ok ? "app" : null,
  errors: ok ? [] : ["The selected path does not exist."]
});

describe("project form logic", () => {
  it("validates required fields and builds a minimal update", () => {
    const p = project({ id: "p1", name: "App", max_concurrent_agents: 2 });
    const form = projectToForm(p);
    expect(validateProjectForm({ ...form, name: " " })).toHaveProperty("name");
    const update = formToUpdate(p, { ...form, name: "Renamed" });
    expect(update).toEqual({ name: "Renamed" });
  });
});

describe("AddRepositoryDialog", () => {
  it("validates the folder with the runtime, then registers the resolved checkout once", async () => {
    const runtime = createMockRuntime({
      "POST /projects/validate": ({ body }) => validation((body as { repo_path: string }).repo_path === "/src/app/sub"),
      "POST /projects": ({ body }) => project({ id: "new", name: (body as { name: string }).name, repo_path: "/src/app" }),
      "GET /projects": () => [],
      "GET /providers": () => [providerStatus({ provider: "simulated", installed: true })],
      "GET /settings": () => settings()
    });
    useUiStore.setState({ addProjectOpen: true });
    renderWithProviders(<AddRepositoryDialog />, { runtime });
    const dialog = await screen.findByRole("dialog", { name: "Add repository" });

    fireEvent.change(within(dialog).getByLabelText(/^Repository path/), { target: { value: "/nope" } });
    fireEvent.click(await within(dialog).findByRole("button", { name: "Validate" }));
    expect(await within(dialog).findByText("The selected path does not exist.")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Continue" })).toBeDisabled();

    fireEvent.click(await within(dialog).findByRole("button", { name: "Choose another folder" }));
    fireEvent.change(within(dialog).getByLabelText(/^Repository path/), { target: { value: "/src/app/sub" } });
    fireEvent.click(await within(dialog).findByRole("button", { name: "Validate" }));
    const next = await within(dialog).findByRole("button", { name: "Continue" });
    await waitFor(() => expect(next).toBeEnabled());
    fireEvent.click(next);
    fireEvent.change(within(dialog).getByLabelText(/^Name/), { target: { value: "App" } });
    const create = within(dialog).getByRole("button", { name: "Add repository" });
    fireEvent.click(create);
    fireEvent.click(create);
    await waitFor(() => expect(runtime.calls.filter((c) => c.method === "POST" && c.path === "/projects")).toHaveLength(1));
    const body = runtime.calls.find((c) => c.method === "POST" && c.path === "/projects")!.body as Record<string, unknown>;
    expect(body).toMatchObject({ name: "App", repo_path: "/src/app" });
  });
});

describe("ProjectsPage", () => {
  it("surfaces the runtime's 409 when deleting a project with a running task", async () => {
    const runtime = createMockRuntime({
      "GET /projects": () => [project({ id: "p1", name: "App" })],
      "GET /projects/:id": () => project({ id: "p1", name: "App" }),
      "GET /tasks": () => [],
      "GET /providers": () => [],
      "GET /settings": () => settings(),
      "DELETE /projects/:id": () => ({ status: 409, body: { error: { code: "conflict", message: "conflict: Project has a running task" } } })
    });
    renderWithProviders(<ProjectsPage />, { runtime, route: "/projects/p1", path: "/projects/:projectId" });
    fireEvent.click(await screen.findByRole("button", { name: /Delete/ }));
    const dialog = await screen.findByRole("alertdialog");
    fireEvent.click(await within(dialog).findByRole("button", { name: /Delete|Remove/ }));
    expect((await screen.findAllByText(/Project has a running task/)).length).toBeGreaterThan(0);
    expect(runtime.calls.filter((c) => c.method === "DELETE")).toHaveLength(1);
  });

  it("rebases a clean edit form onto a project another client changed", async () => {
    let current = project({ id: "p1", name: "App", updated_at: "2026-10-01T10:00:00Z" });
    const runtime = createMockRuntime({
      "GET /projects": () => [current],
      "GET /projects/:id": () => current,
      "GET /tasks": () => [],
      "GET /providers": () => [],
      "GET /settings": () => settings()
    });
    const { queryClient } = renderWithProviders(<ProjectsPage />, { runtime, route: "/projects/p1", path: "/projects/:projectId" });
    await waitFor(() => expect((screen.getByLabelText(/^Name/) as HTMLInputElement).value).toBe("App"));

    current = project({ id: "p1", name: "Renamed elsewhere", updated_at: "2026-10-01T11:00:00Z" });
    await act(() => queryClient.invalidateQueries());
    await waitFor(() => expect((screen.getByLabelText(/^Name/) as HTMLInputElement).value).toBe("Renamed elsewhere"));
    expect(runtime.calls.filter((c) => c.method === "PATCH")).toHaveLength(0);
  });
});
