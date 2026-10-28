/**
 * Layout regressions: nothing overlaps or scrolls sideways where it used to (project drawer fields,
 * the squeezed project table, the Concise/Technical toggle), at the default and the minimum window.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { api, fixtureRepo, openApp, prepareRuntime } from "./helpers.js";

interface Project {
  id: string;
  name: string;
}

let project: Project;

test.beforeAll(async () => {
  await prepareRuntime();
  const projects = await api<Project[]>("GET", "/projects");
  project =
    projects.find((p) => p.name === "E2E fixture") ??
    (await api<Project>("POST", "/projects", { name: "E2E fixture", repo_path: fixtureRepo(), preferred_provider: "simulated" }));
});

const SIZES = [
  [1440, 900],
  [960, 640]
] as const;

/** Elements that scroll sideways inside `root` (a horizontal scrollbar the user would see). */
async function sidewaysScrollers(root: Locator): Promise<string[]> {
  return root.evaluate((el) => {
    const found: string[] = [];
    for (const node of [el, ...Array.from(el.querySelectorAll<HTMLElement>("*"))]) {
      const style = getComputedStyle(node);
      if ((style.overflowX === "auto" || style.overflowX === "scroll") && node.scrollWidth > node.clientWidth + 1) {
        found.push(`${node.tagName.toLowerCase()}.${String(node.className).slice(0, 60)} ${node.scrollWidth}>${node.clientWidth}`);
      }
    }
    return found;
  });
}

async function noPageOverflow(page: Page) {
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
}

for (const [width, height] of SIZES) {
  test(`project table and drawer do not overflow or overlap at ${width}x${height}`, async ({ page }) => {
    await page.setViewportSize({ width, height });
    await openApp(page, "/projects");

    // The table drops low-priority columns instead of scrolling sideways.
    const table = page.getByRole("main").getByRole("table", { name: /./ }).first();
    await expect(table).toBeVisible();
    const tableScroller = page.getByRole("main").locator("[data-slot=table-container]").first().locator("xpath=..");
    expect(await sidewaysScrollers(tableScroller)).toEqual([]);

    // The drawer holds the form; provider and model sit side by side without overlapping.
    await page.goto(`/#/projects/${project.id}`);
    const drawer = page.getByRole("dialog");
    await expect(drawer).toBeVisible();
    const provider = drawer.getByRole("combobox", { name: "Preferred provider" });
    const model = drawer.getByRole("combobox", { name: "Preferred model" });
    await provider.scrollIntoViewIfNeeded();
    const a = (await provider.boundingBox())!;
    const b = (await model.boundingBox())!;
    const overlaps = a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
    expect(overlaps).toBe(false);
    expect(await sidewaysScrollers(drawer)).toEqual([]);
    await noPageOverflow(page);
  });
}

test("the Concise / Technical toggle keeps its two labels apart", async ({ page }) => {
  await openApp(page, "/activity");
  const group = page.getByLabel("Concise / Technical");
  await expect(group).toBeVisible();
  const concise = (await group.getByRole("radio", { name: "Concise" }).boundingBox())!;
  const technical = (await group.getByRole("radio", { name: "Technical" }).boundingBox())!;
  expect(concise.x + concise.width).toBeLessThanOrEqual(technical.x + 0.5);
});
