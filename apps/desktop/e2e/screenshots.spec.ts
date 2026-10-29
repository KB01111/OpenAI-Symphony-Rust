/**
 * Captures the documentation screenshots (docs/screenshots) from the real UI and runtime with
 * seeded simulated-agent data, in light and dark themes and at several window sizes.
 */
import { expect, test, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { api, fixtureRepo, openApp, prepareRuntime } from "./helpers.js";

const outDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "docs", "screenshots");

interface Project {
  id: string;
  name: string;
}
interface Task {
  id: string;
  status: string;
  issue: { identifier: string };
}

let runningTask: Task;
let reviewId: string | null = null;
let reviewTitle = "";

async function waitUntil(check: () => Promise<boolean>, timeoutMs = 45_000) {
  const deadline = Date.now() + timeoutMs;
  while (!(await check())) {
    if (Date.now() > deadline) throw new Error("timed out seeding screenshot data");
    await new Promise((r) => setTimeout(r, 300));
  }
}

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  await prepareRuntime();
  mkdirSync(outDir, { recursive: true });
  const projects = await api<Project[]>("GET", "/projects");
  const project =
    projects.find((p) => p.name === "E2E fixture") ??
    (await api<Project>("POST", "/projects", { name: "E2E fixture", repo_path: fixtureRepo(), preferred_provider: "simulated" }));
  const create = (title: string, description: string, autoStart: boolean, priority?: number) =>
    api<Task>("POST", "/tasks", { project_id: project.id, title, description, preferred_provider: "simulated", auto_start: autoStart, priority });

  // A finished run (diff + PR review), queued/backlog work and a run waiting for approval.
  const done = await create("Add structured logging to the scheduler", "Use tracing spans for each tick.", true, 2);
  await waitUntil(async () => ["human_review", "completed"].includes((await api<Task>("GET", `/tasks/${done.id}`)).status));
  await create("Document the workspace isolation model", "Explain worktrees and cleanup rules.", false, 3);
  await create("Speed up provider detection", "Cache version probes between refreshes.", false, 1);
  runningTask = await create("Run the full test suite before merging", "Execute cargo test. SIMULATE_APPROVAL", true, 1);
  await waitUntil(async () => (await api<{ task_id: string }[]>("GET", "/approvals?only_pending=true")).some((a) => a.task_id === runningTask.id));
  const reviews = await api<{ id: string; title: string }[]>("GET", "/prs");
  if (!reviews[0]) throw new Error("the finished simulated run did not produce a PR review");
  reviewId = reviews[0].id;
  reviewTitle = reviews[0].title;
});

async function shoot(page: Page, name: string, route: string, theme: "light" | "dark", ready: (page: Page) => Promise<void>) {
  await openApp(page, route, { theme });
  await ready(page);
  // Let streaming text and lazy panels settle.
  await page.waitForTimeout(600);
  await page.screenshot({ path: join(outDir, `${name}-${theme}.png`) });
}

const VIEWS: [string, () => string, (page: Page) => Promise<void>][] = [
  ["overview", () => "/overview", (p) => expect(p.getByText("Live agent activity")).toBeVisible()],
  ["projects", () => "/projects", (p) => expect(p.getByRole("main").getByText("E2E fixture").first()).toBeVisible()],
  ["task-board", () => "/tasks", (p) => expect(p.getByText("Speed up provider detection").first()).toBeVisible()],
  ["task-runs", () => "/tasks?view=runs", (p) => expect(p.getByRole("button", { name: "Run the full test suite before merging" })).toBeVisible()],
  ["agent-execution", () => `/runs/${runningTask.id}`, (p) => expect(p.getByRole("button", { name: "Approve once" }).first()).toBeVisible()],
  ["pr-review", () => `/reviews/${reviewId}`, (p) => expect(p.getByRole("main").getByText(reviewTitle).first()).toBeVisible()],
  ["workflow-studio", () => "/workflow", (p) => expect(p.getByText("Workflow is valid").or(p.getByText("Workflow has errors")).first()).toBeVisible()],
  ["settings", () => "/settings", (p) => expect(p.getByRole("main").getByRole("heading").first()).toBeVisible()]
];

for (const theme of ["dark", "light"] as const) {
  for (const [name, route, ready] of VIEWS) {
    test(`${name} (${theme})`, async ({ page }) => {
      await shoot(page, name, route(), theme, ready);
    });
  }
}

for (const [width, height] of [
  [1280, 720],
  [1920, 1080]
] as const) {
  test(`overview at ${width}x${height}`, async ({ page }) => {
    await page.setViewportSize({ width, height });
    await openApp(page, "/overview", { theme: "dark" });
    await page.waitForTimeout(600);
    await page.screenshot({ path: join(outDir, `overview-${width}x${height}-dark.png`) });
  });
}

test("task board at minimum window size (960x640)", async ({ page }) => {
  await page.setViewportSize({ width: 960, height: 640 });
  await openApp(page, "/tasks", { theme: "light" });
  await expect(page.getByText("Speed up provider detection").first()).toBeVisible();
  await page.waitForTimeout(600);
  await page.screenshot({ path: join(outDir, "task-board-960x640-light.png") });
});
