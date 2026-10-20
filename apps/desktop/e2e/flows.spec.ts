/**
 * Critical user flows against the real runtime with the deterministic simulated agent:
 * repository registration, task creation, board moves (allowed and rejected), live execution,
 * the approval workflow and inline error handling.
 */
import { expect, test, type Page } from "@playwright/test";
import { api, fixtureRepo, openApp, prepareRuntime } from "./helpers.js";

interface Project {
  id: string;
  name: string;
  repo_path: string;
}
interface Task {
  id: string;
  status: string;
  issue: { identifier: string; title: string };
}

test.describe.configure({ mode: "serial" });

let project: Project;

async function ensureProject(): Promise<Project> {
  const projects = await api<Project[]>("GET", "/projects");
  const existing = projects.find((p) => p.name === "E2E fixture");
  if (existing) return existing;
  return api<Project>("POST", "/projects", { name: "E2E fixture", repo_path: fixtureRepo(), preferred_provider: "simulated" });
}

async function createTask(title: string, description: string, autoStart: boolean): Promise<Task> {
  return api<Task>("POST", "/tasks", {
    project_id: project.id,
    title,
    description,
    preferred_provider: "simulated",
    auto_start: autoStart
  });
}

async function waitForStatus(taskId: string, statuses: string[], timeoutMs = 45_000): Promise<Task> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const task = await api<Task>("GET", `/tasks/${taskId}`);
    if (statuses.includes(task.status)) return task;
    if (Date.now() > deadline) throw new Error(`task ${taskId} stuck in ${task.status}`);
    await new Promise((r) => setTimeout(r, 300));
  }
}

async function openTaskMenu(page: Page, identifier: string) {
  await page.getByRole("button", { name: `Actions for ${identifier}` }).click();
}

test.beforeAll(async () => {
  await prepareRuntime();
});

test("registers a repository through the guided dialog", async ({ page }) => {
  const before = await api<Project[]>("GET", "/projects");
  test.skip(before.some((p) => p.name === "E2E fixture"), "fixture already registered by an earlier run");

  await openApp(page, "/projects");
  await page.getByRole("button", { name: "Add repository" }).first().click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Repository path").fill(fixtureRepo());
  await dialog.getByRole("button", { name: "Validate" }).click();
  await expect(dialog.getByText("Is a Git repository")).toBeVisible();
  await dialog.getByRole("button", { name: "Continue" }).click();
  await dialog.getByLabel("Name").fill("E2E fixture");
  await dialog.getByRole("button", { name: "Add repository" }).click();
  await expect(page.getByText("E2E fixture added").first()).toBeVisible();

  const after = await api<Project[]>("GET", "/projects");
  const created = after.find((p) => p.name === "E2E fixture");
  expect(created).toBeTruthy();
  await expect(page.getByRole("main").getByText("E2E fixture").first()).toBeVisible();
});

test("shows inline validation for a path that is not a Git repository", async ({ page }) => {
  await openApp(page, "/projects");
  await page.getByRole("button", { name: "Add repository" }).first().click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Repository path").fill("/definitely/not/a/repository");
  await dialog.getByRole("button", { name: "Validate" }).click();
  await expect(dialog.getByRole("img", { name: "Failed" }).first()).toBeVisible();
  await expect(dialog.getByText("The selected path does not exist.")).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Continue" })).toBeDisabled();
});

test("creates a task from the keyboard and shows it on the board", async ({ page }) => {
  project = await ensureProject();
  // Tasks use the simulated agent so no paid model is ever dispatched.
  await api("PATCH", `/projects/${project.id}`, { preferred_provider: "simulated" });

  await openApp(page, "/tasks");
  await page.keyboard.press("Control+n");
  const dialog = page.getByRole("dialog", { name: "New task" });
  await expect(dialog).toBeVisible();
  const title = `E2E create ${Date.now()}`;
  await dialog.getByLabel("Title").fill(title);
  await dialog.getByLabel("Description").fill("Created by the end-to-end suite.");
  await dialog.getByRole("button", { name: "Create task" }).click();
  await expect(dialog).toBeHidden();

  await expect(page.getByRole("group", { name: /board/i }).getByText(title)).toBeVisible();
  const tasks = await api<Task[]>("GET", "/tasks");
  const created = tasks.find((t) => t.issue.title === title);
  expect(created?.status).toBe("ready");
});

test("moves a task with the board menu and refuses moves the state machine forbids", async ({ page }) => {
  project = await ensureProject();
  const task = await createTask(`E2E move ${Date.now()}`, "Board interaction", false);
  await openApp(page, "/tasks");
  await expect(page.getByText(task.issue.title)).toBeVisible();

  await openTaskMenu(page, task.issue.identifier);
  // Running is reached only by dispatching, never by a manual move.
  await expect(page.getByRole("menuitem", { name: "Running", exact: true })).toHaveCount(0);
  await page.getByRole("menuitem", { name: "Blocked", exact: true }).click();
  await expect.poll(async () => (await api<Task>("GET", `/tasks/${task.id}`)).status).toBe("blocked");
});

test("dispatches a simulated agent and inspects the live execution workspace", async ({ page }) => {
  project = await ensureProject();
  const task = await createTask(`E2E run ${Date.now()}`, "Write a short note.", true);
  await openApp(page, `/runs/${task.id}`);
  await expect(page.getByRole("heading", { level: 1 })).toContainText(task.issue.identifier);
  await waitForStatus(task.id, ["human_review", "completed"]);
  await expect(page.getByText("SYMPHONY_TASK_NOTE.md").first()).toBeVisible({ timeout: 20_000 });
  await expect(page.getByText("Human review").first()).toBeVisible();
});

test("approves a command from the execution view exactly once", async ({ page }) => {
  project = await ensureProject();
  const task = await createTask(`E2E approval ${Date.now()}`, "Run the tests. SIMULATE_APPROVAL", true);
  await openApp(page, `/runs/${task.id}`);
  const approve = page.getByRole("button", { name: "Approve once" }).first();
  await expect(approve).toBeVisible({ timeout: 30_000 });
  await approve.click();
  await waitForStatus(task.id, ["human_review", "completed"]);
  const approvals = await api<{ task_id: string; status: string; decision: string | null }[]>("GET", "/approvals?only_pending=false");
  const decided = approvals.filter((a) => a.task_id === task.id);
  expect(decided).toHaveLength(1);
  expect(decided[0]!.decision).toBe("accept");
});
