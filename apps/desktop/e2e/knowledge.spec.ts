/**
 * Repository Knowledge Base against the real runtime and the fixture repository: worktrees and the
 * commit timeline come from git, selecting a commit opens its impact, and the views degrade with a
 * clear reason when the optional engines (Graphify, Serena) are not installed - which is the CI case.
 */
import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { api, fixtureRepo, openApp, prepareRuntime } from "./helpers.js";

interface Project {
  id: string;
  name: string;
}

test.describe.configure({ mode: "serial" });

let project: Project;
let headSha: string;

test.beforeAll(async () => {
  await prepareRuntime();
  const projects = await api<Project[]>("GET", "/projects");
  project =
    projects.find((p) => p.name === "E2E fixture") ??
    (await api<Project>("POST", "/projects", { name: "E2E fixture", repo_path: fixtureRepo(), preferred_provider: "simulated" }));
  // A second commit with a source file, so the timeline and the impact have something to show.
  // (A reused data directory already has it: committing again would fail with nothing to commit.)
  const repo = fixtureRepo();
  if (execFileSync("git", ["ls-files", "lib.rs"], { cwd: repo }).toString().trim() === "") {
    writeFileSync(join(repo, "lib.rs"), "pub fn answer() -> u32 {\n    42\n}\n");
    execFileSync("git", ["add", "."], { cwd: repo });
    execFileSync("git", ["commit", "-q", "-m", "Add the answer"], { cwd: repo });
  }
  headSha = execFileSync("git", ["rev-parse", "HEAD"], { cwd: repo }).toString().trim();
});

test("shows the repository's worktrees and engines on the overview", async ({ page }) => {
  await openApp(page, "/knowledge");
  await expect(page.getByRole("heading", { name: "Repository Knowledge Base" })).toBeVisible();
  // Git is always required and present; the optional engines say how to install them.
  await expect(page.getByText("Engines")).toBeVisible();
  await expect(page.getByRole("tab", { name: "Commits" })).toBeVisible();
  await expect(page.getByText(/worktree/).first()).toBeVisible();
});

test("lists the commits once and opens the impact of the selected one", async ({ page }) => {
  await openApp(page, "/knowledge?tab=commits");
  const list = page.getByRole("list", { name: "Commits across all worktrees" }).first();
  await expect(list.getByText("Add the answer")).toHaveCount(1);
  await list.getByText("Add the answer").click();
  // The commit panel names the changed file; the impact tab explains what is missing without an index.
  await expect(page.getByRole("tab", { name: "Impact" })).toBeVisible();
  await expect(page.getByText("lib.rs").first()).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`commit=${headSha}`));
});

test("the worktrees tab lists the main worktree", async ({ page }) => {
  await openApp(page, "/knowledge?tab=worktrees");
  await expect(page.getByText("main", { exact: true }).first()).toBeVisible();
  await expect(page.getByText("Main", { exact: true })).toBeVisible();
});

test("the graph asks for an index instead of drawing a made-up one", async ({ page }) => {
  await openApp(page, "/knowledge?tab=graph");
  // Either the index does not exist yet, or it exists and the graph is drawn; never an empty guess.
  const overview = await api<{ index: unknown }>("GET", `/projects/${project.id}/knowledge`);
  if (overview.index === null) {
    await expect(page.getByText("Index the repository to explore its graph.")).toBeVisible();
  } else {
    await expect(page.getByRole("img", { name: /Interactive code graph/ })).toBeVisible();
  }
});

test("the knowledge page works in Swedish", async ({ page }) => {
  await openApp(page, "/knowledge", { locale: "sv" });
  await expect(page.getByRole("heading", { name: "Kunskapsbas för repository" })).toBeVisible();
  await expect(page.getByRole("tab", { name: "Arbetsträd" })).toBeVisible();
});
